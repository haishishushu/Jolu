//! 路由流水：每次网关请求一行，给「统一网关」页展示最近去向。
//!
//! 只记元数据：时间、agent、请求模型、路由到的来源与连接、已验证的调用方会话、状态码、等待上游响应的耗时。
//! 不记提示词、不记凭证。流式响应的耗时是「到响应头」的时间，不是首字时间，界面按「耗时」展示。
//!
//! 只保留最近约 1000 行（分批清理，可能暂时超过），分页统计只代表当前保留范围。

use rusqlite::{params, Connection};

/// 保留行数；清理每 50 次写执行一次。
const KEEP_ROWS: i64 = 1000;

#[derive(Debug, Clone, serde::Serialize)]
pub struct RouteLogEntry {
    /// 自增流水 id：分页的稳定行标识与唯一次级排序键
    pub id: i64,
    pub ts: i64,
    pub agent: Option<String>,
    pub requested_model: String,
    pub source: Option<String>,
    pub upstream_model: Option<String>,
    pub status_code: Option<i64>,
    /// ok / rejected / failed
    pub result: String,
    pub duration_ms: Option<i64>,
    pub connection_id: Option<String>,
    pub session_platform: Option<String>,
    pub session_id: Option<String>,
}

pub struct NewEntry<'a> {
    pub agent: Option<&'a str>,
    pub requested_model: &'a str,
    pub source: Option<&'a str>,
    pub upstream_model: Option<&'a str>,
    pub status_code: Option<i64>,
    pub result: &'a str,
    pub duration_ms: Option<i64>,
    pub connection_id: Option<&'a str>,
    pub session_platform: Option<&'a str>,
    pub session_id: Option<&'a str>,
}

pub fn record(conn: &Connection, entry: NewEntry<'_>) {
    let _ = conn.execute(
        "INSERT INTO gateway_route_log(ts, agent, requested_model, source, upstream_model, status_code, result, duration_ms,
                                       connection_id, session_platform, session_id)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            chrono::Local::now().timestamp_millis(),
            entry.agent,
            entry.requested_model,
            entry.source,
            entry.upstream_model,
            entry.status_code,
            entry.result,
            entry.duration_ms,
            entry.connection_id,
            entry.session_platform,
            entry.session_id,
        ],
    );
    // 只保留最近 1000 行；每 50 次写清理一次，避免每条请求都做一次 NOT IN 子查询。
    use std::sync::atomic::{AtomicU32, Ordering};
    static WRITES: AtomicU32 = AtomicU32::new(0);
    if WRITES.fetch_add(1, Ordering::Relaxed) % 50 == 49 {
        let _ = conn.execute(
            "DELETE FROM gateway_route_log WHERE id NOT IN (SELECT id FROM gateway_route_log ORDER BY id DESC LIMIT ?1)",
            [KEEP_ROWS],
        );
    }
}

const COLUMNS: &str = "id, ts, agent, requested_model, source, upstream_model, status_code, result, duration_ms,
                       connection_id, session_platform, session_id";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RouteLogEntry> {
    Ok(RouteLogEntry {
        id: r.get(0)?,
        ts: r.get(1)?,
        agent: r.get(2)?,
        requested_model: r.get(3)?,
        source: r.get(4)?,
        upstream_model: r.get(5)?,
        status_code: r.get(6)?,
        result: r.get(7)?,
        duration_ms: r.get(8)?,
        connection_id: r.get(9)?,
        session_platform: r.get(10)?,
        session_id: r.get(11)?,
    })
}

pub fn recent(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<RouteLogEntry>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM gateway_route_log ORDER BY id DESC LIMIT ?1"))?;
    let rows = stmt.query_map([limit as i64], row)?;
    rows.collect()
}

/// 一页流水与这次浏览的查询范围。
#[derive(Debug, Clone, serde::Serialize)]
pub struct RouteLogPage {
    pub rows: Vec<RouteLogEntry>,
    /// 实际返回的页码（请求页超出范围时收到最后一页；没有数据时为 1）
    pub page: i64,
    pub page_size: i64,
    /// 查询上界内、当前保留范围内的总行数
    pub total: i64,
    /// 没有数据时为 0
    pub page_count: i64,
    /// 本次浏览的查询上界（含）：翻页时带回，新写入的流水不会挤动旧页；没有数据时为 None
    pub upper_id: Option<i64>,
}

/// 分页：按 id 从大到小。`upper_id` 为 None 时以当前最大 id 建立新的查询范围。
/// 计数与列表都限定在同一上界内；保留范围清理掉的旧行会让总数变小，页码随之收拢到最后一页。
pub fn page(conn: &Connection, page: i64, page_size: i64, upper_id: Option<i64>) -> rusqlite::Result<RouteLogPage> {
    let page_size = page_size.clamp(1, 100);
    let upper = match upper_id {
        Some(id) => Some(id),
        None => conn.query_row("SELECT MAX(id) FROM gateway_route_log", [], |r| r.get::<_, Option<i64>>(0))?,
    };
    let Some(upper) = upper else {
        return Ok(RouteLogPage { rows: Vec::new(), page: 1, page_size, total: 0, page_count: 0, upper_id: None });
    };
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM gateway_route_log WHERE id <= ?1", [upper], |r| r.get(0))?;
    let page_count = (total + page_size - 1) / page_size;
    let page = page.clamp(1, page_count.max(1));
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM gateway_route_log WHERE id <= ?1 ORDER BY id DESC LIMIT ?2 OFFSET ?3"
    ))?;
    let rows = stmt
        .query_map(params![upper, page_size, (page - 1) * page_size], row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(RouteLogPage { rows, page, page_size, total, page_count, upper_id: Some(upper) })
}

/// 某个会话经网关到某个连接的历史关联（保留范围内）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionLink {
    pub session_platform: String,
    pub session_id: String,
    /// 被拒或旧流水没有连接 id 时为 None
    pub connection_id: Option<String>,
    pub source: Option<String>,
    /// 最近一次的请求模型与上游模型
    pub requested_model: String,
    pub upstream_model: Option<String>,
    pub agent: Option<String>,
    pub last_ts: i64,
    pub requests: i64,
}

/// 按（会话, 连接）聚合流水里的会话关联，最新的在前。只统计带已验证会话标识的行。
/// 模型、来源与 Agent 取每组 id 最大（最近一次）的那一行。
pub fn session_links(conn: &Connection) -> rusqlite::Result<Vec<SessionLink>> {
    let mut stmt = conn.prepare(
        "SELECT l.session_platform, l.session_id, l.connection_id, l.source, l.requested_model, l.upstream_model, l.agent,
                g.last_id, g.last_ts, g.requests
         FROM (SELECT MAX(id) AS last_id, MAX(ts) AS last_ts, COUNT(*) AS requests
               FROM gateway_route_log
               WHERE session_platform IS NOT NULL AND session_id IS NOT NULL AND session_id <> ''
               GROUP BY session_platform, session_id, connection_id) g
         JOIN gateway_route_log l ON l.id = g.last_id
         ORDER BY g.last_id DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(SessionLink {
            session_platform: r.get(0)?,
            session_id: r.get(1)?,
            connection_id: r.get(2)?,
            source: r.get(3)?,
            requested_model: r.get(4)?,
            upstream_model: r.get(5)?,
            agent: r.get(6)?,
            last_ts: r.get(8)?,
            requests: r.get(9)?,
        })
    })?;
    rows.collect()
}

/// 某个来源（连接）经网关的使用情况（保留范围内）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SourceUsage {
    pub connection_id: String,
    pub last_ts: i64,
    pub requests: i64,
    /// 其中失败的次数（被拒的请求没有连接 id，不计入）
    pub failed: i64,
}

/// 按连接汇总保留范围内的流水；旧流水没有连接 id 的不计入。
pub fn source_usage(conn: &Connection) -> rusqlite::Result<Vec<SourceUsage>> {
    let mut stmt = conn.prepare(
        "SELECT connection_id, MAX(ts), COUNT(*), COALESCE(SUM(result NOT IN ('ok', 'rejected')), 0)
         FROM gateway_route_log WHERE connection_id IS NOT NULL
         GROUP BY connection_id ORDER BY MAX(ts) DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(SourceUsage { connection_id: r.get(0)?, last_ts: r.get(1)?, requests: r.get(2)?, failed: r.get(3)? })
    })?;
    rows.collect()
}

/// 某时刻之后的请求数，按结果分组：(ok, rejected, failed)。供灵动岛「今日经网关」使用。
/// 流水只保留最近 1000 行，当天超过 1000 次时这是下限，调用方据此标注。
pub fn counts_since(conn: &Connection, since_ms: i64) -> rusqlite::Result<(i64, i64, i64)> {
    conn.query_row(
        "SELECT COALESCE(SUM(result = 'ok'), 0), COALESCE(SUM(result = 'rejected'), 0),
                COALESCE(SUM(result NOT IN ('ok', 'rejected')), 0)
         FROM gateway_route_log WHERE ts >= ?1",
        [since_ms],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(result: &str) -> NewEntry<'_> {
        NewEntry {
            agent: None, requested_model: "a/b", source: None, upstream_model: None, status_code: None, result,
            duration_ms: None, connection_id: None, session_platform: None, session_id: None,
        }
    }

    fn db() -> Connection {
        crate::db::open(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn recent_is_newest_first_and_keeps_metadata() {
        let conn = db();
        record(&conn, NewEntry { agent: Some("codex"), requested_model: "kimi/kimi-k2", source: Some("kimi"), upstream_model: Some("kimi-k2"), status_code: Some(200), result: "ok", duration_ms: Some(42), connection_id: Some("c1"), session_platform: Some("codex"), session_id: Some("s1") });
        record(&conn, NewEntry { agent: None, requested_model: "nope/m", source: None, upstream_model: None, status_code: Some(404), result: "rejected", duration_ms: None, connection_id: None, session_platform: None, session_id: None });
        let rows = recent(&conn, 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].result, "rejected");
        assert!(rows[0].id > rows[1].id);
        assert_eq!(rows[1].agent.as_deref(), Some("codex"));
        assert_eq!(rows[1].duration_ms, Some(42));
        assert_eq!(rows[1].session_id.as_deref(), Some("s1"));
        assert_eq!(rows[1].connection_id.as_deref(), Some("c1"));
    }

    #[test]
    fn counts_since_groups_by_result() {
        let conn = db();
        for result in ["ok", "ok", "rejected", "failed"] {
            record(&conn, entry(result));
        }
        assert_eq!(counts_since(&conn, 0).unwrap(), (2, 1, 1));
        assert_eq!(counts_since(&conn, i64::MAX).unwrap(), (0, 0, 0));
    }

    #[test]
    fn page_handles_empty_full_and_partial_pages() {
        let conn = db();
        let empty = page(&conn, 1, 10, None).unwrap();
        assert_eq!((empty.total, empty.page_count, empty.page, empty.rows.len()), (0, 0, 1, 0));
        assert_eq!(empty.upper_id, None);
        for count in [1usize, 9, 1] {
            for _ in 0..count {
                record(&conn, entry("ok"));
            }
            let total = conn.query_row("SELECT COUNT(*) FROM gateway_route_log", [], |r| r.get::<_, i64>(0)).unwrap();
            let first = page(&conn, 1, 10, None).unwrap();
            assert_eq!(first.total, total);
            assert_eq!(first.page_count, (total + 9) / 10);
            assert_eq!(first.rows.len() as i64, total.min(10));
        }
        // 11 条：第 2 页只有 1 条，超出范围的页收拢到最后一页
        let last = page(&conn, 2, 10, None).unwrap();
        assert_eq!((last.page, last.rows.len()), (2, 1));
        let over = page(&conn, 99, 10, None).unwrap();
        assert_eq!(over.page, 2);
    }

    #[test]
    fn upper_bound_keeps_pages_stable_while_new_rows_arrive() {
        let conn = db();
        for _ in 0..25 {
            record(&conn, entry("ok"));
        }
        let first = page(&conn, 1, 10, None).unwrap();
        let upper = first.upper_id;
        for _ in 0..7 {
            record(&conn, entry("failed"));
        }
        let second = page(&conn, 2, 10, upper).unwrap();
        let third = page(&conn, 3, 10, upper).unwrap();
        assert_eq!(second.total, 25, "新写入不进入本次浏览范围");
        let mut ids: Vec<i64> = first.rows.iter().chain(&second.rows).chain(&third.rows).map(|r| r.id).collect();
        assert_eq!(ids.len(), 25);
        ids.dedup();
        assert_eq!(ids.len(), 25, "连续翻页没有重复");
        assert!(ids.windows(2).all(|w| w[0] > w[1]), "按 id 从大到小且无遗漏");
        let refreshed = page(&conn, 1, 10, None).unwrap();
        assert_eq!(refreshed.total, 32, "显式刷新重新建立范围");
    }

    #[test]
    fn source_usage_counts_per_connection_and_skips_rows_without_one() {
        let conn = db();
        let with = |conn_id: &'static str, result: &'static str| NewEntry { connection_id: Some(conn_id), ..entry(result) };
        record(&conn, with("c1", "ok"));
        record(&conn, with("c1", "failed"));
        record(&conn, with("c2", "ok"));
        record(&conn, entry("rejected"));
        let usage = source_usage(&conn).unwrap();
        assert_eq!(usage.len(), 2);
        let c1 = usage.iter().find(|u| u.connection_id == "c1").unwrap();
        assert_eq!((c1.requests, c1.failed), (2, 1));
    }

    #[test]
    fn session_links_group_by_session_and_connection_with_latest_model() {
        let conn = db();
        let link = |model: &'static str, conn_id: &'static str, session: &'static str| NewEntry {
            agent: Some("codex"), requested_model: model, source: Some("kimi"), upstream_model: None, status_code: Some(200),
            result: "ok", duration_ms: None, connection_id: Some(conn_id), session_platform: Some("codex"), session_id: Some(session),
        };
        record(&conn, link("kimi/a", "c1", "s1"));
        record(&conn, link("kimi/b", "c1", "s1"));
        record(&conn, link("kimi/a", "c2", "s1"));
        record(&conn, link("kimi/a", "c1", "s2"));
        record(&conn, entry("ok"));
        let links = session_links(&conn).unwrap();
        assert_eq!(links.len(), 3, "没有会话标识的流水不参与关联");
        assert_eq!((links[0].session_id.as_str(), links[0].connection_id.as_deref()), ("s2", Some("c1")));
        let s1c1 = links.iter().find(|l| l.session_id == "s1" && l.connection_id.as_deref() == Some("c1")).unwrap();
        assert_eq!((s1c1.requests, s1c1.requested_model.as_str()), (2, "kimi/b"));
    }
}
