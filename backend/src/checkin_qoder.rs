//! Qoder 国内、国际版每日 Credits 活动；仅处理有官方依据的每日奖励。
use crate::checkin_types::{Outcome, State};
use crate::qoder_desktop::Session;
use serde_json::Value;
use std::{io::Read, time::Duration};

const CAMPAIGNS: &str = "/sash/api/v1/me/campaigns";
const MAX_BYTES: u64 = 1024 * 1024;

pub(crate) struct Account {
    edition: &'static str,
    session: Session,
    identity: String,
    native: bool,
}

impl Account {
    pub(crate) fn from_native(edition: &str) -> Result<Self, Outcome> {
        api_base(edition)?;
        let session = crate::qoder_desktop::local_session(edition).map_err(quota_error)?;
        let mut account = Self::from_browser(edition, &session)?;
        account.native = true;
        Ok(account)
    }
    pub(crate) fn from_browser(edition: &str, session: &Session) -> Result<Self, Outcome> {
        api_base(edition)?;
        validate_session(session)?;
        Ok(Self {
            edition: if edition == "cn" { "cn" } else { "global" },
            identity: session.user_id.as_deref().unwrap_or_default().trim().into(),
            session: session.clone(),
            native: false,
        })
    }
    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }
    pub(crate) fn session_account(&self) -> Option<&str> { self.session.account.as_deref() }

    pub(crate) fn status(&self) -> Outcome {
        match self.daily_campaign() {
            Ok((_, observed)) => daily_outcome(State::Ready, "本轮 100 Credits 可以领取", observed),
            Err(outcome) => outcome,
        }
    }

    /// 再次查询并严格筛选本轮每日活动；领取只发一次，不自动重试。
    pub(crate) fn claim(&self, before_post: impl FnOnce() -> Result<(), Outcome>) -> Outcome {
        let (id, observed) = match self.daily_campaign() {
            Ok(campaign) => campaign,
            Err(outcome) => return outcome,
        };
        let (status, body, response_time) =
            match self.request(&format!("{CAMPAIGNS}/{id}/claim"), true, before_post) {
                Ok(response) => response,
                Err(outcome) => return outcome,
            };
        claim_response(status, &body, observed, response_time)
    }

    fn daily_campaign(&self) -> Result<(String, i64), Outcome> {
        let (status, value, observed) = self.request(CAMPAIGNS, false, || Ok(()))?;
        campaign_response(status, &value, self.edition, observed)
    }

    fn ensure_current(&self) -> Result<(), Outcome> {
        validate_session(&self.session)?;
        if self.native {
            let current = crate::qoder_desktop::local_session(self.edition).map_err(quota_error)?;
            if current.token != self.session.token || current.user_id != self.session.user_id {
                return Err(Outcome::new(
                    State::Unauthorized,
                    "Qoder 本机登录已变化，请刷新后重试",
                ));
            }
        }
        Ok(())
    }

    fn request(
        &self,
        path: &str,
        claim: bool,
        before_send: impl FnOnce() -> Result<(), Outcome>,
    ) -> Result<(u16, Value, Option<i64>), Outcome> {
        self.ensure_current()?;
        let client = crate::net::blocking_builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| Outcome::new(State::Failed, "无法创建 Qoder 签到客户端"))?;
        let url = format!("{}{path}", api_base(self.edition)?);
        let request = if claim {
            client.post(url)
        } else {
            client.get(url)
        };
        let request = request
            .bearer_auth(&self.session.token)
            .header("Accept", "application/json")
            .header("Cosy-ClientType", "10")
            .header("User-Agent", "Qoder");
        // 活动预检 GET 已结束、请求已准备完毕；在真正发送前复查连接状态及身份。
        let response = guarded_send(before_send, || {
            request.send().map_err(|_| network_error(claim))
        })?;
        let status = response.status().as_u16();
        let observed = server_date(response.headers());
        if !(200..300).contains(&status) {
            return Ok((status, Value::Null, observed));
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| network_error(claim))?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(Outcome::new(State::Failed, "Qoder 活动响应超过大小限制"));
        }
        let value = serde_json::from_slice(&bytes).map_err(|_| {
            Outcome::new(
                if claim { State::Unknown } else { State::Failed },
                "Qoder 活动响应无法识别，请刷新确认领取状态",
            )
        })?;
        Ok((status, value, observed))
    }
}

fn server_date(headers: &reqwest::header::HeaderMap) -> Option<i64> {
    let mut values = headers.get_all(reqwest::header::DATE).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    (date.offset().local_minus_utc() == 0 && date.timestamp() > 0).then_some(date.timestamp())
}

fn campaign_response(
    status: u16,
    value: &Value,
    edition: &str,
    observed: Option<i64>,
) -> Result<(String, i64), Outcome> {
    if !(200..300).contains(&status) {
        return Err(http_error(status));
    }
    let observed = observed.ok_or_else(|| {
        Outcome::new(
            State::Unknown,
            "Qoder 未返回可靠的服务端时间，请刷新确认本轮状态",
        )
    })?;
    parse_status(value, edition, observed)
        .map(|id| (id, observed))
        .map_err(|mut outcome| {
            outcome.server_time = Some(observed);
            outcome
        })
}

fn guarded_send<T>(
    before_post: impl FnOnce() -> Result<(), Outcome>,
    send: impl FnOnce() -> Result<T, Outcome>,
) -> Result<T, Outcome> {
    before_post()?;
    send()
}

fn claim_response(
    status: u16,
    value: &Value,
    preflight_time: i64,
    observed: Option<i64>,
) -> Outcome {
    if !(200..300).contains(&status) {
        return http_error(status);
    }
    let Some(observed) = observed else {
        return Outcome::new(
            State::Unknown,
            "Qoder 未返回可靠的服务端时间，请刷新确认领取结果",
        );
    };
    let mut outcome = claim_result(status, value);
    outcome.server_time = Some(observed);
    if outcome.state == State::Claimed {
        if window(preflight_time).0 != window(observed).0 {
            let mut pending = Outcome::new(
                State::Unknown,
                "Qoder 领取响应跨越每日窗口，请刷新确认所属轮次",
            );
            pending.server_time = Some(observed);
            return pending;
        }
        return daily_outcome(State::Claimed, "本轮 100 Credits 已领取", observed);
    }
    outcome
}

fn api_base(edition: &str) -> Result<&'static str, Outcome> {
    match edition {
        "cn" => Ok("https://openapi.qoder.com.cn"),
        "global" => Ok("https://openapi.qoder.sh"),
        _ => Err(Outcome::new(State::Unsupported, "未知 Qoder 版本")),
    }
}

fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn validate_session(session: &Session) -> Result<(), Outcome> {
    let identity = session.user_id.as_deref().unwrap_or_default();
    let valid_expiry = session.expires_at.as_deref().is_none_or(|value| {
        chrono::DateTime::parse_from_rfc3339(value).is_ok_and(|date| date > chrono::Utc::now())
    });
    if !valid_text(&session.token, 65536) || !valid_text(identity, 320) || !valid_expiry {
        return Err(Outcome::new(
            State::Unauthorized,
            "Qoder 登录不可用，请在原工具重新登录或重新授权",
        ));
    }
    Ok(())
}

fn quota_error(error: crate::quota::QuotaState) -> Outcome {
    use crate::quota::QuotaState;
    let (state, message) = match error {
        QuotaState::Unauthorized { reason } => (State::Unauthorized, reason),
        QuotaState::Unsupported { reason } => (State::Unsupported, reason),
        QuotaState::Forbidden { reason } => (State::Forbidden, reason),
        QuotaState::RateLimited { reason } => (State::RateLimited, reason),
        _ => (State::Failed, "读取 Qoder 本机登录失败".into()),
    };
    Outcome::new(state, message)
}

fn network_error(claim: bool) -> Outcome {
    if claim {
        Outcome::new(
            State::Unknown,
            "Qoder 领取结果尚未确认，请刷新状态；未自动重试",
        )
    } else {
        Outcome::new(State::Failed, "无法读取 Qoder 每日活动，请检查网络与代理")
    }
}

fn http_error(status: u16) -> Outcome {
    let (state, message) = match status {
        401 => (State::Unauthorized, "Qoder 登录已失效，请重新登录或授权"),
        403 => (State::Forbidden, "当前 Qoder 账号无权参与活动"),
        409 => (State::Inactive, "Qoder 活动已结束或当前账号不符合领取条件"),
        429 | 503 => (State::RateLimited, "Qoder 活动繁忙，请稍后刷新"),
        _ => (State::Failed, "Qoder 活动服务返回异常，请稍后刷新"),
    };
    Outcome::new(state, message)
}

fn window(now: i64) -> (String, i64) {
    // UTC+8 的 10:00 即 UTC 02:00；先减两小时再按 UTC 日期划日。
    let shifted = now.saturating_sub(7200);
    let day = chrono::DateTime::from_timestamp(shifted, 0)
        .map(|date| date.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    (
        day,
        (shifted.div_euclid(86400) + 1)
            .saturating_mul(86400)
            .saturating_add(7200),
    )
}

fn daily_outcome(state: State, message: &str, now: i64) -> Outcome {
    let mut outcome = Outcome::new(state, message);
    outcome.unit = "Credits".into();
    if matches!(state, State::Ready | State::Claimed) {
        outcome.reward = Some(100.0);
    }
    let (day, reset) = window(now);
    outcome.period_key = Some(day);
    outcome.reset_at = Some(reset);
    outcome.server_time = Some(now);
    outcome.official_statistics = Some(crate::checkin_types::OfficialCheckinStatistics {
        unit: "Credits".into(), scope: "本轮".into(), week: None, month: None, all: None,
        note: "当前官方活动接口仅提供本轮状态；周、月与累计按 Jolu 已记录的领取统计。".into(),
    });
    outcome
}

fn claim_result(status: u16, value: &Value) -> Outcome {
    if !(200..300).contains(&status) {
        return http_error(status);
    }
    let value = value
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(value);
    match value.get("status").and_then(Value::as_str) {
        Some("CLAIMED") => Outcome::new(State::Claimed, "Qoder 每日奖励已领取"),
        Some("PENDING") => Outcome::new(State::Unknown, "Qoder 领取尚未完成，请刷新状态"),
        _ => Outcome::new(State::Failed, "Qoder 未确认每日奖励领取成功，请刷新状态"),
    }
}

fn official_daily_url(value: &Value, edition: &str) -> bool {
    let Some(raw) = value.as_str() else {
        return false;
    };
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    let host = if edition == "cn" {
        "docs.qoder.cn"
    } else {
        "docs.qoder.com"
    };
    url.scheme() == "https"
        && url.host_str() == Some(host)
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.path(),
            "/events/100credits" | "/en/events/100credits" | "/zh/events/100credits"
        )
}

fn parse_status(value: &Value, edition: &str, now: i64) -> Result<String, Outcome> {
    api_base(edition)?;
    if value.get("showCampaign").and_then(Value::as_bool) == Some(false) {
        return Err(daily_outcome(
            State::Inactive,
            "Qoder 当前未开放每日活动",
            now,
        ));
    }
    let array = value
        .get("campaigns")
        .or_else(|| value.pointer("/data/campaigns"))
        .or_else(|| value.get("data"))
        .unwrap_or(value)
        .as_array()
        .ok_or_else(|| Outcome::new(State::Failed, "Qoder 未返回可识别的活动列表"))?;
    let daily: Vec<_> = array
        .iter()
        .filter(|campaign| {
            campaign
                .get("placements")
                .and_then(Value::as_array)
                .is_some_and(|placements| {
                    placements.iter().any(|placement| {
                        ["zh", "en"].iter().any(|language| {
                            placement
                                .get("content")
                                .and_then(|content| content.get(language))
                                .and_then(|content| content.get("detailUrl"))
                                .is_some_and(|value| official_daily_url(value, edition))
                        })
                    })
                })
        })
        .collect();
    if daily.is_empty() {
        return Err(daily_outcome(
            State::Inactive,
            "Qoder 当前未提供可领取的每日活动",
            now,
        ));
    }
    let unknown = || {
        Outcome::new(
            State::Unknown,
            "Qoder 每日活动资料不完整或存在多个活动，请在原工具核实",
        )
    };
    if daily.len() != 1 {
        return Err(unknown());
    }
    let campaign = daily[0];
    let valid_benefit = campaign["actionType"] == "CLAIM_BENEFIT"
        && campaign["benefit"]["kind"] == "CREDITS"
        && campaign["benefit"]["amount"].as_f64() == Some(100.0)
        && campaign
            .pointer("/benefit/modelScope/modelSeries/key")
            .and_then(Value::as_str)
            == Some("ALL_MODELS")
        && campaign
            .pointer("/benefit/validity/mode")
            .and_then(Value::as_str)
            == Some("RELATIVE_DAYS")
        && campaign
            .pointer("/benefit/validity/days")
            .and_then(Value::as_i64)
            == Some(30);
    let id = campaign["campaignId"]
        .as_str()
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        })
        .ok_or_else(unknown)?;
    let start = campaign["startAt"]
        .as_i64()
        .filter(|value| *value > 0)
        .ok_or_else(unknown)?;
    let end = campaign["endAt"]
        .as_i64()
        .filter(|value| *value > start)
        .ok_or_else(unknown)?;
    if !valid_benefit {
        return Err(unknown());
    }
    if now < start || now >= end {
        return Err(daily_outcome(
            State::Inactive,
            "Qoder 当前每日活动尚未开始或已结束",
            now,
        ));
    }
    match campaign["claimStatus"].as_str() {
        Some("CLAIMABLE") => Ok(id.into()),
        Some("CLAIMED") => Err(daily_outcome(
            State::Claimed,
            "本轮 100 Credits 已领取",
            now,
        )),
        Some("INELIGIBLE" | "UNAVAILABLE" | "EXPIRED") => Err(daily_outcome(
            State::Inactive,
            "当前 Qoder 账号暂不可领取每日奖励",
            now,
        )),
        _ => Err(unknown()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    #[ignore = "鼠鼠已授权：仅通过真实 Account.status 发起只读 GET，不领取、不输出账号或凭证"]
    fn native_status_read_only_smoke() {
        for edition in ["cn", "global"] {
            match Account::from_native(edition) {
                Ok(account) => {
                    let result = account.status();
                    println!(
                        "Qoder {edition}: state={:?} reward={:?} period={:?} reset={:?}",
                        result.state, result.reward, result.period_key, result.reset_at
                    );
                    assert!(
                        matches!(
                            result.state,
                            State::Ready | State::Claimed | State::Inactive
                        ),
                        "{edition}: {:?}",
                        result.state
                    );
                }
                Err(result) => println!("Qoder {edition}: account_state={:?}", result.state),
            }
        }
    }

    fn daily(status: &str) -> Value {
        json!({"campaignId":"synthetic-daily", "campaignKey":"synthetic-campaign", "actionType":"CLAIM_BENEFIT",
            "claimStatus":status,"startAt":1792980000,"endAt":1793066340,
            "benefit":{"amount":100,"kind":"CREDITS","modelScope":{"modelSeries":{"key":"ALL_MODELS"}},
                "validity":{"days":30,"mode":"RELATIVE_DAYS"}},
            "placements":[{"type":"POPUP","content":{"zh":{"detailUrl":"https://docs.qoder.cn/events/100credits","title":"每天领 100 Credits"}}}]})
    }

    #[test]
    fn official_http_date_is_required_and_never_replaced_by_local_time() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::DATE,
            "Tue, 27 Oct 2026 02:00:00 GMT".parse().unwrap(),
        );
        assert_eq!(server_date(&headers), Some(1793066400));
        headers.insert(reqwest::header::DATE, "invalid-date".parse().unwrap());
        assert_eq!(server_date(&headers), None);
        headers.clear();
        assert_eq!(server_date(&headers), None);
    }

    #[test]
    fn server_time_drives_campaign_window_and_missing_date_stays_unknown() {
        let mut campaign = daily("CLAIMABLE");
        campaign["startAt"] = json!(1793066400);
        campaign["endAt"] = json!(1793152740);
        let value = json!({"campaigns":[campaign]});
        assert_eq!(
            campaign_response(200, &value, "cn", Some(1793066400)).unwrap(),
            ("synthetic-daily".into(), 1793066400)
        );
        let missing = campaign_response(200, &value, "cn", None).unwrap_err();
        assert_eq!(missing.state, State::Unknown);
        assert!(missing.period_key.is_none() && missing.reset_at.is_none());
        let result = daily_outcome(State::Ready, "ready", 1793066400);
        assert_eq!(result.server_time, Some(1793066400));
        assert_eq!(result.period_key.as_deref(), Some("2026-10-27"));
    }

    #[test]
    fn failed_final_connection_guard_prevents_any_send() {
        let sent = std::cell::Cell::new(0);
        let result = guarded_send(
            || Err(Outcome::new(State::Unauthorized, "connection changed")),
            || {
                sent.set(sent.get() + 1);
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err().state, State::Unauthorized);
        assert_eq!(sent.get(), 0);
        guarded_send(
            || Ok(()),
            || {
                sent.set(sent.get() + 1);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(sent.get(), 1);
    }

    #[test]
    fn claim_uses_post_date_and_leaves_cross_window_results_unconfirmed() {
        let body = json!({"status":"CLAIMED"});
        let result = claim_response(200, &body, 1793066400, Some(1793066410));
        assert_eq!(result.state, State::Claimed);
        assert_eq!(result.server_time, Some(1793066410));
        assert_eq!(
            claim_response(200, &body, 1793066400, None).state,
            State::Unknown
        );
        let boundary = claim_response(200, &body, 1793066399, Some(1793066400));
        assert_eq!(boundary.state, State::Unknown);
        assert!(boundary.period_key.is_none());
    }

    #[test]
    fn only_verified_daily_credit_campaign_is_claimable() {
        let now = 1793030000;
        assert_eq!(
            parse_status(&json!({"campaigns":[daily("CLAIMABLE")]}), "cn", now).unwrap(),
            "synthetic-daily"
        );
        assert_eq!(
            parse_status(&json!({"data":{"campaigns":[daily("CLAIMED")]}}), "cn", now)
                .unwrap_err()
                .state,
            State::Claimed
        );
        let mut campaign = daily("CLAIMABLE");
        campaign["placements"][0]["content"]["zh"]["detailUrl"] =
            json!("https://docs.qoder.com/zh/events/100credits");
        assert!(parse_status(&json!([campaign.clone()]), "global", now).is_ok());
        assert_eq!(
            parse_status(&json!([campaign]), "cn", now)
                .unwrap_err()
                .state,
            State::Inactive
        );
    }

    #[test]
    fn unrelated_or_unknown_activities_are_never_selected_for_claim() {
        for url in [
            "https://docs.qoder.cn/events/bogo",
            "https://docs.qoder.cn.evil.test/events/100credits",
            "http://docs.qoder.cn/events/100credits",
            "https://other.test/events/100credits",
        ] {
            let mut campaign = daily("CLAIMABLE");
            campaign["placements"][0]["content"]["zh"]["detailUrl"] = json!(url);
            assert_eq!(
                parse_status(&json!({"campaigns":[campaign]}), "cn", 1793030000)
                    .unwrap_err()
                    .state,
                State::Inactive
            );
        }
    }

    #[test]
    fn server_disabled_campaigns_are_not_claimable_even_with_ready_entries() {
        assert_eq!(
            parse_status(
                &json!({"showCampaign":false,"campaigns":[daily("CLAIMABLE")]}),
                "cn",
                1793030000
            )
            .unwrap_err()
            .state,
            State::Inactive
        );
    }

    #[test]
    fn incomplete_or_ambiguous_daily_activity_is_unknown_not_ready_or_claimed() {
        for pointer in [
            "/benefit/modelScope",
            "/benefit/validity",
            "/startAt",
            "/endAt",
            "/claimStatus",
            "/campaignId",
        ] {
            let mut campaign = daily("CLAIMABLE");
            *campaign.pointer_mut(pointer).unwrap() = Value::Null;
            assert_eq!(
                parse_status(&json!({"campaigns":[campaign]}), "cn", 1793030000)
                    .unwrap_err()
                    .state,
                State::Unknown,
                "{pointer}"
            );
        }
        assert_eq!(
            parse_status(
                &json!({"campaigns":[daily("CLAIMABLE"),daily("CLAIMABLE")]}),
                "cn",
                1793030000
            )
            .unwrap_err()
            .state,
            State::Unknown
        );
        assert_eq!(
            parse_status(&json!({}), "cn", 1793030000)
                .unwrap_err()
                .state,
            State::Failed
        );
        assert_eq!(
            parse_status(&json!({"campaigns":[]}), "cn", 1793030000)
                .unwrap_err()
                .state,
            State::Inactive
        );
        assert_eq!(
            parse_status(&json!({"campaigns":[daily("CLAIMABLE")]}), "cn", 1793066400)
                .unwrap_err()
                .state,
            State::Inactive
        );
    }

    #[test]
    fn session_requires_valid_region_identity_token_and_unexpired_expiry() {
        let session = Session {
            token: "synthetic-token".into(),
            refresh_token: None,
            user_id: Some("synthetic-id".into()),
            account: None,
            expires_at: Some("2099-01-01T00:00:00Z".into()),
        };
        assert_eq!(
            Account::from_browser("cn", &session).unwrap().identity(),
            "synthetic-id"
        );
        assert!(Account::from_browser("unknown", &session).is_err());
        let mut expired = session.clone();
        expired.expires_at = Some("2000-01-01T00:00:00Z".into());
        assert!(Account::from_browser("cn", &expired).is_err());
        let mut missing = session.clone();
        missing.user_id = None;
        assert!(Account::from_browser("cn", &missing).is_err());
    }

    #[test]
    fn daily_window_changes_at_utc_plus_eight_ten_oclock() {
        // 2026-10-27 09:59:59 / 10:00:00 UTC+8。
        assert_eq!(window(1_793_066_399), ("2026-10-26".into(), 1_793_066_400));
        assert_eq!(window(1_793_066_400), ("2026-10-27".into(), 1_793_152_800));
    }

    #[test]
    fn claim_requires_claimed_body_and_never_treats_409_as_duplicate_success() {
        assert_eq!(
            claim_result(200, &json!({"status":"CLAIMED"})).state,
            State::Claimed
        );
        assert_eq!(
            claim_result(200, &json!({"data":{"status":"CLAIMED"}})).state,
            State::Claimed
        );
        assert_eq!(
            claim_result(200, &json!({"status":"PENDING"})).state,
            State::Unknown
        );
        assert_eq!(claim_result(200, &json!({})).state, State::Failed);
        assert_eq!(
            claim_result(409, &json!({"status":"CLAIMED"})).state,
            State::Inactive
        );
        assert_eq!(claim_result(401, &json!({})).state, State::Unauthorized);
        assert_eq!(claim_result(403, &json!({})).state, State::Forbidden);
        assert_eq!(claim_result(429, &json!({})).state, State::RateLimited);
    }
}
