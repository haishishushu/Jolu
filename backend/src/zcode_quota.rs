//! 读取本机 ZCode 的 Coding Plan API Key 与官方客户端登录，查询账号额度。
//!
//! 两条链路：
//!
//! - **Coding Plan API Key**（`provider_config.json`）：中国站仍走
//!   `coding_plan::query_plan_direct(PlanProvider::ZhipuCn, …)`。国际站复用同一把 Key
//!   打监控端点已被证伪（`zai-api-2` 那把 Key 返回业务码 1000「身份验证失败」，是 Key
//!   失效而不是站点问题；能鉴权的个人账号又返回业务码 500「当前用户不存在coding plan」），
//!   因此国际站 Key 只作为账号掩码来源，额度改走官方客户端登录。
//! - **官方客户端登录**（`credentials.json` 的 `oauth:active_provider`）：
//!   - `bigmodel`：个人订阅列表 + 监控端点的额度窗口；
//!   - `zai`：`zcodejwttoken` → `GET /api/v1/zcode-plan/billing/balance`
//!     （`Authorization: Bearer` + 一次性 `X-Device-Mid`），见
//!     `docs/verification/2026-11-05-zcode-account-balance.md`。
//!
//! 不刷新登录、不创建 Key、不兑换令牌、不使用私有 appSecret。
//! 只读纪律：**不向 `~/.zcode` 写任何文件**，令牌与设备标识只在内存里用于当次请求；
//! 令牌原文不进日志、错误文案、诊断与文档，对外只有掩码。

use crate::coding_plan::{self, PlanProvider, ZcodePlanBalanceOutcome};
use crate::quota::{QuotaState, AccountMetadata, QuotaWindow};
use aes_gcm::{Aes256Gcm, Nonce, aead::{Aead, KeyInit}};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use serde_json::Value;
use std::path::Path;

/// ZCode 账号额度端点（官方客户端同源，非公开文档 API）。
const ZCODE_BALANCE_URL: &str = "https://zcode.z.ai/api/v1/zcode-plan/billing/balance";
/// 该端点按官方客户端识别请求；UA 之外的字段都不猜。
const ZCODE_USER_AGENT: &str = "ZCode/3.14.3";

/// Coding Plan Key 的 providerId 决定它属于哪个站点：`bigmodel*` 中国站、`zai*` 国际站。
/// 其他 providerId 哪怕写了同一个 access type 也不猜（可能被用户赋予别的含义）。
fn plan_provider_of(provider_id: &str) -> Option<PlanProvider> {
    if provider_id == "bigmodel-api" || provider_id.starts_with("bigmodel") {
        Some(PlanProvider::ZhipuCn)
    } else if provider_id.starts_with("zai") {
        Some(PlanProvider::ZhipuEn)
    } else {
        None
    }
}

/// 本机第一个可用的 Coding Plan Key 及其站点；取不到时由调用方退回登录态查询。
fn coding_plan_key(config: &Value) -> Option<(PlanProvider, &str)> {
    config.pointer("/config/providerConfigRules/providerRules")?.as_array()?
        .iter()
        .filter_map(|rule| {
            let provider = plan_provider_of(rule.get("providerId").and_then(Value::as_str)?)?;
            if rule.pointer("/config/access/type").and_then(Value::as_str) != Some("zhipu-coding-plan-api-key") {
                return None;
            }
            let key = rule.pointer("/config/access/apiKey")?.as_str()?.trim();
            (!key.is_empty()).then_some((provider, key))
        })
        .next()
}

/// 账号掩码：`a***@d***.com`。只留首字母与域名后缀，原文（含邮箱、昵称、user_id、JWT）
/// 一律不出后端，也不进日志、诊断与统计导出。
fn mask_email(email: &str) -> Option<String> {
    let (local, domain) = email.trim().split_once('@')?;
    let local_head = local.trim().chars().next()?;
    let labels: Vec<&str> = domain.split('.').map(str::trim).filter(|label| !label.is_empty()).collect();
    let domain_head = labels.first()?.chars().next()?;
    let tail = if labels.len() >= 2 { format!(".{}", labels[labels.len() - 1]) } else { String::new() };
    Some(format!("{local_head}***@{domain_head}***{tail}"))
}

/// 实际用于鉴权的令牌种类。它是缓存身份的一部分：同一家族的两种令牌不能共用一份缓存。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    /// `zcodejwttoken`：ZCode 客户端签发的 JWT，账号额度端点只认它。
    ZcodeJwt,
    /// `oauth:*:access_token`：智谱 OAuth 会话，用于业务端点与登录态表达。
    OAuth,
}

impl TokenKind {
    fn marker(self) -> u8 {
        match self {
            Self::ZcodeJwt => b'j',
            Self::OAuth => b'o',
        }
    }
}

/// 本机 ZCode 登录：站点、实际使用的令牌种类与令牌、**已打码**的账号。
/// 令牌只在内存里用于查询。
#[derive(Debug, Clone, PartialEq)]
struct ZcodeAccount {
    family: String,
    token: String,
    kind: TokenKind,
    email: Option<String>,
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
    let Some((provider, key)) = coding_plan_key(&config) else {
        return local_zcode_account_quota();
    };
    match provider {
        PlanProvider::ZhipuCn => coding_plan::query_plan_direct(provider, key),
        // 国际站 Key 不接监控端点：2026-11-05 只读实测里，`zai-api-2` 这把 Key 在
        // `api.z.ai` 与 `open.bigmodel.cn` 的 `/api/monitor/usage/quota/limit` 都返回
        // HTTP 200 + 业务码 1000「身份验证失败」——失效的是这把 Key，不是站点。
        // 而 `credentials.json` 里能鉴权的 coding-plan 凭据（个人账号）在同一端点返回
        // 业务码 500「当前用户不存在coding plan」，`data.limits[]` 对该账号不通。
        // 所以国际站的额度从官方客户端令牌的 balance 端点取，见 local_zcode_account_quota。
        _ => local_zcode_account_quota(),
    }
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

/// 官方客户端加密登录用的口令：与官方 credentialCipherProvider 的公开实现一致；
/// 不尝试其他用户或秘密值。
fn credential_secret(home: &Path) -> String {
    let platform = if cfg!(windows) { "win32" } else if cfg!(target_os = "macos") { "darwin" } else { "linux" };
    let username = std::env::var(if cfg!(windows) { "USERNAME" } else { "USER" }).unwrap_or_else(|_| "unknown".into());
    std::env::var("ZCODE_CREDENTIAL_SECRET").ok().filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("zcode-credential-fallback:{platform}:{}:{username}", home.display()))
}

/// 已解密的登录信息里只取邮箱做掩码；昵称、user_id、头像、令牌原文都不返回。
fn masked_account_email(credentials: &Value, secret: &str, family: &str) -> Option<String> {
    let raw = credentials.get(&format!("oauth:{family}:user_info")).and_then(Value::as_str)?;
    let plain = decrypt_credential(raw, secret).ok()?;
    let info: Value = serde_json::from_str(&plain).ok()?;
    mask_email(info.get("email").and_then(Value::as_str)?)
}

fn read_account(home: &Path) -> Result<ZcodeAccount, QuotaState> {
    let raw = std::fs::read_to_string(home.join(".zcode/v2/credentials.json")).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            QuotaState::Unauthorized { reason: "未找到 ZCode 登录，请先在原工具登录".into() }
        } else { QuotaState::Failed { reason: "读取 ZCode 登录失败".into() } }
    })?;
    let credentials: Value = serde_json::from_str(&raw).map_err(|_| QuotaState::Failed { reason: "ZCode 登录文件格式无效".into() })?;
    let secret = credential_secret(home);
    let decode = |raw: &str| -> Result<String, QuotaState> {
        let value = decrypt_credential(raw, &secret)
            .map_err(|_| QuotaState::Failed { reason: "无法解密当前用户的 ZCode 登录，请在原工具检查登录或自定义凭据密钥".into() })?;
        if value.trim().is_empty() { return Err(QuotaState::Unauthorized { reason: "ZCode 登录不完整，请在原工具重新登录".into() }); }
        Ok(value)
    };
    let load = |key: &str| -> Result<String, QuotaState> {
        let raw = credentials.get(key).and_then(Value::as_str).filter(|raw| !raw.trim().is_empty())
            .ok_or_else(|| QuotaState::Unauthorized { reason: "ZCode 登录不完整，请在原工具重新登录".into() })?;
        decode(raw)
    };
    let family = load("oauth:active_provider")?;
    // 额度端点只认 ZCode 客户端签发的 `zcodejwttoken`；`oauth:zai:access_token`
    // 打同一端点返回 HTTP 401（2026-11-05 实测），不能拿来当额度凭证。
    let (token, kind) = match family.as_str() {
        "zai" => match credentials.get("zcodejwttoken").and_then(Value::as_str).filter(|raw| !raw.trim().is_empty()) {
            Some(raw) => (decode(raw)?, TokenKind::ZcodeJwt),
            // 客户端没写 JWT 时仍读出 OAuth 令牌，但只能用来表达登录态，额度如实写「未知」
            None => (load("oauth:zai:access_token")?, TokenKind::OAuth),
        },
        "bigmodel" => (load("oauth:bigmodel:access_token")?, TokenKind::OAuth),
        _ => return Err(QuotaState::Unsupported { reason: "当前 ZCode 登录来源暂不支持查询".into() }),
    };
    // 账号信息只用于显示掩码，读不到不影响登录态判断
    let email = masked_account_email(&credentials, &secret, &family);
    Ok(ZcodeAccount { family, token, kind, email })
}

pub(crate) fn validate_local_account_at(home: &Path) -> Result<(), String> {
    read_account(home).map(|_| ()).map_err(|state| match state {
        QuotaState::Unauthorized { reason } | QuotaState::Failed { reason } | QuotaState::Unsupported { reason } => reason,
        _ => "ZCode 登录不可用".into(),
    })
}

/// 缓存身份：家族 + 令牌种类 + **实际使用的令牌**。换账号或换令牌就换身份，
/// 否则账号卡会显示上一个账号的额度。只用于内部缓存绑定，不可写日志或返回前端。
fn identity_of(family: &str, kind: TokenKind, token: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(family.as_bytes());
    digest.update([0]);
    digest.update([kind.marker()]);
    digest.update(token.as_bytes());
    format!("{:x}", digest.finalize())
}

pub(crate) fn account_identity() -> Result<String, QuotaState> {
    if crate::validation::isolated() {
        return Err(QuotaState::Unauthorized { reason: "隔离验证模式不读取真实 ZCode 登录".into() });
    }
    let home = crate::creds::home().ok_or_else(|| QuotaState::Unauthorized { reason: "无法定位 ZCode 登录目录".into() })?;
    let account = read_account(&home)?;
    Ok(identity_of(&account.family, account.kind, &account.token))
}

/// 鉴权头形态：智谱业务端点收裸令牌，ZCode 账号额度端点只收 `Bearer`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthScheme {
    Raw,
    Bearer,
}

/// `X-Device-Mid`：官方客户端每次请求填一个 v4 UUID。这里在**内存里现生成**一个，
/// 借用仓库已有的 `rsa`（它已重导出带 `getrandom` 的 `rand_core`，不新增依赖）。
/// 绝不写 `~/.zcode` 或任何位置，也不进日志——它就是一次性的请求头。
fn device_mid() -> String {
    use rsa::rand_core::RngCore;
    let mut bytes = [0u8; 16];
    rsa::rand_core::OsRng.fill_bytes(&mut bytes);
    uuid_v4_text(bytes)
}

/// 16 字节 → 带连字符的 v4 文本（写版本位与 RFC 4122 变体位）。
fn uuid_v4_text(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// 业务响应体 → 结果（HTTP 层已确认 2xx）。
/// 业务码 401 表示令牌过期：界面要能提示「需要重新登录」，不能混成查询失败。
fn body_result(body: Value) -> Result<Value, QuotaState> {
    let code = coding_plan::business_code(body.get("code"));
    if code == Some(401) {
        return Err(QuotaState::Unauthorized { reason: "ZCode 令牌已过期，请在 ZCode 客户端重新登录".into() });
    }
    if body.get("success").and_then(Value::as_bool) == Some(false) || !matches!(code, Some(0 | 200)) {
        // 上游 msg 可能回显凭证，只保留安全的数字业务码
        return Err(match code {
            Some(_) => coding_plan::business_failure("ZCode", body.get("code")),
            // 既没有业务码也没有成功标记：来源根本没给出可判定的结果
            None => QuotaState::Failed { reason: "ZCode 账号服务未返回有效查询结果".into() },
        });
    }
    Ok(body)
}

fn account_json(
    client: &reqwest::blocking::Client,
    url: &str,
    token: &str,
    scheme: AuthScheme,
    device_mid: Option<&str>,
) -> Result<Value, QuotaState> {
    let authorization = match scheme {
        AuthScheme::Raw => token.to_string(),
        AuthScheme::Bearer => format!("Bearer {token}"),
    };
    let mut request = client.get(url).header(reqwest::header::AUTHORIZATION, authorization);
    if let Some(mid) = device_mid {
        request = request.header("X-Device-Mid", mid).header(reqwest::header::USER_AGENT, ZCODE_USER_AGENT);
    }
    let response = request.send().map_err(|_| QuotaState::Failed { reason: "ZCode 账号查询网络失败".into() })?;
    let status = response.status().as_u16();
    match status {
        // 401 空 body 是实测形态：令牌过期，需要重新登录，不是查询失败
        401 => return Err(QuotaState::Unauthorized { reason: "ZCode 登录已失效，请在原工具重新登录".into() }),
        403 => return Err(QuotaState::Forbidden { reason: "ZCode 登录没有查询权限".into() }),
        429 => return Err(QuotaState::RateLimited { reason: "ZCode 查询过于频繁，请稍后重试".into() }),
        200..=299 => (),
        _ => return Err(QuotaState::Failed { reason: "ZCode 账号服务查询失败".into() }),
    }
    let body: Value = response.json().map_err(|_| QuotaState::Failed { reason: "ZCode 账号响应格式无效".into() })?;
    body_result(body)
}

/// 订阅状态映射到界面口径（`active` / `expired` / `inactive` / `unknown`）。
/// 来源没给状态时保持 unknown，不从套餐名或到期时间推断。
fn subscription_status(status: Option<&str>) -> &'static str {
    let status = status.map(str::trim).map(str::to_ascii_lowercase);
    match status.as_deref() {
        Some("active" | "valid") => "active",
        Some("expired" | "ended") => "expired",
        Some("inactive" | "cancelled" | "canceled") => "inactive",
        _ => "unknown",
    }
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

/// 兜底：只表达登录态与账号掩码，额度如实写「未知」。
/// 走到这里意味着拿不到可用于额度查询的令牌（例如 ZCode 客户端没有写 `zcodejwttoken`），
/// 既不是查询失败，也不冒充 0%。
///
/// 「未知」在数据层的形状是三个数值字段全为 `None`（`used_percent` / `amount_text` /
/// `remaining_text`），与 `coding_plan.rs`、`grok_quota.rs`、`qoder_desktop.rs` 等既有实现一致。
/// 展示层由前端 `remainingDisplay` 统一渲染：未知 → 半角 `-`，真实的零仍是 `"0"`，
/// 查询失败 / 能力不支持 / 需要重新登录各自走 `QuotaState` 的其它分支，不会被改写成 `-`。
/// 因此这里不要写 `Some("0")`（冒充用尽），也不要把展示符号 `-` 写进数据字段。
fn login_state(account: &ZcodeAccount) -> QuotaState {
    QuotaState::Ok {
        windows: vec![QuotaWindow {
            key: "quota".into(),
            window_name: "套餐额度".into(),
            used_percent: None,
            amount_text: None,
            remaining_text: None,
            resets_at: None,
        }],
        plan: None,
        account: Some(AccountMetadata {
            // 只读登录文件无法证明订阅仍然有效，状态与来源都如实标成未知
            subscription_status: "unknown",
            subscription_source: "unavailable",
            observed_at: Some(chrono::Utc::now().to_rfc3339()),
            email: account.email.clone(),
            ..AccountMetadata::default()
        }),
    }
}

pub(crate) fn local_zcode_account_quota() -> QuotaState {
    if crate::validation::isolated() {
        return QuotaState::Unauthorized { reason: "隔离验证模式不读取真实 ZCode 登录".into() };
    }
    let Some(home) = crate::creds::home() else { return QuotaState::Unauthorized { reason: "无法定位 ZCode 登录目录".into() }; };
    let login = match read_account(&home) { Ok(value) => value, Err(state) => return state };
    let family = login.family.clone();
    let token = login.token.clone();
    let unchanged = || read_account(&home).is_ok_and(|current| current.family == family && current.token == token);
    match (login.family.as_str(), login.kind) {
        // 中国站个人订阅：订阅列表 + 监控端点额度窗口（监控端点只认这把 OAuth 令牌）
        ("bigmodel", _) => query_personal(&token, login.email.clone(), unchanged),
        // 国际站：只有 ZCode 客户端 JWT 能读账号额度
        ("zai", TokenKind::ZcodeJwt) => query_zcode_plan_balance(&token, login.email.clone(), unchanged),
        // 兜底：有登录态但没有可用的额度令牌，额度写「未知」
        _ => login_state(&login),
    }
}

/// BigModel 官方设备流返回的独立业务 token；不读取原工具登录文件。
/// 设备流只申请 `provider=bigmodel`，因此这里保持仅中国站。
pub(crate) fn authorized_quota(session: &crate::zcode_browser_auth::Session) -> QuotaState {
    if crate::validation::isolated() { return QuotaState::Unauthorized { reason: "隔离验收不查询真实 ZCode 授权".into() }; }
    query_personal(&session.access_token, session.account.clone(), || true)
}

fn query_personal(token: &str, email: Option<String>, unchanged: impl Fn() -> bool) -> QuotaState {
    let client = match crate::quota::client() { Ok(client) => client, Err(_) => return QuotaState::Failed { reason: "无法创建 ZCode 查询客户端".into() } };
    let body = match account_json(&client, "https://bigmodel.cn/api/biz/subscription/list", token, AuthScheme::Raw, None) { Ok(body) => body, Err(state) => return state };
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
    let body = match account_json(&client, "https://bigmodel.cn/api/monitor/usage/quota/limit", token, AuthScheme::Raw, None) { Ok(body) => body, Err(state) => return state };
    let Some(data) = body.get("data") else { return QuotaState::Failed { reason: "ZCode 未返回额度数据".into() }; };
    let (windows, _) = coding_plan::parse_zhipu_windows(data);
    if windows.is_empty() { return QuotaState::Failed { reason: "ZCode 未返回可识别的个人套餐额度".into() }; }
    // 查询期间切换登录时丢弃旧账号结果。
    if !unchanged() {
        return QuotaState::Failed { reason: "ZCode 登录在查询期间发生变化，请刷新".into() };
    }
    QuotaState::Ok { windows, plan: subscription.get("productName").and_then(Value::as_str).map(str::to_owned), account: Some(account) }
}

/// 响应体 → 额度状态。`code 0` 之后的三条路必须分开：
/// 有额度 / 成功但没有额度数据 / 形态不认识（查询失败）。
fn balance_state(body: &Value, email: Option<String>) -> QuotaState {
    match coding_plan::parse_zcode_plan_balance(body) {
        ZcodePlanBalanceOutcome::Malformed(reason) => QuotaState::Failed { reason },
        // 端点确实答了 code 0，所以来源是 live；但没有数据，状态与观测时间都不猜
        ZcodePlanBalanceOutcome::NoQuotaData => QuotaState::Ok {
            windows: Vec::new(),
            plan: Some("账号未返回额度数据".into()),
            account: Some(AccountMetadata {
                subscription_status: "unknown",
                subscription_source: "live",
                observed_at: None,
                email,
                ..AccountMetadata::default()
            }),
        },
        ZcodePlanBalanceOutcome::Parsed(balance) => {
            let plan = balance.plan.or_else(|| balance.windows.is_empty().then(|| "账号未返回额度窗口".to_string()));
            QuotaState::Ok {
                windows: balance.windows,
                plan,
                account: Some(AccountMetadata {
                    subscription_status: subscription_status(balance.plan_status.as_deref()),
                    expires_at: balance.plan_ends_at,
                    subscription_source: "live",
                    observed_at: balance.observed_at,
                    email,
                    ..AccountMetadata::default()
                }),
            }
        }
    }
}

/// ZCode 账号额度（`zcodejwttoken` 专属）：`Bearer` 鉴权 + 一次性 `X-Device-Mid`。
fn query_zcode_plan_balance(token: &str, email: Option<String>, unchanged: impl Fn() -> bool) -> QuotaState {
    let client = match crate::quota::client() { Ok(client) => client, Err(_) => return QuotaState::Failed { reason: "无法创建 ZCode 查询客户端".into() } };
    let mid = device_mid();
    let body = match account_json(&client, ZCODE_BALANCE_URL, token, AuthScheme::Bearer, Some(&mid)) { Ok(body) => body, Err(state) => return state };
    // 查询期间切换账号/令牌时丢弃旧账号结果。
    if !unchanged() {
        return QuotaState::Failed { reason: "ZCode 登录在查询期间发生变化，请刷新".into() };
    }
    balance_state(&body, email)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_identity_binds_family_token_and_kind() {
        let identity = identity_of("bigmodel", TokenKind::OAuth, "synthetic-a");
        assert_eq!(identity, identity_of("bigmodel", TokenKind::OAuth, "synthetic-a"));
        assert_ne!(identity, identity_of("bigmodel", TokenKind::OAuth, "synthetic-b"));
        assert_ne!(identity, identity_of("zai", TokenKind::OAuth, "synthetic-a"));
        // 同一家族的两种令牌不能共用缓存身份
        assert_ne!(identity, identity_of("bigmodel", TokenKind::ZcodeJwt, "synthetic-a"));
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

    /// 与官方封装一致的信封，供单元测试构造 `enc:v1:` 密文
    fn envelope(secret: &str, plain: &str) -> String {
        let iv = [3u8; 12];
        let cipher = Aes256Gcm::new_from_slice(&Sha256::digest(secret.as_bytes())).unwrap();
        let encrypted = cipher.encrypt(Nonce::from_slice(&iv), plain.as_bytes()).unwrap();
        let split = encrypted.len() - 16;
        format!("enc:v1:{}.{}.{}", URL_SAFE_NO_PAD.encode(iv), URL_SAFE_NO_PAD.encode(&encrypted[split..]), URL_SAFE_NO_PAD.encode(&encrypted[..split]))
    }

    /// 临时 home：`read_account` 只读 `<home>/.zcode/v2/credentials.json`，写完即删。
    fn temp_home(tag: &str) -> std::path::PathBuf {
        let stamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("jolu-zcode-quota-{tag}-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(dir.join(".zcode").join("v2")).unwrap();
        dir
    }

    fn write_credentials(home: &Path, credentials: &Value) {
        std::fs::write(home.join(".zcode").join("v2").join("credentials.json"), credentials.to_string()).unwrap();
    }

    #[test]
    fn read_account_prefers_the_zcode_client_token_for_zai() {
        let home = temp_home("zai-jwt");
        let secret = credential_secret(&home);
        let credentials = serde_json::json!({
            "oauth:active_provider": envelope(&secret, "zai"),
            "zcodejwttoken": envelope(&secret, "synthetic-zcode-jwt"),
            // 同一份登录里还有 OAuth 令牌：额度端点只认前者，不能被读成后者
            "oauth:zai:access_token": envelope(&secret, "synthetic-oauth-token"),
            "oauth:zai:user_info": envelope(&secret, &serde_json::json!({"email":"alice@z.ai","user_id":"raw-user"}).to_string()),
        });
        write_credentials(&home, &credentials);
        let account = read_account(&home).unwrap();
        assert_eq!(account.family, "zai");
        assert_eq!(account.kind, TokenKind::ZcodeJwt);
        assert_eq!(account.token, "synthetic-zcode-jwt");
        assert_eq!(account.email.as_deref(), Some("a***@z***.ai"));
        // 缓存身份跟的是实际使用的令牌，不是 OAuth 令牌
        assert_ne!(
            identity_of("zai", account.kind, &account.token),
            identity_of("zai", TokenKind::OAuth, "synthetic-oauth-token")
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn read_account_marks_zai_without_client_token_as_login_state_only() {
        let home = temp_home("zai-oauth");
        let secret = credential_secret(&home);
        write_credentials(&home, &serde_json::json!({
            "oauth:active_provider": envelope(&secret, "zai"),
            "oauth:zai:access_token": envelope(&secret, "synthetic-oauth-token"),
        }));
        let account = read_account(&home).unwrap();
        assert_eq!(account.kind, TokenKind::OAuth, "没有 zcodejwttoken 时只能表达登录态");
        assert_eq!(account.token, "synthetic-oauth-token");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn read_account_keeps_bigmodel_on_its_oauth_token_and_rejects_other_families() {
        let home = temp_home("bigmodel");
        let secret = credential_secret(&home);
        write_credentials(&home, &serde_json::json!({
            "oauth:active_provider": envelope(&secret, "bigmodel"),
            "oauth:bigmodel:access_token": envelope(&secret, "synthetic-bigmodel-token"),
        }));
        let account = read_account(&home).unwrap();
        assert_eq!((account.family.as_str(), account.kind), ("bigmodel", TokenKind::OAuth));
        assert_eq!(account.token, "synthetic-bigmodel-token");
        let _ = std::fs::remove_dir_all(&home);

        let home = temp_home("other");
        let secret = credential_secret(&home);
        write_credentials(&home, &serde_json::json!({
            "oauth:active_provider": envelope(&secret, "some-other-site"),
            "oauth:some-other-site:access_token": envelope(&secret, "synthetic-token"),
        }));
        assert!(matches!(read_account(&home), Err(QuotaState::Unsupported { .. })));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn coding_plan_key_recognises_both_sites_and_skips_unknown_ones() {
        let cn = serde_json::json!({"config":{"providerConfigRules":{"providerRules":[
            {"providerId":"other","config":{"access":{"type":"zhipu-coding-plan-api-key","apiKey":"wrong"}}},
            {"providerId":"bigmodel-api","config":{"access":{"type":"zhipu-coding-plan-api-key","apiKey":"correct"}}}
        ]}}});
        assert_eq!(coding_plan_key(&cn), Some((PlanProvider::ZhipuCn, "correct")));
        // 本机形态：Key 在 `zai-api-*` 上，站点应识别为国际站
        let intl = serde_json::json!({"config":{"providerConfigRules":{"providerRules":[
            {"providerId":"zai-api","config":{"group":"x"}},
            {"providerId":"zai-api-2","config":{"access":{"type":"zhipu-coding-plan-api-key","apiKey":"zai-key"}}}
        ]}}});
        assert_eq!(coding_plan_key(&intl), Some((PlanProvider::ZhipuEn, "zai-key")));
        assert!(plan_provider_of("bigmodel-api").is_some_and(|p| p == PlanProvider::ZhipuCn));
        assert!(plan_provider_of("zai-api-2").is_some_and(|p| p == PlanProvider::ZhipuEn));
        assert_eq!(plan_provider_of("new-provider"), None);
    }

    #[test]
    fn rejects_other_access_modes_and_blank_keys() {
        let oauth = serde_json::json!({"config":{"providerConfigRules":{"providerRules":[
            {"providerId":"bigmodel-api","config":{"access":{"type":"oauth","apiKey":"wrong"}}}
        ]}}});
        let blank = serde_json::json!({"config":{"providerConfigRules":{"providerRules":[
            {"providerId":"bigmodel-api","config":{"access":{"type":"zhipu-coding-plan-api-key","apiKey":" "}}}
        ]}}});
        assert_eq!(coding_plan_key(&oauth), None);
        assert_eq!(coding_plan_key(&blank), None);
    }

    #[test]
    fn skips_blank_matching_rule_before_valid_key() {
        let config = serde_json::json!({"config":{"providerConfigRules":{"providerRules":[
            {"providerId":"bigmodel-api","config":{"access":{"type":"zhipu-coding-plan-api-key","apiKey":" "}}},
            {"providerId":"bigmodel-api","config":{"access":{"type":"zhipu-coding-plan-api-key","apiKey":"valid"}}}
        ]}}});
        assert_eq!(coding_plan_key(&config), Some((PlanProvider::ZhipuCn, "valid")));
    }

    #[test]
    fn email_mask_keeps_only_the_first_letter_and_tld() {
        assert_eq!(mask_email("alice@zhipu.com").as_deref(), Some("a***@z***.com"));
        assert_eq!(mask_email("lin.x@z.ai").as_deref(), Some("l***@z***.ai"));
        assert_eq!(mask_email("a@localhost").as_deref(), Some("a***@l***"));
        assert_eq!(mask_email("no-at-sign"), None);
        assert_eq!(mask_email("@zhipu.com"), None);
        assert_eq!(mask_email("alice@"), None);
        let masked = mask_email("alice@zhipu.com").unwrap();
        assert!(!masked.contains("alice") && !masked.contains("zhipu"), "掩码里不得出现原文任何一段");
    }

    #[test]
    fn user_info_yields_only_a_masked_email() {
        let secret = "synthetic-test-secret";
        let info = serde_json::json!({"user_id":"user-raw","email":"alice@zhipu.com","name":"Alice","avatar":"https://example.invalid/a.png"});
        let credentials = serde_json::json!({"oauth:zai:user_info": envelope(secret, &info.to_string())});
        let masked = masked_account_email(&credentials, secret, "zai").unwrap();
        assert_eq!(masked, "a***@z***.com");
        assert!(!masked.contains("user-raw") && !masked.contains("Alice"));
        // 缺 email 时不退回昵称或 user_id
        let no_email = serde_json::json!({"user_id":"user-raw","name":"Alice"});
        let credentials = serde_json::json!({"oauth:zai:user_info": envelope(secret, &no_email.to_string())});
        assert_eq!(masked_account_email(&credentials, secret, "zai"), None);
        // 解不开也只是拿不到掩码，不影响登录态判断
        let credentials = serde_json::json!({"oauth:zai:user_info": envelope(secret, &info.to_string())});
        assert_eq!(masked_account_email(&credentials, "wrong-secret", "zai"), None);
    }

    #[test]
    fn login_state_fallback_reports_unknown_quota_with_mask() {
        let account = ZcodeAccount { family: "zai".into(), token: "synthetic-token".into(), kind: TokenKind::OAuth, email: mask_email("alice@z.ai") };
        match login_state(&account) {
            QuotaState::Ok { windows, plan, account } => {
                assert_eq!(windows.len(), 1);
                assert_eq!(windows[0].key, "quota");
                assert_eq!(windows[0].used_percent, None, "未知额度不画进度条");
                assert_eq!(windows[0].remaining_text, None, "未知额度不写数字文案，前端渲染为 -");
                assert_eq!(windows[0].amount_text, None, "未知额度不写用量文案");
                assert_ne!(windows[0].remaining_text.as_deref(), Some("0"), "未知额度不得冒充已用尽");
                assert!(plan.is_none(), "不由登录文件推断套餐名");
                let metadata = account.expect("登录态应带账号掩码");
                assert_eq!(metadata.email.as_deref(), Some("a***@z***.ai"));
                assert_eq!((metadata.subscription_status, metadata.subscription_source), ("unknown", "unavailable"));
            }
            other => panic!("兜底登录态应为 Ok：{other:?}"),
        }
    }

    #[test]
    fn device_mid_is_a_fresh_uuid_v4_every_time() {
        // 确定性：版本位与变体位必须被写对
        assert_eq!(uuid_v4_text([0u8; 16]), "00000000-0000-4000-8000-000000000000");
        assert_eq!(uuid_v4_text([0xffu8; 16]), "ffffffff-ffff-4fff-bfff-ffffffffffff");
        let first = device_mid();
        let second = device_mid();
        assert_ne!(first, second, "每次请求都用新的设备标识");
        for mid in [&first, &second] {
            assert_eq!(mid.len(), 36);
            let groups: Vec<&str> = mid.split('-').collect();
            assert_eq!(groups.iter().map(|group| group.len()).collect::<Vec<_>>(), vec![8, 4, 4, 4, 12]);
            assert!(groups[2].starts_with('4'), "必须是 v4");
            assert!(matches!(groups[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b'), "必须是 RFC 4122 变体");
            assert!(mid.bytes().all(|byte| byte.is_ascii_hexdigit() || byte == b'-'));
        }
    }

    #[test]
    fn response_body_maps_expired_token_to_unauthorized_and_other_codes_to_failed() {
        // HTTP 200 + 业务码 401：令牌过期，界面要提示重新登录
        for body in [
            serde_json::json!({"code":401,"msg":"token expired or incorrect"}),
            serde_json::json!({"code":"401"}),
        ] {
            assert!(matches!(body_result(body), Err(QuotaState::Unauthorized { .. })));
        }
        // 个人账号没有 coding plan：code 500 是查询失败，文案只保留数字业务码
        let Err(QuotaState::Failed { reason }) = body_result(serde_json::json!({"code":500,"success":false,"msg":"当前用户不存在coding plan synthetic-secret"})) else {
            panic!("code 500 应是查询失败");
        };
        assert!(reason.contains("500") && !reason.contains("synthetic-secret"));
        // 失效 Key：业务码 1000 同样是失败，但不是「需要重新登录」
        assert!(matches!(body_result(serde_json::json!({"code":1000,"success":false,"msg":"身份验证失败。"})), Err(QuotaState::Failed { .. })));
        // 实测 balance 响应没有 success 字段，code 0 必须接受
        assert!(body_result(serde_json::json!({"code":0,"logid":"x"})).is_ok());
        // 既没有业务码也没有 success：来源没给出可判定结果，单独一条文案
        match body_result(serde_json::json!({"success":true})) {
            Err(QuotaState::Failed { reason }) => assert_eq!(reason, "ZCode 账号服务未返回有效查询结果"),
            other => panic!("缺业务码应是查询失败：{other:?}"),
        }
    }

    /// 实测量级夹具（2026-11-05）：`GLM-5.3` 300 万 token/日、`GLM-5.3-Flash` 500 万 token/日。
    fn balance_fixture() -> Value {
        serde_json::json!({
            "code": 0,
            "logid": "fixture-logid",
            "data": {
                "server_time": 1_762_339_200i64,
                "plans": [{
                    "user_plan_id": "up-1",
                    "plan_id": "zcode-v3-start-plan-0817",
                    "name": "ZCode Start Plan",
                    "status": "active",
                    "starts_at": 1_762_000_000i64,
                    "ends_at": 1_762_372_799i64,
                    "entitlements": [
                        {"entitlement_id":"ent-1","show_name":"GLM-5.3","meter":"model_usage","unit_type":"token","period":"daily","grant_units":3_000_000i64},
                        {"entitlement_id":"ent-2","show_name":"GLM-5.3-Flash","meter":"model_usage","unit_type":"token","period":"daily","grant_units":5_000_000i64}
                    ]
                }],
                "balances": [
                    {"bucket_id":"bkt-1","user_plan_id":"up-1","plan_id":"zcode-v3-start-plan-0817",
                     "entitlement_id":"ent-1","show_name":"GLM-5.3","meter":"model_usage","unit_type":"token",
                     "total_units":3_000_000i64,"used_units":0,"remaining_units":3_000_000i64,"available_units":3_000_000i64,
                     "period_start":1_762_300_000i64,"period_end":1_762_339_199i64},
                    {"bucket_id":"bkt-2","user_plan_id":"up-1","plan_id":"zcode-v3-start-plan-0817",
                     "entitlement_id":"ent-2","show_name":"GLM-5.3-Flash","meter":"model_usage","unit_type":"token",
                     "total_units":5_000_000i64,"used_units":5_000_000i64,"remaining_units":0,"available_units":0,
                     "period_start":1_762_300_000i64,"period_end":1_762_339_199i64}
                ]
            }
        })
    }

    #[test]
    fn balance_state_carries_plan_status_expiry_and_observation_time() {
        let state = balance_state(&balance_fixture(), mask_email("alice@z.ai"));
        let QuotaState::Ok { windows, plan, account } = state else { panic!("实测形态应成功") };
        assert_eq!(plan.as_deref(), Some("ZCode Start Plan"));
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].window_name, "GLM-5.3");
        assert_eq!(windows[0].used_percent, Some(0.0), "已用 0 是真实 0，不是未知");
        assert_eq!(windows[1].used_percent, Some(100.0));
        let account = account.expect("必须带账号元数据");
        assert_eq!(account.email.as_deref(), Some("a***@z***.ai"), "令牌原文与邮箱原文都不出后端");
        assert_eq!((account.subscription_status, account.subscription_source), ("active", "live"));
        assert!(account.expires_at.is_some() && account.observed_at.is_some());
        // 12 位以后的时间戳不参与格式化，只校验能被 chrono 解析
        assert!(chrono::DateTime::parse_from_rfc3339(account.expires_at.as_deref().unwrap()).is_ok());
        assert!(chrono::DateTime::parse_from_rfc3339(account.observed_at.as_deref().unwrap()).is_ok());
    }

    #[test]
    fn balance_state_distinguishes_no_data_from_failure() {
        // code 0 但 data 缺失：成功但没有额度数据，不是查询失败
        for body in [serde_json::json!({"code":0}), serde_json::json!({"code":0,"data":null}), serde_json::json!({"code":0,"data":{}})] {
            let state = balance_state(&body, None);
            let QuotaState::Ok { windows, plan, account } = state else { panic!("不应报错：{body}") };
            assert!(windows.is_empty());
            assert!(plan.is_some(), "无额度数据也要有一句可展示的说明");
            let account = account.expect("仍要带账号元数据");
            assert_eq!((account.subscription_status, account.subscription_source), ("unknown", "live"));
            assert_eq!(account.observed_at, None, "没有数据就不编造观测时间");
        }
        // 形态不认识 → 查询失败
        let state = balance_state(&serde_json::json!({"code":0,"data":{"balances":"nope"}}), None);
        assert!(matches!(state, QuotaState::Failed { .. }));
    }

    #[test]
    fn balance_state_maps_only_known_subscription_statuses() {
        assert_eq!(subscription_status(Some("active")), "active");
        assert_eq!(subscription_status(Some("VALID")), "active");
        assert_eq!(subscription_status(Some("expired")), "expired");
        assert_eq!(subscription_status(Some("cancelled")), "inactive");
        assert_eq!(subscription_status(Some("trial-running")), "unknown");
        assert_eq!(subscription_status(None), "unknown");
    }

    #[test]
    #[ignore = "需要本机 ZCode 登录与网络；仅显式只读执行"]
    fn local_account_readonly_smoke() {
        let state = local_zcode_account_quota();
        match state {
            QuotaState::Ok { ref windows, ref plan, ref account } => {
                println!(
                    "ZCode account read-only: windows={}, plan={:?}, status={:?}, masked_account={}",
                    windows.len(),
                    plan,
                    account.as_ref().map(|metadata| metadata.subscription_status),
                    account.as_ref().is_some_and(|a| a.email.is_some())
                );
            }
            other => panic!("账号只读查询未成功：{other:?}"),
        }
    }

    /// 真机只读冒烟：只验证 balance 解析，打印内容不含令牌原文。
    #[test]
    #[ignore = "需要本机 ZCode 登录与网络；仅显式只读执行，不打印令牌"]
    fn local_zcode_balance_readonly_smoke() {
        if crate::validation::isolated() { return; }
        let Some(home) = crate::creds::home() else { return; };
        let account = read_account(&home).expect("本机没有可用的 ZCode 登录");
        assert_eq!(account.family, "zai", "本机登录应为 zai 家族");
        assert_eq!(account.kind, TokenKind::ZcodeJwt, "本机应有 zcodejwttoken");
        let head: String = account.token.chars().take(4).collect();
        println!("ZCode balance smoke: token={head}…（{} 字符）", account.token.chars().count());
        match query_zcode_plan_balance(&account.token, account.email.clone(), || true) {
            QuotaState::Ok { windows, plan, account } => {
                for window in &windows {
                    println!(
                        "  window={} used={:?} amount={:?} remaining={:?} has_reset={}",
                        window.window_name,
                        window.used_percent,
                        window.amount_text,
                        window.remaining_text,
                        window.resets_at.is_some()
                    );
                }
                println!(
                    "ZCode balance smoke: windows={}, plan={:?}, status={:?}, expires={}, observed={}",
                    windows.len(),
                    plan,
                    account.as_ref().map(|metadata| metadata.subscription_status),
                    account.as_ref().and_then(|metadata| metadata.expires_at.clone()).is_some(),
                    account.as_ref().and_then(|metadata| metadata.observed_at.clone()).is_some()
                );
            }
            other => panic!("balance 只读查询未成功：{other:?}"),
        }
    }

    #[test]
    #[ignore = "需要本机 ZCode Key 与网络；仅显式执行"]
    fn local_key_quota_is_read_only_and_explained() {
        // 本机 Key 在 zai-api-2 上：该 Key 已被证明失效（业务码 1000），但国际站额度现在
        // 改由官方客户端令牌的 balance 端点提供，所以这里应当能拿到窗口而不是一句失败。
        match local_zcode_quota() {
            QuotaState::Ok { windows, account, .. } => {
                println!("local quota: windows={}, masked_account={}", windows.len(), account.as_ref().is_some_and(|a| a.email.is_some()));
                assert!(!windows.is_empty(), "国际站账号额度应至少有一个窗口");
            }
            other => panic!("本机 Key 查询未返回可展示状态：{other:?}"),
        }
    }
}
