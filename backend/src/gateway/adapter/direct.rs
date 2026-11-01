//! 同协议直通适配器：换 base_url、凭证头、模型名，其余原样转发。
//!
//! 纪律：
//! - 只处理 `kind=api` 且调用方协议 == 连接协议的请求；跨协议组合由 wire 层提前拒绝
//! - 响应流零缓冲透传（与观测模式的 `timed_body` 同款约束，首字延迟是网关要保的指标）
//! - 被替换掉的调用方凭证头不写任何日志（那只是 jolu-<agent> 令牌，但同样不外泄）

use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;

use crate::gateway::route::PoolEntry;
use crate::proxy::ProxyDeps;

/// 构建发往上游的请求：原请求头剥调用方凭证与 hop-by-hop，换上游凭证与协议头。
pub(crate) fn build_upstream_headers(
    caller_headers: &HeaderMap,
    protocol: &str,
    secret: &str,
) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in caller_headers {
        let lower = name.as_str().to_ascii_lowercase();
        // 剥调用方凭证（jolu-<agent> 令牌）、host 与 hop-by-hop；上游不需要也不该看到
        if matches!(
            lower.as_str(),
            "authorization" | "x-api-key" | "host" | "content-length" | "connection"
                | "accept-encoding" | "transfer-encoding" | "keep-alive" | "proxy-authorization"
                | "proxy-authenticate" | "te" | "trailer" | "upgrade"
        ) {
            continue;
        }
        out.insert(name.clone(), value.clone());
    }
    if protocol == "anthropic" {
        if let Ok(v) = axum::http::HeaderValue::from_str(secret) {
            out.insert("x-api-key", v);
        }
        if !out.contains_key("anthropic-version") {
            out.insert("anthropic-version", axum::http::HeaderValue::from_static("2023-06-01"));
        }
    } else if let Ok(v) = axum::http::HeaderValue::from_str(&format!("Bearer {secret}")) {
        out.insert("authorization", v);
    }
    out
}

/// 改写请求体里的 model 字段，其余 JSON 键原样保留。
/// 解析失败时返回 None（调用方应拿到 400，而不是把带 slug 的模型名发给上游）。
pub(crate) fn rewrite_model(body: &[u8], model: &str) -> Option<Vec<u8>> {
    let mut value: serde_json::Value = serde_json::from_slice(body).ok()?;
    match value.get_mut("model")? {
        serde_json::Value::String(current) => *current = model.to_string(),
        _ => return None,
    }
    serde_json::to_vec(&value).ok()
}

/// 直通转发：`POST {base}{path}`，流式响应原样回传。
pub(crate) async fn forward(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    upstream_model: String,
    method: axum::http::Method,
    path_and_query: &str,
    caller_headers: &HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let base = entry.base_url.as_deref().unwrap_or_default().trim_end_matches('/');
    let secret = entry.secret.as_deref().unwrap_or_default();
    let url = format!("{base}{path_and_query}");

    let rewritten = rewrite_model(&body, &upstream_model).unwrap_or_else(|| body.to_vec());
    let headers = build_upstream_headers(caller_headers, &entry.protocol, secret);

    let mut request = deps.client.request(method, &url).headers(headers);
    if !rewritten.is_empty() {
        request = request.body(rewritten);
    }

    let started = std::time::Instant::now();
    let upstream = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            return crate::gateway::wire::error_response(
                StatusCode::BAD_GATEWAY,
                &format!("连接「{}」上游不可达：{error}", entry.slug),
            );
        }
    };

    let status = upstream.status();
    // 上游信号 → 连接状态（执行文档 §9.3）：429 记冷却；401/403 标 invalid；402 标 exhausted。
    // 状态原样回传给调用方——agent 需要看到真实的 429/401 才会正确退避或提示重新授权。
    crate::gateway::adapter::record_upstream_signal(&deps, entry, &upstream);

    let mut out_headers = HeaderMap::new();
    for (name, value) in upstream.headers() {
        let lower = name.as_str().to_ascii_lowercase();
        if matches!(lower.as_str(), "connection" | "transfer-encoding" | "keep-alive" | "content-length" | "content-encoding") {
            continue;
        }
        out_headers.insert(name.clone(), value.clone());
    }
    let mut builder = Response::builder().status(status);
    if let Some(slot) = builder.headers_mut() {
        *slot = out_headers;
    }
    // 归账标识必须在头表整体替换之后再加，否则会被上面的赋值抹掉
    builder = builder.header("x-jolu-gateway", entry.slug.as_str());
    // 计时与归账（执行文档 §9）：首字时刻随时间流转发记录，流结束回填 requests 行。
    // 与观测模式同一条链路（apply_proxy_timing），只是连接 id 已知、不必靠指纹匹配。
    let connection_id = entry.connection_id.clone();
    let platform = entry.platform.clone();
    let timing = GatewayTiming {
        deps: deps.clone(),
        connection_id,
        platform,
        started,
        first_byte_ms: None,
        status: status.as_u16() as i64,
    };
    let stream = upstream.bytes_stream();
    builder
        .body(Body::from_stream(GatewayTimedStream { inner: stream, timing, head: Vec::new(), settled: false }))
        .unwrap_or_else(|error| {
            crate::gateway::wire::error_response(
                StatusCode::BAD_GATEWAY,
                &format!("连接「{}」响应构造失败：{error}", entry.slug),
            )
        })
}

/// 网关请求的计时状态：记录首字节时刻与连接归属，流结束时回填 `requests` 行。
struct GatewayTiming {
    deps: Arc<ProxyDeps>,
    connection_id: String,
    platform: String,
    started: std::time::Instant,
    first_byte_ms: Option<i64>,
    status: i64,
}

/// 计时流的可变状态：首字、响应头快照（提取去重键）、是否已结算。
struct GatewayTimedStream<S> {
    inner: S,
    timing: GatewayTiming,
    head: Vec<u8>,
    settled: bool,
}

impl<S> futures_util::Stream for GatewayTimedStream<S>
where
    S: futures_util::Stream<Item = Result<axum::body::Bytes, reqwest::Error>> + Unpin,
{
    type Item = Result<axum::body::Bytes, reqwest::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = &mut *self;
        let polled = std::pin::Pin::new(&mut this.inner).poll_next(cx);
        match &polled {
            std::task::Poll::Ready(Some(Ok(bytes))) => {
                if this.timing.first_byte_ms.is_none() {
                    this.timing.first_byte_ms = Some(this.timing.started.elapsed().as_millis() as i64);
                }
                if this.head.len() < crate::proxy::HEAD_CAP {
                    let room = crate::proxy::HEAD_CAP - this.head.len();
                    this.head.extend_from_slice(&bytes[..bytes.len().min(room)]);
                }
            }
            std::task::Poll::Ready(None) | std::task::Poll::Ready(Some(Err(_))) => {
                if !this.settled {
                    this.settled = true;
                    crate::gateway::adapter::settle_gateway_timing(
                        &this.timing.deps,
                        &this.timing.connection_id,
                        &this.timing.platform,
                        &this.head,
                        this.timing.first_byte_ms.unwrap_or_default(),
                        this.timing.started.elapsed().as_millis() as i64,
                        this.timing.status,
                    );
                }
            }
            std::task::Poll::Pending => {}
        }
        polled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 测试里读响应体的上限（与服务端请求体上限同源量级）。
    const BODY_LIMIT: usize = 512 * 1024 * 1024;

    #[test]
    fn rewrite_model_replaces_only_model_and_keeps_other_fields() {
        let body = br#"{"model":"kimi/kimi-k2","max_tokens":128,"messages":[{"role":"user","content":"hi"}]}"#;
        let out = rewrite_model(body, "kimi-k2").unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["model"], "kimi-k2");
        assert_eq!(value["max_tokens"], 128);
        assert_eq!(value["messages"][0]["content"], "hi");
    }

    #[test]
    fn rewrite_model_returns_none_for_invalid_or_missing_model() {
        assert!(rewrite_model(b"not json", "m").is_none());
        assert!(rewrite_model(br#"{"max_tokens":1}"#, "m").is_none());
        // model 非字符串也不猜
        assert!(rewrite_model(br#"{"model":42}"#, "m").is_none());
    }

    #[test]
    fn upstream_headers_strip_caller_credentials_and_set_protocol_auth() {
        let mut caller = HeaderMap::new();
        caller.insert("authorization", "Bearer jolu-claude-code".parse().unwrap());
        caller.insert("x-api-key", "jolu-claude-code".parse().unwrap());
        caller.insert("anthropic-beta", "oauth-2025-04-20".parse().unwrap());
        caller.insert("content-type", "application/json".parse().unwrap());
        caller.insert("host", "127.0.0.1:12731".parse().unwrap());

        let out = build_upstream_headers(&caller, "anthropic", "sk-upstream-key");
        assert_eq!(out.get("x-api-key").unwrap(), "sk-upstream-key");
        assert_eq!(out.get("anthropic-version").unwrap(), "2023-06-01");
        assert!(out.get("authorization").is_none(), "调用方的 Bearer 令牌必须剥掉");
        assert!(out.get("host").is_none());
        assert_eq!(out.get("anthropic-beta").unwrap(), "oauth-2025-04-20");

        let out2 = build_upstream_headers(&caller, "openai", "sk-upstream-key");
        assert_eq!(out2.get("authorization").unwrap(), "Bearer sk-upstream-key");
        assert!(out2.get("x-api-key").is_none());
    }

    /// 端到端：mock 上游断言收到的模型名与凭证头，网关流式回传响应体。
    #[tokio::test]
    async fn direct_forward_rewrites_model_and_auth_end_to_end() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // 手写 mock 上游：读满请求（含 body）再响应——Windows 上未读完就 close 会 RST
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let upstream = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = socket.read(&mut chunk).await.unwrap();
                if n == 0 { break; }
                raw.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&raw);
                let Some(split) = text.find("\r\n\r\n") else { continue };
                let head = &text[..split];
                let length = head
                    .lines()
                    .find_map(|l| l.split_once(':'))
                    .filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if raw.len() >= split + 4 + length { break; }
            }
            let request = String::from_utf8_lossy(&raw).to_lowercase();
            let response = "{\"id\":\"msg_test\",\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}";
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.flush().await.unwrap();
            request
        });

        let deps = crate::proxy::test_support::bare_deps();
        let entry = PoolEntry {
            connection_id: "c1".into(),
            slug: "kimi".into(),
            platform: "custom".into(),
            kind: "api".into(),
            protocol: "anthropic".into(),
            base_url: Some(format!("http://{addr}")),
            secret: Some("sk-upstream-key".into()),
            model: None,
            status: "connected".into(),
        };
        let mut headers = HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        headers.insert("x-api-key", "jolu-claude-code".parse().unwrap());
        headers.insert("anthropic-beta", "context-1m-2025-08-07".parse().unwrap());
        let body = axum::body::Bytes::from(
            r#"{"model":"kimi/kimi-k2","max_tokens":64,"messages":[]}"#,
        );

        let response = forward(
            deps,
            &entry,
            "kimi-k2".into(),
            axum::http::Method::POST,
            "/v1/messages?beta=true",
            &headers,
            body,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("x-jolu-gateway").unwrap(), "kimi");
        let bytes = axum::body::to_bytes(response.into_body(), BODY_LIMIT).await.unwrap();
        assert!(bytes.windows(b"msg_test".len()).any(|w| w == b"msg_test"));

        let request = upstream.await.unwrap();
        assert!(request.starts_with("post /v1/messages?beta=true "), "{request}");
        assert!(request.contains("x-api-key: sk-upstream-key"), "{request}");
        assert!(request.contains("anthropic-version:"), "{request}");
        assert!(!request.contains("jolu-claude-code"), "调用方令牌不能漏给上游：{request}");
        assert!(request.contains("\"model\":\"kimi-k2\""), "模型名必须已改写：{request}");
        assert!(request.contains("anthropic-beta"), "{request}");
    }
}
