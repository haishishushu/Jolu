//! 「检测本机登录」（Jolu.pen 画板 11 / 13）：只读取本机已登录的官方客户端，不落库。
//! 检测成功后由用户点「保存连接」再走 read_local_connections 入库；凭证原值不离开后端。
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
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
        "opencode" => {let found=crate::opencode::openai_logged_in();LoginDetection {found,sources:if found{vec!["已在 OpenCode 中登录 OpenAI".into()]}else{vec![]},message:(!found).then(||"请先在 OpenCode 中登录 OpenAI".into()),..Default::default()}},
        "claude" => detect_claude(),
        "codex" => detect_codex(),
        other if crate::platforms::native(other) => detect_native(other),
        _ => LoginDetection { message: Some("该平台暂不支持订阅连接".into()), ..Default::default() },
    }
}

pub(crate) fn detect_for_edition(platform: &str, edition: Option<&str>) -> LoginDetection {
    if !matches!(platform,"qoder"|"workbuddy"|"trae") { return detect(platform); }
    match native_accounts_for(platform,edition) {
        Ok(accounts) => LoginDetection {found:true, account:if accounts.len()==1 {accounts[0].account.clone()} else {None}, sources:accounts.into_iter().map(|a|a.name.to_string()).collect(),..Default::default()},
        Err(message) => LoginDetection {message:Some(message),..Default::default()},
    }
}

/// 仅从与已保存连接完全匹配的本机凭证快照提取展示邮箱；令牌不离开后端。
pub(crate) fn account_for_secret(platform: &str, expected: &str) -> Option<String> {
    if crate::validation::isolated() { return None; }
    match platform {
        "codex" => {
            let dir = crate::creds::codex_dir()?;
            let auth = read_json(dir.join("auth.json"))?;
            account_from_snapshot(platform, expected, &auth, None)
        }
        "claude" => {
            let auth = read_json(crate::creds::claude_dir()?.join(".credentials.json"))?;
            let profile = claude_profile_path().and_then(read_json);
            account_from_snapshot(platform, expected, &auth, profile.as_ref())
        }
        _ => None,
    }
}

/// 只读本机与已保存连接匹配的订阅快照。JWT exp 是令牌有效期，绝不可用作订阅到期。
pub(crate) fn codex_subscription_for_secret(expected: &str) -> Option<(String, Option<String>)> {
    if crate::validation::isolated() { return None; }
    let dir = crate::creds::codex_dir()?;
    subscription_from_snapshot(expected, &read_json(dir.join("auth.json"))?)
}

fn subscription_from_snapshot(expected: &str, auth: &Value) -> Option<(String, Option<String>)> {
    if expected.is_empty() || str_at(auth, &["tokens", "access_token"]).as_deref() != Some(expected) { return None; }
    let claims = jwt_payload(&str_at(auth, &["tokens", "id_token"])?)?;
    let parse_date = |name| {
        let text = str_at(&claims, &["https://api.openai.com/auth", name])?;
        chrono::DateTime::parse_from_rfc3339(&text).ok().map(|date| date.to_rfc3339())
    };
    Some((parse_date("chatgpt_subscription_active_until")?, parse_date("chatgpt_subscription_last_checked")))
}

fn account_from_snapshot(platform: &str, expected: &str, auth: &Value, profile: Option<&Value>) -> Option<String> {
    if expected.is_empty() { return None; }
    let (actual, detection) = match platform {
        "codex" => (str_at(auth, &["tokens", "access_token"]), parse_codex(Some(auth))),
        "claude" => (str_at(auth, &["claudeAiOauth", "accessToken"]), parse_claude(Some(auth), profile)),
        _ => return None,
    };
    if actual.as_deref() != Some(expected) { return None; }
    detection.account.filter(|value| value.len() <= 320 && !value.chars().any(char::is_control))
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

fn claude_profile_path() -> Option<PathBuf> {
    if std::env::var_os("CLAUDE_CONFIG_DIR").is_some_and(|value| !value.is_empty()) {
        crate::creds::claude_dir().map(|dir| dir.join(".claude.json"))
    } else { crate::creds::home().map(|home| home.join(".claude.json")) }
}

fn detect_claude() -> LoginDetection {
    let Some(dir) = crate::creds::claude_dir() else {
        return LoginDetection { message: Some("无法确定用户目录".into()), ..Default::default() };
    };
    let credentials = read_json(dir.join(".credentials.json"));
    let profile = claude_profile_path().and_then(read_json);
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
    let dir = crate::creds::codex_dir();
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
    if matches!(platform, "qoder" | "workbuddy" | "trae") {
        return detect_for_edition(platform,None);
    }
    let sources = crate::native_sources::sources(platform);
    let available: Vec<String> = sources.iter().filter(|s| s.available).map(|s| s.name.clone()).collect();
    if available.is_empty() {
        let reason = sources.iter().find_map(|s| s.reason.clone()).unwrap_or_else(|| "未发现可用的本机来源".into());
        return LoginDetection { message: Some(reason), ..Default::default() };
    }
    LoginDetection { found: true, sources: available, ..Default::default() }
}

pub(crate) struct NativeAccount {
    pub source_id: &'static str,
    pub name: &'static str,
    pub account: Option<String>,
}
pub(crate) fn native_accounts_for(platform: &str, edition: Option<&str>) -> Result<Vec<NativeAccount>,String> {
    let expected=match (platform,edition) {
        (_,None) => None,
        ("qoder",Some("cn")) => Some("native:qoder:cn"),
        ("qoder",Some("global")) => Some("native:qoder:global"),
        ("workbuddy",Some("cn")) => Some("native:workbuddy:main"),
        ("workbuddy",Some("ai")) => Some("native:workbuddy:ai"),
        ("trae",Some("cn")) => Some("native:trae:cn"),
        ("trae",Some("global")) => Some("native:trae:ide"),
        _ => return Err("未知的供应商账号区域".into()),
    };
    if crate::validation::isolated() { return Err("隔离环境不读取本机登录".into()); }
    let mut result = Vec::new();
    match platform {
        "qoder" => for (edition,id,name) in [("cn","native:qoder:cn","Qoder 国内版"),("global","native:qoder:global","Qoder 国际版")] {
            if expected.is_some_and(|expected|expected!=id) {continue;}
            if let Ok(session) = crate::qoder_desktop::local_session(edition) { result.push(NativeAccount {source_id:id,name,account:session.account}); }
            else if edition=="global" && crate::qoder_desktop::local_auth_missing(edition) {
                // 仅 CLI 登录仍可被发现；没有桌面文件时才核实原 CLI 的只读账号接口。
                if let crate::quota::QuotaState::Ok {account,..}=crate::qoder_quota::local_qoder_quota() {
                    result.push(NativeAccount {source_id:id,name,account:account.and_then(|a|a.email)});
                }
            }
        },
        "workbuddy" => for (id,name) in [("native:workbuddy:main","WorkBuddy 国内版"),("native:workbuddy:ai","WorkBuddy 国际版")] {
            if expected.is_some_and(|expected|expected!=id) {continue;}
            if let Ok(account) = crate::workbuddy_quota::source_account(id) { result.push(NativeAccount {source_id:id,name,account}); }
        },
        "trae" => for (id,name) in [("native:trae:cn","Trae CN"),("native:trae:ide","Trae")] {
            if expected.is_some_and(|expected|expected!=id) {continue;}
            if let Ok(account)=crate::trae_quota::source_account(id) {result.push(NativeAccount{source_id:id,name,account});}
        },
        _ => return Err("该平台不使用此登录来源".into()),
    }
    if result.is_empty() { Err("未读取到有效的本地登录，请在对应官方客户端登录，或选择浏览器授权".into()) } else { Ok(result) }
}

pub(crate) fn jwt_payload(jwt: &str) -> Option<Value> {
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

#[cfg(test)]
mod account_identity_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn identity_requires_exact_credentials_and_never_uses_masked_suffix() {
        let auth = json!({"claudeAiOauth":{"accessToken":"synthetic-a-1234"}});
        let profile = json!({"oauthAccount":{"emailAddress":"person@example.test"}});
        assert_eq!(account_from_snapshot("claude", "synthetic-a-1234", &auth, Some(&profile)).as_deref(), Some("person@example.test"));
        assert_eq!(account_from_snapshot("claude", "synthetic-b-1234", &auth, Some(&profile)), None);
        assert_eq!(account_from_snapshot("claude", "", &auth, Some(&profile)), None);
        assert_eq!(account_from_snapshot("claude", "synthetic-a-1234", &auth, None), None);
        assert_eq!(account_from_snapshot("unsupported", "synthetic-a-1234", &auth, Some(&profile)), None);
    }
}

#[cfg(all(test, debug_assertions))]
mod account_isolation_tests {
    #[test]
    fn isolated_account_lookup_does_not_read_local_identity() {
        const CHILD: &str = "JOLU_ACCOUNT_EMAIL_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            assert_eq!(super::account_for_secret("claude", "synthetic-identity"), None);
            return;
        }
        let root = std::env::temp_dir().join(format!("jolu-account-email-{}", std::process::id()));
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::write(root.join(".acceptance-profile"), "test").unwrap();
        std::fs::write(root.join(".claude/.credentials.json"), r#"{"claudeAiOauth":{"accessToken":"synthetic-identity"}}"#).unwrap();
        std::fs::write(root.join(".claude.json"), r#"{"oauthAccount":{"emailAddress":"private@example.test"}}"#).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "login_detect::account_isolation_tests::isolated_account_lookup_does_not_read_local_identity"])
            .env(CHILD, "1").env("JOLU_TEST_DIR", &root).env("USERPROFILE", &root).env("HOME", &root)
            .output().unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert!(output.status.success(), "isolated account lookup failed");
    }
}

#[cfg(test)]
mod subscription_tests {
    use super::*;
    #[test]
    fn subscription_snapshot_requires_matching_identity_and_explicit_subscription_date() {
        let auth = serde_json::json!({"tokens":{"access_token":"test-identity", "id_token":"h.eyJleHAiOiA5OTk5OTk5OTk5LCAiaHR0cHM6Ly9hcGkub3BlbmFpLmNvbS9hdXRoIjogeyJjaGF0Z3B0X3N1YnNjcmlwdGlvbl9hY3RpdmVfdW50aWwiOiAiMjAyNi0xMC0yNVQwMDowMDowMFoiLCAiY2hhdGdwdF9zdWJzY3JpcHRpb25fbGFzdF9jaGVja2VkIjogIjIwMjYtMTAtMjBUMDA6MDA6MDBaIn19.s"}});
        let (expires, checked) = subscription_from_snapshot("test-identity", &auth).unwrap();
        assert_eq!(expires, "2026-10-25T00:00:00+00:00");
        assert_eq!(checked.as_deref(), Some("2026-10-20T00:00:00+00:00"));
        assert!(subscription_from_snapshot("other-identity", &auth).is_none());
        assert!(subscription_from_snapshot("", &auth).is_none());
        let expiry_only = serde_json::json!({"tokens":{"access_token":"test-identity", "id_token":"h.eyJleHAiOiA5OTk5OTk5OTk5fQ.s"}});
        assert!(subscription_from_snapshot("test-identity", &expiry_only).is_none());
    }
}
