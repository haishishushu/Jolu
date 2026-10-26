//! Grok 官方 RFC 8628 设备授权；只处理后端凭证，不读取原工具配置。
//! 协议依据 xai-org/grok-build 的 xai-grok-login/{config,device_code}.rs。
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::Read,
    time::{Duration, Instant},
};

const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
// 只申请登录身份与计费查询所需的官方 scope，不申请会话或工作区写权限。
const SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub(crate) access_token: String,
    pub(crate) refresh_token: Option<String>,
    pub(crate) expires_at: Option<i64>,
    pub(crate) account: Option<String>,
    pub(crate) user_id: String,
}

pub(crate) struct Pending {
    pub(crate) login_url: String,
    pub(crate) user_code: Option<String>,
    pub(crate) interval: Duration,
    device_code: String,
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
    let url = reqwest::Url::parse(raw).map_err(|_| "Grok 授权地址无效")?;
    if raw.len() > 16384
        || raw.chars().any(char::is_control)
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|p| p != 443)
        || !url
            .host_str()
            .is_some_and(|h| matches!(h, "auth.x.ai" | "accounts.x.ai" | "grok.com"))
    {
        return Err("Grok 返回的授权地址不属于官方站点".into());
    }
    Ok(url.to_string())
}

fn client() -> Result<reqwest::blocking::Client, String> {
    crate::net::blocking_builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "无法创建 Grok 授权客户端".into())
}

fn json(mut response: reqwest::blocking::Response) -> Result<(u16, Value), String> {
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(262145)
        .read_to_end(&mut bytes)
        .map_err(|_| "读取 Grok 授权响应失败")?;
    if bytes.len() > 262144 {
        return Err("Grok 授权响应过大".into());
    }
    serde_json::from_slice(&bytes)
        .map(|v| (status, v))
        .map_err(|_| "Grok 授权响应格式无效".into())
}

fn pending(value: &Value) -> Result<Pending, String> {
    let device_code = text(value, "device_code").ok_or("Grok 未返回设备授权凭据")?;
    let code = text(value, "user_code")
        .filter(|s| s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        .ok_or("Grok 未返回有效设备码")?;
    let complete = text(value, "verification_uri_complete");
    let login_url = login_url(
        complete
            .as_deref()
            .or_else(|| value.get("verification_uri").and_then(Value::as_str))
            .ok_or("Grok 未返回授权地址")?,
    )?;
    let lifetime = value
        .get("expires_in")
        .and_then(Value::as_u64)
        .filter(|n| (1..=3600).contains(n))
        .ok_or("Grok 未返回有效授权时限")?;
    let interval = Duration::from_secs(
        value
            .get("interval")
            .and_then(Value::as_u64)
            .unwrap_or(5)
            .clamp(1, 300),
    );
    let now = Instant::now();
    Ok(Pending {
        login_url,
        user_code: complete.is_none().then_some(code),
        interval,
        device_code,
        expires: now + Duration::from_secs(lifetime),
        next_poll: now + interval,
    })
}

pub(crate) fn begin() -> Result<Pending, String> {
    if crate::validation::isolated() {
        return Err("隔离验收不创建真实 Grok 授权".into());
    }
    let response = client()?
        .post("https://auth.x.ai/oauth2/device/code")
        .header("x-grok-client-surface", "ui")
        .form(&[
            ("client_id", CLIENT_ID),
            ("scope", SCOPE),
            ("referrer", "grok-build"),
        ])
        .send()
        .map_err(|_| "无法创建 Grok 设备授权，请检查网络")?;
    let (status, value) = json(response)?;
    if !(200..300).contains(&status) {
        return Err(format!("Grok 设备授权未成功（HTTP {status}）"));
    }
    pending(&value)
}

fn token_session(value: &Value, profile: &Value) -> Result<Session, String> {
    let user_id = text(profile, "sub").ok_or("Grok 未返回已验证账号身份")?;
    let access_token = text(value, "access_token").ok_or("Grok 未返回有效登录凭据")?;
    let expires_at = value
        .get("expires_in")
        .and_then(Value::as_i64)
        .filter(|n| *n > 0)
        .and_then(|n| n.checked_mul(1000))
        .and_then(|n| chrono::Utc::now().timestamp_millis().checked_add(n));
    Ok(Session {
        access_token,
        refresh_token: text(value, "refresh_token"),
        expires_at,
        account: text(profile, "email"),
        user_id,
    })
}

pub(crate) fn poll(pending: &mut Pending) -> Result<Option<Session>, String> {
    let now = Instant::now();
    if now >= pending.expires {
        return Err("Grok 授权已超时，请重新授权".into());
    }
    if now < pending.next_poll {
        return Ok(None);
    }
    pending.next_poll = now + pending.interval;
    let client = client()?;
    let response = client
        .post("https://auth.x.ai/oauth2/token")
        .header("x-grok-client-surface", "ui")
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", pending.device_code.as_str()),
            ("client_id", CLIENT_ID),
        ])
        .send()
        .map_err(|_| "Grok 授权查询网络失败，请重新授权")?;
    let (status, value) = json(response)?;
    if !(200..300).contains(&status) {
        return match value.get("error").and_then(Value::as_str) {
            Some("authorization_pending") => Ok(None),
            Some("slow_down") => {
                pending.interval += Duration::from_secs(5);
                pending.next_poll = Instant::now() + pending.interval;
                Ok(None)
            }
            Some("access_denied") => Err("Grok 授权已被拒绝".into()),
            Some("expired_token") => Err("Grok 设备码已过期，请重新授权".into()),
            _ => Err(format!("Grok 授权查询未成功（HTTP {status}）")),
        };
    }
    let token = text(&value, "access_token").ok_or("Grok 未返回有效登录凭据")?;
    let profile = client
        .get("https://auth.x.ai/oauth2/userinfo")
        .bearer_auth(&token)
        .send()
        .map_err(|_| "Grok 账号身份查询失败")?;
    let (status, profile) = json(profile)?;
    if !(200..300).contains(&status) {
        return Err(format!("Grok 账号身份验证未成功（HTTP {status}）"));
    }
    token_session(&value, &profile).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_flow_waits_first_interval_and_rejects_untrusted_urls() {
        let p = pending(&serde_json::json!({"device_code":"synthetic-device","user_code":"ABCD-1234","verification_uri":"https://accounts.x.ai/device","expires_in":600,"interval":5})).unwrap();
        assert_eq!(p.user_code.as_deref(), Some("ABCD-1234"));
        assert!(p.next_poll > Instant::now());
        assert!(login_url("https://accounts.x.ai.evil.test/device").is_err());
        assert!(login_url("https://evil@accounts.x.ai/device").is_err());
        assert!(token_session(
            &serde_json::json!({"access_token":"synthetic-token"}),
            &serde_json::json!({"email":"a@example.test"})
        )
        .is_err());
    }
}
