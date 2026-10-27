//! Trae CN 每日积分签到；协议依据官方 3.4.4 安装包。
//! 只读当前登录与 AHA TinyStorage，禁用跨区域回退、设备伪造和领取重试。
use crate::checkin_types::{OfficialCheckinStatistics, Outcome, State};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

const HOST: &str = "https://api.trae.cn";
const MAX_BYTES: u64 = 1024 * 1024;

pub(crate) struct Account {
    user_id: String,
    token: String,
    account_label: Option<String>,
    device: String,
    uuid: String,
    root: PathBuf,
    native: bool,
    expires_at: Option<DateTime<Utc>>,
}

impl Account {
    pub(crate) fn from_native(edition: &str) -> Result<Self, Outcome> {
        require_cn(edition)?;
        let (root, auth) = crate::trae_quota::source_auth("native:trae:cn").map_err(quota_error)?;
        let (user_id, token, account_label) = account_fields(&auth)?;
        let uuid = hardware_uuid()?;
        let device = read_device(&root, &uuid)?;
        Ok(Self {
            user_id,
            token,
            account_label,
            device,
            uuid,
            root,
            native: true,
            expires_at: expiry(&auth["expiredAt"]),
        })
    }

    pub(crate) fn from_browser(
        edition: &str,
        session: &crate::trae_browser_auth::Session,
    ) -> Result<Self, Outcome> {
        require_cn(edition)?;
        if crate::validation::isolated() {
            return Err(Outcome::new(
                State::Unauthorized,
                "隔离环境不读取真实 Trae 设备",
            ));
        }
        // Session 仅由 marscode 的 ExchangeToken + /trae/GetUserInfo 生成；无 SaaS/企业入口。
        if session.host != HOST
            || !valid_label(&session.user_id, 320)
            || !valid_label(&session.token, 64 * 1024)
        {
            return Err(Outcome::new(
                State::Unauthorized,
                "Trae 国内版授权无效，请重新授权",
            ));
        }
        let expires_at = DateTime::parse_from_rfc3339(&session.expires_at)
            .ok()
            .map(|v| v.with_timezone(&Utc))
            .filter(|v| *v > Utc::now())
            .ok_or_else(|| Outcome::new(State::Unauthorized, "Trae 授权已过期，请重新授权"))?;
        let root = native_root()?;
        let uuid = hardware_uuid()?;
        let device = read_device(&root, &uuid)?;
        Ok(Self {
            user_id: session.user_id.clone(),
            token: session.token.clone(),
            account_label: normalized_account_label(session.account.as_deref()),
            device,
            uuid,
            root,
            native: false,
            expires_at: Some(expires_at),
        })
    }

    pub(crate) fn identity(&self) -> &str {
        &self.user_id
    }
    pub(crate) fn account_label(&self) -> Option<&str> {
        self.account_label.as_deref()
    }
    pub(crate) fn status(&self) -> Outcome {
        self.request(false, || Ok(()))
    }
    pub(crate) fn claim(&self, before_post: impl FnOnce() -> Result<(), Outcome>) -> Outcome {
        self.request(true, before_post)
    }

    fn ensure_current(&self) -> Result<(), Outcome> {
        if crate::validation::isolated() {
            return Err(Outcome::new(
                State::Unauthorized,
                "隔离环境不使用真实 Trae 签到",
            ));
        }
        if self.expires_at.is_some_and(|v| v <= Utc::now()) {
            return Err(Outcome::new(
                State::Unauthorized,
                "Trae 登录已过期，请在原工具登录或重新授权",
            ));
        }
        if self.native {
            let (_, current) =
                crate::trae_quota::source_auth("native:trae:cn").map_err(quota_error)?;
            let (id, token, _) = account_fields(&current)?;
            if id != self.user_id || token != self.token {
                return Err(Outcome::new(
                    State::Unauthorized,
                    "Trae 原工具登录发生变化，请刷新后重试",
                ));
            }
        }
        if read_device(&self.root, &self.uuid)? != self.device {
            return Err(Outcome::new(
                State::Failed,
                "Trae 设备身份发生变化，请刷新后重试",
            ));
        }
        Ok(())
    }

    fn request(&self, claim: bool, before_post: impl FnOnce() -> Result<(), Outcome>) -> Outcome {
        if let Err(error) = self.ensure_current() {
            return error;
        }
        let client = match crate::net::blocking_builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
        {
            Ok(client) => client,
            Err(_) => return Outcome::new(State::Failed, "无法创建 Trae 签到客户端"),
        };
        let path = if claim { "claim" } else { "status" };
        if claim {
            if let Err(outcome) = before_post() {
                return outcome;
            }
        }
        let response = match client
            .post(format!("{HOST}/trae/api/v2/ug/checkin_credits/{path}"))
            .header("Authorization", format!("Cloud-IDE-JWT {}", self.token))
            .header("x-device-id", &self.device)
            .json(&serde_json::json!({"req_source":1}))
            .send()
        {
            Ok(response) => response,
            Err(_) => {
                return Outcome::new(
                    State::Failed,
                    if claim {
                        "Trae 领取结果未确认，请刷新状态后再试"
                    } else {
                        "Trae 签到状态查询失败，请检查网络"
                    },
                )
            }
        };
        let server_time = response
            .headers()
            .get(reqwest::header::DATE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| DateTime::parse_from_rfc2822(v).ok())
            .map(|v| v.timestamp());
        let status = response.status().as_u16();
        let mut outcome = if status == 200 {
            match read_json(response) {
                Ok(value) => {
                    if claim {
                        parse_claim(&value)
                    } else {
                        parse_status(&value)
                    }
                }
                Err(error) => error,
            }
        } else {
            match status {
                401 => Outcome::new(
                    State::Unauthorized,
                    "Trae 登录已失效，请在原工具登录或重新授权",
                ),
                403 => Outcome::new(State::Forbidden, "当前 Trae 账号无权参与签到"),
                429 => Outcome::new(State::RateLimited, "Trae 签到请求过于频繁，请稍后再试"),
                _ => Outcome::new(
                    State::Failed,
                    if claim {
                        "Trae 领取结果未确认，请刷新状态后再试"
                    } else {
                        "Trae 签到服务响应异常"
                    },
                ),
            }
        };
        // 日期仅由官方 HTTP Date 推导北京时间；缺失时不以本机时钟伪装服务端日期。
        if let Some((day, reset)) = server_time.and_then(beijing_period) {
            outcome.period_key = Some(day);
            outcome.reset_at = Some(reset);
            outcome.server_time = server_time;
        }
        if let Err(error) = self.ensure_current() {
            return error;
        }
        outcome
    }
}

fn require_cn(edition: &str) -> Result<(), Outcome> {
    if edition == "cn" {
        Ok(())
    } else {
        Err(Outcome::new(
            State::Unsupported,
            "Trae 国际版官方客户端未开放每日积分签到",
        ))
    }
}

fn valid_label(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn expiry(value: &Value) -> Option<DateTime<Utc>> {
    value
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc))
}

fn normalized_account_label(value: Option<&str>) -> Option<String> {
    value.map(str::trim).filter(|label| valid_label(label, 320)).map(str::to_owned)
}

fn account_fields(auth: &Value) -> Result<(String, String, Option<String>), Outcome> {
    if auth["host"].as_str() != Some(HOST) || auth["account"]["scope"].as_str() != Some("marscode")
    {
        return Err(Outcome::new(
            State::Unsupported,
            "Trae 每日积分签到仅向国内个人账号开放",
        ));
    }
    let id = auth["userId"].as_str().filter(|s| valid_label(s, 320));
    let token = auth["token"].as_str().filter(|s| valid_label(s, 64 * 1024));
    match (id, token) {
        (Some(id), Some(token)) => Ok((id.into(), token.into(),
            normalized_account_label(auth["account"]["email"].as_str())
                .or_else(|| normalized_account_label(auth["account"]["username"].as_str())))),
        _ => Err(Outcome::new(
            State::Unauthorized,
            "Trae 登录缺少账号身份或凭证，请在原工具重新登录",
        )),
    }
}

fn quota_error(value: crate::quota::QuotaState) -> Outcome {
    use crate::quota::QuotaState;
    let state = match value {
        QuotaState::Unauthorized { .. } => State::Unauthorized,
        QuotaState::Unsupported { .. } => State::Unsupported,
        QuotaState::Forbidden { .. } => State::Forbidden,
        QuotaState::RateLimited { .. } => State::RateLimited,
        _ => State::Failed,
    };
    Outcome::new(
        state,
        "无法使用 Trae 国内版当前登录，请在原工具检查登录状态",
    )
}

fn native_root() -> Result<PathBuf, Outcome> {
    std::env::var_os("APPDATA")
        .map(|v| PathBuf::from(v).join("Trae CN"))
        .ok_or_else(|| Outcome::new(State::Failed, "未找到 Trae 国内版用户目录"))
}

fn read_json(reader: impl Read) -> Result<Value, Outcome> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Outcome::new(State::Failed, "Trae 签到数据读取失败"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Outcome::new(State::Failed, "Trae 签到数据超出大小限制"));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| Outcome::new(State::Failed, "Trae 签到数据格式无法识别"))
}

fn device_error() -> Outcome {
    Outcome::new(
        State::Inactive,
        "未找到匹配本机的 Trae 官方设备身份，请先安装并打开 Trae 国内版后重试",
    )
}

fn read_device(root: &Path, local_uuid: &str) -> Result<String, Outcome> {
    let file = std::fs::File::open(root.join("aha/TinyStorage")).map_err(|_| device_error())?;
    let storage = read_json(file).map_err(|_| device_error())?;
    let cache = storage["tiny_storage_data"]["aha.device.device_id"]
        .as_str()
        .and_then(crate::trae_quota::decrypt_auth)
        .ok_or_else(device_error)?;
    device_from_cache(&cache, local_uuid)
}

fn numeric_id(value: &str) -> bool {
    (8..=32).contains(&value.len())
        && value.starts_with(|c: char| c.is_ascii_digit() && c != '0')
        && value.bytes().all(|c| c.is_ascii_digit())
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
        && !value.eq_ignore_ascii_case("00000000-0000-0000-0000-000000000000")
        && !value.eq_ignore_ascii_case("ffffffff-ffff-ffff-ffff-ffffffffffff")
}

fn device_from_cache(cache: &Value, local_uuid: &str) -> Result<String, Outcome> {
    let id = cache["device_id_str"].as_str().filter(|s| numeric_id(s));
    let install = cache["install_id_str"].as_str().filter(|s| numeric_id(s));
    let uuid = cache["uuid"].as_str().filter(|s| valid_uuid(s));
    match (id, install, uuid) {
        (Some(id), Some(_), Some(uuid))
            if valid_uuid(local_uuid) && uuid.eq_ignore_ascii_case(local_uuid) =>
        {
            Ok(id.into())
        }
        _ => Err(device_error()),
    }
}

#[cfg(windows)]
fn hardware_uuid() -> Result<String, Outcome> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
        time::Instant,
    };
    let path = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:/Windows"))
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let mut child = Command::new(path)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-CimInstance -ClassName Win32_ComputerSystemProduct -ErrorAction Stop).UUID",
        ])
        .creation_flags(0x0800_0000)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| device_error())?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return Err(device_error()),
            Ok(None) if started.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(device_error());
            }
        }
    }
    let output = child.wait_with_output().map_err(|_| device_error())?;
    let uuid = std::str::from_utf8(&output.stdout)
        .map_err(|_| device_error())?
        .trim()
        .to_owned();
    if valid_uuid(&uuid) {
        Ok(uuid)
    } else {
        Err(device_error())
    }
}

#[cfg(not(windows))]
fn hardware_uuid() -> Result<String, Outcome> {
    Err(device_error())
}

fn business_error(value: &Value) -> Option<Outcome> {
    let code = value.get("code")?;
    let Some(code) = code.as_i64() else {
        return Some(Outcome::new(State::Failed, "Trae 签到业务状态格式无效"));
    };
    match code {
        0 => None,
        1001 | 1002 => Some(Outcome::new(
            State::Unauthorized,
            "Trae 账号验证失败，请在原工具登录或重新授权",
        )),
        9090 => Some(Outcome::new(State::Inactive, "Trae 当前未开放签到活动")),
        9095 => Some(Outcome::new(
            State::Inactive,
            "该设备今日已签到，当前账号是否领取需刷新核实",
        )),
        9004 | 4001 => Some(Outcome::new(State::Failed, "Trae 签到参数未通过官方校验")),
        9093 => Some(Outcome::new(
            State::Failed,
            "Trae 签到活动配置异常，请稍后再试",
        )),
        _ => Some(Outcome::new(
            State::Failed,
            "Trae 签到服务返回异常状态，请稍后再试",
        )),
    }
}

fn positive_number(value: &Value, key: &str) -> Option<f64> {
    value
        .get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v > 0.0)
}

fn parse_status(value: &Value) -> Outcome {
    if let Some(error) = business_error(value) {
        return error;
    }
    let (Some(enable), Some(checked)) = (value["enable"].as_bool(), value["checked_in"].as_bool())
    else {
        return Outcome::new(State::Failed, "Trae 签到状态格式无效");
    };
    if value.get("did_checked_in").is_some_and(|v| !v.is_boolean()) {
        return Outcome::new(State::Failed, "Trae 设备签到状态格式无效");
    }
    let mut outcome = if checked {
        Outcome::new(State::Claimed, "今日已签到")
    } else if value["did_checked_in"].as_bool() == Some(true) {
        Outcome::new(State::Inactive, "该设备今日已签到，当前账号未显示已领取")
    } else if !enable {
        Outcome::new(State::Inactive, "Trae 当前未开放签到")
    } else {
        Outcome::new(State::Ready, "今日可签到")
    };
    outcome.reward = positive_number(value, "credits");
    outcome.official_statistics = Some(OfficialCheckinStatistics {
        unit: "积分".into(),
        scope: "today".into(),
        week: None,
        month: None,
        all: None,
        note: "Trae 当前签到接口仅返回今日状态与奖励，未提供可核验的周、月及累计签到历史".into(),
    });
    outcome
}

fn parse_claim(value: &Value) -> Outcome {
    if !value.is_object() {
        return Outcome::new(State::Failed, "Trae 领取结果格式无效，请刷新状态核实");
    }
    if let Some(error) = business_error(value) {
        return error;
    }
    let mut outcome = Outcome::new(State::Claimed, "签到成功");
    outcome.reward = positive_number(value, "credits");
    outcome
}

fn beijing_period(seconds: i64) -> Option<(String, i64)> {
    let date = DateTime::<Utc>::from_timestamp(seconds.checked_add(8 * 3600)?, 0)?.date_naive();
    let next = date
        .succ_opt()?
        .and_hms_opt(0, 0, 0)?
        .and_utc()
        .timestamp()
        .checked_sub(8 * 3600)?;
    Some((date.to_string(), next))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_requires_boolean_flags_and_preserves_account_and_device_distinction() {
        for (value, expected) in [
            (
                json!({"enable":true,"checked_in":false,"credits":100}),
                State::Ready,
            ),
            (json!({"enable":true,"checked_in":true}), State::Claimed),
            (
                json!({"enable":true,"checked_in":false,"did_checked_in":true}),
                State::Inactive,
            ),
            (json!({"enable":false,"checked_in":false}), State::Inactive),
            (json!({"enable":1,"checked_in":false}), State::Failed),
            (json!({"enable":true}), State::Failed),
            (
                json!({"code":1002,"enable":true,"checked_in":false}),
                State::Unauthorized,
            ),
        ] {
            assert_eq!(parse_status(&value).state, expected, "{value}");
        }
        let outcome = parse_status(
            &json!({"enable":true,"checked_in":false,"credits":100,"extra_credits":888}),
        );
        assert_eq!(outcome.reward, Some(100.0));
        assert_eq!(
            parse_status(&json!({"enable":true,"checked_in":false,"credits":"100"})).reward,
            None
        );
    }

    #[test]
    fn today_status_does_not_invent_official_week_month_or_lifetime_statistics() {
        let outcome = parse_status(&json!({"enable":true,"checked_in":true,"credits":100}));
        let statistics = outcome.official_statistics.expect("must explain the official history coverage");
        assert_eq!(statistics.scope, "today");
        assert!(statistics.week.is_none());
        assert!(statistics.month.is_none());
        assert!(statistics.all.is_none());
        assert!(statistics.note.contains("今日"));
        assert!(statistics.note.contains("历史"));
    }

    #[test]
    fn claim_only_accepts_success_and_never_calls_device_limit_account_claimed() {
        for (value, expected) in [
            (json!({"code":0}), State::Claimed),
            (json!({}), State::Claimed),
            (json!(null), State::Failed),
            (json!({"code":"0"}), State::Failed),
            (json!({"code":9095}), State::Inactive),
            (json!({"code":9090}), State::Inactive),
            (json!({"code":9004}), State::Failed),
            (json!({"code":1002}), State::Unauthorized),
            (json!({"code":2001}), State::Failed),
        ] {
            assert_eq!(parse_claim(&value).state, expected, "{value}");
        }
    }

    #[test]
    fn device_cache_requires_valid_real_device_bound_to_local_hardware() {
        let cache = json!({"device_id_str":"1234567890123456","install_id_str":"2345678901234567","uuid":"12345678-1234-1234-1234-123456789abc","version":"1.0"});
        assert_eq!(
            device_from_cache(&cache, "12345678-1234-1234-1234-123456789ABC")
                .ok()
                .as_deref(),
            Some("1234567890123456")
        );
        assert!(device_from_cache(&cache, "00000000-0000-0000-0000-000000000000").is_err());
        for field in ["device_id_str", "install_id_str", "uuid"] {
            let mut invalid = cache.clone();
            invalid[field] = json!("");
            assert!(device_from_cache(&invalid, "12345678-1234-1234-1234-123456789abc").is_err());
        }
        let mut invalid = cache.clone();
        invalid["device_id_str"] = json!("synthetic-random-device");
        assert!(device_from_cache(&invalid, "12345678-1234-1234-1234-123456789abc").is_err());
    }

    #[test]
    fn native_account_requires_cn_personal_scope_and_stable_user_id() {
        let auth = json!({"host":"https://api.trae.cn","token":"synthetic-token","userId":"123456789","account":{"scope":"marscode"}});
        assert_eq!(
            account_fields(&auth).ok(),
            Some(("123456789".into(), "synthetic-token".into(), None))
        );
        for (key, value) in [
            ("host", json!("https://grow-normal.trae.ai")),
            ("userId", json!("")),
            ("token", json!("")),
            ("account", json!({"scope":"saas"})),
        ] {
            let mut invalid = auth.clone();
            invalid[key] = value;
            assert!(account_fields(&invalid).is_err());
        }
    }

    #[test]
    fn native_account_fields_bind_visible_label_to_credentials_without_uid_fallback() {
        for (profile, expected) in [
            (json!({"scope":"marscode","email":" first@example.invalid ","username":"first"}), Some("first@example.invalid")),
            (json!({"scope":"marscode","email":"","username":" second "}), Some("second")),
            (json!({"scope":"marscode","email":"bad\nlabel","username":""}), None),
            (json!({"scope":"marscode"}), None),
        ] {
            let auth = json!({"host":HOST,"token":"synthetic-token","userId":"synthetic-id","account":profile});
            let fields = account_fields(&auth).unwrap();
            assert_eq!(serde_json::to_value(&fields).unwrap(), json!(["synthetic-id","synthetic-token",expected]));
            let (user_id, token, account_label) = fields;
            let account = Account {
                user_id, token, account_label, device: String::new(), uuid: String::new(),
                root: PathBuf::new(), native: true, expires_at: None,
            };
            assert_eq!(account.account_label(), expected);
        }
    }

    #[test]
    fn beijing_period_rolls_over_at_utc_sixteen() {
        assert_eq!(
            beijing_period(1_793_116_799),
            Some(("2026-10-27".into(), 1_793_116_800))
        );
        assert_eq!(
            beijing_period(1_793_116_800),
            Some(("2026-10-28".into(), 1_793_203_200))
        );
    }

    #[test]
    fn real_device_storage_decodes_authenticated_envelope_and_rejects_corruption() {
        // 由 Node crypto 独立生成的合成 AHA 设备缓存，不含真实设备或账号数据。
        let encoded = "dGMFEAAABwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwe5/54dlWKQIDFZgdG58xKIe62Gl/6ULWSEbcsAnluLFxtoQF4F527FCW297/4xuRsbr3mcFch8IpdVLKtOMtLdUAyB9ZWrjBlGk29JvIUEb3c3pUuq4MiacaAYD/H+Sj2Dy0OTUjh1+YcB5wvnmk/UmI7Y2BbsZZd6FaA72rZfMkJpDgi605674m6GFjtbConXCEY0iDtgHrY0W78atURVF8+RqzFFNUAi+DfUB4qAEP5At11t/gmATmg3FiDDFolL9rlsgLYleNMsQCOXoRHP";
        let root = std::env::temp_dir().join(format!("jolu-trae-device-{}", std::process::id()));
        std::fs::create_dir_all(root.join("aha")).unwrap();
        std::fs::write(
            root.join("aha/TinyStorage"),
            json!({"tiny_storage_data":{"aha.device.device_id":encoded}}).to_string(),
        )
        .unwrap();
        assert_eq!(
            read_device(&root, "12345678-1234-1234-1234-123456789abc")
                .ok()
                .as_deref(),
            Some("1234567890123456")
        );
        std::fs::write(
            root.join("aha/TinyStorage"),
            json!({"tiny_storage_data":{"aha.device.device_id":"corrupt"}}).to_string(),
        )
        .unwrap();
        assert!(read_device(&root, "12345678-1234-1234-1234-123456789abc").is_err());
        std::fs::remove_file(root.join("aha/TinyStorage")).unwrap();
        std::fs::remove_dir(root.join("aha")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn global_edition_fails_before_reading_any_native_account_or_device() {
        let outcome = Account::from_native("global").err().unwrap();
        assert_eq!(outcome.state, State::Unsupported);
        let session = crate::trae_browser_auth::Session {
            token: "synthetic".into(),
            host: "https://growsg-normal.trae.ai".into(),
            expires_at: "2099-01-01T00:00:00Z".into(),
            account: None,
            user_id: "synthetic-user".into(),
        };
        assert_eq!(
            Account::from_browser("global", &session)
                .err()
                .unwrap()
                .state,
            State::Unsupported
        );
    }

    #[test]
    #[ignore = "显式执行：只读取原工具当前登录、官方设备及签到状态；绝不领取"]
    fn native_status_read_only_smoke() {
        let account = Account::from_native("cn").unwrap_or_else(|outcome| {
            panic!("只读构建失败：{:?} {}", outcome.state, outcome.message)
        });
        let outcome = account.status();
        println!(
            "Trae CN: state={:?} reward={:?} has_official_period={}",
            outcome.state,
            outcome.reward,
            outcome.period_key.is_some()
        );
        assert!(
            matches!(
                outcome.state,
                State::Ready | State::Claimed | State::Inactive
            ),
            "只读状态未成功，原始认证和响应不输出"
        );
    }

    #[test]
    #[ignore = "真实领取一次每日免费福利：必须显式设置 JOLU_RUN_REAL_CHECKIN=1；无重试"]
    fn native_claim_once() {
        assert!(
            std::env::var("JOLU_RUN_REAL_CHECKIN").as_deref() == Ok("1"),
            "真实签到测试需要显式设置 JOLU_RUN_REAL_CHECKIN=1"
        );
        let account = Account::from_native("cn")
            .unwrap_or_else(|outcome| panic!("Trae CN: state={:?}", outcome.state));
        let before = account.status();
        println!(
            "Trae CN: state={:?} reward={:?}",
            before.state, before.reward
        );
        if before.state != State::Ready {
            return;
        }
        let claim = account.claim(|| {
            let current = Account::from_native("cn")?;
            if current.identity() != account.identity()
                || current.token != account.token
                || current.device != account.device
            {
                return Err(Outcome::new(
                    State::Unauthorized,
                    "Trae 账号或官方设备发生变化，已取消本次领取",
                ));
            }
            current.ensure_current()
        });
        let after = account.status();
        println!(
            "Trae CN: claim_state={:?} reward={:?} after_state={:?}",
            claim.state, claim.reward, after.state
        );
        assert_eq!(
            claim.state,
            State::Claimed,
            "单次领取未确认成功；不自动重试"
        );
        assert_eq!(
            after.state,
            State::Claimed,
            "领取后状态未确认已签到；不自动重试"
        );
    }
}
