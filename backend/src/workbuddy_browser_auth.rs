//! WorkBuddy 官方桌面端的系统浏览器授权协议。
//!
//! 依据本机官方 cli/product.json 与 main/dev-env-override.js（国际版 main/common.js）：
//! CN 为 account-scoped-external-link，AI 为 cli-external-link；均由服务端分配 state，
//! 打开 authUrl 后轮询 token，不使用公开 OAuth client ID、PKCE 或本地回调服务器。
//! 本模块不打开浏览器、不读取/写入原客户端文件；取消、轮询间隔和总超时由调用方管理。

use reqwest::blocking::{RequestBuilder, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Read;

#[derive(Clone, Copy)]
enum Edition { Cn, Ai }

impl Edition {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "cn" => Ok(Self::Cn), "ai" => Ok(Self::Ai),
            _ => Err("未知 WorkBuddy 授权版本".into()),
        }
    }
    fn base(self) -> &'static str {
        match self { Self::Cn => "https://www.workbuddy.cn", Self::Ai => "https://www.workbuddy.ai" }
    }
    fn domain(self) -> &'static str {
        match self { Self::Cn => "www.workbuddy.cn", Self::Ai => "www.workbuddy.ai" }
    }
    fn platform(self) -> &'static str {
        match self { Self::Cn => "workbuddy", Self::Ai => "workbuddy-ai" }
    }
    fn allows_domain(self, host: &str) -> bool {
        match self {
            Self::Cn => matches!(host, "workbuddy.cn" | "www.workbuddy.cn" | "codebuddy.cn" | "www.codebuddy.cn" | "copilot.tencent.com" | "wb.tencentbuddy.com"),
            Self::Ai => matches!(host, "workbuddy.ai" | "www.workbuddy.ai" | "codebuddy.ai" | "www.codebuddy.ai"),
        }
    }
}

/// 仅在后端持有；state 和 token 都不得写入日志或普通前端事件。
pub(crate) struct PendingLogin {
    pub(crate) state: String,
    pub(crate) login_url: String,
    edition: Edition,
    token: Option<Token>,
}

struct Token { value: String, domain: String, expires_at: Option<i64> }

/// expires_at 是服务端 expiresAt 或 expiresIn 得出的 Unix 毫秒，仅供存储/过期提示。
/// 账号资料由官方账号端点验证后返回；不信任 JWT 中未经验证的账号字段。
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub(crate) token: String,
    pub(crate) domain: String,
    pub(crate) user_id: String,
    pub(crate) enterprise_id: Option<String>,
    pub(crate) email: Option<String>,
    pub(crate) expires_at: Option<i64>,
}

fn text(value: &Value, key: &str, limit: usize) -> Option<String> {
    value.get(key)?.as_str().map(str::trim)
        .filter(|text| !text.is_empty() && text.len() <= limit && !text.chars().any(char::is_control))
        .map(str::to_string)
}

fn validate_login_url(edition: Edition, raw: &str, state: &str) -> Result<String, String> {
    let invalid = || "WorkBuddy 返回的授权地址不属于所选版本的官方站点".to_string();
    if raw.len() > 16384 || raw.chars().any(char::is_control) { return Err(invalid()); }
    let url = reqwest::Url::parse(raw).map_err(|_| invalid())?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some()
        || url.port().is_some_and(|port| port != 443) || !url.host_str().is_some_and(|host| edition.allows_domain(host)) {
        return Err(invalid());
    }
    if url.query_pairs().any(|(key, value)| key == "state" && value != state) {
        return Err("WorkBuddy 授权状态与登录地址不匹配".into());
    }
    Ok(url.to_string())
}

fn unauthenticated(request: RequestBuilder) -> RequestBuilder {
    request.header("Accept", "application/json")
        .header("X-No-Authorization", "true").header("X-No-User-Id", "true")
        .header("X-No-Enterprise-Id", "true").header("X-No-Department-Info", "true")
}

fn read_json(mut response: Response) -> Result<(u16, Value), String> {
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    response.by_ref().take(1_048_577).read_to_end(&mut bytes)
        .map_err(|_| "读取 WorkBuddy 授权响应失败，请重试".to_string())?;
    if bytes.len() > 1_048_576 { return Err("WorkBuddy 授权响应过大，请重试".into()); }
    let body = serde_json::from_slice(&bytes).map_err(|_| "WorkBuddy 授权响应格式无效，请重试".to_string())?;
    Ok((status, body))
}

fn request_json(request: RequestBuilder) -> Result<(u16, Value), String> {
    let response = request.send().map_err(|_| "WorkBuddy 授权请求失败，请检查网络后重试".to_string())?;
    read_json(response)
}

fn parse_begin(edition: Edition, status: u16, body: &Value) -> Result<PendingLogin, String> {
    if status != 200 || body.get("code").and_then(Value::as_i64) != Some(0) {
        return Err("WorkBuddy 暂时无法创建授权，请稍后重试".into());
    }
    let data = &body["data"];
    let state = text(data, "state", 2048).ok_or("WorkBuddy 授权响应缺少登录状态")?;
    let raw = text(data, "authUrl", 16384).ok_or("WorkBuddy 授权响应缺少登录地址")?;
    let login_url = validate_login_url(edition, &raw, &state)?;
    Ok(PendingLogin { state, login_url, edition, token: None })
}

/// 创建 Jolu 自己的 state；调用方只能将返回的 login_url 交给系统默认浏览器。
pub(crate) fn begin(edition: &str) -> Result<PendingLogin, String> {
    let edition = Edition::parse(edition)?;
    let client = crate::quota::client().map_err(|_| "无法创建 WorkBuddy 授权客户端".to_string())?;
    // 官方设备风控/归因头在获取失败时也允许缺省，不读取或仿造原客户端设备标识。
    let request = unauthenticated(client.post(format!("{}/v2/plugin/auth/state", edition.base())))
        .query(&[("platform", edition.platform())]).header("X-Domain", edition.domain())
        .header("X-Custom-Channel-Id", "unknown").json(&json!({}));
    let (status, body) = request_json(request)?;
    parse_begin(edition, status, &body)
}

fn parse_token(edition: Edition, status: u16, body: &Value) -> Result<Option<Token>, String> {
    if status == 200 && body.get("code").and_then(Value::as_i64) == Some(11217) { return Ok(None); }
    if status != 200 || body.get("code").and_then(Value::as_i64) != Some(0) {
        return Err("WorkBuddy 登录授权失败或已过期，请重新授权".into());
    }
    let data = &body["data"];
    let token = text(data, "accessToken", 32768).filter(|token| token.bytes().all(|b| b.is_ascii_alphanumeric() || b"._~+/-=".contains(&b)))
        .ok_or("WorkBuddy 授权响应缺少有效令牌")?;
    let domain = text(data, "domain", 255).unwrap_or_else(|| edition.domain().into());
    if !edition.allows_domain(&domain.to_ascii_lowercase()) {
        return Err("WorkBuddy 授权令牌所属区域与所选版本不匹配".into());
    }
    let numeric = |key| data.get(key).and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()));
    let expires_at = numeric("expiresAt").or_else(|| numeric("expiresIn").filter(|seconds| *seconds > 0)
        .and_then(|seconds| chrono::Utc::now().timestamp_millis().checked_add(seconds.checked_mul(1000)?)));
    if expires_at.is_some_and(|expires| expires <= chrono::Utc::now().timestamp_millis()) {
        return Err("WorkBuddy 返回的登录令牌已过期，请重新授权".into());
    }
    Ok(Some(Token { value: token, domain, expires_at }))
}

fn parse_account(token: &Token, status: u16, body: &Value) -> Result<Option<Session>, String> {
    if status == 200 && body.get("code").and_then(Value::as_i64) == Some(12151) { return Ok(None); }
    if status != 200 || body.get("code").and_then(Value::as_i64) != Some(0) {
        return Err("WorkBuddy 未能验证授权账号，请重新授权".into());
    }
    let data = &body["data"];
    let user_id = text(data, "uid", 1024).ok_or("WorkBuddy 未返回有效的授权账号")?;
    let enterprise_id = text(data, "enterpriseId", 1024);
    let email = text(data, "email", 320).or_else(|| text(data, "nickname", 320));
    Ok(Some(Session { token: token.value.clone(), domain: token.domain.clone(), user_id, enterprise_id, email, expires_at: token.expires_at }))
}

fn verify_account_response(edition: Edition, session: &Session, status: u16, body: &Value) -> Result<(), String> {
    if status != 200 || body.get("code").and_then(Value::as_i64) != Some(0) {
        return Err("WorkBuddy 授权账号校验失败，请重试或重新授权".into());
    }
    let matches = |account: &Value| text(account, "uid", 1024).as_deref() == Some(session.user_id.as_str())
        && text(account, "enterpriseId", 1024).as_deref() == session.enterprise_id.as_deref();
    let found = match edition {
        Edition::Cn => matches(&body["data"]),
        Edition::Ai => body.pointer("/data/accounts").and_then(Value::as_array)
            .is_some_and(|accounts| accounts.iter().any(matches)),
    };
    if !found { return Err("WorkBuddy 授权账号身份发生变化，请重新授权".into()); }
    Ok(())
}

/// CN 核实当前账号；AI 按官方续期路径读取账号列表并绑定已选择的身份。
pub(crate) fn verify_session(edition: &str, session: &Session) -> Result<(), String> {
    let edition = Edition::parse(edition)?;
    if crate::validation::isolated() { return Err("隔离环境不读取 WorkBuddy 真实授权".into()); }
    if !edition.allows_domain(&session.domain.to_ascii_lowercase())
        || session.token.is_empty() || session.token.len() > 32768
        || !session.token.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._~+/-=".contains(&byte))
        || session.user_id.trim().is_empty() || session.user_id.len() > 1024 || session.user_id.chars().any(char::is_control)
        || session.enterprise_id.as_ref().is_some_and(|id| id.trim().is_empty() || id.len() > 1024 || id.chars().any(char::is_control))
        || session.expires_at.is_some_and(|at| at <= chrono::Utc::now().timestamp_millis()) {
        return Err("WorkBuddy 浏览器授权已失效或区域不匹配，请重新授权".into());
    }
    let client = crate::quota::client().map_err(|_| "无法创建 WorkBuddy 身份验证客户端")?;
    let path = match edition { Edition::Cn => "/v2/plugin/account", Edition::Ai => "/v2/plugin/accounts" };
    let request = client.get(format!("{}{path}", edition.base())).bearer_auth(&session.token)
        .header("Accept", "application/json").header("X-Domain", &session.domain)
        .header("X-No-User-Id", "true").header("X-No-Enterprise-Id", "true").header("X-No-Department-Info", "true");
    let (status, body) = request_json(request)?;
    verify_account_response(edition, session, status, &body)
}

/// 单次轮询；None 表示官网尚未完成登录/账号选择，不在此函数内等待或切换账号。
pub(crate) fn poll(pending: &mut PendingLogin) -> Result<Option<Session>, String> {
    let client = crate::quota::client().map_err(|_| "无法创建 WorkBuddy 授权客户端".to_string())?;
    let base = pending.edition.base();
    if pending.token.is_none() {
        let request = unauthenticated(client.get(format!("{base}/v2/plugin/auth/token")))
            .query(&[("state", pending.state.as_str())]);
        let (status, body) = request_json(request)?;
        pending.token = parse_token(pending.edition, status, &body)?;
    }
    let Some(token) = pending.token.as_ref() else { return Ok(None) };
    let mut request = match pending.edition {
        Edition::Cn => client.get(format!("{base}/v2/plugin/account")),
        Edition::Ai => client.get(format!("{base}/v2/plugin/login/account"))
            .query(&[("state", pending.state.as_str())]),
    };
    request = request.bearer_auth(&token.value).header("Accept", "application/json")
        .header("X-Domain", &token.domain).header("X-No-User-Id", "true")
        .header("X-No-Enterprise-Id", "true").header("X-No-Department-Info", "true");
    let (status, body) = request_json(request)?;
    parse_account(token, status, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_configuration_pins_edition_and_platform() {
        assert_eq!(Edition::parse("cn").unwrap().base(), "https://www.workbuddy.cn");
        assert_eq!(Edition::parse("ai").unwrap().platform(), "workbuddy-ai");
        assert!(Edition::parse("other").is_err());
        assert!(Edition::Cn.allows_domain("codebuddy.cn"));
        assert!(Edition::Cn.allows_domain("copilot.tencent.com"));
        assert!(Edition::Cn.allows_domain("wb.tencentbuddy.com"));
        assert!(Edition::Ai.allows_domain("workbuddy.ai"));
        assert!(!Edition::Ai.allows_domain("codebuddy.cn"));
    }

    #[test]
    fn login_url_accepts_only_https_official_region_and_matching_state() {
        assert!(validate_login_url(Edition::Cn, "https://www.workbuddy.cn/login?state=synthetic", "synthetic").is_ok());
        for url in ["http://www.workbuddy.cn/login", "https://www.workbuddy.cn.evil.invalid/login", "https://www.workbuddy.cn@evil.invalid/login", "https://www.workbuddy.ai/login", "https://www.workbuddy.cn:444/login", "https://www.workbuddy.cn/login?state=other"] {
            assert!(validate_login_url(Edition::Cn, url, "synthetic").is_err());
        }
        assert!(validate_login_url(Edition::Ai, "https://www.workbuddy.ai/login", "synthetic").is_ok());
    }

    #[test]
    fn begin_requires_success_and_server_generated_state() {
        let response = json!({"code":0,"data":{"state":"synthetic","authUrl":"https://www.workbuddy.cn/login?state=synthetic"}});
        assert_eq!(parse_begin(Edition::Cn, 200, &response).unwrap().state, "synthetic");
        assert!(parse_begin(Edition::Cn, 500, &response).is_err());
        assert!(parse_begin(Edition::Cn, 200, &json!({"code":0,"data":{"authUrl":"https://www.workbuddy.cn/login"}})).is_err());
    }

    #[test]
    fn polling_distinguishes_pending_rejection_and_validates_official_account() {
        assert!(parse_token(Edition::Cn, 200, &json!({"code":11217})).unwrap().is_none());
        assert!(parse_token(Edition::Cn, 200, &json!({"code":401})).is_err());
        assert!(parse_token(Edition::Cn, 200, &json!({"code":0,"data":{"accessToken":"synthetic","domain":"www.workbuddy.ai"}})).is_err());
        assert!(parse_token(Edition::Cn, 200, &json!({"code":0,"data":{"accessToken":"synthetic","domain":"codebuddy.cn"}})).unwrap().is_some());
        let token = parse_token(Edition::Cn, 200, &json!({"code":0,"data":{"accessToken":"synthetic","domain":"www.workbuddy.cn","expiresAt":1893456000000_i64}})).unwrap().unwrap();
        assert!(parse_account(&token, 200, &json!({"code":12151})).unwrap().is_none());
        assert!(parse_account(&token, 200, &json!({"code":0,"data":{}})).is_err());
        assert!(parse_account(&token, 401, &json!({"code":0,"data":{"uid":"synthetic-user"}})).is_err());
        let session = parse_account(&token, 200, &json!({"code":0,"data":{"uid":"synthetic-user","email":"test@example.invalid"}})).unwrap().unwrap();
        assert_eq!(session.user_id, "synthetic-user");
        assert_eq!(session.email.as_deref(), Some("test@example.invalid"));
    }
    #[test]
    fn pending_business_codes_do_not_hide_http_authentication_errors() {
        assert!(parse_token(Edition::Cn, 401, &json!({"code":11217})).is_err());
        let token = Token { value: "synthetic-token".into(), domain: "www.workbuddy.cn".into(), expires_at: None };
        assert!(parse_account(&token, 401, &json!({"code":12151})).is_err());
    }
    #[test]
    fn fresh_identity_verification_requires_matching_user_and_enterprise_context() {
        let session = Session { token: "synthetic-token".into(), domain: "www.workbuddy.cn".into(),
            user_id: "synthetic-user".into(), enterprise_id: None, email: None, expires_at: None };
        assert!(verify_account_response(Edition::Cn, &session, 200, &json!({"code":0,"data":{"uid":"synthetic-user"}})).is_ok());
        assert!(verify_account_response(Edition::Cn, &session, 401, &json!({"code":0,"data":{"uid":"synthetic-user"}})).is_err());
        assert!(verify_account_response(Edition::Cn, &session, 200, &json!({"code":0,"data":{"uid":"other-user"}})).is_err());
        let accounts = json!({"code":0,"data":{"accounts":[{"uid":"synthetic-user","enterpriseId":"enterprise"},{"uid":"synthetic-user"}]}});
        assert!(verify_account_response(Edition::Ai, &session, 200, &accounts).is_ok());
        let enterprise_only = json!({"code":0,"data":{"accounts":[{"uid":"synthetic-user","enterpriseId":"enterprise"}]}});
        assert!(verify_account_response(Edition::Ai, &session, 200, &enterprise_only).is_err());
        assert!(verify_account_response(Edition::Ai, &session, 200, &json!({"code":0,"data":{"accounts":[]}})).is_err());
    }
}
