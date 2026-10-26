//! Gemini CLI（Google 账号登录）的套餐与额度查询，端点与字段对齐 gemini-cli 源码
//! `packages/core/src/code_assist/{server,setup,types,oauth2}.ts`：
//!
//! - 凭证：`~/.gemini/oauth_creds.json`（google-auth Credentials：access_token / refresh_token /
//!   expiry_date 毫秒）；邮箱：`~/.gemini/google_accounts.json` 的 `active`
//! - 套餐：`POST https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist`
//! - 额度：`POST https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota`，
//!   按模型返回 `buckets[].remainingFraction / remainingAmount / resetTime`
//!
//! 纪律：两个接口都是查询语义，不发送提示内容；访问令牌过期时从本机官方 gemini-cli 安装读取
//! 安装应用客户端，在内存里换取新令牌，**不写回** Gemini 的凭证文件；找不到安装时不换令牌。
//! 只读默认的明文凭证文件；用户开启加密存储（`GEMINI_FORCE_ENCRYPTED_FILE_STORAGE`）时报不支持。

use crate::quota::{self, AccountMetadata, HttpOutcome, QuotaState, QuotaWindow};
use serde_json::{json, Value};
use std::path::Path;

const CODE_ASSIST: &str = "https://cloudcode-pa.googleapis.com/v1internal";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

#[derive(Debug, PartialEq)]
struct GeminiCreds {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expiry_ms: Option<i64>,
}

fn parse_creds(text: &str) -> Option<GeminiCreds> {
    let value: Value = serde_json::from_str(text).ok()?;
    let text_at = |key: &str| value.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let creds = GeminiCreds {
        access_token: text_at("access_token"),
        refresh_token: text_at("refresh_token"),
        expiry_ms: value.get("expiry_date").and_then(Value::as_i64),
    };
    (creds.access_token.is_some() || creds.refresh_token.is_some()).then_some(creds)
}

/// 访问令牌还能用（预留 60 秒余量）；没有过期时间时先直接用，401 再刷新
fn token_fresh(creds: &GeminiCreds, now_ms: i64) -> Option<&str> {
    let token = creds.access_token.as_deref()?;
    match creds.expiry_ms {
        Some(expiry) if expiry <= now_ms + 60_000 => None,
        _ => Some(token),
    }
}

pub(crate) fn active_email(dir: &Path) -> Option<String> {
    let value: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("google_accounts.json")).ok()?).ok()?;
    value.get("active")?.as_str().map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 320 && !s.chars().any(char::is_control))
        .map(str::to_string)
}

fn gemini_dir() -> Option<std::path::PathBuf> {
    crate::creds::home().map(|home| home.join(".gemini"))
}

fn outcome_state(outcome: HttpOutcome) -> Result<String, QuotaState> {
    match outcome {
        HttpOutcome::Body(body) => Ok(body),
        HttpOutcome::Unauthorized(_) => Err(QuotaState::Unauthorized { reason: "Gemini 登录已失效，请在 Gemini CLI 中重新登录".into() }),
        HttpOutcome::Forbidden(reason) => Err(QuotaState::Forbidden { reason }),
        HttpOutcome::RateLimited(reason) => Err(QuotaState::RateLimited { reason }),
        HttpOutcome::Failed(reason) => Err(QuotaState::Failed { reason: format!("Gemini 额度查询失败：{reason}") }),
    }
}

/// 打包产物里形如 `NAME = "value"` 的字符串常量；变量名后只允许空白、`=` 与引号。
fn bundle_constant(text: &str, name: &str) -> Option<String> {
    let mut rest = text;
    while let Some(index) = rest.find(name) {
        let tail = rest[index + name.len()..].trim_start();
        rest = &rest[index + name.len()..];
        let Some(tail) = tail.strip_prefix('=') else { continue };
        let tail = tail.trim_start();
        let Some(quote) = tail.chars().next().filter(|c| matches!(c, '"' | '\'' | '`')) else { continue };
        let Some(end) = tail[1..].find(quote) else { continue };
        let value = &tail[1..1 + end];
        if !value.is_empty() && value.len() <= 200 && value.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) {
            return Some(value.to_string());
        }
    }
    None
}

/// gemini-cli 安装应用客户端只从本机官方安装读取，不随 Jolu 源码分发；格式不符时视为不可用。
fn client_from_bundle(dir: &Path) -> Option<(String, String)> {
    const MAX_FILE: u64 = 32 * 1024 * 1024;
    let mut files: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "js"))
        .filter(|path| std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_FILE))
        .collect();
    files.sort();
    files.iter().take(500).find_map(|path| {
        let text = std::fs::read_to_string(path).ok()?;
        let id = bundle_constant(&text, "OAUTH_CLIENT_ID").filter(|id| id.ends_with(".apps.googleusercontent.com"))?;
        let secret = bundle_constant(&text, "OAUTH_CLIENT_SECRET").filter(|secret| secret.starts_with("GOCSPX-"))?;
        Some((id, secret))
    })
}

fn installed_client() -> Option<(String, String)> {
    let prefix = crate::cli_browser_auth::npm_prefix("gemini")?;
    client_from_bundle(&prefix.join("node_modules/@google/gemini-cli/bundle"))
}

fn refresh(client: &reqwest::blocking::Client, refresh_token: &str) -> Result<String, QuotaState> {
    let Some((client_id, client_secret)) = installed_client() else {
        return Err(QuotaState::Unauthorized { reason: "Gemini 访问令牌已过期，未找到官方 Gemini CLI，无法换新；请运行一次 Gemini CLI 后重试".into() });
    };
    let response = client.post(TOKEN_ENDPOINT)
        .form(&[("client_id", client_id.as_str()), ("client_secret", client_secret.as_str()), ("refresh_token", refresh_token), ("grant_type", "refresh_token")])
        .send()
        .map_err(|_| QuotaState::Failed { reason: "Gemini 令牌刷新失败，请检查网络后刷新".into() })?;
    // 刷新令牌被撤销时 Google 返回 400 invalid_grant，按授权失效处理
    if response.status().as_u16() == 400 {
        return Err(QuotaState::Unauthorized { reason: "Gemini 登录已失效，请在 Gemini CLI 中重新登录".into() });
    }
    let body = outcome_state(quota::read(response))?;
    serde_json::from_str::<Value>(&body).ok()
        .and_then(|v| v.get("access_token")?.as_str().map(str::to_string))
        .filter(|token| !token.is_empty())
        .ok_or_else(|| QuotaState::Failed { reason: "Gemini 令牌刷新响应格式无效".into() })
}

fn post(client: &reqwest::blocking::Client, token: &str, method: &str, body: &Value) -> Result<String, QuotaState> {
    let response = client.post(format!("{CODE_ASSIST}:{method}"))
        .bearer_auth(token)
        .header("Accept", "application/json")
        .json(body)
        .send()
        .map_err(|_| QuotaState::Failed { reason: "Gemini 额度查询失败，请检查网络后刷新".into() })?;
    outcome_state(quota::read(response))
}

/// loadCodeAssist：取套餐名与 Code Assist 项目。付费层（paidTier）优先于当前层
fn parse_load(body: &str) -> Result<(Option<String>, Option<String>), QuotaState> {
    let value: Value = serde_json::from_str(body).map_err(|_| QuotaState::Failed { reason: "Gemini 套餐响应格式无效".into() })?;
    let tier = ["paidTier", "currentTier"].iter().find_map(|key| {
        let tier = value.get(*key)?;
        tier.get("name").and_then(Value::as_str).or_else(|| tier.get("id").and_then(Value::as_str))
            .map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
    });
    let project = value.get("cloudaicompanionProject").and_then(Value::as_str)
        .map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    Ok((tier, project))
}

/// retrieveUserQuota：每个模型一行，remainingFraction 换算成已用百分比；
/// remainingAmount 是来源给的精确剩余次数，原样展示，不推断总量
fn parse_buckets(body: &str) -> Result<Vec<QuotaWindow>, QuotaState> {
    let value: Value = serde_json::from_str(body).map_err(|_| QuotaState::Failed { reason: "Gemini 额度响应格式无效".into() })?;
    // 官方 buckets 可省略；显式给出错误类型或非对象条目属于响应损坏。
    if !value.is_object() || value.get("buckets").is_some_and(|buckets| {
        buckets.as_array().is_none_or(|items| items.iter().any(|item| !item.is_object()))
    }) {
        return Err(QuotaState::Failed { reason: "Gemini 额度响应格式无效".into() });
    }
    let mut windows: Vec<QuotaWindow> = value.get("buckets").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|bucket| {
            let model = bucket.get("modelId").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())?;
            let fraction = bucket.get("remainingFraction").and_then(Value::as_f64).filter(|f| f.is_finite() && (0.0..=1.0).contains(f));
            let token_type = bucket.get("tokenType").and_then(Value::as_str).filter(|s| !s.is_empty());
            let key = match token_type { Some(kind) => format!("gemini:{model}:{kind}"), None => format!("gemini:{model}") };
            Some(QuotaWindow {
                key,
                window_name: model.to_string(),
                used_percent: fraction.map(|f| ((1.0 - f) * 1000.0).round() / 10.0),
                amount_text: None,
                remaining_text: bucket.get("remainingAmount").and_then(Value::as_str)
                    .and_then(|s| s.trim().parse::<u64>().ok()).map(|n| format!("剩余 {n} 次")),
                resets_at: bucket.get("resetTime").and_then(Value::as_str)
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map(|t| t.to_rfc3339()),
            })
        })
        .collect();
    windows.sort_by(|a, b| a.key.cmp(&b.key));
    windows.dedup_by(|a, b| a.key == b.key);
    Ok(windows)
}

pub(crate) fn local_gemini_quota() -> QuotaState {
    if crate::validation::isolated() {
        return QuotaState::Unauthorized { reason: "隔离环境不读取本机 Gemini 登录".into() };
    }
    if std::env::var("GEMINI_FORCE_ENCRYPTED_FILE_STORAGE").is_ok_and(|v| v == "true") {
        return QuotaState::Unsupported { reason: "Gemini CLI 使用加密凭证存储，Jolu 无法只读查询额度".into() };
    }
    let Some(dir) = gemini_dir() else {
        return QuotaState::Unauthorized { reason: "无法定位本机 Gemini 配置目录".into() };
    };
    let creds = match std::fs::read_to_string(dir.join("oauth_creds.json")) {
        Ok(text) => parse_creds(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return QuotaState::Failed { reason: "读取 Gemini 登录凭证失败".into() },
    };
    let Some(creds) = creds else {
        return QuotaState::Unauthorized { reason: "未找到 Gemini CLI 的 Google 账号登录，请先运行 gemini 并选择 Login with Google".into() };
    };
    let project = ["GOOGLE_CLOUD_PROJECT", "GOOGLE_CLOUD_PROJECT_ID"].iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.trim().is_empty()));
    query_credentials(&creds, active_email(&dir), project)
}

/// 独立浏览器授权使用自身凭证与邮箱，不读取当前 Gemini CLI 登录或本机项目环境变量。
pub(crate) fn authorized_quota(credentials: &Value, account: Option<&str>) -> QuotaState {
    if crate::validation::isolated() {
        return QuotaState::Unauthorized { reason: "隔离环境不查询真实 Gemini 授权".into() };
    }
    let Some(creds) = parse_creds(&credentials.to_string()) else {
        return QuotaState::Unauthorized { reason: "Gemini 浏览器授权缺少有效凭证，请重新授权".into() };
    };
    let email = account.map(str::trim).filter(|value| !value.is_empty() && value.len() <= 320 && !value.chars().any(char::is_control)).map(str::to_string);
    query_credentials(&creds, email, None)
}

fn query_credentials(creds: &GeminiCreds, email: Option<String>, env_project: Option<String>) -> QuotaState {
    let client = match quota::client() { Ok(c) => c, Err(reason) => return QuotaState::Failed { reason } };
    let token = match token_fresh(&creds, chrono::Utc::now().timestamp_millis()) {
        Some(token) => token.to_string(),
        None => match creds.refresh_token.as_deref() {
            Some(refresh_token) => match refresh(&client, refresh_token) { Ok(token) => token, Err(state) => return state },
            None => return QuotaState::Unauthorized { reason: "Gemini 登录已过期，请在 Gemini CLI 中重新登录".into() },
        },
    };
    let mut metadata = json!({"ideType": "IDE_UNSPECIFIED", "platform": "PLATFORM_UNSPECIFIED", "pluginType": "GEMINI"});
    let mut load = json!({"metadata": metadata});
    if let Some(project) = &env_project {
        metadata["duetProject"] = json!(project);
        load = json!({"cloudaicompanionProject": project, "metadata": metadata});
    }
    let (plan, project) = match post(&client, &token, "loadCodeAssist", &load).and_then(|body| parse_load(&body)) {
        Ok(result) => result,
        Err(state) => return state,
    };
    let Some(project) = env_project.or(project) else {
        return QuotaState::Unsupported { reason: "Gemini 账号尚未完成 Code Assist 初始化，请先运行一次 gemini".into() };
    };
    let windows = match post(&client, &token, "retrieveUserQuota", &json!({"project": project})).and_then(|body| parse_buckets(&body)) {
        Ok(windows) => windows,
        Err(state) => return state,
    };
    QuotaState::Ok {
        windows,
        plan,
        account: Some(AccountMetadata { email, ..AccountMetadata::default() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "需要鼠鼠授权：读取本机 Gemini 登录并向官方接口发起只读查询，只输出状态类别"]
    fn local_read_only_smoke() {
        let state = local_gemini_quota();
        let kind = serde_json::to_value(&state).unwrap()["state"].as_str().unwrap_or("?").to_string();
        let windows = match &state { QuotaState::Ok { windows, .. } => windows.len(), _ => 0 };
        println!("Gemini: state={kind} windows={windows}");
    }

    #[test]
    #[ignore = "读取本机 gemini-cli 安装，只输出是否找到客户端，不联网也不输出值"]
    fn installed_client_smoke() {
        println!("Gemini CLI client found={}", installed_client().is_some());
    }

    #[test]
    fn bundle_constant_reads_assignments_and_skips_lookalikes() {
        assert_eq!(bundle_constant(r#"var OAUTH_CLIENT_ID = "abc-1.x";"#, "OAUTH_CLIENT_ID").as_deref(), Some("abc-1.x"));
        assert_eq!(bundle_constant("const OAUTH_CLIENT_ID='abc'", "OAUTH_CLIENT_ID").as_deref(), Some("abc"));
        assert_eq!(bundle_constant(r#"x.OAUTH_CLIENT_ID; OAUTH_CLIENT_ID_X = "no"; OAUTH_CLIENT_ID = "yes""#, "OAUTH_CLIENT_ID").as_deref(), Some("yes"));
        assert_eq!(bundle_constant(r#"OAUTH_CLIENT_ID = "has space""#, "OAUTH_CLIENT_ID"), None);
        assert_eq!(bundle_constant(r#"OAUTH_CLIENT_ID = """#, "OAUTH_CLIENT_ID"), None);
        assert_eq!(bundle_constant("OAUTH_CLIENT_ID = process.env.X", "OAUTH_CLIENT_ID"), None);
    }

    #[test]
    fn client_from_bundle_requires_both_values_in_google_formats() {
        let dir = std::env::temp_dir().join(format!("jolu-gemini-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(client_from_bundle(&dir), None);
        std::fs::write(dir.join("a.js"), r#"OAUTH_CLIENT_ID = "synthetic"; OAUTH_CLIENT_SECRET = "GOCSPX-test""#).unwrap();
        std::fs::write(dir.join("b.txt"), r#"OAUTH_CLIENT_ID = "1-test.apps.googleusercontent.com"; OAUTH_CLIENT_SECRET = "GOCSPX-test""#).unwrap();
        assert_eq!(client_from_bundle(&dir), None);
        std::fs::write(dir.join("c.js"), r#"OAUTH_CLIENT_ID = "1-test.apps.googleusercontent.com"; var OAUTH_CLIENT_SECRET = "GOCSPX-test";"#).unwrap();
        assert_eq!(client_from_bundle(&dir), Some(("1-test.apps.googleusercontent.com".into(), "GOCSPX-test".into())));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(client_from_bundle(&dir), None);
    }

    #[test]
    fn credentials_need_a_token_and_expiry_is_respected() {
        assert_eq!(parse_creds("{}"), None);
        assert_eq!(parse_creds("bad"), None);
        let creds = parse_creds(r#"{"access_token":"a","refresh_token":"r","expiry_date":1000000}"#).unwrap();
        assert_eq!(token_fresh(&creds, 0), Some("a"));
        assert_eq!(token_fresh(&creds, 950_000), None);
        let no_expiry = parse_creds(r#"{"access_token":"a"}"#).unwrap();
        assert_eq!(token_fresh(&no_expiry, i64::MAX / 2), Some("a"));
    }

    #[test]
    fn load_prefers_paid_tier_and_keeps_project() {
        let (plan, project) = parse_load(r#"{"currentTier":{"id":"free-tier","name":"Gemini Code Assist for individuals"},"paidTier":{"id":"g1-pro-tier","name":"Gemini Code Assist in Google One AI Pro"},"cloudaicompanionProject":"p-1"}"#).unwrap();
        assert_eq!(plan.as_deref(), Some("Gemini Code Assist in Google One AI Pro"));
        assert_eq!(project.as_deref(), Some("p-1"));
        let (plan, project) = parse_load(r#"{"currentTier":{"id":"standard-tier"}}"#).unwrap();
        assert_eq!((plan.as_deref(), project), (Some("standard-tier"), None));
    }

    #[test]
    fn buckets_keep_real_zero_and_unknown_fraction() {
        let windows = parse_buckets(r#"{"buckets":[
            {"modelId":"gemini-2.5-pro","tokenType":"REQUESTS","remainingFraction":0,"remainingAmount":"0","resetTime":"2026-10-25T00:00:00Z"},
            {"modelId":"gemini-2.5-flash","tokenType":"REQUESTS","remainingFraction":0.75},
            {"modelId":"gemini-x","remainingFraction":7},
            {"tokenType":"REQUESTS","remainingFraction":0.5}
        ]}"#).unwrap();
        assert_eq!(windows.len(), 3);
        let pro = windows.iter().find(|w| w.window_name == "gemini-2.5-pro").unwrap();
        assert_eq!(pro.used_percent, Some(100.0));
        assert_eq!(pro.remaining_text.as_deref(), Some("剩余 0 次"));
        assert!(pro.resets_at.is_some());
        let flash = windows.iter().find(|w| w.window_name == "gemini-2.5-flash").unwrap();
        assert_eq!(flash.used_percent, Some(25.0));
        assert_eq!(flash.remaining_text, None);
        // 越界比例不当成真实数值
        assert_eq!(windows.iter().find(|w| w.window_name == "gemini-x").unwrap().used_percent, None);
        assert!(parse_buckets(r#"{}"#).unwrap().is_empty());
        assert!(parse_buckets("nope").is_err());
    }

    #[test]
    fn active_email_reads_only_the_active_account() {
        let dir = std::env::temp_dir().join(format!("jolu-gemini-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("google_accounts.json"), r#"{"active":"me@example.test","old":["old@example.test"]}"#).unwrap();
        assert_eq!(active_email(&dir).as_deref(), Some("me@example.test"));
        std::fs::write(dir.join("google_accounts.json"), r#"{"active":null,"old":[]}"#).unwrap();
        assert_eq!(active_email(&dir), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn buckets_reject_malformed_response_structure() {
        for body in ["null", "[]", r#"{"buckets":{}}"#, r#"{"buckets":[null]}"#] {
            assert!(parse_buckets(body).is_err(), "{body}");
        }
        assert!(parse_buckets("{}").unwrap().is_empty());
    }
}
