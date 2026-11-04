//! Gemini API 兼容层：把 Gemini CLI 的 `generateContent` / `streamGenerateContent`
//! 翻成 OpenAI Chat Completions，再翻回 Gemini 的响应形状。
//!
//! 只服务 `protocol = openai` 的 API 连接。Gemini CLI 的原生协议是
//! `/v1beta/models/{model}:generateContent`，Jolu 的连接没有 Gemini 上游，
//! 所以这里做一次有明确边界的翻译：文本、图片、函数调用、用量与流式文本。
//! 不支持的字段（googleSearch 等内置工具、多候选）直接报错，不静默丢弃。

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::gateway::route::{self, RouteDecision};
use crate::proxy::ProxyDeps;

/// Gemini 入口的请求方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GeminiMethod {
    Generate,
    Stream,
    /// 本地估算：Gemini CLI 用它做上下文规划，不发上游、不伪造真实用量。
    Count,
}

pub(crate) async fn handle(
    deps: Arc<ProxyDeps>,
    model_id: &str,
    method: GeminiMethod,
    body: axum::body::Bytes,
) -> Response {
    if method == GeminiMethod::Count {
        return json_response(
            StatusCode::OK,
            serde_json::json!({"totalTokens": estimate_tokens(&body)}).to_string(),
        );
    }
    // Gemini 资源名可能带 `models/` 前缀，菜单 id 是 `<slug>/<model>`，两种都接受
    let model_id = model_id.strip_prefix("models/").unwrap_or(model_id);
    let pool = {
        let Ok(guard) = deps.pool_index() else {
            return error_response(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用");
        };
        route::pool_snapshot(&deps.db, guard)
    };
    let history = |slug: &str| -> Option<String> {
        deps.db.try_lock().ok().and_then(|conn| route::historical_slug_message(&conn, slug))
    };
    let decision = route::decide(&pool, model_id, None, &history);
    if let RouteDecision::Direct(entry, _) | RouteDecision::Subscription(entry) = &decision {
        if !route::source_is_current(&deps, entry) {
            return error_response(StatusCode::SERVICE_UNAVAILABLE, "来源已关闭或配置已变化，请刷新后重试");
        }
    }
    let (entry, upstream_model) = match decision {
        RouteDecision::Direct(entry, model) => (entry, model),
        RouteDecision::Subscription(entry) if entry.platform == "gemini" => {
            let upstream_model = model_id.split_once('/').map(|(_, model)| model).unwrap_or(model_id);
            return super::gemini_subscription::forward(deps.clone(), entry, upstream_model, method, body).await;
        }
        RouteDecision::Subscription(_) => {
            return error_response(StatusCode::BAD_REQUEST, "Gemini 请求只支持 API Key 来源，订阅适配器暂不支持");
        }
        RouteDecision::Reject { kind, message, retry_after } => {
            let status = match kind {
                route::RejectKind::BadRequest => StatusCode::BAD_REQUEST,
                route::RejectKind::NotFound => StatusCode::NOT_FOUND,
                route::RejectKind::Unauthorized => StatusCode::UNAUTHORIZED,
                route::RejectKind::PaymentRequired => StatusCode::PAYMENT_REQUIRED,
                route::RejectKind::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
                route::RejectKind::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            };
            let mut response = error_response(status, &message);
            if let Some(seconds) = retry_after {
                if let Ok(value) = axum::http::HeaderValue::from_str(&seconds.to_string()) {
                    response.headers_mut().insert(axum::http::header::RETRY_AFTER, value);
                }
            }
            return response;
        }
    };
    if entry.protocol != "openai" {
        return error_response(
            StatusCode::BAD_REQUEST,
            &format!(
                "来源「{}」是 {} 协议，Gemini 请求目前只能转到 OpenAI 兼容来源",
                entry.slug, entry.protocol
            ),
        );
    }
    let now = now_ms();
    if let Some(cooldowns) = deps.cooldowns.as_ref() {
        if let Ok(table) = cooldowns.lock() {
            let remaining = table.remaining_ms(&entry.cred_key(), now);
            if remaining > 0 {
                return error_response(
                    StatusCode::TOO_MANY_REQUESTS,
                    &format!("来源「{}」正在限流，约 {} 秒后恢复", entry.slug, remaining / 1000 + 1),
                );
            }
        }
    }

    let stream = method == GeminiMethod::Stream;
    let openai_body = match to_openai_chat(&body, &upstream_model, stream) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, &message),
    };
    let base = entry.base_url.as_deref().unwrap_or_default().trim_end_matches('/');
    let url = chat_endpoint(base);
    let secret = entry.secret.as_deref().unwrap_or_default();
    let mut request = deps
        .client
        .post(url)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {secret}"))
        .json(&openai_body);
    if stream {
        request = request
            .header("accept", "text/event-stream")
            .header("accept-encoding", "identity");
    }

    let upstream = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            return error_response(StatusCode::BAD_GATEWAY, &format!("来源「{}」上游不可达：{error}", entry.slug));
        }
    };
    let status = upstream.status();
    crate::gateway::adapter::record_upstream_signal(&deps, entry, &upstream);
    if !status.is_success() {
        let body = upstream.text().await.unwrap_or_default();
        return error_response(status, &upstream_error_message(&body));
    }

    if !stream {
        let value: serde_json::Value = match upstream.json().await {
            Ok(value) => value,
            Err(error) => {
                return error_response(StatusCode::BAD_GATEWAY, &format!("来源「{}」返回的不是有效 JSON：{error}", entry.slug));
            }
        };
        let gemini = from_openai_chat(&value, &upstream_model);
        return json_response(StatusCode::OK, gemini.to_string());
    }

    let stream_body = GeminiSseStream::new(upstream.bytes_stream(), upstream_model.clone());
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream; charset=utf-8")
        .header("cache-control", "no-cache");
    if let Some(headers) = builder.headers_mut() {
        headers.insert("x-jolu-gateway", axum::http::HeaderValue::from_str(&entry.slug).unwrap_or_else(|_| axum::http::HeaderValue::from_static("jolu")));
    }
    builder
        .body(Body::from_stream(stream_body))
        .unwrap_or_else(|error| error_response(StatusCode::BAD_GATEWAY, &format!("响应构造失败：{error}")))
}

fn chat_endpoint(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/* ─────────────────────────── Gemini → OpenAI ─────────────────────────── */

/// Gemini 的 `GenerateContentRequest` → OpenAI Chat Completions 请求体。
pub(crate) fn to_openai_chat(body: &[u8], model: &str, stream: bool) -> Result<serde_json::Value, String> {
    let root: serde_json::Value = serde_json::from_slice(body).map_err(|error| format!("请求体不是有效 JSON：{error}"))?;
    let object = root.as_object().ok_or("请求体顶层不是对象")?;
    let mut messages: Vec<serde_json::Value> = Vec::new();
    if let Some(system) = object.get("systemInstruction") {
        let text = parts_text(system.get("parts"));
        if !text.is_empty() {
            messages.push(serde_json::json!({"role": "system", "content": text}));
        }
    }
    let contents = object
        .get("contents")
        .and_then(|v| v.as_array())
        .ok_or("请求缺少 contents 数组")?;
    for content in contents {
        let role = content.get("role").and_then(|v| v.as_str()).unwrap_or("user");
        let parts = content.get("parts").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let mut texts: Vec<String> = Vec::new();
        let mut images: Vec<serde_json::Value> = Vec::new();
        let mut tool_calls: Vec<serde_json::Value> = Vec::new();
        let mut tool_messages: Vec<serde_json::Value> = Vec::new();
        for part in &parts {
            if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                if !text.is_empty() {
                    texts.push(text.to_string());
                }
                continue;
            }
            if let Some(inline) = part.get("inlineData") {
                let mime = inline.get("mimeType").and_then(|v| v.as_str()).unwrap_or("image/png");
                let data = inline.get("data").and_then(|v| v.as_str()).unwrap_or_default();
                if !data.is_empty() {
                    images.push(serde_json::json!({
                        "type": "image_url",
                        "image_url": {"url": format!("data:{mime};base64,{data}")}
                    }));
                }
                continue;
            }
            if let Some(call) = part.get("functionCall") {
                let name = call.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                let args = call.get("args").cloned().unwrap_or_else(|| serde_json::json!({}));
                tool_calls.push(serde_json::json!({
                    "id": format!("call_{name}"),
                    "type": "function",
                    "function": {"name": name, "arguments": args.to_string()}
                }));
                continue;
            }
            if let Some(response) = part.get("functionResponse") {
                let name = response.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                let payload = response.get("response").cloned().unwrap_or_else(|| serde_json::json!({}));
                tool_messages.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": format!("call_{name}"),
                    "content": payload.to_string()
                }));
                continue;
            }
            // 未知 part（fileData / executableCode 等）不静默忽略
            return Err("请求里包含暂不支持的 Gemini 内容块（仅支持 text / inlineData / functionCall / functionResponse）".into());
        }
        let text = texts.join("\n");
        if role == "model" {
            let mut message = serde_json::json!({"role": "assistant", "content": text});
            if !tool_calls.is_empty() {
                message["tool_calls"] = serde_json::Value::Array(tool_calls);
                if text.is_empty() {
                    message["content"] = serde_json::Value::Null;
                }
            }
            messages.push(message);
        } else {
            let mut content = if images.is_empty() {
                serde_json::Value::String(text)
            } else {
                let mut blocks = Vec::new();
                if !text.is_empty() {
                    blocks.push(serde_json::json!({"type": "text", "text": text}));
                }
                blocks.extend(images);
                serde_json::Value::Array(blocks)
            };
            if content == serde_json::Value::String(String::new()) {
                content = serde_json::Value::Null;
            }
            // 只有 functionResponse 时不要再插一条空的 user 消息
            if !content.is_null() {
                messages.push(serde_json::json!({"role": "user", "content": content}));
            }
            messages.extend(tool_messages);
        }
    }

    let mut out = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": stream,
    });
    if let Some(config) = object.get("generationConfig") {
        if let Some(value) = config.get("temperature") {
            out["temperature"] = value.clone();
        }
        if let Some(value) = config.get("topP") {
            out["top_p"] = value.clone();
        }
        if let Some(value) = config.get("maxOutputTokens") {
            out["max_tokens"] = value.clone();
        }
        if let Some(value) = config.get("stopSequences") {
            out["stop"] = value.clone();
        }
        if let Some(value) = config.get("presencePenalty") {
            out["presence_penalty"] = value.clone();
        }
        if let Some(value) = config.get("frequencyPenalty") {
            out["frequency_penalty"] = value.clone();
        }
        if let Some(count) = config.get("candidateCount").and_then(|v| v.as_u64()) {
            if count > 1 {
                return Err("暂不支持要求多个候选的 Gemini 请求".into());
            }
        }
    }
    if let Some(tools) = object.get("tools").and_then(|v| v.as_array()) {
        let mut declarations = Vec::new();
        for tool in tools {
            let Some(functions) = tool.get("functionDeclarations").and_then(|v| v.as_array()) else {
                return Err("暂不支持 googleSearch 等 Gemini 内置工具；请只使用函数声明".into());
            };
            for function in functions {
                let name = function.get("name").and_then(|v| v.as_str()).ok_or("函数声明缺少 name")?;
                let mut entry = serde_json::json!({
                    "type": "function",
                    "function": {"name": name}
                });
                if let Some(description) = function.get("description") {
                    entry["function"]["description"] = description.clone();
                }
                if let Some(parameters) = function.get("parameters") {
                    entry["function"]["parameters"] = parameters.clone();
                }
                declarations.push(entry);
            }
        }
        if !declarations.is_empty() {
            out["tools"] = serde_json::Value::Array(declarations);
        }
        if let Some(mode) = root
            .pointer("/toolConfig/functionCallingConfig/mode")
            .and_then(|v| v.as_str())
        {
            out["tool_choice"] = serde_json::Value::String(match mode {
                "ANY" => "required",
                "NONE" => "none",
                _ => "auto",
            }
            .to_string());
        }
    }
    if stream {
        out["stream_options"] = serde_json::json!({"include_usage": true});
    }
    Ok(out)
}

/// 本地 token 估算：按文本字符数 / 4，只用于 CLI 的上下文规划。
/// 这是估算值，不冒充上游的真实用量；真实用量仍以上游响应里的 usageMetadata 为准。
fn estimate_tokens(body: &[u8]) -> usize {
    fn walk(value: &serde_json::Value, chars: &mut usize) {
        match value {
            serde_json::Value::String(text) => *chars += text.chars().count(),
            serde_json::Value::Array(items) => items.iter().for_each(|item| walk(item, chars)),
            serde_json::Value::Object(map) => {
                for (key, item) in map {
                    if key == "text" || key == "response" {
                        walk(item, chars);
                    } else if key == "contents" || key == "parts" || key == "systemInstruction" {
                        walk(item, chars);
                    }
                }
            }
            _ => {}
        }
    }
    let mut chars = 0usize;
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) {
        walk(&value, &mut chars);
    }
    (chars / 4).max(1)
}

fn parts_text(parts: Option<&serde_json::Value>) -> String {
    parts
        .and_then(|v| v.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/* ─────────────────────────── OpenAI → Gemini ─────────────────────────── */

pub(crate) fn from_openai_chat(value: &serde_json::Value, model: &str) -> serde_json::Value {
    let choice = value.get("choices").and_then(|v| v.get(0));
    let message = choice.and_then(|c| c.get("message"));
    let mut parts: Vec<serde_json::Value> = Vec::new();
    if let Some(text) = message.and_then(|m| m.get("content")).and_then(|v| v.as_str()) {
        if !text.is_empty() {
            parts.push(serde_json::json!({"text": text}));
        }
    }
    if let Some(calls) = message.and_then(|m| m.get("tool_calls")).and_then(|v| v.as_array()) {
        for call in calls {
            let name = call.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("tool");
            let args = call
                .pointer("/function/arguments")
                .and_then(|v| v.as_str())
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .unwrap_or_else(|| serde_json::json!({}));
            parts.push(serde_json::json!({"functionCall": {"name": name, "args": args}}));
        }
    }
    if parts.is_empty() {
        parts.push(serde_json::json!({"text": ""}));
    }
    let finish = choice.and_then(|c| c.get("finish_reason")).and_then(|v| v.as_str());
    let mut candidate = serde_json::json!({
        "content": {"role": "model", "parts": parts},
        "index": 0
    });
    if let Some(reason) = finish.and_then(finish_reason) {
        candidate["finishReason"] = serde_json::Value::String(reason.to_string());
    }
    let mut out = serde_json::json!({
        "candidates": [candidate],
        "modelVersion": model
    });
    if let Some(usage) = value.get("usage") {
        out["usageMetadata"] = usage_metadata(usage);
    }
    out
}

fn usage_metadata(usage: &serde_json::Value) -> serde_json::Value {
    let prompt = usage.get("prompt_tokens").cloned().unwrap_or(serde_json::json!(0));
    let completion = usage.get("completion_tokens").cloned().unwrap_or(serde_json::json!(0));
    let total = usage.get("total_tokens").cloned().unwrap_or(serde_json::json!(0));
    serde_json::json!({
        "promptTokenCount": prompt,
        "candidatesTokenCount": completion,
        "totalTokenCount": total
    })
}

fn finish_reason(reason: &str) -> Option<&'static str> {
    match reason {
        "length" => Some("MAX_TOKENS"),
        "content_filter" => Some("SAFETY"),
        "stop" | "tool_calls" | "function_call" => Some("STOP"),
        _ => None,
    }
}

/* ─────────────────────────── 流式转换 ─────────────────────────── */

/// 把 OpenAI 的 SSE 流转成 Gemini 的 SSE 流。
// 所有字段都是 Unpin（Pin<Box<S>> 本身是 Unpin），手写 impl 让 poll_next 能取到 &mut。
impl<S, E> Unpin for GeminiSseStream<S, E> {}

struct GeminiSseStream<S, E> {
    inner: std::pin::Pin<Box<S>>,
    buffer: Vec<u8>,
    pending: std::collections::VecDeque<axum::body::Bytes>,
    tool_calls: BTreeMap<usize, (String, String)>,
    model: String,
    done: bool,
    _error: std::marker::PhantomData<E>,
}

impl<S, E> GeminiSseStream<S, E>
where
    S: futures_util::Stream<Item = Result<axum::body::Bytes, E>> + Send + 'static,
{
    fn new(inner: S, model: String) -> Self {
        Self {
            inner: Box::pin(inner),
            buffer: Vec::new(),
            pending: std::collections::VecDeque::new(),
            tool_calls: BTreeMap::new(),
            model,
            done: false,
            _error: std::marker::PhantomData,
        }
    }

    /// 处理一段上游字节；产出的 Gemini SSE 事件放进 `pending`。
    fn feed(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
        // 兼容 CRLF 的 SSE：统一去掉 \r 再按空行切事件
        if self.buffer.contains(&b'\r') {
            self.buffer.retain(|byte| *byte != b'\r');
        }
        loop {
            let Some(index) = self.buffer.windows(2).position(|w| w == b"\n\n") else {
                break;
            };
            let event: Vec<u8> = self.buffer.drain(..index + 2).collect();
            let text = String::from_utf8_lossy(&event);
            for line in text.lines() {
                let line = line.trim_end_matches('\r');
                let Some(data) = line.strip_prefix("data:") else { continue };
                let data = data.trim();
                if data == "[DONE]" {
                    self.finish();
                    return;
                }
                if data.is_empty() {
                    continue;
                }
                self.push_chunk(data);
            }
        }
    }

    fn push_chunk(&mut self, data: &str) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else { return };
        let choice = value.get("choices").and_then(|v| v.get(0));
        let delta = choice.and_then(|c| c.get("delta"));
        if let Some(text) = delta.and_then(|d| d.get("content")).and_then(|v| v.as_str()) {
            if !text.is_empty() {
                self.emit(serde_json::json!({
                    "candidates": [{"content": {"role": "model", "parts": [{"text": text}]}, "index": 0}],
                    "modelVersion": self.model
                }));
            }
        }
        if let Some(calls) = delta.and_then(|d| d.get("tool_calls")).and_then(|v| v.as_array()) {
            for call in calls {
                let index = call.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let entry = self.tool_calls.entry(index).or_default();
                if let Some(name) = call.pointer("/function/name").and_then(|v| v.as_str()) {
                    entry.0 = name.to_string();
                }
                if let Some(args) = call.pointer("/function/arguments").and_then(|v| v.as_str()) {
                    entry.1.push_str(args);
                }
            }
        }
        if let Some(reason) = choice.and_then(|c| c.get("finish_reason")).and_then(|v| v.as_str()) {
            if reason != "tool_calls" && reason != "function_call" {
                self.emit_finish(finish_reason(reason));
            }
        }
        if let Some(usage) = value.get("usage").filter(|v| !v.is_null()) {
            self.emit(serde_json::json!({"usageMetadata": usage_metadata(usage)}));
        }
    }

    fn emit(&mut self, value: serde_json::Value) {
        self.pending.push_back(axum::body::Bytes::from(format!("data: {value}\n\n")));
    }

    fn emit_finish(&mut self, reason: Option<&str>) {
        let mut parts: Vec<serde_json::Value> = Vec::new();
        for (_, (name, arguments)) in std::mem::take(&mut self.tool_calls) {
            let args = serde_json::from_str::<serde_json::Value>(&arguments).unwrap_or_else(|_| serde_json::json!({}));
            parts.push(serde_json::json!({"functionCall": {"name": name, "args": args}}));
        }
        let mut candidate = serde_json::json!({"content": {"role": "model", "parts": parts}, "index": 0});
        if let Some(reason) = reason {
            candidate["finishReason"] = serde_json::Value::String(reason.to_string());
        }
        self.emit(serde_json::json!({"candidates": [candidate], "modelVersion": self.model}));
    }

    /// 上游结束：把累积的工具调用和结束原因发出去。
    fn finish(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        self.emit_finish(Some("STOP"));
    }
}

impl<S, E> futures_util::Stream for GeminiSseStream<S, E>
where
    S: futures_util::Stream<Item = Result<axum::body::Bytes, E>> + Send + 'static,
    E: 'static,
{
    type Item = Result<axum::body::Bytes, std::io::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = &mut *self;
        loop {
            if let Some(chunk) = this.pending.pop_front() {
                return std::task::Poll::Ready(Some(Ok(chunk)));
            }
            if this.done {
                return std::task::Poll::Ready(None);
            }
            match this.inner.as_mut().poll_next(cx) {
                std::task::Poll::Ready(Some(Ok(bytes))) => this.feed(&bytes),
                std::task::Poll::Ready(Some(Err(_))) | std::task::Poll::Ready(None) => this.finish(),
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    }
}

/* ─────────────────────────── 响应与错误 ─────────────────────────── */

fn upstream_error_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| format!("上游返回错误：{}", body.chars().take(200).collect::<String>()))
}

pub(crate) fn error_response(status: StatusCode, message: &str) -> Response {
    let code = status.as_u16();
    let gemini_status = match code {
        400 => "INVALID_ARGUMENT",
        401 => "UNAUTHENTICATED",
        403 => "PERMISSION_DENIED",
        404 => "NOT_FOUND",
        429 => "RESOURCE_EXHAUSTED",
        402 => "RESOURCE_EXHAUSTED",
        503 | 504 => "UNAVAILABLE",
        _ if code >= 500 => "INTERNAL",
        _ => "UNKNOWN",
    };
    let body = serde_json::json!({
        "error": {"code": code, "message": message, "status": gemini_status}
    });
    json_response(status, body.to_string())
}

fn json_response(status: StatusCode, body: String) -> Response {
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_text_history_system_and_generation_config() {
        let body = br#"{
            "systemInstruction": {"parts": [{"text": "be brief"}]},
            "contents": [
                {"role": "user", "parts": [{"text": "hi"}]},
                {"role": "model", "parts": [{"text": "hello"}]}
            ],
            "generationConfig": {"temperature": 0.2, "maxOutputTokens": 64, "stopSequences": ["x"]}
        }"#;
        let value = to_openai_chat(body, "kimi-k2", false).unwrap();
        assert_eq!(value["model"], "kimi-k2");
        assert_eq!(value["messages"][0]["role"], "system");
        assert_eq!(value["messages"][1]["content"], "hi");
        assert_eq!(value["messages"][2]["role"], "assistant");
        assert_eq!(value["temperature"], 0.2);
        assert_eq!(value["max_tokens"], 64);
        assert_eq!(value["stop"][0], "x");
        assert_eq!(value["stream"], false);
    }

    #[test]
    fn translates_function_declarations_and_calls() {
        let body = br#"{
            "contents": [{"role": "user", "parts": [{"text": "weather?"}]}],
            "tools": [{"functionDeclarations": [{"name": "weather", "description": "get weather", "parameters": {"type": "object"}}]}],
            "toolConfig": {"functionCallingConfig": {"mode": "ANY"}}
        }"#;
        let value = to_openai_chat(body, "m", true).unwrap();
        assert_eq!(value["tools"][0]["function"]["name"], "weather");
        assert_eq!(value["tool_choice"], "required");
        assert_eq!(value["stream"], true);
        assert_eq!(value["stream_options"]["include_usage"], true);
    }

    #[test]
    fn rejects_internal_tools_and_multiple_candidates() {
        let body = br#"{"contents":[],"tools":[{"googleSearch": {}}]}"#;
        assert!(to_openai_chat(body, "m", false).unwrap_err().contains("内置工具"));
        let body = br#"{"contents":[],"generationConfig":{"candidateCount":2}}"#;
        assert!(to_openai_chat(body, "m", false).unwrap_err().contains("多个候选"));
    }

    #[test]
    fn translates_openai_response_with_tool_calls_and_usage() {
        let value = serde_json::json!({
            "choices": [{"message": {"content": "ok", "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "weather", "arguments": "{\"city\":\"sh\"}"}}
            ]}, "finish_reason": "tool_calls"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });
        let gemini = from_openai_chat(&value, "m");
        assert_eq!(gemini["candidates"][0]["content"]["parts"][0]["text"], "ok");
        assert_eq!(gemini["candidates"][0]["content"]["parts"][1]["functionCall"]["name"], "weather");
        assert_eq!(gemini["candidates"][0]["content"]["parts"][1]["functionCall"]["args"]["city"], "sh");
        assert_eq!(gemini["candidates"][0]["finishReason"], "STOP");
        assert_eq!(gemini["usageMetadata"]["promptTokenCount"], 10);
        assert_eq!(gemini["usageMetadata"]["totalTokenCount"], 15);
    }

    #[test]
    fn token_estimate_counts_text_fields_locally() {
        let body = br#"{"contents":[{"parts":[{"text":"12345678"}]}],"systemInstruction":{"parts":[{"text":"1234"}]}}"#;
        assert_eq!(estimate_tokens(body), 3);
        assert_eq!(estimate_tokens(b"{}"), 1);
    }

    #[test]
    fn finish_reason_mapping() {
        assert_eq!(finish_reason("length"), Some("MAX_TOKENS"));
        assert_eq!(finish_reason("content_filter"), Some("SAFETY"));
        assert_eq!(finish_reason("stop"), Some("STOP"));
        assert_eq!(finish_reason("weird"), None);
    }

    #[tokio::test]
    async fn stream_translates_text_tool_calls_and_finish() {
        use axum::body::to_bytes;
        let tool = serde_json::json!({
            "choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"name": "weather", "arguments": "{\"city\":\"sh\"}"}}]}, "finish_reason": "tool_calls"}]
        });
        let chunks: Vec<Result<axum::body::Bytes, std::io::Error>> = vec![
            // 第一段用 CRLF，验证两种行尾都能切事件
            Ok("data: {\"choices\":[{\"delta\":{\"content\":\"he\"}}]}\r\n\r\n".into()),
            Ok("data: {\"choices\":[{\"delta\":{\"content\":\"llo\"}}]}\n\n".into()),
            Ok(format!("data: {tool}\n\n").into()),
            Ok("data: [DONE]\n\n".into()),
        ];
        let stream = GeminiSseStream::new(futures_util::stream::iter(chunks), "m".into());
        let body = to_bytes(axum::body::Body::from_stream(stream), 1 << 20).await.unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("\"text\":\"he\""), "{text}");
        assert!(text.contains("\"text\":\"llo\""), "{text}");
        assert!(text.contains("\"functionCall\""), "{text}");
        assert!(text.contains("\"name\":\"weather\""), "{text}");
        assert!(text.contains("\"args\":{\"city\":\"sh\"}"), "{text}");
        assert!(text.contains("\"finishReason\":\"STOP\""), "{text}");
    }

    #[test]
    fn chat_endpoint_handles_bases_with_and_without_v1() {
        assert_eq!(chat_endpoint("https://gw.test"), "https://gw.test/v1/chat/completions");
        assert_eq!(chat_endpoint("https://gw.test/v1/"), "https://gw.test/v1/chat/completions");
    }
}
