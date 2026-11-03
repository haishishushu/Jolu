//! 网关路由：slug 索引、模型解析、协议匹配。
//!
//! slug 是**持久化**的菜单 id 前缀（`connections.slug`），改名不再改变它；
//! 冷却按账号指纹（`account_fp`）计数，同一把 Key 的多个端点共享冷却。
//! 路由与接线代码不读取任何分组表——分组只影响菜单顺序。

use std::sync::{Arc, Mutex};

use crate::connections;

/// 模型池里的一条连接（快照，含凭证——**绝不可序列化给前端或写日志**）。
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct PoolEntry {
    pub connection_id: String,
    /// 持久化 slug；模型 id 的前缀。
    pub slug: String,
    /// 来源名称，仅用于菜单 display_name 与错误文案。
    pub name: String,
    pub platform: String,
    pub kind: String,
    /// anthropic / openai：该连接上游说的协议。
    pub protocol: String,
    /// 上游 base（无尾斜杠）；auth 连接为 None（订阅适配器自定端点）。
    pub base_url: Option<String>,
    /// 凭证原值；auth 连接为 None（订阅适配器现场读本机登录）。
    pub secret: Option<String>,
    /// 账号指纹；没有凭证的行为 None。
    pub account_fp: Option<String>,
    /// 手动保存的默认模型。
    pub model: Option<String>,
    pub status: String,
}

impl PoolEntry {
    /// 冷却与去重的键：优先账号指纹，缺失时退回连接 id。
    pub(crate) fn cred_key(&self) -> String {
        self.account_fp
            .clone()
            .unwrap_or_else(|| format!("id:{}", self.connection_id))
    }
}

/// slug 索引缓存。
pub(crate) struct PoolIndex {
    refreshed_ms: i64,
    entries: Vec<PoolEntry>,
}

const POOL_TTL_MS: i64 = 30_000;

impl PoolIndex {
    pub(crate) fn new() -> Self {
        Self { refreshed_ms: 0, entries: Vec::new() }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 从数据库读出模型池（serve_enabled=1 的连接）。
/// slug 以列为准；列缺失（迁移未跑完）或为空时跳过并记诊断，绝不再按名字现算——
/// 现算会让改名悄悄改掉菜单 id，已经存进 agent 配置的 id 就失效了。
pub(crate) fn load_pool(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<PoolEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, slug, name, platform, kind, protocol, base_url, secret, account_fp, model, status
         FROM connections WHERE serve_enabled = 1 ORDER BY created_ms ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            PoolEntry {
                connection_id: r.get(0)?,
                slug: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                name: r.get(2)?,
                platform: r.get(3)?,
                kind: r.get(4)?,
                protocol: String::new(),
                base_url: r.get(6)?,
                secret: r.get(7)?,
                account_fp: r.get(8)?,
                model: r.get(9)?,
                status: r.get(10)?,
            },
            r.get::<_, Option<String>>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (mut entry, declared_protocol) = row?;
        if entry.slug.trim().is_empty() {
            eprintln!("[网关] 连接 {} 缺少 slug，已跳过；请重启应用完成迁移", entry.connection_id);
            continue;
        }
        entry.protocol = connections::auth_protocol(&entry.platform, declared_protocol.as_deref()).to_string();
        entry.base_url = entry.base_url.map(|b| b.trim().trim_end_matches('/').to_string());
        out.push(entry);
    }
    Ok(out)
}

/// 解析请求里的模型 id：`slug/model` → (slug, 模型名)；无斜杠时为 None。
pub(crate) fn split_model_id(model: &str) -> Option<(&str, &str)> {
    let (slug, rest) = model.split_once('/')?;
    let slug = slug.trim();
    let rest = rest.trim();
    if slug.is_empty() || rest.is_empty() {
        return None;
    }
    Some((slug, rest))
}

/// 旧 slug 的说明：命中历史记录时给出「现在叫什么 / 已删除」，而不是落到别的来源上。
pub(crate) fn historical_slug_message(conn: &rusqlite::Connection, slug: &str) -> Option<String> {
    let row: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT c.slug, c.name FROM slug_history h
             LEFT JOIN connections c ON c.id = h.connection_id
             WHERE h.slug = ?1 ORDER BY h.retired_ms DESC LIMIT 1",
            [slug],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    match row {
        Some((Some(current), Some(name))) => Some(format!(
            "没有名为「{slug}」的来源；它现在叫「{current}」（{name}），请改用「{current}/…」"
        )),
        Some(_) => Some(format!("来源「{slug}」已删除，请重新添加或在模型入口选择其他来源")),
        None => None,
    }
}

/// 会话亲和表：会话键 → 已应答的连接 id。
/// prompt cache 有前缀一致性——换连接等于缓存全失效按写重算，所以会话黏住首个应答的连接
/// （对齐 magpie 的 Stays 语义）。LRU 上限 [`AFFINITY_CAP`]，进程重启自然清空。
pub(crate) struct Affinity {
    entries: std::collections::HashMap<String, (String, i64)>,
}

pub(crate) const AFFINITY_CAP: usize = 1000;

impl Affinity {
    pub(crate) fn new() -> Self {
        Self { entries: std::collections::HashMap::new() }
    }

    pub(crate) fn get(&self, session: &str) -> Option<&str> {
        self.entries.get(session).map(|(id, _)| id.as_str())
    }

    /// 记录绑定；超上限时淘汰最久未使用的条目。
    pub(crate) fn bind(&mut self, session: &str, connection_id: &str, now: i64) {
        if self.entries.len() >= AFFINITY_CAP && !self.entries.contains_key(session) {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(session.into(), (connection_id.into(), now));
    }
}

/// 连接冷却表：按账号指纹计数（429 后短期内不再把新请求派给该账号）。
pub(crate) struct Cooldowns {
    until: std::collections::HashMap<String, i64>,
}

impl Cooldowns {
    pub(crate) fn new() -> Self {
        Self { until: std::collections::HashMap::new() }
    }

    pub(crate) fn cool(&mut self, cred_key: &str, until_ms: i64) {
        self.until.insert(cred_key.into(), until_ms);
    }

    pub(crate) fn remaining_ms(&self, cred_key: &str, now: i64) -> i64 {
        self.until.get(cred_key).map_or(0, |until| (*until - now).max(0))
    }
}

/// 拒绝原因的类型，决定返回给调用方的 HTTP 状态码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RejectKind {
    BadRequest,
    NotFound,
    Unauthorized,
    PaymentRequired,
    TooManyRequests,
    ServiceUnavailable,
}

/// 一次选路的结果。
#[derive(Debug)]
pub(crate) enum RouteDecision<'a> {
    /// 命中连接：按其协议直通（direct 适配器）。
    Direct(&'a PoolEntry, String),
    /// 命中订阅连接：走订阅适配器。
    Subscription(&'a PoolEntry),
    /// 无法服务：原因直接面向调用方（中文，说明下一步怎么做）。
    Reject { kind: RejectKind, message: String, retry_after: Option<u64> },
}

impl RouteDecision<'_> {
    fn reject(kind: RejectKind, message: impl Into<String>) -> Self {
        Self::Reject { kind, message: message.into(), retry_after: None }
    }
}

/// 旧 slug 查询：只在 slug 未命中时调用，避免每次请求都读历史表。
pub(crate) type SlugHistoryLookup<'a> = dyn Fn(&str) -> Option<String> + 'a;

/// 选路：模型 id → slug 命中 → 协议检查 → 按连接类型分发。
pub(crate) fn decide<'a>(
    entries: &'a [PoolEntry],
    model: &str,
    default_connection: Option<&str>,
    history: &SlugHistoryLookup<'_>,
) -> RouteDecision<'a> {
    let (slug, model_name) = match split_model_id(model) {
        Some(parts) => parts,
        None => {
            // 无 slug 前缀：仅当设置了默认连接才兜底
            let Some(id) = default_connection else {
                return RouteDecision::reject(
                    RejectKind::BadRequest,
                    format!(
                        "模型「{model}」未带来源前缀：请使用「来源/模型」形式，或先在模型入口设置中选择默认来源"
                    ),
                );
            };
            let Some(entry) = entries.iter().find(|e| e.connection_id == id) else {
                return RouteDecision::reject(RejectKind::ServiceUnavailable, "默认来源已不存在，请在模型入口设置中重新选择");
            };
            if let Some(reject) = status_reject(entry, None) {
                return reject;
            }
            return classify(entry, model.trim().to_string());
        }
    };
    let Some(entry) = entries.iter().find(|e| e.slug == slug) else {
        if let Some(message) = history(slug) {
            return RouteDecision::reject(RejectKind::NotFound, message);
        }
        let known: Vec<String> = entries.iter().map(|e| e.slug.clone()).collect();
        return RouteDecision::reject(
            if known.is_empty() { RejectKind::BadRequest } else { RejectKind::NotFound },
            if known.is_empty() {
                "模型池为空：先在模型入口中把至少一个来源设为对外服务".to_string()
            } else {
                format!("没有名为「{slug}」的来源；可用的来源：{}", known.join("、"))
            },
        );
    };
    if let Some(reject) = status_reject(entry, None) {
        return reject;
    }
    classify(entry, model_name.to_string())
}

/// 非 connected 状态按原因给出不同的状态码，让 agent 能正确退避或提示用户。
fn status_reject(entry: &PoolEntry, retry_after: Option<u64>) -> Option<RouteDecision<'static>> {
    let (kind, message) = match entry.status.as_str() {
        "paused" => (
            RejectKind::ServiceUnavailable,
            format!("来源「{}」已暂停，请在供应商页恢复后再试", entry.slug),
        ),
        "invalid" | "expired" => (
            RejectKind::Unauthorized,
            format!("来源「{}」的登录已失效或过期，请在供应商页重新登录", entry.slug),
        ),
        "exhausted" => (
            RejectKind::PaymentRequired,
            format!("来源「{}」的额度已用尽，请切换其他来源或等待恢复", entry.slug),
        ),
        _ => return None,
    };
    Some(RouteDecision::Reject { kind, message, retry_after })
}

fn classify(entry: &PoolEntry, model_name: String) -> RouteDecision<'_> {
    if entry.kind == "api" {
        if entry.base_url.as_deref().map_or(true, |b| b.is_empty()) {
            return RouteDecision::reject(
                RejectKind::ServiceUnavailable,
                format!("来源「{}」缺少上游地址，无法直通", entry.slug),
            );
        }
        if entry.secret.as_deref().map_or(true, |s| s.is_empty()) {
            return RouteDecision::reject(
                RejectKind::ServiceUnavailable,
                format!("来源「{}」缺少凭证，无法直通", entry.slug),
            );
        }
        RouteDecision::Direct(entry, model_name)
    } else {
        RouteDecision::Subscription(entry)
    }
}

/// 带会话亲和的选路（主入口）。
/// slug 是调用方的明确意图：换 slug = 故意换模型，亲和不覆盖它。
/// 亲和的真实作用：请求本应命中会话已绑定的连接、而该账号正处 429 冷却时，
/// **不切换**——换连接等于 prompt cache 全失效按写重算（对齐 magpie 的 Stays 语义）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn decide_with_affinity<'a>(
    entries: &'a [PoolEntry],
    model: &str,
    default_connection: Option<&str>,
    session: Option<&str>,
    affinity: &mut Affinity,
    cooldowns: &Cooldowns,
    history: &SlugHistoryLookup<'_>,
) -> RouteDecision<'a> {
    let now = now_ms();
    let decision = decide(entries, model, default_connection, history);
    let target = match &decision {
        RouteDecision::Direct(entry, _) | RouteDecision::Subscription(entry) => Some(entry),
        RouteDecision::Reject { .. } => None,
    };
    let Some(entry) = target else { return decision };
    // 冷却按账号指纹：单来源不换别家；会话已绑定时更要说清楚为什么不切换。
    let remaining = cooldowns.remaining_ms(&entry.cred_key(), now);
    if remaining > 0 {
        let seconds = remaining as u64 / 1000 + 1;
        let bound = session.is_some_and(|session| affinity.get(session) == Some(entry.connection_id.as_str()));
        let message = if bound {
            format!(
                "会话固定的来源「{}」已被限流，约 {seconds} 秒后恢复；为保留提示词缓存本会话不自动切换来源",
                entry.slug
            )
        } else {
            format!("来源「{}」正在限流，约 {seconds} 秒后恢复", entry.slug)
        };
        return RouteDecision::Reject {
            kind: RejectKind::TooManyRequests,
            message,
            retry_after: Some(seconds),
        };
    }
    if let Some(session) = session {
        // 命中即绑定（含续期）：调用方换 slug 换了连接，会话跟着走
        affinity.bind(session, &entry.connection_id, now);
    }
    decision
}

/// 模型池快照（带 TTL 缓存）。锁被采集占用时沿用旧快照，与观测模式 KeyIndex 一致。
pub(crate) fn pool_snapshot(
    db: &Arc<Mutex<rusqlite::Connection>>,
    index: &Mutex<PoolIndex>,
) -> Vec<PoolEntry> {
    let now = now_ms();
    if let Ok(guard) = index.lock() {
        if guard.refreshed_ms > 0 && now - guard.refreshed_ms < POOL_TTL_MS {
            return guard.entries.clone();
        }
    }
    if let Ok(conn) = db.try_lock() {
        if let Ok(entries) = load_pool(&conn) {
            if let Ok(mut guard) = index.lock() {
                guard.refreshed_ms = now;
                guard.entries = entries.clone();
            }
            return entries;
        }
    }
    // 拿不到锁或读取失败：返回旧快照（可能为空），下次再刷新
    if let Ok(guard) = index.lock() {
        return guard.entries.clone();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(slug: &str, kind: &str, protocol: &str, status: &str) -> PoolEntry {
        PoolEntry {
            connection_id: format!("c-{slug}"),
            slug: slug.into(),
            name: slug.into(),
            platform: "custom".into(),
            kind: kind.into(),
            protocol: protocol.into(),
            base_url: if kind == "api" { Some("https://gw.test".into()) } else { None },
            secret: if kind == "api" { Some("sk-x".into()) } else { None },
            account_fp: if kind == "api" { Some(format!("fp-{slug}")) } else { None },
            model: None,
            status: status.into(),
        }
    }

    fn no_history(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn split_model_requires_both_parts() {
        assert_eq!(split_model_id("kimi/kimi-k2"), Some(("kimi", "kimi-k2")));
        assert_eq!(split_model_id("kimi/"), None);
        assert_eq!(split_model_id("/k2"), None);
        assert_eq!(split_model_id("kimi-k2"), None);
    }

    #[test]
    fn decide_routes_direct_subscription_and_rejects() {
        let pool = vec![
            entry("kimi", "api", "anthropic", "connected"),
            entry("codex", "auth", "openai", "connected"),
        ];
        match decide(&pool, "kimi/kimi-k2", None, &no_history) {
            RouteDecision::Direct(e, m) => {
                assert_eq!(e.slug, "kimi");
                assert_eq!(m, "kimi-k2");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(decide(&pool, "codex/gpt-5.5", None, &no_history), RouteDecision::Subscription(_)));
        let err = match decide(&pool, "nope/m", None, &no_history) {
            RouteDecision::Reject { message, kind, .. } => {
                assert_eq!(kind, RejectKind::NotFound);
                message
            }
            other => panic!("{other:?}"),
        };
        assert!(err.contains("kimi"), "{err}");
        // 无前缀 + 无默认连接 → 明确指引
        assert!(matches!(
            decide(&pool, "kimi-k2", None, &no_history),
            RouteDecision::Reject { kind: RejectKind::BadRequest, .. }
        ));
        // 无前缀 + 默认连接命中该连接
        match decide(&pool, "kimi-k2", Some("c-kimi"), &no_history) {
            RouteDecision::Direct(_, m) => assert_eq!(m, "kimi-k2"),
            other => panic!("{other:?}"),
        }
        // 暂停的连接明确拒绝，返回 503
        let paused = vec![entry("kimi", "api", "anthropic", "paused")];
        assert!(matches!(
            decide(&paused, "kimi/kimi-k2", None, &no_history),
            RouteDecision::Reject { kind: RejectKind::ServiceUnavailable, .. }
        ));
        // 失效的登录返回 401，额度用尽返回 402
        let invalid = vec![entry("kimi", "api", "anthropic", "invalid")];
        assert!(matches!(
            decide(&invalid, "kimi/kimi-k2", None, &no_history),
            RouteDecision::Reject { kind: RejectKind::Unauthorized, .. }
        ));
        let exhausted = vec![entry("kimi", "api", "anthropic", "exhausted")];
        assert!(matches!(
            decide(&exhausted, "kimi/kimi-k2", None, &no_history),
            RouteDecision::Reject { kind: RejectKind::PaymentRequired, .. }
        ));
    }

    #[test]
    fn historical_lookup_explains_rename_and_deletion() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO connections(id,platform,kind,name,slug,created_ms)
             VALUES('c1','custom','api','Kimi','kimi',1)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO slug_history(slug,connection_id,account_fp,retired_ms) VALUES('old-name','c1',NULL,1)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO slug_history(slug,connection_id,account_fp,retired_ms) VALUES('gone','deleted-id',NULL,2)",
            [],
        )
        .unwrap();
        let renamed = historical_slug_message(&db, "old-name").unwrap();
        assert!(renamed.contains("现在叫「kimi」"), "{renamed}");
        let deleted = historical_slug_message(&db, "gone").unwrap();
        assert!(deleted.contains("已删除"), "{deleted}");
        assert_eq!(historical_slug_message(&db, "never-existed"), None);
    }

    #[test]
    fn old_slug_returns_404_with_explanation() {
        let pool = vec![entry("kimi", "api", "anthropic", "connected")];
        let history = |slug: &str| {
            (slug == "old").then(|| "没有名为「old」的来源；它现在叫「kimi」".to_string())
        };
        match decide(&pool, "old/kimi-k2", None, &history) {
            RouteDecision::Reject { kind: RejectKind::NotFound, message, .. } => {
                assert!(message.contains("现在叫「kimi」"), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn load_pool_reads_persisted_slug_and_fingerprint() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        for (id, name, slug) in [("c1", "Kimi", "kimi"), ("c2", "Kimi", "kimi-2")] {
            db.execute(
                "INSERT INTO connections(id,platform,kind,name,secret,masked,base_url,protocol,serve_enabled,created_ms,slug,account_fp)
                 VALUES(?1,'custom','api',?2,'k','k','https://gw.test','anthropic',1,1,?3,?4)",
                rusqlite::params![id, name, slug, format!("fp-{id}")],
            )
            .unwrap();
        }
        let pool = load_pool(&db).unwrap();
        let slugs: Vec<&str> = pool.iter().map(|e| e.slug.as_str()).collect();
        assert_eq!(slugs, vec!["kimi", "kimi-2"]);
        assert_eq!(pool[0].name, "Kimi");
        assert_eq!(pool[0].protocol, "anthropic");
    }

    #[test]
    fn affinity_follows_request_slug_and_rebinds() {
        let pool = vec![
            entry("kimi", "api", "anthropic", "connected"),
            entry("glm", "api", "anthropic", "connected"),
        ];
        let mut affinity = Affinity::new();
        let cooldowns = Cooldowns::new();
        affinity.bind("s1", "c-kimi", 100);
        // 换 slug = 调用方故意换模型：亲和不覆盖明确意图，改走新连接并重绑
        match decide_with_affinity(&pool, "glm/glm-4.6", None, Some("s1"), &mut affinity, &cooldowns, &no_history) {
            RouteDecision::Direct(e, m) => {
                assert_eq!(e.slug, "glm");
                assert_eq!(m, "glm-4.6");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(affinity.get("s1"), Some("c-glm"), "命中新连接后应重绑会话");
        // 无 slug + 默认连接兜底也照常
        match decide_with_affinity(&pool, "glm-4.6", Some("c-kimi"), Some("s1"), &mut affinity, &cooldowns, &no_history) {
            RouteDecision::Direct(e, _) => assert_eq!(e.slug, "kimi"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn affinity_rejects_instead_of_switching_when_account_rate_limited() {
        let pool = vec![
            entry("kimi", "api", "anthropic", "connected"),
            entry("glm", "api", "anthropic", "connected"),
        ];
        let mut affinity = Affinity::new();
        let mut cooldowns = Cooldowns::new();
        affinity.bind("s1", "c-kimi", 100);
        cooldowns.cool(&pool[0].cred_key(), now_ms() + 30_000);
        // 请求命中的正是会话绑定的 kimi，且 kimi 冷却中：拒绝而非悄悄切到 glm
        let decision =
            decide_with_affinity(&pool, "kimi/kimi-k2", None, Some("s1"), &mut affinity, &cooldowns, &no_history);
        let RouteDecision::Reject { kind: RejectKind::TooManyRequests, message, retry_after } = decision else {
            panic!("{decision:?}")
        };
        assert!(message.contains("限流") && message.contains("不自动切换"), "{message}");
        assert!(retry_after.is_some());
        // 未绑定该账号的会话同样按账号冷却拒绝（单来源不换别家）
        let decision =
            decide_with_affinity(&pool, "kimi/kimi-k2", None, Some("s2"), &mut affinity, &cooldowns, &no_history);
        assert!(matches!(decision, RouteDecision::Reject { kind: RejectKind::TooManyRequests, .. }));
        // 冷却过期后恢复服务
        cooldowns.cool(&pool[0].cred_key(), now_ms() - 1);
        assert!(matches!(
            decide_with_affinity(&pool, "kimi/kimi-k2", None, Some("s1"), &mut affinity, &cooldowns, &no_history),
            RouteDecision::Direct(_, _)
        ));
    }

    #[test]
    fn affinity_no_switch_guard_requires_bound_connection() {
        // 会话绑定 kimi，请求指向 glm：即使 kimi 冷却也不拦截 glm（意图明确）
        let pool = vec![
            entry("kimi", "api", "anthropic", "connected"),
            entry("glm", "api", "anthropic", "connected"),
        ];
        let mut affinity = Affinity::new();
        let mut cooldowns = Cooldowns::new();
        affinity.bind("s1", "c-kimi", 100);
        cooldowns.cool(&pool[0].cred_key(), now_ms() + 30_000);
        assert!(matches!(
            decide_with_affinity(&pool, "glm/glm-4.6", None, Some("s1"), &mut affinity, &cooldowns, &no_history),
            RouteDecision::Direct(_, _)
        ));
    }

    #[test]
    fn affinity_lru_evicts_oldest_beyond_cap() {
        let mut affinity = Affinity::new();
        for i in 0..AFFINITY_CAP {
            affinity.bind(&format!("s{i}"), "c", i as i64);
        }
        affinity.bind("newcomer", "c", AFFINITY_CAP as i64 + 10);
        assert!(affinity.get("s0").is_none(), "最旧的条目应被淘汰");
        assert_eq!(affinity.get("newcomer"), Some("c"));
        assert!(affinity.entries.len() <= AFFINITY_CAP);
    }
}
