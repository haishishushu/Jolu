//! ZCode 官方 BigModel 设备轮询授权；不接管 zcode://，不读取原工具凭证。
//! 依据 zai-org/zcode packages/services/src/oauth/oauthService.ts 及本机官方 CLI。
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::Read,
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub(crate) access_token: String,
    pub(crate) refresh_token: Option<String>,
    pub(crate) user_id: String,
    pub(crate) account: Option<String>,
    pub(crate) zcode_token: String,
}

pub(crate) struct Pending {
    pub(crate) login_url: String,
    pub(crate) user_code: Option<String>,
    pub(crate) interval: Duration,
    flow_id: String,
    poll_token: String,
    expires: Instant,
    next_poll: Instant,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 32768 && !s.chars().any(char::is_control))
        .map(str::to_owned)
}

fn login_url(raw: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "ZCode 授权地址无效")?;
    if raw.len() > 16384
        || raw.chars().any(char::is_control)
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|p| p != 443)
        || !url
            .host_str()
            .is_some_and(|h| matches!(h, "bigmodel.cn" | "www.bigmodel.cn" | "zcode.z.ai"))
    {
        return Err("ZCode 返回的授权地址不属于 BigModel 官方站点".into());
    }
    Ok(url.to_string())
}

#[cfg(windows)]
fn poll_token() -> Result<String, String> {
    use windows_sys::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };
    let mut bytes = [0u8; 32];
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err("无法生成 ZCode 授权安全随机数".into());
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
#[cfg(not(windows))]
fn poll_token() -> Result<String, String> {
    Err("ZCode 浏览器授权当前仅支持 Windows".into())
}

fn client() -> Result<reqwest::blocking::Client, String> {
    crate::net::blocking_builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "无法创建 ZCode 授权客户端".into())
}

fn json_response(mut response: reqwest::blocking::Response) -> Result<Value, String> {
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("ZCode 授权服务未成功（HTTP {status}）"));
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(262145)
        .read_to_end(&mut bytes)
        .map_err(|_| "读取 ZCode 授权响应失败")?;
    if bytes.len() > 262144 {
        return Err("ZCode 授权响应过大".into());
    }
    let envelope: Value = serde_json::from_slice(&bytes).map_err(|_| "ZCode 授权响应格式无效")?;
    if envelope.get("code").and_then(Value::as_i64) != Some(0) {
        return Err("ZCode 授权服务拒绝了请求，请重新授权".into());
    }
    envelope
        .get("data")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or("ZCode 授权响应缺少有效数据".into())
}

fn pending(value: &Value, poll_token: String) -> Result<Pending, String> {
    let flow_id = text(value, "flow_id")
        .filter(|s| s.len() <= 512)
        .ok_or("ZCode 未返回授权流程标识")?;
    let login_url = login_url(
        value
            .get("authorize_url")
            .and_then(Value::as_str)
            .ok_or("ZCode 未返回授权地址")?,
    )?;
    let seconds = value
        .get("expires_at")
        .and_then(Value::as_i64)
        .and_then(|expiry| expiry.checked_sub(chrono::Utc::now().timestamp()))
        .ok_or("ZCode 未返回有效授权时限")?;
    if !(1..=3600).contains(&seconds) {
        return Err("ZCode 授权时限无效或已过期".into());
    }
    let interval_secs = value
        .get("poll_interval_sec")
        .and_then(Value::as_u64)
        .filter(|n| (1..=300).contains(n))
        .ok_or("ZCode 未返回有效查询间隔")?;
    let now = Instant::now();
    Ok(Pending {
        login_url,
        user_code: None,
        interval: Duration::from_secs(interval_secs),
        flow_id,
        poll_token,
        expires: now + Duration::from_secs(seconds as u64),
        next_poll: now + Duration::from_secs(interval_secs),
    })
}

pub(crate) fn begin() -> Result<Pending, String> {
    if crate::validation::isolated() {
        return Err("隔离验收不创建真实 ZCode 授权".into());
    }
    let token = poll_token()?;
    let response = client()?
        .post("https://zcode.z.ai/api/v1/oauth/cli/init")
        .bearer_auth(&token)
        .json(&json!({"provider":"bigmodel"}))
        .send()
        .map_err(|_| "无法创建 ZCode 授权，请检查网络")?;
    pending(&json_response(response)?, token)
}

fn session(value: &Value) -> Result<Session, String> {
    let user = value.get("user").ok_or("ZCode 未返回已验证账号")?;
    let provider = value
        .get("bigmodel")
        .ok_or("ZCode 未返回 BigModel 登录凭据")?;
    Ok(Session {
        user_id: text(user, "user_id").ok_or("ZCode 未返回有效账号身份")?,
        account: text(user, "email").or_else(|| text(user, "name")),
        zcode_token: text(value, "token").ok_or("ZCode 未返回有效会话")?,
        access_token: text(provider, "access_token")
            .or_else(|| text(provider, "accessToken"))
            .ok_or("ZCode 未返回 BigModel 业务凭据")?,
        refresh_token: text(provider, "refresh_token").or_else(|| text(provider, "refreshToken")),
    })
}

pub(crate) fn poll(pending: &mut Pending) -> Result<Option<Session>, String> {
    let now = Instant::now();
    if now >= pending.expires {
        return Err("ZCode 授权已超时，请重新授权".into());
    }
    if now < pending.next_poll {
        return Ok(None);
    }
    pending.next_poll = now + pending.interval;
    let mut url = reqwest::Url::parse("https://zcode.z.ai/api/v1/oauth/cli/poll/")
        .map_err(|_| "ZCode 授权端点无效")?;
    url.path_segments_mut()
        .map_err(|_| "ZCode 授权端点无效")?
        .pop_if_empty()
        .push(&pending.flow_id);
    let response = client()?
        .get(url)
        .bearer_auth(&pending.poll_token)
        .send()
        .map_err(|_| "ZCode 授权查询网络失败，请重新授权")?;
    let value = json_response(response)?;
    match value.get("status").and_then(Value::as_str) {
        Some("pending") => Ok(None),
        Some("ready") => session(&value).map(Some),
        Some("failed") => Err("ZCode 官方授权未成功，请重新授权".into()),
        _ => Err("ZCode 返回了未知授权状态".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bigmodel_ready_requires_identity_and_provider_specific_token() {
        let value = json!({"token":"synthetic-session","user":{"user_id":"test-user","email":"test@example.test"},"bigmodel":{"accessToken":"synthetic-business-token"}});
        assert_eq!(
            session(&value).unwrap().account.as_deref(),
            Some("test@example.test")
        );
        let mut missing = value.clone();
        missing["user"] = json!({});
        assert!(session(&missing).is_err());
        missing = value;
        missing["bigmodel"] = Value::Null;
        assert!(session(&missing).is_err());
        assert!(login_url("https://bigmodel.cn.evil.test/login").is_err());
        assert!(login_url("https://chat.z.ai/api/oauth/authorize").is_err());
    }
}
