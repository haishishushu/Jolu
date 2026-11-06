//! 连接归因：把一条请求指到「已存连接」上，指不出就退回平台（灵动岛「哪条连接在跑」）。
//!
//! 三档置信度，前端必须能分开看：
//!
//! - [`Confidence::Exact`]：调用方已经带 `connection_id`。只有本应用的请求路径给得出
//!   （本地代理 / 模型入口网关），写入 `requests.connection_id` 的就是它。
//! - [`Confidence::Mapped`]：靠端点事实推出来的唯一结论。三条来源：
//!   1. `~/.zcode/v2/provider_config.json` 里该 provider 规则的 `config.api.baseUrl`
//!      （规范化成 host + path 后**唯一**命中一条连接的 `base_url`）；
//!   2. `account:*` 类 provider（ZCode 的订阅 / 账号来源，如 `account:zai-start-plan`）：
//!      配置里**没有**它们的规则条目，改为按「该平台在连接表里只有一条 `auth` 连接」判断
//!      ——那条就是它，见 [`attribute_connection`] 第 3 步；
//!   3. `model` **唯一**落在某条连接保存的模型清单（`connections.models_json`）里。
//! - [`Confidence::Platform`]：其余全部情况——多命中、无匹配、缺数据、provider 未知、
//!   连接已被删除。界面按平台名展示，不显示任何连接。
//!
//! **误归因代价高于命中率**：多命中一律落回 `Platform`，绝不按 id / 创建时间「取第一条」。
//! 同一条 `base_url` 上有多个连接（同中转的多把 Key）时，用模型清单再收窄一次，
//! 收不到唯一结论同样落回 `Platform`——两者矛盾（baseUrl 指向 A/B，模型指向 C）也算多命中。
//!
//! 只读纪律：`provider_config.json` 里 `apiKey` 是明文，本模块**不解析、不缓存、不记录**
//! 任何凭据字段，只取 `providerId` 与 `config.api.baseUrl`；`base_url` 只以指纹形式进入
//! 缓存键，缓存键与诊断输出里都不出现明文地址（地址本身可能夹带 `user:pass@`，
//! [`normalize_base_url`] 只取 `host_str`，authority 里的用户信息在规范化时就被丢掉）。

use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// 归因置信度。序列化到前端是小写字符串：`exact` / `mapped` / `platform`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Exact,
    Mapped,
    Platform,
}

/// 一条请求（或一段时间）的归因结论。`Platform` 档不带连接，界面直接显示平台名。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AttributionView {
    /// 归到的已存连接 id；`Platform` 档为 `None`
    pub connection_id: Option<String>,
    /// 连接名（展示用，不含任何凭据）；`Platform` 档为 `None`
    pub name: Option<String>,
    pub confidence: Confidence,
}

impl AttributionView {
    /// 不确定：只有平台，没有任何连接。
    pub fn platform() -> Self {
        Self { connection_id: None, name: None, confidence: Confidence::Platform }
    }

    pub fn is_connection(&self) -> bool {
        self.confidence != Confidence::Platform && self.connection_id.is_some()
    }
}

/// 参与归因的连接视图：只有 id / 名称 / 平台 / 类型 / baseUrl / 模型清单，**不含 secret**。
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionRef {
    pub id: String,
    pub name: String,
    pub platform: String,
    /// `auth`（订阅 / 账号）或 `api`（自带 Key）。只有 `account:*` 规则看它。
    pub kind: String,
    pub base_url: Option<String>,
    pub models: Vec<String>,
}

/// 纯归因输入：调用方把「已知的端点事实」摆进来，函数不再回头查库或读文件。
///
/// `platform` 只给 `account:*` 规则用（判断「该平台有几条 auth 连接」）。baseUrl 与模型
/// 规则**不按平台收窄**：第三方中转常被存成 `claude` / `codex` / `custom` 平台的连接，
/// 按平台过滤会漏掉正是要认的那一条。
#[derive(Debug, Clone, Default)]
pub struct AttributionInput<'a> {
    /// 精确归属：网关 / 本地代理写入的 `requests.connection_id`
    pub connection_id: Option<&'a str>,
    /// 这条请求属于哪套工具（`zcode` / `claude` / …），只服务于 `account:*` 规则
    pub platform: Option<&'a str>,
    /// provider 规则给出的 `config.api.baseUrl`（已确认 provider 已知才可能带值）
    pub base_url: Option<&'a str>,
    pub model: Option<&'a str>,
    /// providerId 以 `account:` 开头：订阅 / 账号来源，规则表里没有它的端点条目
    pub account_provider: bool,
}

/// `account:zai-start-plan` 这类 providerId：ZCode 用它标注「走账号订阅」的来源。
pub(crate) fn is_account_provider(provider_key: &str) -> bool {
    provider_key.trim().to_ascii_lowercase().starts_with("account:")
}

/// 纯函数：不读库、不读文件、不带状态，规则全在注释里。
///
/// 顺序即优先级：精确归属 > baseUrl 唯一命中 > `account:*` 账号唯一 > 模型清单唯一 > 平台。
pub fn attribute_connection(input: &AttributionInput<'_>, connections: &[ConnectionRef]) -> AttributionView {
    // 1) 调用方已经知道是哪条连接：精确档。连接被删掉之后 id 还在，这只是「曾经归到过」，
    //    不能拿来显示一个已不存在的连接，落回平台。
    if let Some(id) = input.connection_id.map(str::trim).filter(|id| !id.is_empty()) {
        return match connections.iter().find(|c| c.id == id) {
            Some(connection) => view(connection, Confidence::Exact),
            None => AttributionView::platform(),
        };
    }
    let model = input.model.map(normalize_model).filter(|m| !m.is_empty());

    // 2) baseUrl 规范化后比对（host + path，见 normalize_base_url）。
    if let Some(key) = input.base_url.and_then(normalize_base_url) {
        let hits: Vec<&ConnectionRef> = connections
            .iter()
            .filter(|c| c.base_url.as_deref().and_then(normalize_base_url).as_deref() == Some(key.as_str()))
            .collect();
        match hits.len() {
            1 => return view(hits[0], Confidence::Mapped),
            // 同一中转存了多条连接：只有模型清单能把它收窄到唯一一条才认，否则一律平台。
            2.. => {
                let Some(model) = &model else { return AttributionView::platform() };
                let narrowed: Vec<&&ConnectionRef> =
                    hits.iter().filter(|c| c.models.iter().any(|m| &normalize_model(m) == model)).collect();
                return match narrowed.as_slice() {
                    [only] => view(only, Confidence::Mapped),
                    _ => AttributionView::platform(),
                };
            }
            0 => {}
        }
    }

    // 3) `account:*` 类 provider：provider_config.json 里没有它们的规则条目（它们指的是
    //    ZCode 自己的账号订阅，如 `account:zai-start-plan`），所以不能按「provider 未知」
    //    直接落平台——那样这类请求永远显示不出连接名。
    //
    //    判据是平台内的唯一性：该平台在连接表里只有一条 `auth`（订阅 / 账号）连接时，
    //    这条请求只可能属于它；多条 auth 连接就分不出是谁，落平台，不取第一条。
    //    `api`（自带 Key）连接不参与：`account:` 明确说的是账号订阅那条路。
    //
    //    位置：放在 baseUrl 之后、模型规则之前。地址是端点事实，比「平台只有一条账号连接」
    //    这种结构推断更硬，有地址就先按地址定；而模型清单只说明「这条连接能用这个模型」，
    //    同名模型可能同时挂在账号连接和自带 Key 连接上，比账号唯一性更弱，所以排在后面。
    if input.account_provider {
        let Some(platform) = input.platform else { return AttributionView::platform() };
        let accounts: Vec<&ConnectionRef> =
            connections.iter().filter(|c| c.platform == platform && c.kind == "auth").collect();
        return match accounts.as_slice() {
            [only] => view(only, Confidence::Mapped),
            _ => AttributionView::platform(),
        };
    }

    // 4) 模型清单唯一命中。模型名重复出现在多条连接里（官方订阅与自带 Key 同名模型）
    //    就是多命中，同样落回平台。
    let Some(model) = model else { return AttributionView::platform() };
    let hits: Vec<&ConnectionRef> = connections
        .iter()
        .filter(|c| c.models.iter().any(|m| normalize_model(m) == model))
        .collect();
    match hits.as_slice() {
        [only] => view(only, Confidence::Mapped),
        _ => AttributionView::platform(),
    }
}

fn view(connection: &ConnectionRef, confidence: Confidence) -> AttributionView {
    AttributionView {
        connection_id: Some(connection.id.clone()),
        name: Some(connection.name.clone()),
        confidence,
    }
}

/// 模型名规范化：去空白 + 小写。`GLM-5.3-FlashX` 与 `glm-5.3-flashx` 是同一个模型。
/// 自动回填按同一个键去重，两边不能各写一套。
pub(crate) fn normalize_model(model: &str) -> String {
    model.trim().to_ascii_lowercase()
}

/// baseUrl 规范化：**host + path**，忽略协议、默认端口、尾斜杠、查询串与片段。
///
/// - `user:pass@host` 里的凭据不参与比较，也不会出现在返回值里（只取 `host_str`）；
/// - 没写协议（provider 规则里常见只写 host）时按 https 解析，两边用同一套规则才不会漏配；
/// - `https://API.Example.com/V1/` → `api.example.com/V1`：主机名大小写与尾斜杠归一，
///   **路径大小写保留**（路径本身大小写敏感，`/V1` 与 `/v1` 可能是两个端点）。
pub fn normalize_base_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let candidate = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let url = url::Url::parse(&candidate).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    // host_str 已经排除 authority 里的 userinfo；端口只在显式写了的时候保留
    let port = url.port().map(|port| format!(":{port}")).unwrap_or_default();
    let path = url.path().trim_end_matches('/');
    Some(format!("{host}{port}{path}"))
}

/// base_url 指纹：缓存键里只放这个数，不放在明文地址。
fn base_fingerprint(normalized: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    hasher.finish()
}

/// 写进连接前的地址清洗：返回（展示 / 使用形式, 规范化比对键）。
///
/// 三条都丢掉：authority 里的 `user:pass@`、查询串、片段——它们都可能夹带凭据，
/// 而网关只需要协议 + 主机 + 端口 + 路径。非 http(s) 直接拒绝，避免把 `file://` /
/// `gopher://` 这类地址写进连接当成网关。没写协议时补 https，与 [`normalize_base_url`] 同源。
pub(crate) fn sanitize_base_url(raw: &str) -> Option<(String, String)> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let candidate = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let url = url::Url::parse(&candidate).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let port = url.port().map(|port| format!(":{port}")).unwrap_or_default();
    let path = url.path().trim_end_matches('/');
    Some((format!("{}://{host}{port}{path}", url.scheme()), format!("{host}{port}{path}")))
}

fn parse_models(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else { return Vec::new() };
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

/* ------------------------------------------------------------------------ */
/*  provider_config.json：providerId → api.baseUrl + 模型清单（只取这三项）    */
/* ------------------------------------------------------------------------ */

/// 一条 provider 规则。`base_url` 为 `None` = 该 provider 在配置里存在但没写
/// `config.api.baseUrl`；表里查不到这个 providerId 才算「未知 provider」，两者不能混。
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ProviderRule {
    pub base_url: Option<String>,
    /// `config.modelOrder` ∪ `config.personalModelIds`（去重、保序）。
    /// 归因本身不用它——归因只认连接上**已存**的模型清单；自动回填由
    /// `catalog_backfill` 用它补 `connections.models_json`。
    pub models: Vec<String>,
}

pub(crate) type ProviderRules = HashMap<String, ProviderRule>;

/// 文件名与 apiKey 无关：这里只关心规则里的地址与模型清单，凭据字段整体跳过。
fn provider_config_path() -> Option<PathBuf> {
    let home = crate::creds::home()?;
    Some(home.join(".zcode").join("v2").join("provider_config.json"))
}

/// 文件指纹：mtime（秒）+ 长度。文件不存在时是 `None`，将来出现即视为变化。
fn file_stamp(path: &std::path::Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some((modified.as_secs(), meta.len()))
}

/// 规则里的模型清单：`modelOrder` 是这个 provider 认得的全部模型（顺序也由它给），
/// `personalModelIds` 是用户启用过的子集，两者取并集才不会被「只启用了一个」骗到。
/// 同一模型大小写不同算同一个（与 [`normalize_model`] 同源），保留第一次出现的写法。
fn rule_models(rule: &serde_json::Value) -> Vec<String> {
    let mut models: Vec<String> = Vec::new();
    for pointer in ["/config/modelOrder", "/config/personalModelIds"] {
        let Some(items) = rule.pointer(pointer).and_then(|value| value.as_array()) else { continue };
        for item in items {
            let Some(model) = item.as_str().map(str::trim).filter(|model| !model.is_empty()) else { continue };
            if !models.iter().any(|seen| normalize_model(seen) == normalize_model(model)) {
                models.push(model.to_string());
            }
        }
    }
    models
}

fn parse_provider_rules(text: &str) -> ProviderRules {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(text) else { return ProviderRules::new() };
    let mut rules = ProviderRules::new();
    let Some(items) = doc.pointer("/config/providerConfigRules/providerRules").and_then(|v| v.as_array()) else {
        return rules;
    };
    for rule in items {
        let Some(id) = rule.get("providerId").and_then(|v| v.as_str()).map(str::trim).filter(|id| !id.is_empty()) else {
            continue;
        };
        let base = rule
            .pointer("/config/api/baseUrl")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let models = rule_models(rule);
        // 同一个 providerId 出现多次时保留最先写下的那条：顺序固定，不随 map 迭代顺序变化。
        rules.entry(id.to_string()).or_insert(ProviderRule { base_url: base, models });
    }
    rules
}

/// 进程内缓存的 provider 规则；文件指纹变了才重读。
pub(crate) fn provider_rules() -> Arc<ProviderRules> {
    static CACHE: OnceLock<Mutex<(Option<(u64, u64)>, Arc<ProviderRules>)>> = OnceLock::new();
    // 隔离验收模式不读真实用户配置（与 zcode_quota 同一条纪律）。
    if crate::validation::isolated() {
        return Arc::new(ProviderRules::new());
    }
    let Some(path) = provider_config_path() else { return Arc::new(ProviderRules::new()) };
    let stamp = file_stamp(&path);
    let cell = CACHE.get_or_init(|| Mutex::new((None, Arc::new(ProviderRules::new()))));
    let mut guard = cell.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (cached_stamp, cached_rules) = guard.clone();
    if cached_stamp == stamp {
        return cached_rules;
    }
    let rules = std::fs::read_to_string(&path).map(|text| parse_provider_rules(&text)).unwrap_or_default();
    let rules = Arc::new(rules);
    *guard = (stamp, rules.clone());
    rules
}

/* ------------------------------------------------------------------------ */
/*  连接快照 + 映射缓存                                                       */
/* ------------------------------------------------------------------------ */

/// 缓存容量：一条平台上的请求只会有几十种 (baseUrl, model) 组合，512 足够覆盖多窗口。
const CACHE_CAPACITY: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    platform: String,
    /// 规范化 baseUrl 的指纹；没有 baseUrl 时为 0
    base_fp: u64,
    /// 规范化后的模型名；没有模型时为 `None`
    model: Option<String>,
    /// `account:*` 规则：同样的 baseUrl + 模型，结论与普通 provider 不同，必须分开缓存
    account_provider: bool,
}

/// 简单 LRU：命中不重排（够用且便宜），插入排在队尾，超容量丢队头。
struct MappingCache {
    /// 连接集合签名：签名变了整表作废，避免改完连接名 / 换过 base_url 后还吃旧结论
    signature: u64,
    entries: HashMap<CacheKey, AttributionView>,
    order: VecDeque<CacheKey>,
}

impl MappingCache {
    fn empty(signature: u64) -> Self {
        Self { signature, entries: HashMap::new(), order: VecDeque::new() }
    }

    fn get_or_insert(&mut self, key: CacheKey, compute: impl FnOnce() -> AttributionView) -> AttributionView {
        if let Some(hit) = self.entries.get(&key) {
            return hit.clone();
        }
        let value = compute();
        if self.entries.len() >= CACHE_CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(key.clone(), value.clone());
        self.order.push_back(key);
        value
    }
}

fn mapping_cache() -> &'static Mutex<MappingCache> {
    static CACHE: OnceLock<Mutex<MappingCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(MappingCache::empty(0)))
}

/// 一次读取得到的连接快照：同一次归因里所有请求共用，避免逐条查库 / 重复读配置。
pub struct Snapshot {
    connections: Vec<ConnectionRef>,
    providers: Arc<ProviderRules>,
    signature: u64,
}

impl Snapshot {
    /// 取连接清单与 provider 规则。连接表很小（个位到几十行），按需读一次即可。
    pub fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut stmt =
            conn.prepare_cached("SELECT id, name, platform, kind, base_url, models_json FROM connections")?;
        let connections = stmt
            .query_map([], |r| {
                let models_json: Option<String> = r.get(5)?;
                Ok(ConnectionRef {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    platform: r.get(2)?,
                    kind: r.get(3)?,
                    base_url: r.get(4)?,
                    models: parse_models(models_json.as_deref()),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let signature = signature_of(&connections);
        Ok(Self { connections, providers: provider_rules(), signature })
    }

    /// 测试用：自备连接清单与 provider 规则，不碰真实库与真实配置文件。
    #[cfg(test)]
    pub fn new(connections: Vec<ConnectionRef>, providers: ProviderRules) -> Self {
        let signature = signature_of(&connections);
        Self { connections, providers: Arc::new(providers), signature }
    }

    /// 一条请求的归因（带缓存）。
    ///
    /// `provider_key` 是 ZCode 库里的 `model_usage.provider_id`，**不能**塞进 platform：
    /// 平台是「哪套工具」，provider 是「这套工具在用哪个端点」，两件事。
    ///
    /// provider 未知（配置里没有这条规则）直接落平台：连它打的是哪个端点都不知道，
    /// 靠同名模型猜一条连接正是要避免的误归因。
    pub fn resolve(
        &self,
        platform: &str,
        connection_id: Option<&str>,
        provider_key: Option<&str>,
        model: Option<&str>,
    ) -> AttributionView {
        // 精确档不进缓存：它不依赖 baseUrl / 模型，也不该被别的行复用。
        if let Some(id) = connection_id.map(str::trim).filter(|id| !id.is_empty()) {
            return attribute_connection(
                &AttributionInput { connection_id: Some(id), platform: Some(platform), base_url: None, model, account_provider: false },
                &self.connections,
            );
        }
        let provider_key = provider_key.map(str::trim).filter(|key| !key.is_empty());
        let account_provider = provider_key.is_some_and(is_account_provider);
        let base_url = match provider_key {
            // `account:*` 走账号唯一性规则（第 3 步）。它也可能带规则（将来 ZCode 补上时），
            // 那就先按地址定，规则表里没有条目也不算 provider 未知。
            Some(key) if account_provider => self.providers.get(key).and_then(|rule| rule.base_url.as_deref()),
            Some(key) => match self.providers.get(key) {
                None => return AttributionView::platform(),
                Some(rule) => rule.base_url.as_deref(),
            },
            // 来源没有 provider 概念（Claude Code / Codex 等）：只剩模型清单这一条线索，
            // 仍然要求唯一命中，否则落平台。
            None => None,
        };
        let base_fp = base_url.and_then(normalize_base_url).map_or(0, |normalized| base_fingerprint(&normalized));
        let model = model.map(normalize_model).filter(|m| !m.is_empty());
        // `account_provider` 必须进缓存键：同一平台上，账号 provider 与普通 provider 的
        // (baseUrl, 模型) 完全一样，但结论不同（前者看账号唯一性），共用一条缓存会串答案。
        let key = CacheKey { platform: platform.to_string(), base_fp, model: model.clone(), account_provider };
        let cache = mapping_cache();
        let mut guard = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.signature != self.signature {
            *guard = MappingCache::empty(self.signature);
        }
        let connections = &self.connections;
        guard.get_or_insert(key, || {
            attribute_connection(
                &AttributionInput {
                    connection_id: None,
                    platform: Some(platform),
                    base_url,
                    model: model.as_deref(),
                    account_provider,
                },
                connections,
            )
        })
    }
}

/// 连接集合签名：id / 名称 / 平台 / 类型 / base_url / 模型清单一起算。
/// 改名、换地址、换模型、增删连接都会变，缓存随之整表作废（不用等 TTL）。
fn signature_of(connections: &[ConnectionRef]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for connection in connections {
        connection.id.hash(&mut hasher);
        connection.name.hash(&mut hasher);
        connection.platform.hash(&mut hasher);
        // kind 影响 `account:*` 规则的结论，必须进签名
        connection.kind.hash(&mut hasher);
        connection.base_url.as_deref().unwrap_or_default().hash(&mut hasher);
        connection.models.hash(&mut hasher);
    }
    connections.len().hash(&mut hasher);
    hasher.finish()
}

/// 会话级归因窗口：最近 **3 分钟**内有请求的会话才算「在跑」，避免把已经结束的那条连接
/// 长时间挂在收缩态那一行上（界面上的运行 / 最近活跃仍按后端原有窗口判定）。
///
/// 取 3 分钟的理由：一次模型调用里相邻步骤（读文件 → 改代码 → 再读 → 收尾）的间隔通常
/// 是十几秒到一两分钟，3 分钟够覆盖这些间隔；而原来的 10 分钟会把「上一轮早已结束的连接」
/// 一直留着，收缩态那一行就成了过期信息。窗口只影响展示，不影响统计（统计照旧按平台）。
pub(crate) const SESSION_ATTRIBUTION_WINDOW_MS: i64 = 3 * 60 * 1_000;

/// 一次读出这些会话「各自最新一条请求」的归因，供会话列表与平台合计共用。
///
/// 同一会话在同一时间戳上有多条请求（不同 dedup_key）且端点事实不一致时按多命中处理，
/// 该会话落平台——不挑其中一条当代表。
pub(crate) fn session_attribution(
    conn: &Connection,
    platform: &str,
    since_ms: i64,
    snapshot: &Snapshot,
) -> rusqlite::Result<HashMap<String, AttributionView>> {
    let mut stmt = conn.prepare_cached(
        "SELECT r.session_id, r.connection_id, r.provider_key, r.model
         FROM requests r
         WHERE r.platform = ?1 AND r.ts >= ?2 AND r.session_id IS NOT NULL
           AND r.ts = (SELECT MAX(x.ts) FROM requests x
                        WHERE x.platform = r.platform AND x.session_id = r.session_id)",
    )?;
    let rows = stmt
        .query_map(params![platform, since_ms], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut facts: HashMap<String, Vec<(Option<String>, Option<String>, Option<String>)>> = HashMap::new();
    for (session, connection_id, provider_key, model) in rows {
        facts.entry(session).or_default().push((connection_id, provider_key, model));
    }
    Ok(facts
        .into_iter()
        .map(|(session, rows)| {
            let view = match rows.as_slice() {
                [(connection_id, provider_key, model)] => {
                    snapshot.resolve(platform, connection_id.as_deref(), provider_key.as_deref(), model.as_deref())
                }
                // 同一会话同一时刻出现互相矛盾的端点事实：不猜
                _ => AttributionView::platform(),
            };
            (session, view)
        })
        .collect())
}

/// 平台合计：窗口内所有「最近一条请求」都指向同一条连接才算归到它，否则落平台。
///
/// 这是收缩态那一行的数据来源。多会话跑在不同连接上时，一句话说不清「哪条连接在跑」，
/// 与其显示其中一条，不如显示平台名。没有任何会话 / 会话都缺数据时同样是平台档。
pub(crate) fn aggregate(per_session: &HashMap<String, AttributionView>) -> AttributionView {
    let mut distinct: Option<&AttributionView> = None;
    for view in per_session.values() {
        if !view.is_connection() {
            return AttributionView::platform();
        }
        match distinct {
            None => distinct = Some(view),
            Some(current) if current.connection_id == view.connection_id => {}
            // 两个会话各跑一条连接：平台档
            Some(_) => return AttributionView::platform(),
        }
    }
    distinct.cloned().unwrap_or_else(AttributionView::platform)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 默认的连接视图：`auth`（订阅 / 账号），`account:*` 规则只认这一种。
    fn connection(id: &str, name: &str, platform: &str, base_url: Option<&str>, models: &[&str]) -> ConnectionRef {
        ConnectionRef {
            id: id.into(),
            name: name.into(),
            platform: platform.into(),
            kind: "auth".into(),
            base_url: base_url.map(str::to_owned),
            models: models.iter().map(|m| (*m).to_string()).collect(),
        }
    }

    /// 自带 Key 的连接：`account:*` 规则不把它算作账号连接。
    fn api_connection(id: &str, name: &str, platform: &str, base_url: Option<&str>, models: &[&str]) -> ConnectionRef {
        ConnectionRef { kind: "api".into(), ..connection(id, name, platform, base_url, models) }
    }

    fn providers(items: &[(&str, Option<&str>)]) -> ProviderRules {
        items
            .iter()
            .map(|(id, base)| ((*id).to_string(), ProviderRule { base_url: base.map(str::to_owned), models: Vec::new() }))
            .collect()
    }

    fn snapshot(connections: Vec<ConnectionRef>, rules: ProviderRules) -> Snapshot {
        Snapshot::new(connections, rules)
    }

    /// 网关 / 本地代理带 connection_id：精确档，且优先级高于任何地址推断。
    #[test]
    fn gateway_exact_wins_over_endpoint_facts() {
        let s = snapshot(
            vec![
                connection("c-a", "连接 A", "zcode", Some("https://api.example.test"), &[]),
                connection("c-b", "连接 B", "zcode", None, &["glm-5.3"]),
            ],
            providers(&[("relay", Some("https://api.example.test"))]),
        );
        let exact = s.resolve("zcode", Some("c-b"), Some("relay"), Some("glm-5.3"));
        assert_eq!(exact.connection_id.as_deref(), Some("c-b"));
        assert_eq!(exact.confidence, Confidence::Exact);
        assert_eq!(exact.name.as_deref(), Some("连接 B"));
    }

    /// 连接已删除：`connection_id` 还留在历史行上，但不能拿来显示一个不存在的连接。
    #[test]
    fn deleted_connection_falls_back_to_platform() {
        let s = snapshot(vec![connection("c-a", "连接 A", "zcode", None, &[])], providers(&[]));
        let gone = s.resolve("zcode", Some("c-gone"), None, None);
        assert_eq!(gone, AttributionView::platform());
    }

    /// baseUrl 唯一命中（大小写 / 尾斜杠 / 带路径都按同一套规范化比对）。
    #[test]
    fn unique_base_url_maps_to_connection() {
        let s = snapshot(
            vec![
                connection("c-hualong", "华龙", "custom", Some("https://API.Hualong.Online/"), &[]),
                connection("c-other", "其它", "custom", Some("https://other.example.test/v1"), &[]),
            ],
            providers(&[("new-provider", Some("api.hualong.online"))]),
        );
        let view = s.resolve("zcode", None, Some("new-provider"), Some("claude-opus-5-5"));
        assert_eq!(view.connection_id.as_deref(), Some("c-hualong"));
        assert_eq!(view.confidence, Confidence::Mapped);

        // 带路径的地址：`/v1/` 与 `/v1` 同一条
        let s = snapshot(
            vec![connection("c-path", "带路径", "custom", Some("https://relay.example.test/v1"), &[])],
            providers(&[("p", Some("https://RELAY.example.test/v1/"))]),
        );
        assert_eq!(s.resolve("zcode", None, Some("p"), None).connection_id.as_deref(), Some("c-path"));

        // 路径不同就是另一条端点，不许命中
        let s = snapshot(
            vec![connection("c-path", "带路径", "custom", Some("https://relay.example.test/v2"), &[])],
            providers(&[("p", Some("https://relay.example.test/v1"))]),
        );
        assert_eq!(s.resolve("zcode", None, Some("p"), None), AttributionView::platform());
    }

    /// 同一中转上存了多条连接：模型清单能收窄到唯一一条才算 Mapped。
    #[test]
    fn shared_base_url_narrowed_by_model() {
        let connections = vec![
            connection("c-1", "华龙 1", "custom", Some("https://relay.example.test"), &["claude-opus-5-5"]),
            connection("c-2", "华龙 2", "custom", Some("https://relay.example.test"), &["claude-sonnet-5-5"]),
        ];
        let s = snapshot(connections, providers(&[("new-provider", Some("https://relay.example.test"))]));
        assert_eq!(s.resolve("zcode", None, Some("new-provider"), Some("claude-opus-5-5")).connection_id.as_deref(), Some("c-1"));
        // 模型不在任何一条的清单里（或没有模型）：多命中，落平台
        assert_eq!(s.resolve("zcode", None, Some("new-provider"), Some("claude-haiku-5")).confidence, Confidence::Platform);
        assert_eq!(s.resolve("zcode", None, Some("new-provider"), None).confidence, Confidence::Platform);
    }

    /// 模型唯一命中（provider 已知但没有 baseUrl）：按模型清单归因，大小写不敏感。
    #[test]
    fn unique_model_maps_to_connection() {
        let s = snapshot(
            vec![
                connection("c-flash", "Flash 账号", "zcode", None, &["GLM-5.3-Flash"]),
                connection("c-opus", "Opus 账号", "zcode", None, &["claude-opus-5-5"]),
            ],
            providers(&[("deepseek", None)]),
        );
        let view = s.resolve("zcode", None, Some("deepseek"), Some("glm-5.3-flash"));
        assert_eq!(view.connection_id.as_deref(), Some("c-flash"));
        assert_eq!(view.confidence, Confidence::Mapped);
    }

    /// 模型多命中：落平台，不按 id 顺序挑第一条。
    #[test]
    fn model_multi_hit_stays_platform() {
        let s = snapshot(
            vec![
                connection("c-a", "账号 A", "zcode", None, &["GLM-5.3"]),
                connection("c-b", "账号 B", "zcode", None, &["GLM-5.3"]),
            ],
            providers(&[("zai-api-2", None)]),
        );
        assert_eq!(s.resolve("zcode", None, Some("zai-api-2"), Some("GLM-5.3")), AttributionView::platform());
    }

    /// provider 未知（配置里没有这条规则）：两档映射都不做，直接落平台。
    /// `account:*` 不是「未知 provider」，见下面那组账号规则测试。
    #[test]
    fn unknown_provider_stays_platform() {
        let s = snapshot(
            vec![connection("c-a", "账号 A", "zcode", None, &["GLM-5.3"])],
            providers(&[("zai-api-2", None)]),
        );
        // 模型清单里明明有这条模型，但 provider 本身不认识 → 不猜
        let view = s.resolve("zcode", None, Some("zai-unlisted-provider"), Some("GLM-5.3"));
        assert_eq!(view, AttributionView::platform());
    }

    /// 决定一：`account:*` 类 provider（ZCode 的账号订阅）没有规则条目，按平台内唯一
    /// `auth` 连接认。单连接命中 → Mapped。
    #[test]
    fn account_provider_maps_to_the_only_auth_connection() {
        let s = snapshot(
            vec![
                connection("c-acct", "Zcode 账号", "zcode", None, &[]),
                // 自带 Key 的连接不算账号连接：有它也不影响结论
                api_connection("c-key", "自带 Key", "zcode", Some("https://relay.example.test"), &["GLM-5.3"]),
            ],
            ProviderRules::new(),
        );
        let view = s.resolve("zcode", None, Some("account:zai-start-plan"), Some("GLM-5.3"));
        assert_eq!(view.connection_id.as_deref(), Some("c-acct"));
        assert_eq!(view.name.as_deref(), Some("Zcode 账号"));
        assert_eq!(view.confidence, Confidence::Mapped);
    }

    /// 决定一：平台下有**多条** auth 连接 → 分不出是哪一条，落平台，不取第一条。
    #[test]
    fn account_provider_with_several_auth_connections_stays_platform() {
        let s = snapshot(
            vec![
                connection("c-acct-1", "账号 1", "zcode", None, &[]),
                connection("c-acct-2", "账号 2", "zcode", None, &[]),
            ],
            ProviderRules::new(),
        );
        assert_eq!(s.resolve("zcode", None, Some("account:zai-start-plan"), Some("GLM-5.3")), AttributionView::platform());
    }

    /// 决定一：平台下没有连接（或只有 api 连接）→ 落平台。
    #[test]
    fn account_provider_without_auth_connection_stays_platform() {
        // 完全没有连接
        let empty = snapshot(Vec::new(), ProviderRules::new());
        assert_eq!(empty.resolve("zcode", None, Some("account:zai-start-plan"), None), AttributionView::platform());

        // 只有自带 Key 的连接：`account:` 说的是账号订阅那条路，不能认到 Key 上
        let api_only = snapshot(
            vec![api_connection("c-key", "自带 Key", "zcode", None, &["GLM-5.3"])],
            ProviderRules::new(),
        );
        assert_eq!(api_only.resolve("zcode", None, Some("account:zai-start-plan"), None), AttributionView::platform());
    }

    /// 决定一：`account:` 前缀但平台不匹配 → 落平台（按请求所属平台过滤，不跨平台借连接）。
    #[test]
    fn account_provider_on_another_platform_stays_platform() {
        let s = snapshot(
            vec![connection("c-acct", "Zcode 账号", "zcode", None, &[])],
            ProviderRules::new(),
        );
        // 请求来自 claude，但手上的账号连接是 zcode 的
        assert_eq!(s.resolve("claude", None, Some("account:zai-start-plan"), None), AttributionView::platform());
        // 反过来也必须成立：跨平台唯一不代表就是它
        let other = snapshot(
            vec![connection("c-claude", "Claude 账号", "claude", None, &[])],
            ProviderRules::new(),
        );
        assert_eq!(other.resolve("zcode", None, Some("account:zai-start-plan"), None), AttributionView::platform());
        // 平台内确实只有那一条时仍然是 Mapped（对照组）
        assert_eq!(other.resolve("claude", None, Some("account:zai-start-plan"), None).connection_id.as_deref(), Some("c-claude"));
    }

    /// provider_config.json 缺失 / 解析不出来：规则表为空，等于所有 provider 都未知。
    #[test]
    fn missing_provider_config_stays_platform() {
        let s = snapshot(
            vec![connection("c-a", "账号 A", "zcode", Some("https://relay.example.test"), &["glm-5.3"])],
            ProviderRules::new(),
        );
        assert_eq!(s.resolve("zcode", None, Some("new-provider"), Some("glm-5.3")), AttributionView::platform());
    }

    /// 来源没有 provider 概念：只剩模型清单这一条线索，要求唯一命中。
    #[test]
    fn providerless_rows_use_model_list_only() {
        let s = snapshot(
            vec![
                connection("c-1", "API 1", "claude", None, &["claude-sonnet-5-5"]),
                connection("c-2", "API 2", "claude", None, &["claude-opus-5-5"]),
            ],
            ProviderRules::new(),
        );
        assert_eq!(s.resolve("claude", None, None, Some("claude-sonnet-5-5")).connection_id.as_deref(), Some("c-1"));
        assert_eq!(s.resolve("claude", None, None, Some("claude-haiku-5")).confidence, Confidence::Platform);
        assert_eq!(s.resolve("claude", None, None, None).confidence, Confidence::Platform);
    }

    /// 规范化：协议、主机大小写、默认端口、尾斜杠、查询串都不影响比对；凭据不进结果。
    #[test]
    fn base_url_normalization() {
        let key = |raw: &str| normalize_base_url(raw).unwrap();
        // 主机名大小写、尾斜杠、默认端口、查询串、片段都不影响比对
        assert_eq!(key("https://API.Example.com/v1/"), key("api.example.com/v1"));
        assert_eq!(key("http://api.example.com"), key("https://api.example.com/"));
        assert_eq!(key("https://api.example.com:443/v1"), key("https://api.example.com/v1"));
        assert_eq!(key("https://api.example.com/v1?x=1#frag"), key("https://api.example.com/v1"));
        assert_ne!(key("https://api.example.com/v1"), key("https://api.example.com/v2"));
        assert_ne!(key("https://api.example.com"), key("https://api.example.com/v1"));
        // 路径本身大小写敏感：`/V1` 与 `/v1` 可能是两个端点，宁可漏配也不许错配
        assert_ne!(key("https://api.example.com/V1"), key("https://api.example.com/v1"));
        // authority 里的凭据不参与比较，也不会出现在返回的键里
        let with_secret = key("https://user:pw@api.example.com/v1");
        assert_eq!(with_secret, key("https://api.example.com/v1"));
        assert!(!with_secret.contains("user") && !with_secret.contains("pw"));
        assert_eq!(normalize_base_url("   "), None);
    }

    /// 写进连接前的清洗：userinfo / 查询串 / 片段全部丢掉，非 http(s) 直接拒绝。
    #[test]
    fn base_url_sanitizing_drops_credentials() {
        let (shown, normalized) =
            sanitize_base_url("https://user:pw@API.Example.com:8443/v1/?token=secret#frag").unwrap();
        assert_eq!(shown, "https://api.example.com:8443/v1");
        assert_eq!(normalized, "api.example.com:8443/v1");
        assert!(!shown.contains("user") && !shown.contains("pw") && !shown.contains("secret"));
        // 没写协议时补 https（provider 规则里常见只写主机）
        assert_eq!(sanitize_base_url("api.example.com").unwrap().0, "https://api.example.com");
        // 非 http(s) 不当网关地址
        assert_eq!(sanitize_base_url("file:///etc/passwd"), None);
        assert_eq!(sanitize_base_url("   "), None);
    }

    /// 规则解析：只取 baseUrl 与模型清单（`modelOrder` ∪ `personalModelIds`，去重保序）；
    /// `apiKey` 之类凭据字段一个都不进规则表。
    #[test]
    fn provider_rules_parse_base_url_and_model_lists_only() {
        let text = r#"{"config":{"providerConfigRules":{"providerRules":[
            {"providerId":"new-provider","config":{
                "api":{"type":"anthropic-messages","baseUrl":"https://relay.example.test/v1/"},
                "modelOrder":["claude-opus-5-5","claude-sonnet-5-5"],
                "personalModelIds":["claude-sonnet-5-5"],
                "access":{"type":"api-key","apiKey":"sk-fixture-secret"}}},
            {"providerId":"deepseek","config":{
                "modelOrder":["deepseek-v4-pro"],
                "personalModelIds":["deepseek-v4-pro","deepseek-flash"]}},
            {"providerId":"zai-api-2","config":{
                "modelOrder":["GLM-5.3"],"personalModelIds":["glm-5.3","GLM-5.3-FlashX"]}},
            {"providerId":"   ","config":{}},
            {"config":{}}
        ]}}}"#;
        let rules = parse_provider_rules(text);
        assert_eq!(rules.len(), 3, "空 providerId / 缺 providerId 的规则不占位");
        assert_eq!(rules["new-provider"].base_url.as_deref(), Some("https://relay.example.test/v1/"));
        assert_eq!(rules["new-provider"].models, vec!["claude-opus-5-5", "claude-sonnet-5-5"]);
        assert_eq!(rules["deepseek"].base_url, None, "规则存在但没写 baseUrl");
        assert_eq!(rules["deepseek"].models, vec!["deepseek-v4-pro", "deepseek-flash"]);
        // 大小写不同算同一模型，保留第一次出现的写法
        assert_eq!(rules["zai-api-2"].models, vec!["GLM-5.3", "GLM-5.3-FlashX"]);
        let debug = format!("{rules:?}");
        assert!(!debug.contains("sk-fixture-secret") && !debug.contains("apiKey"));
    }

    /// 决定二：归因窗口从 10 分钟改到 3 分钟。边界按 `ts >= since` 判：刚好 3 分钟前的
    /// 那条仍算在内，再早 1 毫秒就出窗。
    #[test]
    fn attribution_window_is_three_minutes_with_inclusive_lower_edge() {
        assert_eq!(SESSION_ATTRIBUTION_WINDOW_MS, 180_000);
        assert_eq!(SESSION_ATTRIBUTION_WINDOW_MS / 60_000, 3);

        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        for (session, ts) in [
            ("in", now - 179_000),
            ("edge", now - SESSION_ATTRIBUTION_WINDOW_MS),
            ("out", now - SESSION_ATTRIBUTION_WINDOW_MS - 1),
        ] {
            conn.execute(
                "INSERT INTO requests(platform,source,dedup_key,session_id,ts,model,provider_key)
                 VALUES('zcode','native:zcode:main',?1,?2,?3,'GLM-5.3','account:zai-start-plan')",
                rusqlite::params![format!("r-{session}"), format!("s-{session}"), ts],
            )
            .unwrap();
        }
        let s = snapshot(vec![connection("c-acct", "Zcode 账号", "zcode", None, &[])], ProviderRules::new());
        let since = now - SESSION_ATTRIBUTION_WINDOW_MS;
        let views = session_attribution(&conn, "zcode", since, &s).unwrap();
        let mut keys: Vec<&str> = views.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["s-edge", "s-in"], "窗口外的会话不进归因表");
        assert!(views.values().all(|v| v.connection_id.as_deref() == Some("c-acct")));
    }

    /// 缓存：同签名下命中直接复用；改过连接（签名变化）后旧结论作废。
    #[test]
    fn mapping_cache_tracks_connection_signature() {
        let before = snapshot(
            vec![connection("c-a", "账号 A", "zcode", None, &["GLM-5.3"])],
            providers(&[("deepseek", None)]),
        );
        assert_eq!(before.resolve("zcode", None, Some("deepseek"), Some("GLM-5.3")).connection_id.as_deref(), Some("c-a"));
        // 同签名：直接吃缓存
        assert_eq!(before.resolve("zcode", None, Some("deepseek"), Some("GLM-5.3")).connection_id.as_deref(), Some("c-a"));

        // 连接改名 / 换模型后签名变化，缓存整表作废：同名模型的另一条连接不再被旧结论顶掉
        let after = snapshot(
            vec![
                connection("c-a", "账号 A", "zcode", None, &["GLM-5.3"]),
                connection("c-b", "账号 B", "zcode", None, &["GLM-5.3"]),
            ],
            providers(&[("deepseek", None)]),
        );
        assert_eq!(after.resolve("zcode", None, Some("deepseek"), Some("GLM-5.3")), AttributionView::platform());
    }

    /// 真机对照表（只读）：本机 ZCode 库里的每个 provider → 归到的连接 → 置信度。
    ///
    /// `cargo test --manifest-path backend/Cargo.toml attribution -- --ignored --nocapture`
    /// 只打印 providerId、归因到的连接 id 与连接名前 6 个字符，不打印任何地址与凭据。
    #[test]
    #[ignore]
    fn report_local_providers() {
        let Some(home) = crate::creds::home() else { panic!("无法定位用户目录") };
        let zcode_db = home.join(".zcode").join("cli").join("db").join("db.sqlite");
        let input = Connection::open_with_flags(&zcode_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("打开 ZCode 库失败");
        let mut stmt = input
            .prepare("SELECT provider_id, COUNT(*), GROUP_CONCAT(DISTINCT model_id) FROM model_usage GROUP BY provider_id ORDER BY provider_id")
            .expect("读取 provider 列表失败");
        let providers: Vec<(String, i64, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<String>>(2)?.unwrap_or_default())))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        drop(stmt);
        drop(input);

        let jolu_db = std::env::var_os("APPDATA")
            .map(|appdata| PathBuf::from(appdata).join("dev.ningz.jolu").join("usage.db"))
            .filter(|path| path.is_file())
            .expect("未找到本机 Jolu 数据库");
        let conn = Connection::open_with_flags(&jolu_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let snapshot = Snapshot::load(&conn).unwrap();
        println!("Jolu 已存连接 {} 条", snapshot.connections.len());
        for connection in &snapshot.connections {
            // 只脱敏到前 6 个字符，地址一律不打印
            let short: String = connection.name.chars().take(6).collect();
            println!(
                "  连接 {} / {} / 平台 {} / 类型 {} / 有 baseUrl {} / 模型 {} 条",
                connection.id,
                short,
                connection.platform,
                connection.kind,
                connection.base_url.is_some(),
                connection.models.len()
            );
        }
        println!();
        for (provider, count, models) in providers {
            let first_model = models.split(',').next();
            let view = snapshot.resolve("zcode", None, Some(&provider), first_model);
            let target = match (&view.connection_id, &view.name) {
                (Some(id), Some(name)) => format!("{id} / {}", name.chars().take(6).collect::<String>()),
                _ => "（无连接，按平台）".to_string(),
            };
            println!(
                "{} → {} → {:?}\n    原因：{}（{} 行；模型 {}）",
                provider,
                target,
                view.confidence,
                explain(&snapshot, "zcode", &provider, first_model),
                count,
                models
            );
        }
    }

    /// 为什么是这个结论：把每一步的判定事实打出来，便于人眼核对有没有错认。
    /// 不打印地址原文，只说「有几条连接用的是同一个端点」。
    fn explain(snapshot: &Snapshot, platform: &str, provider: &str, model: Option<&str>) -> String {
        // `account:*`：结论只看「该平台有几条 auth 连接」
        if is_account_provider(provider) {
            let auths = snapshot.connections.iter().filter(|c| c.platform == platform && c.kind == "auth").count();
            return match snapshot.providers.get(provider) {
                Some(rule) if rule.base_url.is_some() => {
                    format!("account:* provider；注释里说它没有规则条目，这里却有 baseUrl（{} 条 {platform} auth 连接）", auths)
                }
                _ => format!("account:* provider（provider_config 里没有它的端点规则）：{platform} 下有 {auths} 条 auth 连接"),
            };
        }
        let base = match snapshot.providers.get(provider) {
            None => return format!("provider_config 里没有 `{provider}` 这条规则（provider 未知）"),
            Some(rule) => rule.base_url.clone(),
        };
        let mut notes = Vec::new();
        match base.as_deref().and_then(normalize_base_url) {
            None => notes.push("该 provider 没写 api.baseUrl".to_string()),
            Some(key) => {
                let hits: Vec<&ConnectionRef> = snapshot
                    .connections
                    .iter()
                    .filter(|c| c.base_url.as_deref().and_then(normalize_base_url).as_deref() == Some(key.as_str()))
                    .collect();
                if hits.is_empty() {
                    notes.push("已存连接里没有一条 base_url 与它一致".to_string());
                } else {
                    notes.push(format!("{} 条连接共用这个 base_url", hits.len()));
                }
            }
        }
        let model = model.map(normalize_model);
        let model_hits = model.as_ref().map_or(0, |model| {
            snapshot
                .connections
                .iter()
                .filter(|c| c.models.iter().any(|m| normalize_model(m) == *model))
                .count()
        });
        notes.push(format!("模型落在 {model_hits} 条连接的清单里"));
        notes.join("；")
    }
}
