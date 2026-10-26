//! 读取本机 ZCode 配置中的 BigModel Coding Plan API Key，查询该 Key 的套餐额度。
//! 也支持复用官方客户端已有 BigModel 登录，GET 同源个人订阅与额度。
//! 不刷新登录、不创建 Key、不兑换令牌、不使用私有 appSecret。

use crate::coding_plan::{self, PlanProvider};
use crate::quota::{QuotaState, AccountMetadata};
use aes_gcm::{Aes256Gcm, Nonce, aead::{Aead, KeyInit}};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use serde_json::Value;
use std::path::Path;

fn bigmodel_coding_key(config: &Value) -> Option<&str> {
    config.pointer("/config/providerConfigRules/providerRules")?.as_array()?
        .iter()
        .filter(|rule| rule.get("providerId").and_then(Value::as_str) == Some("bigmodel-api")
            && rule.pointer("/config/access/type").and_then(Value::as_str) == Some("zhipu-coding-plan-api-key"))
        .find_map(|rule| rule.pointer("/config/access/apiKey")?.as_str()
            .map(str::trim).filter(|key| !key.is_empty()))
}

pub(crate) fn local_zcode_quota() -> QuotaState {
    if crate::validation::isolated() {
        return QuotaState::Unauthorized { reason: "隔离验证模式不读取真实 ZCode 配置".into() };
    }
    let Some(home) = crate::creds::home() else {
        return QuotaState::Unauthorized { reason: "无法定位本机 ZCode 配置目录".into() };
    };
    let dir = home.join(".zcode").join("v2");
    let config = match std::fs::read_to_string(dir.join("provider_config.json")) {
        Ok(config) => config,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return QuotaState::Unauthorized { reason: "未找到本机 ZCode 提供商配置".into() };
        }
        Err(_) => return QuotaState::Failed { reason: "读取 ZCode 提供商配置失败".into() },
    };
    let Ok(config) = serde_json::from_str::<Value>(&config) else {
        return QuotaState::Failed { reason: "ZCode 提供商配置格式无效".into() };
    };
    let Some(key) = bigmodel_coding_key(&config) else {
        return local_zcode_account_quota();
    };
    coding_plan::query_plan_direct(PlanProvider::ZhipuCn, key)
}

fn decrypt_credential(raw: &str, secret: &str) -> Result<String, ()> {
    let Some(payload) = raw.strip_prefix("enc:v1:") else {
        return if raw.starts_with("enc:") { Err(()) } else { Ok(raw.to_owned()) };
    };
    let parts: Vec<_> = payload.split('.').collect();
    if parts.len() != 3 { return Err(()); }
    let iv = URL_SAFE_NO_PAD.decode(parts[0]).map_err(|_| ())?;
    let tag = URL_SAFE_NO_PAD.decode(parts[1]).map_err(|_| ())?;
    let mut encrypted = URL_SAFE_NO_PAD.decode(parts[2]).map_err(|_| ())?;
    if iv.len() != 12 || tag.len() != 16 { return Err(()); }
    encrypted.extend_from_slice(&tag);
    let cipher = Aes256Gcm::new_from_slice(&Sha256::digest(secret.as_bytes())).map_err(|_| ())?;
    let bytes = cipher.decrypt(Nonce::from_slice(&iv), encrypted.as_slice()).map_err(|_| ())?;
    String::from_utf8(bytes).map_err(|_| ())
}

fn read_account(home: &Path) -> Result<(String, String), QuotaState> {
    let raw = std::fs::read_to_string(home.join(".zcode/v2/credentials.json")).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            QuotaState::Unauthorized { reason: "未找到 ZCode 登录，请先在原工具登录".into() }
        } else { QuotaState::Failed { reason: "读取 ZCode 登录失败".into() } }
    })?;
    let credentials: Value = serde_json::from_str(&raw).map_err(|_| QuotaState::Failed { reason: "ZCode 登录文件格式无效".into() })?;
    // 与官方 credentialCipherProvider 的公开实现一致；不尝试其他用户或秘密值。
    let platform = if cfg!(windows) { "win32" } else if cfg!(target_os = "macos") { "darwin" } else { "linux" };
    let username = std::env::var(if cfg!(windows) { "USERNAME" } else { "USER" }).unwrap_or_else(|_| "unknown".into());
    let secret = std::env::var("ZCODE_CREDENTIAL_SECRET").ok().filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("zcode-credential-fallback:{platform}:{}:{username}", home.display()));
    let load = |key: &str| -> Result<String, QuotaState> {
        let raw = credentials.get(key).and_then(Value::as_str).ok_or_else(|| QuotaState::Unauthorized { reason: "ZCode 登录不完整，请在原工具重新登录".into() })?;
        let value = decrypt_credential(raw, &secret).map_err(|_| QuotaState::Failed { reason: "无法解密当前用户的 ZCode 登录，请在原工具检查登录或自定义凭据密钥".into() })?;
        if value.trim().is_empty() { return Err(QuotaState::Unauthorized { reason: "ZCode 登录不完整，请在原工具重新登录".into() }); }
        Ok(value)
    };
    let family = load("oauth:active_provider")?;
    if !matches!(family.as_str(), "bigmodel" | "zai") { return Err(QuotaState::Unsupported { reason: "当前 ZCode 登录来源暂不支持查询".into() }); }
    let token = load(&format!("oauth:{family}:access_token"))?;
    Ok((family, token))
}

pub(crate) fn validate_local_account_at(home: &Path) -> Result<(), String> {
    read_account(home).map(|_| ()).map_err(|state| match state {
        QuotaState::Unauthorized { reason } | QuotaState::Failed { reason } | QuotaState::Unsupported { reason } => reason,
        _ => "ZCode 登录不可用".into(),
    })
}

fn identity_of(family: &str, token: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(family.as_bytes());
    digest.update([0]);
    digest.update(token.as_bytes());
    format!("{:x}", digest.finalize())
}

/// 仅用于内部缓存绑定；不可写日志或返回前端。
pub(crate) fn account_identity() -> Result<String, QuotaState> {
    if crate::validation::isolated() {
        return Err(QuotaState::Unauthorized { reason: "隔离验证模式不读取真实 ZCode 登录".into() });
    }
    let home = crate::creds::home().ok_or_else(|| QuotaState::Unauthorized { reason: "无法定位 ZCode 登录目录".into() })?;
    let (family, token) = read_account(&home)?;
    Ok(identity_of(&family, &token))
}

fn account_json(client: &reqwest::blocking::Client, url: &str, token: &str) -> Result<Value, QuotaState> {
    let response = client.get(url).header(reqwest::header::AUTHORIZATION, token).send()
        .map_err(|_| QuotaState::Failed { reason: "ZCode 账号查询网络失败".into() })?;
    let status = response.status().as_u16();
    match status {
        401 => return Err(QuotaState::Unauthorized { reason: "ZCode 登录已失效，请在原工具重新登录".into() }),
        403 => return Err(QuotaState::Forbidden { reason: "ZCode 登录没有查询权限".into() }),
        429 => return Err(QuotaState::RateLimited { reason: "ZCode 查询过于频繁，请稍后重试".into() }),
        200..=299 => (),
        _ => return Err(QuotaState::Failed { reason: "ZCode 账号服务查询失败".into() }),
    }
    let body: Value = response.json().map_err(|_| QuotaState::Failed { reason: "ZCode 账号响应格式无效".into() })?;
    if body.get("success").and_then(Value::as_bool) == Some(false)
        || !matches!(body.get("code").and_then(Value::as_i64), Some(0 | 200)) {
        return Err(QuotaState::Failed { reason: "ZCode 账号服务未返回有效查询结果".into() });
    }
    Ok(body)
}

#[derive(Debug)]
enum PersonalSubscription<'a> {
    Active(&'a Value),
    Expired,
    None,
}

impl PersonalSubscription<'_> {
    fn status(&self) -> &'static str {
        match self {
            Self::Active(_) => "active",
            Self::Expired => "expired",
            Self::None => "inactive",
        }
    }
}

fn personal_subscription(body: &Value) -> Result<PersonalSubscription<'_>, QuotaState> {
    let list = body.get("data").and_then(Value::as_array).ok_or_else(|| QuotaState::Failed { reason: "ZCode 个人订阅响应格式无效".into() })?;
    let mut malformed = false;
    let mut expired = false;
    for item in list {
        let coding = ["productId", "productName"].iter().any(|key| item.get(*key).and_then(Value::as_str).is_some_and(|s| s.to_ascii_lowercase().contains("coding")));
        if !coding { if !item.is_object() { malformed = true; } continue; }
        let status = item.get("status").and_then(Value::as_str);
        let current = item.get("inCurrentPeriod").and_then(Value::as_bool);
        if status == Some("VALID") && current == Some(true) { return Ok(PersonalSubscription::Active(item)); }
        // 到期只采用明确状态；继续扫描，后续有效订阅必须优先。
        if status == Some("EXPIRED") && current == Some(false) { expired = true; }
        if !matches!(status, Some("VALID" | "EXPIRED")) || current.is_none() { malformed = true; }
    }
    if malformed { return Err(QuotaState::Failed { reason: "ZCode 个人订阅缺少有效状态，无法确认权益".into() }); }
    Ok(if expired { PersonalSubscription::Expired } else { PersonalSubscription::None })
}

pub(crate) fn local_zcode_account_quota() -> QuotaState {
    if crate::validation::isolated() {
        return QuotaState::Unauthorized { reason: "隔离验证模式不读取真实 ZCode 登录".into() };
    }
    let Some(home) = crate::creds::home() else { return QuotaState::Unauthorized { reason: "无法定位 ZCode 登录目录".into() }; };
    let (family, token) = match read_account(&home) { Ok(value) => value, Err(state) => return state };
    // Z.ai 的业务令牌交换尚未实测，不能把其 OAuth token 发给 BigModel。
    if family != "bigmodel" { return QuotaState::Unsupported { reason: "ZCode 国际账号的只读订阅查询尚待验证；可使用现有 Coding Plan API Key 查询".into() }; }
    query_personal(&token, None, || read_account(&home).is_ok_and(|current| current == (family.clone(), token.clone())))
}

/// BigModel 官方设备流返回的独立业务 token；不读取原工具登录文件。
pub(crate) fn authorized_quota(session: &crate::zcode_browser_auth::Session) -> QuotaState {
    if crate::validation::isolated() { return QuotaState::Unauthorized { reason: "隔离验收不查询真实 ZCode 授权".into() }; }
    query_personal(&session.access_token, session.account.clone(), || true)
}

fn query_personal(token: &str, email: Option<String>, unchanged: impl Fn() -> bool) -> QuotaState {
    let client = match crate::quota::client() { Ok(client) => client, Err(_) => return QuotaState::Failed { reason: "无法创建 ZCode 查询客户端".into() } };
    let body = match account_json(&client, "https://bigmodel.cn/api/biz/subscription/list", token) { Ok(body) => body, Err(state) => return state };
    let subscription = match personal_subscription(&body) { Ok(value) => value, Err(state) => return state };
    let account = AccountMetadata {
        email,
        subscription_status: subscription.status(),
        subscription_source: "live",
        observed_at: Some(chrono::Utc::now().to_rfc3339()),
        ..AccountMetadata::default()
    };
    if !unchanged() {
        return QuotaState::Failed { reason: "ZCode 登录在查询期间发生变化，请刷新".into() };
    }
    let subscription = match subscription {
        PersonalSubscription::Active(subscription) => subscription,
        PersonalSubscription::Expired => return QuotaState::Ok { windows: vec![], plan: Some("个人编程套餐已到期".into()), account: Some(account) },
        PersonalSubscription::None => return QuotaState::Ok { windows: vec![], plan: Some("未开通个人编程套餐".into()), account: Some(account) },
    };
    let body = match account_json(&client, "https://bigmodel.cn/api/monitor/usage/quota/limit", token) { Ok(body) => body, Err(state) => return state };
    let Some(data) = body.get("data") else { return QuotaState::Failed { reason: "ZCode 未返回额度数据".into() }; };
    let (windows, _) = coding_plan::parse_zhipu_windows(data);
    if windows.is_empty() { return QuotaState::Failed { reason: "ZCode 未返回可识别的个人套餐额度".into() }; }
    // 查询期间切换登录时丢弃旧账号结果。
    if !unchanged() {
        return QuotaState::Failed { reason: "ZCode 登录在查询期间发生变化，请刷新".into() };
    }
    QuotaState::Ok { windows, plan: subscription.get("productName").and_then(Value::as_str).map(str::to_owned), account: Some(account) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_identity_binds_family_and_token() {
        let identity = identity_of("bigmodel", "synthetic-a");
        assert_eq!(identity, identity_of("bigmodel", "synthetic-a"));
        assert_ne!(identity, identity_of("bigmodel", "synthetic-b"));
        assert_ne!(identity, identity_of("zai", "synthetic-a"));
        assert_eq!(identity.len(), 64);
        assert!(identity.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn decrypts_official_envelope_and_rejects_tampering() {
        let secret = "synthetic-test-secret";
        let iv = [7u8; 12];
        let cipher = Aes256Gcm::new_from_slice(&Sha256::digest(secret.as_bytes())).unwrap();
        let encrypted = cipher.encrypt(Nonce::from_slice(&iv), b"synthetic-token".as_slice()).unwrap();
        let split = encrypted.len() - 16;
        let raw = format!("enc:v1:{}.{}.{}", URL_SAFE_NO_PAD.encode(iv), URL_SAFE_NO_PAD.encode(&encrypted[split..]), URL_SAFE_NO_PAD.encode(&encrypted[..split]));
        assert_eq!(decrypt_credential(&raw, secret).unwrap(), "synthetic-token");
        assert!(decrypt_credential(&raw, "wrong-secret").is_err());
        assert!(decrypt_credential("enc:v1:bad.bad.bad", secret).is_err());
        assert!(decrypt_credential("enc:v2:unknown", secret).is_err());
        assert_eq!(decrypt_credential("legacy-plain", secret).unwrap(), "legacy-plain");
    }

    #[test]
    fn expired_personal_subscription_is_not_absent() {
        let body = serde_json::json!({"data":[{"productName":"Coding Lite","status":"EXPIRED","inCurrentPeriod":false}]});
        let subscription = personal_subscription(&body).unwrap();
        assert!(matches!(subscription, PersonalSubscription::Expired));
        assert_eq!(subscription.status(), "expired");
    }

    #[test]
    fn active_subscription_wins_over_multiple_expired_records() {
        let mut body = serde_json::json!({"data":[
            {"productName":"Coding Old A","status":"EXPIRED","inCurrentPeriod":false},
            {"productName":"Coding Old B","status":"EXPIRED","inCurrentPeriod":false}
        ]});
        assert!(matches!(personal_subscription(&body).unwrap(), PersonalSubscription::Expired));
        body["data"].as_array_mut().unwrap().push(serde_json::json!({"productName":"Coding Current","status":"VALID","inCurrentPeriod":true}));
        assert!(matches!(personal_subscription(&body).unwrap(), PersonalSubscription::Active(item) if item["productName"] == "Coding Current"));
    }

    #[test]
    fn personal_subscription_distinguishes_empty_active_and_malformed() {
        assert!(matches!(personal_subscription(&serde_json::json!({"data":[]})).unwrap(), PersonalSubscription::None));
        assert!(matches!(personal_subscription(&serde_json::json!({"data":[{"productName":"Coding Lite","status":"VALID","inCurrentPeriod":true}]})).unwrap(), PersonalSubscription::Active(_)));
        assert!(personal_subscription(&serde_json::json!({"data":[{"productName":"Coding Lite","status":"VALID"}]})).is_err());
        assert!(personal_subscription(&serde_json::json!({"data":null})).is_err());
        assert!(personal_subscription(&serde_json::json!({"data":[null]})).is_err());
        assert!(matches!(personal_subscription(&serde_json::json!({"data":[{"productName":"Coding Lite","status":"EXPIRED","inCurrentPeriod":false}]})).unwrap(), PersonalSubscription::Expired));
    }

    #[test]
    #[ignore = "需要本机 ZCode 登录与网络；仅显式只读执行"]
    fn local_account_readonly_smoke() {
        let state = local_zcode_account_quota();
        assert!(matches!(state, QuotaState::Ok { .. }), "账号只读查询未成功");
        if let QuotaState::Ok { windows, account, .. } = state {
            println!("ZCode read-only: windows={}, subscription_status={}", windows.len(), account.unwrap().subscription_status);
        }
    }

    #[test]
    fn selects_only_bigmodel_coding_plan_key() {
        let config = serde_json::json!({"config":{"providerConfigRules":{"providerRules":[
            {"providerId":"other","config":{"access":{"type":"zhipu-coding-plan-api-key","apiKey":"wrong"}}},
            {"providerId":"bigmodel-api","config":{"access":{"type":"zhipu-coding-plan-api-key","apiKey":"correct"}}}
        ]}}});
        assert_eq!(bigmodel_coding_key(&config), Some("correct"));
    }

    #[test]
    fn rejects_other_access_modes_and_blank_keys() {
        let oauth = serde_json::json!({"config":{"providerConfigRules":{"providerRules":[
            {"providerId":"bigmodel-api","config":{"access":{"type":"oauth","apiKey":"wrong"}}}
        ]}}});
        let blank = serde_json::json!({"config":{"providerConfigRules":{"providerRules":[
            {"providerId":"bigmodel-api","config":{"access":{"type":"zhipu-coding-plan-api-key","apiKey":" "}}}
        ]}}});
        assert_eq!(bigmodel_coding_key(&oauth), None);
        assert_eq!(bigmodel_coding_key(&blank), None);
    }

    #[test]
    fn skips_blank_matching_rule_before_valid_key() {
        let config = serde_json::json!({"config":{"providerConfigRules":{"providerRules":[
            {"providerId":"bigmodel-api","config":{"access":{"type":"zhipu-coding-plan-api-key","apiKey":" "}}},
            {"providerId":"bigmodel-api","config":{"access":{"type":"zhipu-coding-plan-api-key","apiKey":"valid"}}}
        ]}}});
        assert_eq!(bigmodel_coding_key(&config), Some("valid"));
    }

    #[test]
    #[ignore = "需要本机 ZCode BigModel Key 与网络；仅显式执行"]
    fn local_key_can_query_remaining_credits() {
        let state = local_zcode_quota();
        assert!(matches!(state, QuotaState::Ok { ref windows, .. }
            if windows.iter().any(|window| window.remaining_text.is_some())),
            "本机 Key 未能取得精确剩余积分：{state:?}");
    }
}
