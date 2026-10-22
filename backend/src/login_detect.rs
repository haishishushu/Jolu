//! 「检测本机登录」（Jolu.pen 画板 11 / 13）：只读取本机已登录的官方客户端，不落库。
//! 检测成功后由用户点「保存连接」再走 read_local_connections 入库；凭证原值不离开后端。
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct LoginDetection {
    pub found: bool,
    /// 凭证脱敏标识；本机来源平台没有凭证时为 None
    pub masked: Option<String>,
    /// 账号邮箱；客户端没有记录时为 None，界面显示「未获取」
    pub account: Option<String>,
    /// 套餐名称，原样透传，不由额度推断
    pub plan: Option<String>,
    /// 检测到的本机来源名称（本机来源平台）
    pub sources: Vec<String>,
    /// 未检测到时的原因
    pub message: Option<String>,
}

pub fn detect(platform: &str) -> LoginDetection {
    match platform {
        "claude" => detect_claude(),
        "codex" => detect_codex(),
        other if crate::platforms::native(other) => detect_native(other),
        _ => LoginDetection { message: Some("该平台暂不支持订阅连接".into()), ..Default::default() },
    }
}

fn read_json(path: PathBuf) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn str_at(v: &Value, path: &[&str]) -> Option<String> {
    let mut cur = v;
    for key in path {
        cur = cur.get(key)?;
    }
    cur.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn detect_claude() -> LoginDetection {
    let Some(home) = crate::creds::home() else {
        return LoginDetection { message: Some("无法确定用户目录".into()), ..Default::default() };
    };
    let credentials = read_json(home.join(".claude").join(".credentials.json"));
    let profile = read_json(home.join(".claude.json"));
    parse_claude(credentials.as_ref(), profile.as_ref())
}

/// `.credentials.json` 存令牌与套餐类型，`~/.claude.json` 的 oauthAccount 存账号邮箱。
fn parse_claude(credentials: Option<&Value>, profile: Option<&Value>) -> LoginDetection {
    let Some(token) = credentials.and_then(|v| str_at(v, &["claudeAiOauth", "accessToken"])) else {
        return LoginDetection { message: Some("未检测到 Claude Code 登录，请先在客户端完成登录".into()), ..Default::default() };
    };
    LoginDetection {
        found: true,
        masked: Some(crate::connections::mask(&token)),
        account: profile.and_then(|v| str_at(v, &["oauthAccount", "emailAddress"])),
        plan: credentials.and_then(|v| str_at(v, &["claudeAiOauth", "subscriptionType"])),
        ..Default::default()
    }
}

fn detect_codex() -> LoginDetection {
    let dir = std::env::var_os("CODEX_HOME").map(PathBuf::from).or_else(|| crate::creds::home().map(|h| h.join(".codex")));
    let Some(dir) = dir else {
        return LoginDetection { message: Some("无法确定用户目录".into()), ..Default::default() };
    };
    parse_codex(read_json(dir.join("auth.json")).as_ref())
}

/// `auth.json` 的 id_token 是 JWT：载荷里有 email 和 ChatGPT 套餐类型。只解码，不校验签名，也不外发。
fn parse_codex(auth: Option<&Value>) -> LoginDetection {
    let Some(token) = auth.and_then(|v| str_at(v, &["tokens", "access_token"])) else {
        return LoginDetection { message: Some("未检测到 Codex 登录，请先用 ChatGPT 账号登录官方客户端".into()), ..Default::default() };
    };
    let claims = auth.and_then(|v| str_at(v, &["tokens", "id_token"])).and_then(|jwt| jwt_payload(&jwt));
    LoginDetection {
        found: true,
        masked: Some(crate::connections::mask(&token)),
        account: claims.as_ref().and_then(|c| str_at(c, &["email"])),
        plan: claims.as_ref().and_then(|c| str_at(c, &["https://api.openai.com/auth", "chatgpt_plan_type"])),
        ..Default::default()
    }
}

fn detect_native(platform: &str) -> LoginDetection {
    let sources = crate::native_sources::sources(platform);
    let available: Vec<String> = sources.iter().filter(|s| s.available).map(|s| s.name.clone()).collect();
    if available.is_empty() {
        let reason = sources.iter().find_map(|s| s.reason.clone()).unwrap_or_else(|| "未发现可用的本机来源".into());
        return LoginDetection { message: Some(reason), ..Default::default() };
    }
    LoginDetection { found: true, sources: available, ..Default::default() }
}

fn jwt_payload(jwt: &str) -> Option<Value> {
    let payload = jwt.split('.').nth(1)?;
    serde_json::from_slice(&base64url_decode(payload)?).ok()
}

/// JWT 段落用无填充的 base64url；只解码 id_token 一处，不为此引入依赖。
fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for byte in input.trim_end_matches('=').bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        } as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_login_reads_plan_and_email_without_returning_token() {
        let credentials = json!({ "claudeAiOauth": { "accessToken": "sk-ant-oat01-secret-token-value", "subscriptionType": "max" } });
        let profile = json!({ "oauthAccount": { "emailAddress": "momo@example.com" } });
        let found = parse_claude(Some(&credentials), Some(&profile));
        assert!(found.found);
        assert_eq!(found.account.as_deref(), Some("momo@example.com"));
        assert_eq!(found.plan.as_deref(), Some("max"));
        assert!(!found.masked.unwrap().contains("secret-token"));
        // 没有邮箱记录时如实留空，不编造
        assert_eq!(parse_claude(Some(&credentials), None).account, None);
        assert!(!parse_claude(None, Some(&profile)).found);
    }

    #[test]
    fn codex_login_decodes_id_token_claims() {
        // {"email":"alex@example.com","https://api.openai.com/auth":{"chatgpt_plan_type":"pro"}}
        let payload = "eyJlbWFpbCI6ImFsZXhAZXhhbXBsZS5jb20iLCJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9wbGFuX3R5cGUiOiJwcm8ifX0";
        let auth = json!({ "tokens": { "access_token": "eyJhbGciOi.access.token-value", "id_token": format!("header.{payload}.signature") } });
        let found = parse_codex(Some(&auth));
        assert!(found.found);
        assert_eq!(found.account.as_deref(), Some("alex@example.com"));
        assert_eq!(found.plan.as_deref(), Some("pro"));
        // id_token 损坏时仍判定已登录，只是账号信息未获取
        let broken = json!({ "tokens": { "access_token": "token-value-123456", "id_token": "bad" } });
        let partial = parse_codex(Some(&broken));
        assert!(partial.found && partial.account.is_none());
        assert!(!parse_codex(Some(&json!({}))).found);
    }

    #[test]
    fn base64url_decodes_unpadded_segments() {
        assert_eq!(base64url_decode("aGk").unwrap(), b"hi");
        assert_eq!(base64url_decode("aGVsbG8_").unwrap(), b"hello?");
        assert!(base64url_decode("a*b").is_none());
    }
}
