//! Trae SSE 到 Chat Completions；响应体拥有上游与账号锁，取消即释放。
use super::trae_chat::{Event, Events};
use crate::{
    gateway::{route::PoolEntry, wire::error_response},
    proxy::ProxyDeps,
};
use axum::{
    body::{Body, Bytes},
    http::StatusCode,
    response::Response,
};
use futures_util::{Stream, StreamExt};
use serde_json::{json, Value};
use std::{collections::VecDeque, pin::Pin, sync::Arc};

pub(super) struct Reader {
    upstream: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
    events: Events,
    queue: VecDeque<Event>,
    deps: Arc<ProxyDeps>,
    entry: PoolEntry,
    secret: String,
    _guard: tokio::sync::MutexGuard<'static, ()>,
    started: std::time::Instant,
    id: String,
    model: String,
}
impl Reader {
    pub fn new(
        response: reqwest::Response,
        deps: Arc<ProxyDeps>,
        entry: PoolEntry,
        secret: String,
        guard: tokio::sync::MutexGuard<'static, ()>,
        model: String,
        tools: &[Value],
    ) -> Self {
        Self {
            upstream: Box::pin(response.bytes_stream()),
            events: Events::new(tools),
            queue: VecDeque::new(),
            deps,
            entry,
            secret,
            _guard: guard,
            started: std::time::Instant::now(),
            id: format!(
                "chatcmpl-{}",
                super::trae_chat::nonce()
                    .unwrap_or_else(|_| chrono::Utc::now().timestamp_micros().to_string())
            ),
            model,
        }
    }
    async fn next(&mut self) -> Result<Option<Event>, String> {
        loop {
            super::trae::current(&self.deps, &self.entry, &self.secret)?;
            if let Some(event) = self.queue.pop_front() {
                return Ok(Some(event));
            }
            if self.events.done() {
                return Ok(None);
            }
            let events = match self.upstream.next().await {
                Some(Ok(bytes)) => self.events.push(&bytes)?,
                Some(Err(_)) => return Err("Trae 回复读取中断".into()),
                None => self.events.finish()?,
            };
            self.queue.extend(events);
        }
    }
    fn record(&self, usage: &Value) {
        let mapped = accounting_usage(usage);
        crate::gateway::subscription::record_usage(
            &self.deps,
            &self.entry,
            &json!({"id":self.id,"model":self.model,"usage":mapped}),
            self.started.elapsed().as_millis() as i64,
        );
    }
    fn chunk(&self, delta: Value, finish: Option<&str>, usage: Option<Value>) -> Bytes {
        let mut value = json!({"id":self.id,"object":"chat.completion.chunk","created":chrono::Utc::now().timestamp(),"model":self.model,"choices":[{"index":0,"delta":delta,"finish_reason":finish}]});
        if let Some(usage) = usage {
            value["usage"] = usage;
        }
        Bytes::from(format!("data: {value}\n\n"))
    }
}
pub(super) async fn response(mut reader: Reader, streaming: bool) -> Response {
    // 初始错误在下游成功响应头前报告；没有另起后台生成任务。
    let first = match reader.next().await {
        Ok(Some(v)) => v,
        Ok(None) => return error_response(StatusCode::BAD_GATEWAY, "Trae 未返回回答"),
        Err(e) => return error_response(StatusCode::BAD_GATEWAY, &e),
    };
    if !streaming {
        return complete(reader, first).await;
    }
    let state = (reader, Some(first), false, 0usize);
    let body = futures_util::stream::unfold(
        state,
        |(mut reader, first, closed, mut tool_index)| async move {
            if closed {
                return None;
            }
            let event = match first {
                Some(v) => Ok(Some(v)),
                None => reader.next().await,
            };
            let (chunk,closed)=match event {
            Ok(Some(Event::Text(s)))=>(reader.chunk(json!({"content":s}),None,None),false),
            Ok(Some(Event::Reasoning(s)))=>(reader.chunk(json!({"reasoning_content":s}),None,None),false),
            Ok(Some(Event::Tool(mut call)))=>{call["index"]=json!(tool_index);tool_index+=1;(reader.chunk(json!({"tool_calls":[call]}),None,None),false)},
            Ok(Some(Event::Done{reason,usage}))=>{
                reader.record(&usage);
                let mut bytes=reader.chunk(json!({}),Some(&reason),if usage.as_object().is_some_and(|m| !m.is_empty()){Some(usage)}else{None}).to_vec();
                bytes.extend_from_slice(b"data: [DONE]\n\n");(Bytes::from(bytes),true)
            },
            Err(_)|Ok(None)=>(Bytes::from("data: {\"error\":{\"type\":\"jolu_gateway\",\"message\":\"Trae 回复中断或协议无效，未完成回答\"}}\n\n"),true),
        };
            Some((
                Ok::<Bytes, std::convert::Infallible>(chunk),
                (reader, None, closed, tool_index),
            ))
        },
    );
    Response::builder()
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(Body::from_stream(body))
        .unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounting_preserves_cache_subset_and_unknown_counts() {
        assert_eq!(
            accounting_usage(
                &json!({"prompt_tokens":100,"completion_tokens":1,"prompt_tokens_details":{"cached_tokens":80},"total_tokens":101})
            ),
            json!({"input_tokens":20,"output_tokens":1,"cache_read_input_tokens":80,"total_tokens":101})
        );
        assert_eq!(accounting_usage(&json!({})), json!({}));
        assert_eq!(
            accounting_usage(&json!({"prompt_tokens":0,"cache_read_input_tokens":0})),
            json!({"input_tokens":0,"cache_read_input_tokens":0})
        );
    }
}

fn accounting_usage(usage: &Value) -> Value {
    let mut mapped = crate::gateway::translate::usage_of(usage);
    if let Some(n) = usage.get("total_tokens").and_then(Value::as_u64) {
        mapped["total_tokens"] = json!(n);
    }
    mapped
}

async fn complete(mut reader: Reader, first: Event) -> Response {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tools = Vec::new();
    let mut next = Some(first);
    while let Some(event) = next {
        match event {
            Event::Text(s) => content.push_str(&s),
            Event::Reasoning(s) => reasoning.push_str(&s),
            Event::Tool(v) => tools.push(v),
            Event::Done { reason, usage } => {
                let mut message = json!({"role":"assistant","content":content});
                if !reasoning.is_empty() {
                    message["reasoning_content"] = json!(reasoning);
                }
                if !tools.is_empty() {
                    message["tool_calls"] = json!(tools);
                }
                let mut value = json!({"id":reader.id,"object":"chat.completion","created":chrono::Utc::now().timestamp(),"model":reader.model,"choices":[{"index":0,"message":message,"finish_reason":reason}]});
                if usage.as_object().is_some_and(|m| !m.is_empty()) {
                    value["usage"] = usage.clone();
                }
                reader.record(&usage);
                return Response::builder()
                    .header("content-type", "application/json")
                    .body(Body::from(value.to_string()))
                    .unwrap();
            }
        }
        next = match reader.next().await {
            Ok(e) => e,
            Err(e) => return error_response(StatusCode::BAD_GATEWAY, &e),
        };
    }
    error_response(StatusCode::BAD_GATEWAY, "Trae 回复未完成")
}
