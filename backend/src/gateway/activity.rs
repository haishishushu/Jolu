//! 网关请求的实时事件（胶囊灵动岛设计说明 §9.1）。
//!
//! - `gateway-route-start`：选路得到 Direct / Subscription 之后、交给上游适配器之前发出。
//!   被拒（Reject）在选路时就判定了，不发开始事件。
//! - `gateway-route`：每个请求结束时发出，含被拒与失败；与开始事件带同一个 `request_id`。
//!
//! `request_id` 由进程内自增生成，只在内存和事件里使用，不写库。
//! 事件只带元数据（时间、Agent、请求模型、来源 slug、上游模型、状态码、结果、耗时），
//! 不带提示词、请求体与凭证。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// 发事件的回调：生产环境转给 Tauri `emit`，测试里收集到内存。
pub(crate) type Emit = Arc<dyn Fn(&'static str, serde_json::Value) + Send + Sync>;

pub(crate) const START_EVENT: &str = "gateway-route-start";
pub(crate) const END_EVENT: &str = "gateway-route";

pub(crate) fn next_request_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct RouteStart<'a> {
    pub request_id: u64,
    pub ts: i64,
    pub agent: Option<&'a str>,
    pub requested_model: &'a str,
    pub source: &'a str,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct RouteEnd<'a> {
    pub request_id: u64,
    pub ts: i64,
    pub agent: Option<&'a str>,
    pub requested_model: &'a str,
    pub source: Option<&'a str>,
    pub upstream_model: Option<&'a str>,
    pub status_code: Option<u16>,
    /// ok / rejected / failed
    pub result: &'a str,
    pub duration_ms: i64,
}

pub(crate) fn emit_start(emit: &Emit, event: &RouteStart<'_>) {
    if let Ok(value) = serde_json::to_value(event) {
        emit(START_EVENT, value);
    }
}

pub(crate) fn emit_end(emit: &Emit, event: &RouteEnd<'_>) {
    if let Ok(value) = serde_json::to_value(event) {
        emit(END_EVENT, value);
    }
}

/// 保证每个已发出的开始事件都有结束事件：请求被取消（客户端断开、future 被丢弃）时，
/// 在 Drop 里补发一条 `failed`。正常结束时调用 [`EndGuard::finish`] 交由调用方写结果。
pub(crate) struct EndGuard {
    emit: Emit,
    request_id: u64,
    agent: Option<String>,
    requested_model: String,
    source: String,
    started: std::time::Instant,
    done: bool,
}

impl EndGuard {
    pub(crate) fn new(emit: Emit, request_id: u64, agent: Option<&str>, requested_model: &str, source: &str, started: std::time::Instant) -> Self {
        Self {
            emit,
            request_id,
            agent: agent.map(str::to_string),
            requested_model: requested_model.to_string(),
            source: source.to_string(),
            started,
            done: false,
        }
    }

    pub(crate) fn finish(mut self) {
        self.done = true;
    }
}

impl Drop for EndGuard {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        emit_end(
            &self.emit,
            &RouteEnd {
                request_id: self.request_id,
                ts: chrono::Local::now().timestamp_millis(),
                agent: self.agent.as_deref(),
                requested_model: &self.requested_model,
                source: Some(&self.source),
                upstream_model: None,
                status_code: None,
                result: "failed",
                duration_ms: self.started.elapsed().as_millis() as i64,
            },
        );
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::Mutex;

    /// 收集发出的事件，供断言顺序与字段。
    pub(crate) fn collector() -> (Emit, Arc<Mutex<Vec<(&'static str, serde_json::Value)>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let emit: Emit = Arc::new(move |name, value| sink.lock().unwrap().push((name, value)));
        (emit, events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_increasing() {
        let a = next_request_id();
        let b = next_request_id();
        assert!(b > a);
    }

    #[test]
    fn dropped_guard_emits_failed_end_with_same_id() {
        let (emit, events) = test_support::collector();
        {
            let _guard = EndGuard::new(emit.clone(), 7, Some("codex"), "kimi/k2", "kimi", std::time::Instant::now());
        }
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, END_EVENT);
        assert_eq!(events[0].1["request_id"], 7);
        assert_eq!(events[0].1["result"], "failed");
        assert_eq!(events[0].1["source"], "kimi");
    }

    #[test]
    fn finished_guard_emits_nothing() {
        let (emit, events) = test_support::collector();
        EndGuard::new(emit, 8, None, "m", "s", std::time::Instant::now()).finish();
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn payload_has_no_secret_fields() {
        let value = serde_json::to_value(RouteStart { request_id: 1, ts: 0, agent: None, requested_model: "a/b", source: "a" }).unwrap();
        let keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["agent", "request_id", "requested_model", "source", "ts"]);
    }
}
