//! WorkBuddy 国内版每日签到；国际版使用自动活跃奖励，不提供手动签到。
use crate::checkin_types::{OfficialCheckinStatistics, OfficialCheckinTotals, Outcome, State};
use chrono::{DateTime, Datelike, Days, NaiveDate};
use serde_json::{json, Value};
use std::{collections::BTreeSet, io::Read, time::Duration};

const BASE: &str = "https://www.workbuddy.cn";
const BODY_LIMIT: u64 = 1_048_576;

/// 凭证仅在后端内存中持有，不实现 Debug 或序列化。
pub(crate) struct Account {
    token: String,
    headers: Vec<(&'static str, String)>,
    identity: String,
    label: Option<String>,
}

impl Account {
    pub(crate) fn from_native(edition: &str) -> Result<Self, Outcome> {
        require_cn(edition)?;
        let credentials =
            crate::workbuddy_quota::native_checkin_credentials().map_err(from_quota_error)?;
        let mut account = Self::new(credentials.token, credentials.headers)?;
        account.label = credentials.account;
        Ok(account)
    }

    pub(crate) fn from_browser(
        edition: &str,
        session: &crate::workbuddy_browser_auth::Session,
    ) -> Result<Self, Outcome> {
        require_cn(edition)?;
        if session
            .expires_at
            .is_some_and(|at| at <= chrono::Utc::now().timestamp_millis())
        {
            return Err(Outcome::new(
                State::Unauthorized,
                "WorkBuddy 浏览器授权已过期，请重新授权",
            ));
        }
        if !matches!(
            session.domain.to_ascii_lowercase().as_str(),
            "workbuddy.cn"
                | "www.workbuddy.cn"
                | "codebuddy.cn"
                | "www.codebuddy.cn"
                | "copilot.tencent.com"
                | "wb.tencentbuddy.com"
        ) {
            return Err(Outcome::new(
                State::Unauthorized,
                "WorkBuddy 授权区域与国内签到不匹配",
            ));
        }
        let mut headers = vec![
            ("X-Domain", session.domain.clone()),
            ("X-User-Id", session.user_id.clone()),
        ];
        if let Some(enterprise) = &session.enterprise_id {
            headers.push(("X-Enterprise-Id", enterprise.clone()));
        }
        let mut account = Self::new(session.token.clone(), headers)?;
        account.label = session.email.clone();
        Ok(account)
    }

    fn new(token: String, headers: Vec<(&'static str, String)>) -> Result<Self, Outcome> {
        let identity = headers
            .iter()
            .find(|(name, _)| *name == "X-User-Id")
            .map(|(_, value)| value.as_str())
            .filter(|value| {
                !value.trim().is_empty()
                    && value.len() <= 1024
                    && !value.chars().any(char::is_control)
            })
            .ok_or_else(|| {
                Outcome::new(
                    State::Unauthorized,
                    "WorkBuddy 未返回有效的账号身份，请重新授权",
                )
            })?
            .to_string();
        if token.is_empty()
            || token.len() > 32768
            || !token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._~+/-=".contains(&byte))
        {
            return Err(Outcome::new(
                State::Unauthorized,
                "WorkBuddy 登录凭证无效，请重新授权",
            ));
        }
        if headers
            .iter()
            .any(|(name, value)| *name == "X-Enterprise-Id" && !value.is_empty())
        {
            return Err(Outcome::new(
                State::Forbidden,
                "WorkBuddy 每日签到仅对符合活动条件的个人账号开放",
            ));
        }
        Ok(Self {
            token,
            headers,
            identity,
            label: None,
        })
    }

    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }
    pub(crate) fn account_label(&self) -> Option<&str> {
        self.label
            .as_deref()
            .filter(|label| !label.chars().any(char::is_control))
            .map(str::trim)
            .filter(|label| {
                !label.is_empty()
                    && label.len() <= 320
                    && label.contains('@')
                    && !label.chars().any(char::is_whitespace)
            })
    }
    pub(crate) fn status(&self) -> Outcome {
        self.request(false, || Ok(()))
    }
    /// 公共服务先预检；本方法只发起一次领取，不自动重试。
    pub(crate) fn claim(&self, before_post: impl FnOnce() -> Result<(), Outcome>) -> Outcome {
        self.request(true, before_post)
    }

    fn request(&self, claim: bool, before_post: impl FnOnce() -> Result<(), Outcome>) -> Outcome {
        if crate::validation::isolated() {
            return Outcome::new(State::Unauthorized, "隔离环境不访问 WorkBuddy 真实签到接口");
        }
        let client = match crate::net::blocking_builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
        {
            Ok(client) => client,
            Err(_) => return Outcome::new(State::Failed, "无法创建 WorkBuddy 签到网络客户端"),
        };
        let path = if claim {
            "/v2/billing/meter/daily-checkin"
        } else {
            "/v2/billing/meter/checkin-activity-status"
        };
        let mut request = client
            .post(format!("{BASE}{path}"))
            .bearer_auth(&self.token)
            .header("Accept", "application/json")
            .json(&json!({}));
        for (name, value) in &self.headers {
            request = request.header(*name, value);
        }
        if claim {
            if let Err(outcome) = before_post() {
                return outcome;
            }
        }
        // 官方 provider 缺省时同样不带风控头；不仿造或复用设备风控令牌。
        let response = match request.send() {
            Ok(response) => response,
            Err(_) => return network_failure(claim),
        };
        let server_time = response
            .headers()
            .get(reqwest::header::DATE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| DateTime::parse_from_rfc2822(value).ok())
            .map(|value| value.timestamp());
        let status = response.status().as_u16();
        if status != 200 {
            return parse_response(status, &Value::Null, claim, server_time);
        }
        let mut bytes = Vec::new();
        if response
            .take(BODY_LIMIT + 1)
            .read_to_end(&mut bytes)
            .is_err()
        {
            return network_failure(claim);
        }
        if bytes.len() as u64 > BODY_LIMIT {
            return Outcome::new(
                State::Failed,
                "WorkBuddy 签到响应过大，请在官方客户端核对结果",
            );
        }
        let mut outcome = match serde_json::from_slice(&bytes) {
            Ok(body) => parse_response(status, &body, claim, server_time),
            Err(_) => Outcome::new(
                State::Failed,
                "WorkBuddy 签到响应格式无效，请刷新状态核对结果",
            ),
        };
        outcome.server_time = server_time;
        outcome
    }
}

fn require_cn(edition: &str) -> Result<(), Outcome> {
    match edition {
        "cn" => Ok(()),
        "ai" => Err(Outcome::new(
            State::Unsupported,
            "WorkBuddy 国际版按有效客户端任务自动发放活跃奖励，无手动签到",
        )),
        _ => Err(Outcome::new(State::Unsupported, "未知 WorkBuddy 签到版本")),
    }
}

fn from_quota_error(error: crate::quota::QuotaState) -> Outcome {
    use crate::quota::QuotaState;
    let (state, message) = match error {
        QuotaState::Unauthorized { .. } => (
            State::Unauthorized,
            "WorkBuddy 国内版登录不可用，请在原工具登录或重新授权",
        ),
        QuotaState::Unsupported { .. } => (
            State::Unsupported,
            "无法读取当前 WorkBuddy 官方客户端的登录格式",
        ),
        QuotaState::Forbidden { .. } => (State::Forbidden, "当前 WorkBuddy 账号不能参与签到"),
        QuotaState::RateLimited { .. } => {
            (State::RateLimited, "WorkBuddy 请求过于频繁，请稍后重试")
        }
        _ => (
            State::Failed,
            "读取 WorkBuddy 国内版登录失败，请在原工具检查登录状态",
        ),
    };
    Outcome::new(state, message)
}

fn network_failure(claim: bool) -> Outcome {
    Outcome::new(
        State::Failed,
        if claim {
            "WorkBuddy 签到请求未能确认结果，请先刷新状态核对，请勿连续领取"
        } else {
            "WorkBuddy 签到状态查询失败，请检查网络后重试"
        },
    )
}

fn parse_response(
    http_status: u16,
    body: &Value,
    claim: bool,
    server_time: Option<i64>,
) -> Outcome {
    match http_status {
        200 => {}
        401 => return Outcome::new(State::Unauthorized, "WorkBuddy 登录已失效，请重新授权"),
        403 => {
            return Outcome::new(
                State::Forbidden,
                "WorkBuddy 拒绝了请求，请到官方客户端核对活动或完成验证",
            )
        }
        429 => return Outcome::new(State::RateLimited, "WorkBuddy 请求过于频繁，请稍后重试"),
        _ => {
            return Outcome::new(
                State::Failed,
                "WorkBuddy 签到接口暂不可用，请在官方客户端核对结果",
            )
        }
    }
    match body.get("code").and_then(Value::as_i64) {
        Some(0) => {}
        Some(1001) => return Outcome::new(State::Claimed, "WorkBuddy 今日已签到"),
        Some(1002) => {
            return Outcome::new(State::Forbidden, "当前 WorkBuddy 账号不符合签到活动资格")
        }
        Some(1003) => return Outcome::new(State::Inactive, "WorkBuddy 本期签到活动已结束"),
        _ => {
            return Outcome::new(
                State::Failed,
                "WorkBuddy 未能确认签到状态，请到官方客户端核对活动或完成验证",
            )
        }
    }
    let Some(data) = body.get("data").filter(|data| data.is_object()) else {
        return Outcome::new(State::Failed, "WorkBuddy 签到响应缺少有效数据");
    };
    let mut outcome = if claim {
        Outcome::new(State::Claimed, "WorkBuddy 签到成功")
    } else {
        match (
            data.get("active").and_then(Value::as_bool),
            data.get("today_checked_in").and_then(Value::as_bool),
        ) {
            (Some(false), _) => Outcome::new(State::Inactive, "WorkBuddy 当前没有开放的签到活动"),
            (Some(true), Some(true)) => Outcome::new(State::Claimed, "WorkBuddy 今日已签到"),
            (Some(true), Some(false)) => Outcome::new(State::Ready, "WorkBuddy 今日可签到"),
            _ => return Outcome::new(State::Failed, "WorkBuddy 未返回完整的签到活动状态"),
        }
    };
    outcome.reward = data
        .get(if claim { "credit" } else { "today_credit" })
        .and_then(Value::as_f64)
        .filter(|reward| reward.is_finite() && *reward >= 0.0);
    outcome.server_time = server_time;
    if !claim {
        outcome.official_statistics = Some(parse_official_statistics(data, server_time));
    }
    // 官方未给出服务端签到日时区；本机日期、活动 season 和结束时间均不是当日去重依据。
    outcome
}

fn parse_official_statistics(data: &Value, server_time: Option<i64>) -> OfficialCheckinStatistics {
    let mut all = OfficialCheckinTotals {
        days: None,
        reward: data
            .get("total_credits")
            .and_then(Value::as_f64)
            .filter(|reward| reward.is_finite() && *reward >= 0.0),
    };
    // 只有完整有效的日期集合才参与计数；不悄悄舍弃坏项后冒充已知历史。
    let dates = data
        .get("checkin_dates")
        .and_then(Value::as_array)
        .and_then(|dates| {
            dates
                .iter()
                .map(|value| {
                    let value = value.as_str()?;
                    if value.len() != 10 {
                        return None;
                    }
                    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
                    (date.format("%Y-%m-%d").to_string() == value).then_some(date)
                })
                .collect::<Option<BTreeSet<_>>>()
        });
    // 北京时间只定义展示统计窗口，不推断官方签到日边界或活动起止时区。
    let today = server_time
        .and_then(|time| time.checked_add(8 * 60 * 60))
        .and_then(|time| DateTime::from_timestamp(time, 0))
        .map(|time| time.date_naive());
    let mut week = None;
    let mut month = None;
    let mut note =
        "天数仅按官方返回日期统计；官方未提供逐日奖励或完整历史范围；累计奖励仅限本期活动"
            .to_string();
    match (dates, today) {
        (Some(dates), Some(today)) => {
            let dates: Vec<_> = dates.into_iter().filter(|date| *date <= today).collect();
            all.days = u32::try_from(dates.len()).ok();
            if let Some(start) =
                today.checked_sub_days(Days::new(today.weekday().num_days_from_monday().into()))
            {
                week = Some(OfficialCheckinTotals {
                    days: u32::try_from(dates.iter().filter(|date| **date >= start).count()).ok(),
                    reward: None,
                });
            }
            month = Some(OfficialCheckinTotals {
                days: u32::try_from(
                    dates
                        .iter()
                        .filter(|date| date.year() == today.year() && date.month() == today.month())
                        .count(),
                )
                .ok(),
                reward: None,
            });
        }
        (None, _) => note.push_str("；官方未返回有效日期集合，签到天数未知"),
        (_, None) => note.push_str("；缺少有效服务端日期，签到天数暂无法核对"),
    }
    OfficialCheckinStatistics {
        unit: "积分".into(),
        scope: "本期活动".into(),
        week,
        month,
        all: Some(all),
        note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_uses_server_activity_and_today_flag() {
        let ready = parse_response(
            200,
            &json!({"code":0,"data":{"active":true,"today_checked_in":false,"today_credit":100}}),
            false,
            None,
        );
        assert_eq!(ready.state, State::Ready);
        assert_eq!(ready.reward, Some(100.0));
        assert!(ready.period_key.is_none());
        assert!(ready.reset_at.is_none());
        assert_eq!(
            parse_response(
                200,
                &json!({"code":0,"data":{"active":true,"today_checked_in":true,"today_credit":0}}),
                false,
                None
            )
            .state,
            State::Claimed
        );
        assert_eq!(
            parse_response(200, &json!({"code":0,"data":{"active":false}}), false, None).state,
            State::Inactive
        );
    }

    #[test]
    fn malformed_status_never_becomes_available() {
        for body in [
            json!({"code":0,"data":{}}),
            json!({"code":0,"data":{"active":true}}),
            json!({"data":{"active":true,"today_checked_in":false}}),
            json!({"code":0,"data":{"active":"true","today_checked_in":false}}),
        ] {
            assert_eq!(parse_response(200, &body, false, None).state, State::Failed);
        }
    }

    #[test]
    fn claim_success_and_duplicate_are_idempotent() {
        let claimed = parse_response(
            200,
            &json!({"code":0,"data":{"credit":100,"streak_days":1,"is_streak_day":false}}),
            true,
            None,
        );
        assert_eq!(claimed.state, State::Claimed);
        assert_eq!(claimed.reward, Some(100.0));
        let duplicate = parse_response(
            200,
            &json!({"code":1001,"msg":"synthetic-private-server-message"}),
            true,
            None,
        );
        assert_eq!(duplicate.state, State::Claimed);
        assert!(duplicate.reward.is_none());
        assert!(!duplicate.message.contains("synthetic-private"));
    }

    #[test]
    fn unknown_rewards_are_not_reported_as_zero() {
        for credit in [Value::Null, json!("unknown"), json!(-1), json!({})] {
            let result =
                parse_response(200, &json!({"code":0,"data":{"credit":credit}}), true, None);
            assert_eq!(result.state, State::Claimed);
            assert!(result.reward.is_none());
        }
        assert_eq!(
            parse_response(200, &json!({"code":0,"data":{"credit":0}}), true, None).reward,
            Some(0.0)
        );
    }

    #[test]
    fn business_and_http_failures_remain_distinct() {
        for (status, body, expected) in [
            (200, json!({"code":1002}), State::Forbidden),
            (200, json!({"code":1003}), State::Inactive),
            (
                200,
                json!({"code":98765,"msg":"synthetic-private-token"}),
                State::Failed,
            ),
            (
                401,
                json!({"code":0,"data":{"credit":100}}),
                State::Unauthorized,
            ),
            (403, json!({}), State::Forbidden),
            (429, json!({}), State::RateLimited),
            (302, json!({}), State::Failed),
        ] {
            let result = parse_response(status, &body, true, None);
            assert_eq!(result.state, expected);
            assert!(!result.message.contains("synthetic-private"));
        }
        assert_eq!(
            parse_response(200, &json!({"code":0}), true, None).state,
            State::Failed
        );
    }

    // 若丢弃官方日期、按每日奖励估算汇总或把未来日期纳入，这些计数会失败。
    #[test]
    fn official_dates_count_returned_days_and_preserve_activity_reward() {
        let observed = chrono::DateTime::parse_from_rfc3339("2026-10-26T20:00:00Z")
            .unwrap()
            .timestamp();
        let result = parse_response(
            200,
            &json!({"code":0,"data":{
                "active":true,"today_checked_in":true,"today_credit":100,
                "daily_credit":100,"streak_days":99,"total_credits":731.5,
                "checkin_dates":["2026-09-30","2026-10-19","2026-10-25",
                    "2026-10-26","2026-10-27","2026-10-27","2026-10-28"]
            }}),
            false,
            Some(observed),
        );
        assert_eq!(result.state, State::Claimed);
        let stats = result.official_statistics.expect("应保留官方统计");
        assert_eq!(stats.scope, "本期活动");
        assert_eq!(stats.week.as_ref().unwrap().days, Some(2));
        assert_eq!(stats.month.as_ref().unwrap().days, Some(4));
        assert_eq!(stats.all.as_ref().unwrap().days, Some(5));
        assert_eq!(stats.all.as_ref().unwrap().reward, Some(731.5));
        assert!(stats.week.as_ref().unwrap().reward.is_none());
        assert!(stats.month.as_ref().unwrap().reward.is_none());
        assert!(stats.note.contains("官方返回日期"));
        assert!(stats.note.contains("完整历史范围"));
        assert!(result.period_key.is_none());
        assert!(result.reset_at.is_none());
    }

    #[test]
    fn missing_or_malformed_history_does_not_erase_claimed_or_make_up_days() {
        for dates in [
            Value::Null,
            json!("2026-10-27"),
            json!(["2026-10-27", "2026-02-30"]),
            json!(["2026-10-27", "2026-1-01"]),
            json!(["2026-10-27", 27]),
        ] {
            let result = parse_response(
                200,
                &json!({"code":0,"data":{"active":true,"today_checked_in":true,
                    "total_credits":0,"checkin_dates":dates}}),
                false,
                Some(1793059200),
            );
            assert_eq!(result.state, State::Claimed);
            let stats = result.official_statistics.expect("真实零累计仍应保留");
            assert_eq!(stats.all.as_ref().unwrap().reward, Some(0.0));
            assert!(stats.all.as_ref().unwrap().days.is_none());
            assert!(stats.week.is_none());
            assert!(stats.month.is_none());
        }
    }

    #[test]
    fn unknown_totals_and_missing_server_date_are_not_invented() {
        for total in [Value::Null, json!("100"), json!(-1)] {
            let result = parse_response(
                200,
                &json!({"code":0,"data":{"active":true,"today_checked_in":true,
                    "today_credit":100,"daily_credit":100,"streak_days":10,
                    "total_credits":total,"checkin_dates":["2026-10-27"]}}),
                false,
                None,
            );
            assert_eq!(result.state, State::Claimed);
            let stats = result
                .official_statistics
                .expect("缺失范围也应说明未知原因");
            assert!(stats.all.as_ref().unwrap().reward.is_none());
            assert!(stats.all.as_ref().unwrap().days.is_none());
            assert!(stats.week.is_none());
            assert!(stats.month.is_none());
        }
    }

    #[test]
    fn inactive_activity_keeps_official_statistics_and_empty_dates_are_real_zero() {
        let result = parse_response(
            200,
            &json!({"code":0,"data":{"active":false,"total_credits":0,
                "checkin_dates":[]}}),
            false,
            Some(1793059200),
        );
        assert_eq!(result.state, State::Inactive);
        let stats = result.official_statistics.expect("活动结束不丢已返回汇总");
        assert_eq!(stats.week.unwrap().days, Some(0));
        assert_eq!(stats.month.unwrap().days, Some(0));
        assert_eq!(stats.all.as_ref().unwrap().days, Some(0));
        assert_eq!(stats.all.unwrap().reward, Some(0.0));
    }

    #[test]
    fn browser_identity_uses_user_id_and_rejects_wrong_region() {
        let mut session = crate::workbuddy_browser_auth::Session {
            token: "synthetic-first-token".into(),
            domain: "www.workbuddy.cn".into(),
            user_id: "synthetic-stable-user".into(),
            enterprise_id: None,
            email: None,
            expires_at: None,
        };
        let first =
            Account::from_browser("cn", &session).unwrap_or_else(|_| panic!("合成账号应有效"));
        session.token = "synthetic-refreshed-token".into();
        let second =
            Account::from_browser("cn", &session).unwrap_or_else(|_| panic!("合成账号应有效"));
        assert_eq!(first.identity(), "synthetic-stable-user");
        assert_eq!(first.identity(), second.identity());
        assert!(matches!(
            Account::from_browser("ai", &session),
            Err(Outcome {
                state: State::Unsupported,
                ..
            })
        ));
        assert!(matches!(
            Account::from_native("ai"),
            Err(Outcome {
                state: State::Unsupported,
                ..
            })
        ));
        session.domain = "www.workbuddy.ai".into();
        assert!(matches!(
            Account::from_browser("cn", &session),
            Err(Outcome {
                state: State::Unauthorized,
                ..
            })
        ));
        session.domain = "www.workbuddy.cn".into();
        session.expires_at = Some(1);
        assert!(matches!(
            Account::from_browser("cn", &session),
            Err(Outcome {
                state: State::Unauthorized,
                ..
            })
        ));
    }

    #[test]
    fn account_label_is_bound_to_its_authorized_session_without_identity_fallback() {
        let mut session = crate::workbuddy_browser_auth::Session {
            token: "synthetic-first-token".into(),
            domain: "www.workbuddy.cn".into(),
            user_id: "synthetic-first-user".into(),
            enterprise_id: None,
            email: Some("first@example.invalid".into()),
            expires_at: None,
        };
        let first =
            Account::from_browser("cn", &session).unwrap_or_else(|_| panic!("合成账号应有效"));
        session.token = "synthetic-second-token".into();
        session.user_id = "synthetic-second-user".into();
        session.email = Some("second@example.invalid".into());
        let second =
            Account::from_browser("cn", &session).unwrap_or_else(|_| panic!("合成账号应有效"));
        assert_eq!(first.account_label(), Some("first@example.invalid"));
        assert_eq!(second.account_label(), Some("second@example.invalid"));
        assert_ne!(first.identity(), second.identity());
        for label in [
            None,
            Some(""),
            Some("Nickname"),
            Some("private\n@example.invalid"),
        ] {
            session.email = label.map(str::to_string);
            let account =
                Account::from_browser("cn", &session).unwrap_or_else(|_| panic!("合成账号应有效"));
            assert!(account.account_label().is_none());
        }
    }

    #[test]
    #[ignore = "需要本机 WorkBuddy 国内版登录和网络；仅显式执行，只读查询签到状态"]
    fn native_status_read_only_smoke() {
        let outcome = match Account::from_native("cn") {
            Ok(account) => account.status(),
            Err(outcome) => outcome,
        };
        let statistics = outcome.official_statistics.as_ref();
        println!(
            "state={:?} statistics_present={} week_days_known={} month_days_known={} activity_days_known={} activity_reward_known={} server_date_known={}",
            outcome.state,
            statistics.is_some(),
            statistics.and_then(|stats| stats.week.as_ref()).and_then(|total| total.days).is_some(),
            statistics.and_then(|stats| stats.month.as_ref()).and_then(|total| total.days).is_some(),
            statistics.and_then(|stats| stats.all.as_ref()).and_then(|total| total.days).is_some(),
            statistics.and_then(|stats| stats.all.as_ref()).and_then(|total| total.reward).is_some(),
            outcome.server_time.is_some(),
        );
        assert!(
            matches!(
                outcome.state,
                State::Ready | State::Claimed | State::Inactive
            ),
            "state={:?}",
            outcome.state
        );
        let statistics = statistics.expect("状态响应应携带有范围说明的官方统计");
        assert_eq!(statistics.scope, "本期活动");
        assert!(statistics
            .week
            .as_ref()
            .is_none_or(|total| total.reward.is_none()));
        assert!(statistics
            .month
            .as_ref()
            .is_none_or(|total| total.reward.is_none()));
        assert!(outcome.period_key.is_none());
        assert!(outcome.reset_at.is_none());
    }

    #[test]
    #[ignore = "真实领取最多一次；必须显式设置 JOLU_RUN_REAL_CHECKIN=1，且本机当前账号状态为 Ready"]
    fn native_claim_once() {
        assert_eq!(
            std::env::var("JOLU_RUN_REAL_CHECKIN").ok().as_deref(),
            Some("1"),
            "需要显式设置 JOLU_RUN_REAL_CHECKIN=1"
        );
        let account = match Account::from_native("cn") {
            Ok(account) => account,
            Err(outcome) => {
                println!(
                    "state={:?} reward={:?} claimed={}",
                    outcome.state,
                    outcome.reward,
                    outcome.state == State::Claimed
                );
                panic!("state={:?}", outcome.state);
            }
        };
        let identity = account.identity().to_string();
        let before = account.status();
        println!(
            "before state={:?} reward={:?} claimed={}",
            before.state,
            before.reward,
            before.state == State::Claimed
        );
        if before.state != State::Ready {
            return;
        }
        let claimed = account.claim(|| {
            let current = Account::from_native("cn")?;
            if current.identity() != identity {
                return Err(Outcome::new(
                    State::Unauthorized,
                    "WorkBuddy 当前账号已变化，本次未领取",
                ));
            }
            Ok(())
        });
        println!(
            "claim state={:?} reward={:?} claimed={}",
            claimed.state,
            claimed.reward,
            claimed.state == State::Claimed
        );
        let after = match Account::from_native("cn") {
            Ok(current) if current.identity() == identity => current.status(),
            Ok(_) => Outcome::new(
                State::Unauthorized,
                "WorkBuddy 当前账号已变化，无法确认原账号领取结果",
            ),
            Err(outcome) => outcome,
        };
        println!(
            "after state={:?} reward={:?} claimed={}",
            after.state,
            after.reward,
            after.state == State::Claimed
        );
        assert_eq!(claimed.state, State::Claimed, "state={:?}", claimed.state);
        assert_eq!(after.state, State::Claimed, "state={:?}", after.state);
    }
}
