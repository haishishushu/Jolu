//! 网关 HTTP 入口。四个协议端点 + 模型列表；调用方协议由路径判定，不嗅探请求头。

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;

use crate::proxy::ProxyDeps;

/// 与观测模式一致：长上下文请求可达数十 MB，取 512 MB 上限。
const BODY_LIMIT: usize = 512 * 1024 * 1024;

/// 网关路由：挂在 `proxy.rs::start` 的监听端口上（仅 127.0.0.1）。
/// 观测模式的 `forward` 兜底不参与网关模式——两套数据路径严格分流。
pub fn router(deps: Arc<ProxyDeps>) -> Router {
    Router::new()
        .route("/v1/models", get(models))
        .route("/models", get(models))
        .route("/v1/messages", post(messages))
        .route("/messages", post(messages))
        .route("/v1/chat/completions", post(chat))
        .route("/chat/completions", post(chat))
        .route("/v1/responses", post(responses))
        .route("/responses", post(responses))
        .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(deps)
}

/// JSON 响应。不启用 axum 的 json feature，手工拼装保持依赖面不变。
fn json_response(status: StatusCode, body: String) -> Response {
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

fn error_json(status: StatusCode, message: &str) -> Response {
    json_response(
        status,
        format!("{{\"error\":{{\"type\":\"jolu_gateway\",\"message\":\"{message}\"}}}}"),
    )
}

/// `GET /v1/models`：池内连接的模型清单，供各工具的模型选择器消费。
async fn models(State(_deps): State<Arc<ProxyDeps>>) -> Response {
    // 第 4 步接入模型列表后填充真实清单；骨架阶段返回空列表保持端点可用。
    json_response(StatusCode::OK, "{\"object\":\"list\",\"data\":[]}".into())
}
async fn split(request: Request) -> Result<(String, HeaderMap, axum::body::Bytes), Response> {
    let (parts, body) = request.into_parts();
    let path = parts
        .uri
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    let bytes = axum::body::to_bytes(body, BODY_LIMIT)
        .await
        .map_err(|error| error_json(StatusCode::BAD_REQUEST, &format!("读取请求体失败：{error}")))?;
    Ok((path, parts.headers, bytes))
}

/// `POST /v1/messages`（Anthropic Messages 协议入口）。
async fn messages(State(deps): State<Arc<ProxyDeps>>, request: Request) -> Response {
    match split(request).await {
        Ok((path, headers, body)) => {
            crate::gateway::wire::handle(crate::gateway::wire::CallerProtocol::Anthropic, deps, headers, body, &path)
                .await
        }
        Err(response) => response,
    }
}

/// `POST /v1/chat/completions`（OpenAI Chat 协议入口）。
async fn chat(State(deps): State<Arc<ProxyDeps>>, request: Request) -> Response {
    match split(request).await {
        Ok((path, headers, body)) => {
            crate::gateway::wire::handle(crate::gateway::wire::CallerProtocol::Chat, deps, headers, body, &path)
                .await
        }
        Err(response) => response,
    }
}

/// `POST /v1/responses`（OpenAI Responses 协议入口；当前无直通连接，仅保留路由）。
async fn responses(State(deps): State<Arc<ProxyDeps>>, request: Request) -> Response {
    match split(request).await {
        Ok((path, headers, body)) => {
            crate::gateway::wire::handle(crate::gateway::wire::CallerProtocol::Responses, deps, headers, body, &path)
                .await
        }
        Err(response) => response,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与 proxy.rs 测试同款手法：真实监听随机端口 + reqwest 调用，不引入 tower 依赖。
    async fn spawn_router(deps: Arc<ProxyDeps>) -> String {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = router(deps);
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn models_endpoint_answers_empty_list_in_skeleton() {
        let base = spawn_router(crate::proxy::test_support::bare_deps()).await;
        let body = reqwest::Client::new()
            .get(format!("{base}/v1/models"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "{\"object\":\"list\",\"data\":[]}");
    }

    #[tokio::test]
    async fn messages_endpoint_rejects_when_pool_is_empty() {
        let base = spawn_router(crate::proxy::test_support::bare_deps()).await;
        let res = reqwest::Client::new()
            .post(format!("{base}/v1/messages"))
            .header("content-type", "application/json")
            .body(r#"{"model":"kimi/kimi-k2"}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);
        let body = res.text().await.unwrap();
        assert!(body.contains("模型池为空"), "空池要给用户下一步指引：{body}");
    }

    /// 端到端主路径：serve_enabled 的 API 连接 → slug 命中 → 模型改写 + 凭证替换 → 上游响应回传。
    #[tokio::test]
    async fn full_relay_path_serves_enabled_connection_end_to_end() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let upstream_addr = listener.local_addr().unwrap();
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
                let length = text[..split]
                    .lines()
                    .find_map(|l| l.split_once(':'))
                    .filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if raw.len() >= split + 4 + length { break; }
            }
            let request = String::from_utf8_lossy(&raw).to_lowercase();
            let body = "{\"content\":[{\"text\":\"hi\"}]}";
            let head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
            socket.flush().await.unwrap();
            request
        });

        // 构造网关 deps：库里有且仅有一条 serve_enabled 的 anthropic 连接
        let deps = crate::proxy::test_support::bare_deps();
        {
            let conn = deps.db.lock().unwrap();
            conn.execute(
                "INSERT INTO connections(id,platform,kind,name,secret,masked,base_url,protocol,serve_enabled,created_ms)
                 VALUES('c-kimi','custom','api','Kimi','sk-upstream-key','****','anthropic-upstream-placeholder','anthropic',1,1)",
                [],
            )
            .unwrap();
            conn.execute("UPDATE connections SET base_url = ?1 WHERE id='c-kimi'", [format!("http://{upstream_addr}")])
                .unwrap();
        }

        let base = spawn_router(deps.clone()).await;
        let res = reqwest::Client::builder().no_proxy().build().unwrap()
            .post(format!("{base}/v1/messages"))
            .header("content-type", "application/json")
            .header("x-api-key", "jolu-claude-code")
            .body(r#"{"model":"kimi/kimi-k2","max_tokens":64,"messages":[]}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), reqwest::StatusCode::OK);
        assert_eq!(res.headers().get("x-jolu-gateway").unwrap(), "kimi");
        let body = res.text().await.unwrap();
        assert!(body.contains("hi"), "{body}");

        let request = upstream.await.unwrap();
        assert!(request.contains("x-api-key: sk-upstream-key"), "{request}");
        assert!(!request.contains("jolu-claude-code"), "调用方令牌不能漏给上游：{request}");
        assert!(request.contains("\"model\":\"kimi-k2\""), "模型名必须已改写：{request}");
    }
}
