//! 统一网关「模型来源 → 最近会话」的聚合数据（统一网关开发说明 §3.2–3.4、§6.1）。
//!
//! - 会话身份：平台 + 原始会话 ID，只来自请求里已验证的会话标识（见 `wire::session_identity`）；
//! - 来源关系：路由流水里按（会话, 连接）聚合的历史关联 + 当前在途请求；
//! - 显示信息与 Token：沿用会话线程的权威统计口径（可能含经其他入口的消耗，不是「仅经网关」）；
//! - 运行状态：在途请求集合由前端结合快照序号 `seq` 与实时事件维护，这里只给快照。
//!
//! 采集器还没扫到的会话照样返回，显示信息与 Token 标为缺失，不拿示例补齐。

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use serde::Serialize;

use crate::gateway::activity::LiveRequest;
use crate::gateway::route_log::SessionLink;

#[derive(Debug, Serialize)]
pub struct SessionsOverview {
    /// 在途快照对应的事件序号：前端只应用序号更大的开始 / 结束事件
    pub seq: u64,
    /// 快照时刻（毫秒）
    pub at: i64,
    pub in_flight: Vec<LiveRequest>,
    pub sessions: Vec<GatewaySession>,
}

/// Token 可用性：known = 四个分项都可信；partial = 有分项缺失；missing = 会话线程里还没有这条会话。
#[derive(Debug, Serialize)]
pub struct SessionTokens {
    pub fresh_input: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    pub output: Option<i64>,
    /// 只有四个分项都可信时才有值（会话线程已把含缓存的输入换算成普通输入，分项互斥）
    pub total: Option<i64>,
    pub state: &'static str,
}

#[derive(Debug, Serialize)]
pub struct GatewaySession {
    pub platform: String,
    pub session_id: String,
    /// 最近一次经网关的 Agent
    pub agent: Option<String>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    /// 会话线程的最近活动；还没采集到时为 None
    pub thread_last_ts: Option<i64>,
    /// 经网关的最近请求时间（保留范围内）；只有在途请求时为 None
    pub gateway_last_ts: Option<i64>,
    /// 会话线程状态：running / done / failed / recent / usage_only；还没采集到时为 None
    pub thread_status: Option<String>,
    pub tokens: SessionTokens,
    /// 历史来源关联，最近的在前
    pub links: Vec<SessionLink>,
}

fn tokens(item: Option<&crate::session_threads::SessionItem>) -> SessionTokens {
    let Some(item) = item else {
        return SessionTokens { fresh_input: None, cache_read: None, cache_write: None, output: None, total: None, state: "missing" };
    };
    let parts = [item.fresh_input, item.cache_read, item.cache_write, item.output];
    let total = parts.iter().copied().sum::<Option<i64>>();
    SessionTokens {
        fresh_input: item.fresh_input,
        cache_read: item.cache_read,
        cache_write: item.cache_write,
        output: item.output,
        total,
        state: if total.is_some() { "known" } else { "partial" },
    }
}

pub fn overview(conn: &Connection) -> rusqlite::Result<SessionsOverview> {
    // 先取在途快照：之后结束的请求会以更大的序号到达前端，不会被这份快照覆盖
    let (seq, in_flight) = crate::gateway::activity::live_snapshot();
    let at = chrono::Local::now().timestamp_millis();
    let links = crate::gateway::route_log::session_links(conn)?;

    let mut order: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut by_key: HashMap<(String, String), Vec<SessionLink>> = HashMap::new();
    for link in links {
        let key = (link.session_platform.clone(), link.session_id.clone());
        if seen.insert(key.clone()) {
            order.push(key.clone());
        }
        by_key.entry(key).or_default().push(link);
    }
    let mut live_agent: HashMap<(String, String), (i64, Option<String>)> = HashMap::new();
    for request in &in_flight {
        if let (Some(platform), Some(session)) = (&request.session_platform, &request.session_id) {
            let key = (platform.clone(), session.clone());
            if seen.insert(key.clone()) {
                order.push(key.clone());
            }
            let entry = live_agent.entry(key).or_insert((request.ts, request.agent.clone()));
            if request.ts >= entry.0 {
                *entry = (request.ts, request.agent.clone());
            }
        }
    }

    let ids: Vec<String> = order.iter().map(|(_, id)| id.clone()).collect::<HashSet<_>>().into_iter().collect();
    let filter = crate::session_threads::SessionFilter { session_ids: Some(ids), ..Default::default() };
    let threads: HashMap<(String, String), crate::session_threads::SessionItem> = crate::session_threads::list(conn, &filter)?
        .sessions
        .into_iter()
        .map(|item| ((item.platform.clone(), item.session_id.clone()), item))
        .collect();

    let mut sessions: Vec<GatewaySession> = order
        .into_iter()
        .map(|key| {
            let links = by_key.remove(&key).unwrap_or_default();
            let thread = threads.get(&key);
            let live = live_agent.get(&key);
            let agent = live.and_then(|(_, a)| a.clone()).or_else(|| links.first().and_then(|l| l.agent.clone()));
            GatewaySession {
                agent,
                title: thread.and_then(|t| t.title.clone()),
                cwd: thread.and_then(|t| t.cwd.clone()),
                thread_last_ts: thread.map(|t| t.last_ts),
                gateway_last_ts: links.iter().map(|l| l.last_ts).max(),
                thread_status: thread.map(|t| t.status.clone()),
                tokens: tokens(thread),
                links,
                platform: key.0,
                session_id: key.1,
            }
        })
        .collect();
    let activity = |s: &GatewaySession| {
        let live = in_flight
            .iter()
            .filter(|r| r.session_platform.as_deref() == Some(&s.platform) && r.session_id.as_deref() == Some(&s.session_id))
            .map(|r| r.ts)
            .max();
        [s.thread_last_ts, s.gateway_last_ts, live].into_iter().flatten().max().unwrap_or(0)
    };
    sessions.sort_by(|a, b| {
        activity(b)
            .cmp(&activity(a))
            .then_with(|| a.platform.cmp(&b.platform))
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    Ok(SessionsOverview { seq, at, in_flight, sessions })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::route_log::{record, NewEntry};

    fn link<'a>(conn_id: &'a str, platform: &'a str, session: &'a str) -> NewEntry<'a> {
        NewEntry {
            agent: Some("codex"), requested_model: "kimi/k2", source: Some("kimi"), upstream_model: Some("k2"),
            status_code: Some(200), result: "ok", duration_ms: Some(10), connection_id: Some(conn_id),
            session_platform: Some(platform), session_id: Some(session),
        }
    }

    #[test]
    fn sessions_come_from_verified_links_and_unknown_tokens_stay_missing() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        record(&conn, link("c1", "codex", "gw-test-a"));
        record(&conn, link("c2", "codex", "gw-test-a"));
        record(&conn, link("c1", "claude", "gw-test-b"));
        record(&conn, NewEntry { session_platform: None, session_id: None, ..link("c1", "codex", "x") });
        let view = overview(&conn).unwrap();
        let mine: Vec<_> = view.sessions.iter().filter(|s| s.session_id.starts_with("gw-test-")).collect();
        assert_eq!(mine.len(), 2, "没有会话标识的流水不生成会话");
        let a = mine.iter().find(|s| s.session_id == "gw-test-a").unwrap();
        assert_eq!(a.links.len(), 2, "同一会话保留多个来源关联");
        assert_eq!(a.tokens.state, "missing");
        assert_eq!(a.tokens.total, None, "还没采集到的会话不能当零");
        assert_eq!(a.thread_status, None);
        assert_eq!(a.agent.as_deref(), Some("codex"));
    }

    #[test]
    fn token_total_requires_all_parts() {
        let mut item = crate::session_threads::SessionItem {
            platform: "codex".into(), session_id: "s".into(), title: None, cwd: None, first_ts: 0, last_ts: 0, requests: 1,
            fresh_input: Some(0), cache_read: Some(0), cache_write: Some(0), output: Some(0), connection_id: None,
            status: "recent".into(), has_source: false,
        };
        let known = tokens(Some(&item));
        assert_eq!((known.total, known.state), (Some(0), "known"), "完整数据合计为零就是 0");
        item.fresh_input = Some(1000);
        item.output = Some(234);
        assert_eq!(tokens(Some(&item)).total, Some(1234));
        item.cache_write = None;
        let partial = tokens(Some(&item));
        assert_eq!((partial.total, partial.state), (None, "partial"));
    }
}
