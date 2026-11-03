//! 路由流水：每次网关请求一行，给「模型入口」页展示最近去向。
//!
//! 只记元数据：时间、agent、请求模型、路由到的来源、状态码、等待上游响应的耗时。
//! 不记提示词、不记凭证。流式响应的耗时是「到响应头」的时间，不是首字时间，界面按「耗时」展示。

use rusqlite::{params, Connection};

#[derive(Debug, Clone, serde::Serialize)]
pub struct RouteLogEntry {
    pub ts: i64,
    pub agent: Option<String>,
    pub requested_model: String,
    pub source: Option<String>,
    pub upstream_model: Option<String>,
    pub status_code: Option<i64>,
    /// ok / rejected / failed
    pub result: String,
    pub duration_ms: Option<i64>,
}

pub struct NewEntry<'a> {
    pub agent: Option<&'a str>,
    pub requested_model: &'a str,
    pub source: Option<&'a str>,
    pub upstream_model: Option<&'a str>,
    pub status_code: Option<i64>,
    pub result: &'a str,
    pub duration_ms: Option<i64>,
}

pub fn record(conn: &Connection, entry: NewEntry<'_>) {
    let _ = conn.execute(
        "INSERT INTO gateway_route_log(ts, agent, requested_model, source, upstream_model, status_code, result, duration_ms)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            chrono::Local::now().timestamp_millis(),
            entry.agent,
            entry.requested_model,
            entry.source,
            entry.upstream_model,
            entry.status_code,
            entry.result,
            entry.duration_ms
        ],
    );
    // 只保留最近 1000 行；每 50 次写清理一次，避免每条请求都做一次 NOT IN 子查询。
    use std::sync::atomic::{AtomicU32, Ordering};
    static WRITES: AtomicU32 = AtomicU32::new(0);
    if WRITES.fetch_add(1, Ordering::Relaxed) % 50 == 49 {
        let _ = conn.execute(
            "DELETE FROM gateway_route_log WHERE id NOT IN (SELECT id FROM gateway_route_log ORDER BY id DESC LIMIT 1000)",
            [],
        );
    }
}

pub fn recent(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<RouteLogEntry>> {
    let mut stmt = conn.prepare(
        "SELECT ts, agent, requested_model, source, upstream_model, status_code, result, duration_ms
         FROM gateway_route_log ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit as i64], |r| {
        Ok(RouteLogEntry {
            ts: r.get(0)?,
            agent: r.get(1)?,
            requested_model: r.get(2)?,
            source: r.get(3)?,
            upstream_model: r.get(4)?,
            status_code: r.get(5)?,
            result: r.get(6)?,
            duration_ms: r.get(7)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_is_newest_first_and_keeps_metadata() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        record(&conn, NewEntry { agent: Some("codex"), requested_model: "kimi/kimi-k2", source: Some("kimi"), upstream_model: Some("kimi-k2"), status_code: Some(200), result: "ok", duration_ms: Some(42) });
        record(&conn, NewEntry { agent: None, requested_model: "nope/m", source: None, upstream_model: None, status_code: Some(404), result: "rejected", duration_ms: None });
        let rows = recent(&conn, 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].result, "rejected");
        assert_eq!(rows[1].agent.as_deref(), Some("codex"));
        assert_eq!(rows[1].duration_ms, Some(42));
    }
}
