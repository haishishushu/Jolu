//! Responses SSE → Chat Completions SSE。按事件边界解码，保留跨网络块的 UTF-8。
use axum::body::Bytes;
use futures_util::{Stream, StreamExt};
use serde_json::{json, Value};
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct Decoder {
    buffer: Vec<u8>,
}
impl Decoder {
    pub(super) fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Value>, String> {
        self.buffer
            .extend(bytes.iter().copied().filter(|b| *b != b'\r'));
        if self.buffer.len() > 16 * 1024 * 1024 {
            return Err("响应事件超过大小限制".into());
        }
        let mut events = Vec::new();
        while let Some(end) = self.buffer.windows(2).position(|w| w == b"\n\n") {
            let frame: Vec<_> = self.buffer.drain(..end + 2).collect();
            let text = std::str::from_utf8(&frame).map_err(|_| "响应编码无效")?;
            let data = text
                .lines()
                .filter_map(|line| line.strip_prefix("data:").map(str::trim))
                .collect::<Vec<_>>()
                .join("\n");
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            events.push(serde_json::from_str(&data).map_err(|_| "响应事件格式无效")?);
        }
        Ok(events)
    }
}

struct Chat {
    decoder: Decoder,
    pending: VecDeque<Bytes>,
    id: String,
    model: String,
    created: i64,
    done: bool,
}

/// 观察完成事件以记录用量，不修改或缓存整个响应流。
pub(super) fn observe_completed<S, E, F>(stream: S, callback: F) -> impl Stream<Item = Result<Bytes, E>> + Send
where S: Stream<Item = Result<Bytes, E>> + Send + 'static, E: Send + 'static, F: FnMut(&Value) + Send + 'static {
    futures_util::stream::unfold((Box::pin(stream), Decoder::default(), callback, false), |(mut stream, mut decoder, mut callback, mut recorded)| async move {
        let chunk = stream.next().await?;
        if !recorded {
            if let Ok(bytes) = &chunk {
                if let Ok(events) = decoder.feed(bytes) {
                    for event in events {
                        if event.get("type").and_then(Value::as_str) == Some("response.completed") {
                            if let Some(response) = event.get("response") { callback(response); recorded = true; break; }
                        }
                    }
                } else { recorded = true; }
            }
        }
        Some((chunk, (stream, decoder, callback, recorded)))
    })
}
impl Chat {
    fn new(model: String) -> Self {
        let now = chrono::Utc::now();
        Self {
            decoder: Decoder::default(),
            pending: VecDeque::new(),
            id: format!("chatcmpl-jolu-{}", now.timestamp_micros()),
            model,
            created: now.timestamp(),
            done: false,
        }
    }
    fn emit(&mut self, value: Value) {
        self.pending
            .push_back(Bytes::from(format!("data: {value}\n\n")));
    }
    fn chunk(&self, delta: Value, finish: Value) -> Value {
        json!({"id":self.id,"object":"chat.completion.chunk","created":self.created,"model":self.model,"choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
    }
    fn finish(&mut self) {
        self.pending
            .push_back(Bytes::from_static(b"data: [DONE]\n\n"));
        self.done = true;
    }
    fn fail(&mut self) {
        self.emit(json!({"error":{"type":"upstream_error","message":"Codex 生成未完成或响应中断，请重试"}}));
        self.finish();
    }
    fn event(&mut self, event: Value) {
        if self.done {
            return;
        }
        match event.get("type").and_then(Value::as_str) {
            Some("response.created") => {
                if let Some(id) = event.pointer("/response/id").and_then(Value::as_str) {
                    self.id = id.into();
                }
                self.emit(self.chunk(json!({"role":"assistant","content":""}), Value::Null));
            }
            Some("response.output_text.delta") => {
                if let Some(text) = event.get("delta").and_then(Value::as_str) {
                    self.emit(self.chunk(json!({"content":text}), Value::Null));
                }
            }
            Some("response.refusal.delta") => {
                if let Some(text) = event.get("delta").and_then(Value::as_str) {
                    self.emit(self.chunk(json!({"refusal":text}), Value::Null));
                }
            }
            Some("response.completed") => {
                self.emit(self.chunk(json!({}), json!("stop")));
                if let Some(usage) = event.pointer("/response/usage") {
                    self.emit(json!({"id":self.id,"object":"chat.completion.chunk","created":self.created,"model":self.model,"choices":[],"usage":super::codex::chat_usage(usage)}));
                }
                self.finish();
            }
            Some("response.failed" | "response.incomplete" | "error") => self.fail(),
            _ => {}
        }
    }
}

pub(super) fn chat_stream<S, E>(
    stream: S,
    model: String,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send
where
    S: Stream<Item = Result<Bytes, E>> + Send + 'static,
    E: Send + 'static,
{
    futures_util::stream::unfold(
        (Box::pin(stream), Chat::new(model)),
        |(mut stream, mut chat)| async move {
            loop {
                if let Some(bytes) = chat.pending.pop_front() {
                    return Some((Ok(bytes), (stream, chat)));
                }
                if chat.done {
                    return None;
                }
                match stream.next().await {
                    Some(Ok(bytes)) => match chat.decoder.feed(&bytes) {
                        Ok(events) => {
                            for event in events {
                                chat.event(event);
                            }
                        }
                        Err(_) => chat.fail(),
                    },
                    Some(Err(_)) | None => chat.fail(),
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn chat_stream_handles_split_utf8_crlf_and_usage() {
        let sse = "event: response.created\r\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-a\"}}\r\n\r\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"你好\"}\r\n\r\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}}\r\n\r\n";
        let chunks: Vec<Result<Bytes, std::io::Error>> = sse
            .as_bytes()
            .iter()
            .map(|b| Ok(Bytes::copy_from_slice(&[*b])))
            .collect();
        let stream = chat_stream(futures_util::stream::iter(chunks), "model-a".into());
        futures_util::pin_mut!(stream);
        let mut out = Vec::new();
        while let Some(chunk) = stream.next().await {
            out.extend_from_slice(&chunk.unwrap());
        }
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("你好"));
        assert!(text.contains("chat.completion.chunk"));
        assert!(text.contains("prompt_tokens"));
        assert!(!text.contains("response.output_text.delta"));
        assert_eq!(text.matches("[DONE]").count(), 1);
    }

    #[tokio::test]
    async fn truncated_stream_reports_error_not_success() {
        let stream = chat_stream(
            futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from_static(b"data: {"))]),
            "m".into(),
        );
        futures_util::pin_mut!(stream);
        let first = String::from_utf8(stream.next().await.unwrap().unwrap().to_vec()).unwrap();
        assert!(first.contains("upstream_error"));
        assert!(!first.contains("\"finish_reason\":\"stop\""));
    }
}
