//! 网关请求的实时事件（胶囊灵动岛设计说明 §9.1；统一网关开发说明 §6.1）。
//!
//! - `gateway-route-start`：选路得到 Direct / Subscription 之后、交给上游适配器之前发出。
//!   被拒（Reject）在选路时就判定了，不发开始事件。
//! - `gateway-route`：每个请求结束时发出，含被拒与失败；与开始事件带同一个 `request_id`。
//!
//! `request_id` 由进程内自增生成，只在内存和事件里使用，不写库，不能当跨重启的标识。
//! 事件只带元数据（时间、Agent、请求模型、来源 slug、连接 id、会话标识、上游模型、状态码、结果、耗时），
//! 不带提示词、请求体与凭证。
//!
//! 在途登记：开始事件登记、结束事件注销，二者共用一个递增序号 `seq`。
//! 「统一网关」页进入时读 [`live_snapshot`]，之后只应用序号更大的事件，快照不会覆盖更新的事件；
//! 重复的结束事件只是空操作，不会产生负计数。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// 发事件的回调：生产环境转给 Tauri `emit`，测试里收集到内存。
pub(crate) type Emit = Arc<dyn Fn(&'static str, serde_json::Value) + Send + Sync>;

pub(crate) const START_EVENT: &str = "gateway-route-start";
pub(crate) const END_EVENT: &str = "gateway-route";

pub(crate) fn next_request_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// 已验证的调用方会话：平台（与会话线程的 platform 一致）+ 原始会话 ID。
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub(crate) struct SessionRef {
    pub platform: String,
    pub session_id: String,
}

/// 一次请求在开始与结束事件里共用的元数据。
#[derive(Debug, Clone, Default)]
pub(crate) struct RouteMeta {
    pub agent: Option<String>,
    pub requested_model: String,
    /// 选中的连接 id；被拒时为 None
    pub connection_id: Option<String>,
    /// 只有能从请求里验证出会话身份时才有值；亲和兜底键不算
    pub session: Option<SessionRef>,
}

impl RouteMeta {
    pub(crate) fn session_platform(&self) -> Option<&str> {
        self.session.as_ref().map(|s| s.platform.as_str())
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session.as_ref().map(|s| s.session_id.as_str())
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct RouteStart<'a> {
    pub request_id: u64,
    pub ts: i64,
    pub agent: Option<&'a str>,
    pub requested_model: &'a str,
    pub source: &'a str,
    pub connection_id: Option<&'a str>,
    pub session_platform: Option<&'a str>,
    pub session_id: Option<&'a str>,
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
    pub connection_id: Option<&'a str>,
    pub session_platform: Option<&'a str>,
    pub session_id: Option<&'a str>,
}

/// 在途请求（快照用，字段与开始事件一致）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct LiveRequest {
    pub request_id: u64,
    pub ts: i64,
    pub agent: Option<String>,
    pub requested_model: String,
    pub source: String,
    pub connection_id: Option<String>,
    pub session_platform: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Default)]
struct Live {
    seq: u64,
    requests: BTreeMap<u64, LiveRequest>,
}

static LIVE: LazyLock<Mutex<Live>> = LazyLock::new(Default::default);

fn live() -> std::sync::MutexGuard<'static, Live> {
    LIVE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 当前在途请求与对应的事件序号：序号不大于它的事件已经反映在快照里。
pub(crate) fn live_snapshot() -> (u64, Vec<LiveRequest>) {
    let guard = live();
    (guard.seq, guard.requests.values().cloned().collect())
}

fn with_seq<T: serde::Serialize>(event: &T, seq: u64) -> Option<serde_json::Value> {
    let mut value = serde_json::to_value(event).ok()?;
    value.as_object_mut()?.insert("seq".into(), seq.into());
    Some(value)
}

pub(crate) fn emit_start(emit: &Emit, event: &RouteStart<'_>) {
    let seq = {
        let mut guard = live();
        guard.seq += 1;
        guard.requests.insert(
            event.request_id,
            LiveRequest {
                request_id: event.request_id,
                ts: event.ts,
                agent: event.agent.map(str::to_string),
                requested_model: event.requested_model.to_string(),
                source: event.source.to_string(),
                connection_id: event.connection_id.map(str::to_string),
                session_platform: event.session_platform.map(str::to_string),
                session_id: event.session_id.map(str::to_string),
            },
        );
        guard.seq
    };
    if let Some(value) = with_seq(event, seq) {
        emit(START_EVENT, value);
    }
}

pub(crate) fn emit_end(emit: &Emit, event: &RouteEnd<'_>) {
    let seq = {
        let mut guard = live();
        guard.seq += 1;
        guard.requests.remove(&event.request_id);
        guard.seq
    };
    if let Some(value) = with_seq(event, seq) {
        emit(END_EVENT, value);
    }
}

/// 保证每个已发出的开始事件都有结束事件：请求被取消（客户端断开、future 被丢弃）时，
/// 在 Drop 里补发一条 `failed`。正常结束时调用 [`EndGuard::finish`] 交由调用方写结果。
pub(crate) struct EndGuard {
    emit: Emit,
    request_id: u64,
    meta: RouteMeta,
    source: String,
    started: std::time::Instant,
    done: bool,
}

impl EndGuard {
    pub(crate) fn new(emit: Emit, request_id: u64, meta: &RouteMeta, source: &str, started: std::time::Instant) -> Self {
        Self { emit, request_id, meta: meta.clone(), source: source.to_string(), started, done: false }
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
                agent: self.meta.agent.as_deref(),
                requested_model: &self.meta.requested_model,
                source: Some(&self.source),
                upstream_model: None,
                status_code: None,
                result: "failed",
                duration_ms: self.started.elapsed().as_millis() as i64,
                connection_id: self.meta.connection_id.as_deref(),
                session_platform: self.meta.session_platform(),
                session_id: self.meta.session_id(),
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

    fn meta() -> RouteMeta {
        RouteMeta {
            agent: Some("codex".into()),
            requested_model: "kimi/k2".into(),
            connection_id: Some("conn-1".into()),
            session: Some(SessionRef { platform: "codex".into(), session_id: "ses-1".into() }),
        }
    }

    fn start(emit: &Emit, id: u64) {
        emit_start(emit, &RouteStart {
            request_id: id, ts: 1, agent: None, requested_model: "a/b", source: "a",
            connection_id: Some("conn-1"), session_platform: Some("codex"), session_id: Some("ses-1"),
        });
    }

    fn end(emit: &Emit, id: u64) {
        emit_end(emit, &RouteEnd {
            request_id: id, ts: 2, agent: None, requested_model: "a/b", source: Some("a"), upstream_model: None,
            status_code: Some(200), result: "ok", duration_ms: 1, connection_id: Some("conn-1"),
            session_platform: Some("codex"), session_id: Some("ses-1"),
        });
    }

    fn is_live(id: u64) -> bool {
        live_snapshot().1.iter().any(|r| r.request_id == id)
    }

    #[test]
    fn ids_are_unique_and_increasing() {
        let a = next_request_id();
        let b = next_request_id();
        assert!(b > a);
    }

    #[test]
    fn dropped_guard_emits_failed_end_with_same_id() {
        let (emit, events) = test_support::collector();
        let id = next_request_id();
        start(&emit, id);
        {
            let _guard = EndGuard::new(emit.clone(), id, &meta(), "kimi", std::time::Instant::now());
        }
        assert!(!is_live(id), "取消的请求要从在途登记里移除");
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].0, END_EVENT);
        assert_eq!(events[1].1["request_id"], id);
        assert_eq!(events[1].1["result"], "failed");
        assert_eq!(events[1].1["source"], "kimi");
        assert_eq!(events[1].1["session_id"], "ses-1");
        assert!(events[1].1["seq"].as_u64().unwrap() > events[0].1["seq"].as_u64().unwrap());
    }

    #[test]
    fn finished_guard_emits_nothing() {
        let (emit, events) = test_support::collector();
        EndGuard::new(emit, next_request_id(), &meta(), "s", std::time::Instant::now()).finish();
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn snapshot_tracks_concurrent_requests_and_ignores_duplicate_end() {
        let (emit, _events) = test_support::collector();
        let (a, b) = (next_request_id(), next_request_id());
        start(&emit, a);
        start(&emit, b);
        assert!(is_live(a) && is_live(b));
        end(&emit, a);
        assert!(!is_live(a) && is_live(b), "同一会话结束一个请求，另一个仍在途");
        end(&emit, a);
        assert!(is_live(b), "重复的结束事件不影响其他请求");
        let (seq_before, _) = live_snapshot();
        end(&emit, b);
        let (seq_after, _) = live_snapshot();
        assert!(seq_after > seq_before);
        assert!(!is_live(b));
    }

    #[test]
    fn end_without_start_is_harmless() {
        let (emit, events) = test_support::collector();
        let id = next_request_id();
        end(&emit, id);
        assert!(!is_live(id));
        assert_eq!(events.lock().unwrap().len(), 1);
    }

    #[test]
    fn payload_has_no_secret_fields() {
        let value = serde_json::to_value(RouteStart {
            request_id: 1, ts: 0, agent: None, requested_model: "a/b", source: "a",
            connection_id: None, session_platform: None, session_id: None,
        }).unwrap();
        let keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["agent", "connection_id", "request_id", "requested_model", "session_id", "session_platform", "source", "ts"]);
    }
}
