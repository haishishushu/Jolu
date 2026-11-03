//! 连接适配器：按连接类型选择数据路径。
//!
//! - `direct`：`kind=api` 连接按 `connections.protocol` 同协议直通（第 3 步实现）
//! - `codex`：Codex 官方订阅（第 5 步实现）
//! - `claude`：Claude 官方订阅，真实二进制 + MCP 工具桥（第 7 步实现）

pub mod claude;
pub mod codex;
pub mod direct;
pub mod gemini;

use std::sync::Arc;

use crate::gateway::route::PoolEntry;
use crate::proxy::ProxyDeps;

/// 上游信号落状态（执行文档 §9.3），所有适配器共用：
/// - 429 → 内存冷却（Retry-After 优先，默认 60s，上限 1 小时）
/// - 401/403 → `status='invalid'`（不自动恢复，界面标红）
/// - 402 → `status='exhausted'`（次日零点恢复）
/// 状态码本身仍原样回传，调用方的退避/重授权逻辑依赖真实状态码。
pub(crate) fn record_upstream_signal(deps: &Arc<ProxyDeps>, entry: &PoolEntry, upstream: &reqwest::Response) {
    let status = upstream.status().as_u16();
    if status == 429 {
        let retry_after = upstream
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<i64>().ok())
            .unwrap_or(60)
            .clamp(1, 3600);
        if let Some(table) = deps.cooldowns.as_ref() {
            if let Ok(mut cooldowns) = table.lock() {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                // 按账号冷却：同一把 Key 的多个端点一起退避，不互相「接力」撞限额
                cooldowns.cool(&entry.cred_key(), now + retry_after * 1000);
            }
        }
        return;
    }
    let next_status = match status {
        401 | 403 => Some("invalid"),
        402 => Some("exhausted"),
        _ => None,
    };
    if let Some(next) = next_status {
        if let Ok(conn) = deps.db.try_lock() {
            let _ = conn.execute(
                "UPDATE connections SET status = ?2 WHERE id = ?1 AND status != 'paused'",
                rusqlite::params![entry.connection_id, next],
            );
        }
    }
}

/// 网关请求计时回填（执行文档 §9）：从响应头快照提取上游去重键，按连接归账写 `requests`。
/// 与观测模式共用 `apply_proxy_timing`，区别是连接 id 已知（不必靠凭证指纹匹配）。
/// 记录尚未入库时（CLI 还没写日志）转进待关联队列，由采集扫描后的 `flush_pending` 补写。
pub(crate) fn settle_gateway_timing(
    deps: &Arc<ProxyDeps>,
    connection_id: &str,
    platform: &str,
    head: &[u8],
    first_token_ms: i64,
    duration_ms: i64,
    status_code: i64,
) {
    let Some(dedup_key) = crate::proxy::extract_dedup_key(crate::proxy_config::ProxyPlatform::from_platform(platform), head) else {
        return;
    };
    let applied = match deps.db.try_lock() {
        Ok(conn) => {
            crate::db::apply_proxy_timing(&conn, &dedup_key, first_token_ms, duration_ms, status_code, Some(connection_id))
                .unwrap_or(0)
        }
        Err(_) => 0,
    };
    if applied == 0 {
        crate::proxy::queue_pending_timing(&dedup_key, first_token_ms, duration_ms, status_code, Some(connection_id));
    }
}

/// 订阅适配器的归账：没有 HTTP 响应头可提取去重键，由适配器直接给出唯一键。
/// `requests` 行可能尚未由采集器建立（claude 的会话日志写得晚），此时入待关联队列。
pub(crate) fn settle_subscription_timing(
    deps: &Arc<ProxyDeps>,
    connection_id: &str,
    dedup_key: &str,
    first_token_ms: i64,
    duration_ms: i64,
    status_code: i64,
) {
    let applied = match deps.db.try_lock() {
        Ok(conn) => {
            crate::db::apply_proxy_timing(&conn, dedup_key, first_token_ms, duration_ms, status_code, Some(connection_id))
                .unwrap_or(0)
        }
        Err(_) => 0,
    };
    if applied == 0 {
        crate::proxy::queue_pending_timing(dedup_key, first_token_ms, duration_ms, status_code, Some(connection_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_request(conn: &rusqlite::Connection, dedup: &str) {
        conn.execute(
            "INSERT INTO requests(platform, source, dedup_key, ts, model, total_tokens)
             VALUES('claude','local-session',?1,1,'claude-sonnet-5',100)",
            [dedup],
        )
        .unwrap();
    }

    fn connection_id_of(conn: &rusqlite::Connection, dedup: &str) -> Option<String> {
        conn.query_row(
            "SELECT connection_id FROM requests WHERE source='local-session' AND dedup_key=?1",
            [dedup],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// 归账主路径：网关请求的计时回填到对应 requests 行，并带上连接 id。
    #[test]
    fn settle_writes_timing_and_connection_id_into_requests() {
        let deps = crate::proxy::test_support::bare_deps();
        let conn = deps.db.lock().unwrap();
        insert_request(&conn, "msg_gateway_1");
        drop(conn);

        // 上游响应头快照里带 Anthropic 的 message.id
        let head = br#"{"type":"message_start","message":{"id":"msg_gateway_1"}}"#;
        settle_gateway_timing(&deps, "c-kimi", "custom", head, 120, 3400, 200);

        let conn = deps.db.lock().unwrap();
        let (first, duration, status, cid): (Option<i64>, Option<i64>, Option<i64>, Option<String>) = conn
            .query_row(
                "SELECT first_token_ms, duration_ms, status_code, connection_id FROM requests
                 WHERE source='local-session' AND dedup_key='msg_gateway_1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(first, Some(120), "首字时间要回填");
        assert_eq!(duration, Some(3400), "总时长要回填");
        assert_eq!(status, Some(200));
        assert_eq!(cid.as_deref(), Some("c-kimi"), "归账必须带连接 id");
    }

    /// 记录尚未入库（CLI 还没写日志）→ 进待关联队列，不丢计时。
    #[test]
    fn settle_queues_when_request_row_missing() {
        let deps = crate::proxy::test_support::bare_deps();
        let head = br#"{"type":"message_start","message":{"id":"msg_not_yet_logged"}}"#;
        settle_gateway_timing(&deps, "c-glm", "custom", head, 88, 1200, 200);
        let conn = deps.db.lock().unwrap();
        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM requests WHERE dedup_key='msg_not_yet_logged'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 0, "记录不在库里时不该凭空建行");
        drop(conn);
        // 队列里有待关联项（同进程内共享 PENDING）
        assert!(
            crate::proxy::pending_len() > 0,
            "应进入待关联队列，由采集扫描后的 flush_pending 补写"
        );
    }

    /// 订阅路径：用 claude session_id 作去重键归账，首字传 0（未测到，不编造）。
    #[test]
    fn settle_subscription_records_session_key_and_zero_first_token() {
        let deps = crate::proxy::test_support::bare_deps();
        let conn = deps.db.lock().unwrap();
        insert_request(&conn, "ses_claude_1");
        drop(conn);
        settle_subscription_timing(&deps, "c-claude", "ses_claude_1", 0, 4200, 200);
        let conn = deps.db.lock().unwrap();
        let (first, duration, cid): (Option<i64>, Option<i64>, Option<String>) = conn
            .query_row(
                "SELECT first_token_ms, duration_ms, connection_id FROM requests WHERE dedup_key='ses_claude_1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(first, Some(0), "首字未测到如实记 0，不编造数值");
        assert_eq!(duration, Some(4200));
        assert_eq!(cid.as_deref(), Some("c-claude"));
    }

    /// 没有去重键（响应头里解析不到 id）时不写任何东西，也不误归账。
    #[test]
    fn settle_ignores_responses_without_dedup_key() {
        let deps = crate::proxy::test_support::bare_deps();
        let conn = deps.db.lock().unwrap();
        insert_request(&conn, "msg_other");
        drop(conn);
        settle_gateway_timing(&deps, "c-kimi", "custom", b"not-json-at-all", 10, 20, 200);
        let conn = deps.db.lock().unwrap();
        assert_eq!(connection_id_of(&conn, "msg_other"), None, "无去重键时不能乱归账");
    }
}
