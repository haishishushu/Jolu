//! 网关路由：slug 索引、模型解析、协议匹配。
//! 会话亲和与额度感知排序在第 4 步接入；本步先完成「命中即转发」的主路径。

use std::sync::{Arc, Mutex};

use crate::connections;

/// 模型池里的一条连接（快照，含凭证——**绝不可序列化给前端或写日志**）。
/// `platform` / `model` 在第 4 步（模型映射）与第 9 步（归账）接入，先保留字段。
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct PoolEntry {
    pub connection_id: String,
    pub slug: String,
    pub platform: String,
    pub kind: String,
    /// anthropic / openai：该连接上游说的协议。
    pub protocol: String,
    /// 上游 base（无尾斜杠）；auth 连接为 None（订阅适配器自定端点）。
    pub base_url: Option<String>,
    /// 凭证原值；auth 连接为 None（订阅适配器现场读本机登录）。
    pub secret: Option<String>,
    /// 手动保存的默认模型；模型映射在第 4 步细化，先做 slug 内的直名转发。
    pub model: Option<String>,
    pub status: String,
}

/// slug 索引：连接名 → URL 安全小写 slug；重名追加序号保持唯一。
/// 与 `proxy.rs` 的 KeyIndex 同款策略：带 TTL 缓存，拿不到数据库锁时沿用旧索引。
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

/// 连接名 → slug：小写、空格转连字符、丢弃非字母数字与连字符字符。
pub(crate) fn slugify(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let collapsed = collapsed_dashes(&cleaned);
    if collapsed.is_empty() { "conn".into() } else { collapsed }
}

fn collapsed_dashes(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in s.chars() {
        if c == '-' {
            if !prev_dash && !out.is_empty() {
                out.push('-');
            }
            prev_dash = true;
        } else {
            out.push(c);
            prev_dash = false;
        }
    }
    out.trim_matches('-').to_string()
}

/// 从数据库读出模型池（serve_enabled=1 的连接）。slug 冲突时加 `-2`、`-3`。
pub(crate) fn load_pool(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<PoolEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, platform, kind, name, protocol, base_url, secret, model, status
         FROM connections WHERE serve_enabled = 1 ORDER BY created_ms ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        let platform: String = r.get(1)?;
        let kind: String = r.get(2)?;
        let name: String = r.get(3)?;
        Ok((
            r.get::<_, String>(0)?,      // id
            platform,
            kind,
            name,
            r.get::<_, Option<String>>(4)?, // protocol
            r.get::<_, Option<String>>(5)?, // base_url
            r.get::<_, Option<String>>(6)?, // secret
            r.get::<_, Option<String>>(7)?, // model
            r.get::<_, String>(8)?,      // status
        ))
    })?;
    let mut taken: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for row in rows {
        let (id, platform, kind, name, protocol, base_url, secret, model, status) = row?;
        let base_slug = slugify(&name);
        let mut slug = base_slug.clone();
        let mut n = 2;
        while taken.contains(&slug) {
            slug = format!("{base_slug}-{n}");
            n += 1;
        }
        taken.push(slug.clone());
        let protocol = connections::auth_protocol(&platform, protocol.as_deref()).to_string();
        out.push(PoolEntry {
            connection_id: id,
            slug,
            platform,
            kind,
            protocol,
            base_url: base_url.map(|b| b.trim().trim_end_matches('/').to_string()),
            secret,
            model,
            status,
        });
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

/// 一次选路的结果。
#[derive(Debug)]
pub(crate) enum RouteDecision<'a> {
    /// 命中连接：按其协议直通（direct 适配器）。
    Direct(&'a PoolEntry, String),
    /// 命中订阅连接：走订阅适配器（第 5/7 步接入，本步先报未实现）。
    Subscription(&'a PoolEntry),
    /// 无法服务：原因直接面向调用方（中文，说明下一步怎么做）。
    Reject(String),
}

/// 选路：模型 id → slug 命中 → 协议检查 → 按连接类型分发。
pub(crate) fn decide<'a>(
    entries: &'a [PoolEntry],
    model: &str,
    default_connection: Option<&str>,
) -> RouteDecision<'a> {
    let (slug, model_name) = match split_model_id(model) {
        Some(parts) => parts,
        None => {
            // 无 slug 前缀：仅当设置了默认连接才兜底
            let Some(id) = default_connection else {
                return RouteDecision::Reject(format!(
                    "模型「{model}」未带连接前缀：请使用「连接/模型」形式，或先在网关设置中选择默认连接"
                ));
            };
            let Some(entry) = entries.iter().find(|e| e.connection_id == id) else {
                return RouteDecision::Reject("默认连接已不存在，请在网关设置中重新选择".into());
            };
            return classify(entry, model.trim().to_string());
        }
    };
    let Some(entry) = entries.iter().find(|e| e.slug == slug) else {
        let known: Vec<String> = entries.iter().map(|e| e.slug.clone()).collect();
        return RouteDecision::Reject(if known.is_empty() {
            "模型池为空：先在网关中把至少一个连接设为对外服务".into()
        } else {
            format!("没有名为「{slug}」的连接；可用的连接：{}", known.join("、"))
        });
    };
    if entry.status == "paused" {
        return RouteDecision::Reject(format!("连接「{slug}」已暂停，请在供应商页恢复后再试"));
    }
    classify(entry, model_name.to_string())
}

fn classify(entry: &PoolEntry, model_name: String) -> RouteDecision<'_> {
    if entry.kind == "api" {
        if entry.base_url.as_deref().map_or(true, |b| b.is_empty()) {
            return RouteDecision::Reject(format!("连接「{}」缺少上游地址，无法直通", entry.slug));
        }
        if entry.secret.as_deref().map_or(true, |s| s.is_empty()) {
            return RouteDecision::Reject(format!("连接「{}」缺少凭证，无法直通", entry.slug));
        }
        RouteDecision::Direct(entry, model_name)
    } else {
        RouteDecision::Subscription(entry)
    }
}

/// 模型池快照（带 TTL 缓存）。锁被采集占用时沿用旧快照，与观测模式 KeyIndex 一致。
pub(crate) fn pool_snapshot(
    db: &Arc<Mutex<rusqlite::Connection>>,
    index: &Mutex<PoolIndex>,
) -> Vec<PoolEntry> {
    let now = now_ms();
    if let Ok(guard) = index.lock() {
        if now - guard.refreshed_ms < POOL_TTL_MS && !guard.entries.is_empty() || (now - guard.refreshed_ms < POOL_TTL_MS && guard.refreshed_ms > 0) {
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
            platform: "custom".into(),
            kind: kind.into(),
            protocol: protocol.into(),
            base_url: if kind == "api" { Some("https://gw.test".into()) } else { None },
            secret: if kind == "api" { Some("sk-x".into()) } else { None },
            model: None,
            status: status.into(),
        }
    }

    #[test]
    fn slugify_collapses_separators_and_lowercases() {
        assert_eq!(slugify("智谱 GLM"), "glm");
        assert_eq!(slugify("Kimi -- K2!!"), "kimi-k2");
        assert_eq!(slugify("  --  "), "conn");
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
        match decide(&pool, "kimi/kimi-k2", None) {
            RouteDecision::Direct(e, m) => {
                assert_eq!(e.slug, "kimi");
                assert_eq!(m, "kimi-k2");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(decide(&pool, "codex/gpt-5.5", None), RouteDecision::Subscription(_)));
        let err = match decide(&pool, "nope/m", None) {
            RouteDecision::Reject(msg) => msg,
            other => panic!("{other:?}"),
        };
        assert!(err.contains("kimi"), "{err}");
        // 无前缀 + 无默认连接 → 明确指引
        assert!(matches!(decide(&pool, "kimi-k2", None), RouteDecision::Reject(_)));
        // 无前缀 + 默认连接命中该连接
        match decide(&pool, "kimi-k2", Some("c-kimi")) {
            RouteDecision::Direct(_, m) => assert_eq!(m, "kimi-k2"),
            other => panic!("{other:?}"),
        }
        // 暂停的连接明确拒绝
        let paused = vec![entry("kimi", "api", "anthropic", "paused")];
        assert!(matches!(decide(&paused, "kimi/kimi-k2", None), RouteDecision::Reject(_)));
    }

    #[test]
    fn load_pool_assigns_unique_slugs_for_duplicate_names() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        for name in ["Kimi", "Kimi", "Kimi 专业版"] {
            db.execute(
                "INSERT INTO connections(id,platform,kind,name,secret,masked,base_url,protocol,serve_enabled,created_ms)
                 VALUES(lower(hex(randomblob(4))),'custom','api',?1,'k','k','https://gw.test','anthropic',1,1)",
                [name],
            )
            .unwrap();
        }
        let pool = load_pool(&db).unwrap();
        let slugs: Vec<&str> = pool.iter().map(|e| e.slug.as_str()).collect();
        assert_eq!(slugs, vec!["kimi", "kimi-2", "kimi-3"]);
        assert!(pool.iter().all(|e| e.protocol == "anthropic"));
    }
}
