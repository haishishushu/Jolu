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
pub(crate) fn router(deps: Arc<ProxyDeps>) -> Router {
    Router::new()
        .route("/v1/models", get(models))
        .route("/models", get(models))
        .route("/v1/messages", post(messages))
        .route("/messages", post(messages))
        .route("/v1/chat/completions", post(chat))
        .route("/chat/completions", post(chat))
        .route("/v1/responses", post(responses))
        .route("/responses", post(responses))
        // Codex 保留 OAuth 时把 openai_base_url 指向这个路径（对齐 ChatGPT 后端）
        .route("/backend-api/codex/responses", post(responses))
        .route("/backend-api/codex/models", get(models))
        // Gemini CLI 原生端点：模型 id 里带斜杠，用 catch-all 接
        .route("/v1beta/models", get(gemini_models))
        .route("/v1beta/models/{*call}", post(gemini_generate))
        .route("/__jolu/bridge/callback", post(bridge_callback))
        .layer(axum::middleware::from_fn_with_state(deps.clone(), local_only))
        .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(deps)
}

/// 只接受本机工具的请求：拒绝浏览器跨站（Origin/Referer）与 DNS rebinding（Host 不是回环）。
/// 开通过手动 Key 后同时校验凭据；原生 OAuth 与一键接线沿用各自凭据。
async fn local_only(State(deps): State<Arc<ProxyDeps>>, request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if request.headers().contains_key(axum::http::header::ORIGIN)
        || request.headers().contains_key(axum::http::header::REFERER)
    {
        return error_json(
            StatusCode::FORBIDDEN,
            "模型入口不接受浏览器跨站请求；请让本机 CLI 工具直接访问地址",
        );
    }
    let host_ok = request
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(loopback_host);
    if !host_ok {
        return error_json(StatusCode::FORBIDDEN, "Host 不是本机回环地址，已拒绝");
    }
    let token = request.headers().get("authorization").and_then(|h| h.to_str().ok())
        .and_then(|h| h.split_once(' ')).filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer")).map(|(_, token)| token)
        .or_else(|| request.headers().get("x-api-key").and_then(|h| h.to_str().ok()))
        .or_else(|| request.headers().get("x-goog-api-key").and_then(|h| h.to_str().ok()));
    let allowed = match deps.db.lock() {
        Ok(conn) => super::access::authorize(&conn, token, request.uri().path()),
        Err(_) => Err("网关状态暂不可用".into()),
    };
    match allowed {
        Ok(true) => next.run(request).await,
        Ok(false) => error_json(StatusCode::UNAUTHORIZED, "API Key 无效或已刷新，请从 Jolu 复制最新的 API Key"),
        Err(_) => error_json(StatusCode::SERVICE_UNAVAILABLE, "无法读取网关密钥，请在 Jolu 中重试"),
    }
}

/// Host 头必须是 127.0.0.1 / localhost / [::1]（可带端口）。
fn loopback_host(value: &str) -> bool {
    let host = if let Some(rest) = value.strip_prefix('[') {
        rest.split(']').next().map(|inner| format!("[{inner}]"))
    } else {
        value.split(':').next().map(str::to_string)
    };
    matches!(host.as_deref(), Some("127.0.0.1" | "localhost" | "[::1]"))
}

/// MCP 桥的工具调用回调（§6.3）：挂起等待 agent 的 tool_result 释放。
/// 仅回环监听 + 双下划线私有路径，不进入任何公共文档。
async fn bridge_callback(
    State(deps): State<Arc<ProxyDeps>>,
    body: axum::body::Bytes,
) -> Response {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return error_json(StatusCode::BAD_REQUEST, "回调体不是有效 JSON");
    };
    let Some(call_id) = value.get("tool_call_id").and_then(|v| v.as_str()).map(str::to_string) else {
        return error_json(StatusCode::BAD_REQUEST, "回调缺少 tool_call_id");
    };
    let Some(hub) = deps.tool_hub.as_ref() else {
        return error_json(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用");
    };
    let receiver = hub.register(&call_id);
    match tokio::time::timeout(crate::gateway::bridge::TOOL_CALL_TIMEOUT, receiver).await {
        Ok(Ok(result)) => json_response(StatusCode::OK, result),
        Ok(Err(_)) => json_response(StatusCode::GONE, "调用已被放弃"),
        Err(_) => {
            json_response(StatusCode::GATEWAY_TIMEOUT, "等待工具结果超时（120 秒）")
        }
    }
}

/// JSON 响应。不启用 axum 的 json feature，手工拼装保持依赖面不变。
fn json_response(status: StatusCode, body: impl Into<String>) -> Response {
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body.into(),
    )
        .into_response()
}

fn error_json(status: StatusCode, message: &str) -> Response {
    json_response(
        status,
        serde_json::json!({"error": {"type": "jolu_gateway", "message": message}}).to_string(),
    )
}

/// 订阅连接的内置模型清单（适配器第 5/7 步接入前的兜底）。
/// 名称取自设计稿（Jolu.pen 模块 J 连接池卡片），不得凭空增删。
pub(crate) fn builtin_subscription_models(platform: &str) -> &'static [&'static str] {
    match platform {
        "claude" => &["claude-sonnet-5", "claude-opus-5"],
        "codex" | "opencode" => &["gpt-5.5", "gpt-5-codex"],
        _ => &[],
    }
}

/// 模型清单缓存：connection_id → (刷新时间, 模型列表)。TTL 内不重复打上游。
static MODEL_CACHE: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, (i64, Vec<String>)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

const MODEL_CACHE_TTL_MS: i64 = 5 * 60_000;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `GET /v1/models`：池内连接的模型清单，供各工具的模型选择器消费。
/// API 连接调 `fetch_models`（带 TTL 缓存）；订阅连接用内置清单；
/// 拉取失败的连接不出现在列表里，其 slug 记入 `unavailable` 供诊断。
async fn models(State(deps): State<Arc<ProxyDeps>>) -> Response {
    let mut pool = {
        let guard = match deps.pool_index() {
            Ok(g) => g,
            Err(_) => return error_json(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用"),
        };
        crate::gateway::route::pool_snapshot(&deps.db, guard)
    };
    // 分组顺序决定菜单顺序；未分组排在最后。分组只改顺序，不增删条目。
    if let Ok(conn) = deps.db.try_lock() {
        let order = crate::gateway::groups::source_order(&conn);
        pool.sort_by_key(|entry| order.get(&entry.connection_id).copied().unwrap_or((i64::MAX, i64::MAX)));
    }
    let now = now_ms();
    let mut data: Vec<serde_json::Value> = Vec::new();
    let mut unavailable: Vec<String> = Vec::new();
    let mut unavailable_reasons = std::collections::BTreeMap::<String, String>::new();
    // 来源名重复时在 display_name 里补 slug，避免模型选择器里出现两个一模一样的名字
    let mut name_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for entry in &pool {
        *name_counts.entry(entry.name.as_str()).or_default() += 1;
    }
    let connected = |status: &str| status == "connected";
    for entry in &pool {
        let models: Vec<String> = if entry.kind == "api" {
            let fresh_cache = MODEL_CACHE
                .lock()
                .ok()
                .and_then(|cache| {
                    cache
                        .get(&entry.connection_id)
                        .filter(|(at, _)| now - *at < MODEL_CACHE_TTL_MS)
                        .map(|(_, list)| list.clone())
                });
            // TTL 外的清单还要用：上游这次查不到时，菜单 id 不能因一次失败消失
            let fallback = || {
                load_models_json(&deps.db, &entry.connection_id)
                    .or_else(|| entry.model.clone().map(|model| vec![model]))
                    .unwrap_or_default()
            };
            match fresh_cache {
                Some(list) => list,
                // 暂停 / 失效的来源不向上游发请求
                None if !connected(&entry.status) => fallback(),
                None => {
                    let platform = entry.platform.clone();
                    let base = entry.base_url.clone().unwrap_or_default();
                    let secret = entry.secret.clone().unwrap_or_default();
                    let protocol: Option<&'static str> = if entry.protocol == "anthropic" { Some("anthropic") } else { Some("openai") };
                    let connection_id = entry.connection_id.clone();
                    // fetch_models 是 blocking reqwest：在 async 上下文里直接调用会在
                    // 客户端 drop 时 panic（runtime 嵌套），必须挪到阻塞线程池
                    let fetched = tokio::task::spawn_blocking(move || {
                        crate::connections::fetch_models(&platform, protocol, &base, &secret)
                    })
                    .await;
                    match fetched {
                        Ok(Ok(list)) => {
                            if let Ok(mut cache) = MODEL_CACHE.lock() {
                                cache.insert(connection_id.clone(), (now, list.clone()));
                            }
                            save_models_json(&deps.db, &connection_id, &list);
                            list
                        }
                        Ok(Err(_)) | Err(_) => fallback(),
                    }
                }
            }
        } else if entry.platform == "qoder" {
            match crate::gateway::adapter::qoder::models(&deps, entry).await {
                Ok(list) => list,
                Err(reason) => {
                    unavailable_reasons.insert(entry.slug.clone(), reason);
                    Vec::new()
                }
            }
        } else if entry.platform == "trae" {
            match crate::gateway::adapter::trae_models::models(&deps, entry).await {
                Ok(list) => list,
                Err(reason) => {
                    unavailable_reasons.insert(entry.slug.clone(), reason);
                    Vec::new()
                }
            }
        } else if entry.platform == "workbuddy" {
            // 默认路由名不是模型目录。只展示当前账号向官方查询成功的模型。
            if connected(&entry.status) && super::route::source_is_current(&deps, entry) {
                match crate::gateway::adapter::workbuddy::models(&deps, &entry.connection_id).await {
                    Ok(list) if list.is_empty() => {
                        unavailable_reasons.insert(entry.slug.clone(), "WorkBuddy 未返回可通过此账号直接调用的具体模型".into());
                        list
                    }
                    Ok(list) => list,
                    Err(reason) => {
                        unavailable_reasons.insert(entry.slug.clone(), reason);
                        Vec::new()
                    }
                }
            } else {
                Vec::new()
            }
        } else if matches!(entry.platform.as_str(), "grok" | "gemini" | "zcode") && connected(&entry.status) {
            let fetched = match entry.platform.as_str() {
                "grok" => crate::gateway::adapter::grok::models(&deps, &entry.connection_id).await,
                "zcode" => crate::gateway::adapter::zcode::models(&deps, &entry.connection_id).await,
                _ => crate::gateway::adapter::gemini_subscription::models(&deps, &entry.connection_id).await,
            };
            match fetched {
                Ok(list) => { save_models_json(&deps.db, &entry.connection_id, &list); list }
                Err(_) => load_models_json(&deps.db, &entry.connection_id).unwrap_or_default(),
            }
        } else {
            builtin_subscription_models(&entry.platform)
                .iter()
                .map(|m| m.to_string())
                .collect()
        };
        if models.is_empty() {
            unavailable.push(entry.slug.clone());
            continue;
        }
        let duplicate_name = name_counts.get(entry.name.as_str()).copied().unwrap_or(0) > 1;
        for model in models {
            let mut display = if duplicate_name {
                format!("{} ({}) · {model}", entry.name, entry.slug)
            } else {
                format!("{} · {model}", entry.name)
            };
            if let Some(label) = status_label(&entry.status) {
                display.push_str(&format!("（{label}）"));
            }
            data.push(serde_json::json!({
                "id": format!("{}/{}", entry.slug, model),
                "object": "model",
                "owned_by": entry.slug,
                "status": entry.status,
                "display_name": display,
            }));
        }
    }
    if data.is_empty() {
        if let Some(reason) = unavailable_reasons.values().next() {
            return error_json(StatusCode::BAD_GATEWAY, reason);
        }
    }
    let body = serde_json::json!({"object": "list", "data": data, "unavailable": unavailable, "unavailable_reasons": unavailable_reasons});
    json_response(StatusCode::OK, body.to_string())
}

fn status_label(status: &str) -> Option<&'static str> {
    match status {
        "paused" => Some("已暂停"),
        "invalid" => Some("登录已失效"),
        "exhausted" => Some("额度已用尽"),
        "expired" => Some("登录已过期"),
        _ => None,
    }
}

/// 上次成功拉到的模型清单落库：来源失效或重启后，菜单 id 仍然在。
fn load_models_json(db: &std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>, id: &str) -> Option<Vec<String>> {
    let conn = db.try_lock().ok()?;
    let text: Option<String> = conn
        .query_row("SELECT models_json FROM connections WHERE id = ?1", [id], |r| r.get(0))
        .ok()?;
    serde_json::from_str::<Vec<String>>(text?.as_str()).ok().filter(|list| !list.is_empty())
}

fn save_models_json(
    db: &std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>,
    id: &str,
    models: &[String],
) {
    let Ok(json) = serde_json::to_string(models) else { return };
    if let Ok(conn) = db.try_lock() {
        let _ = conn.execute("UPDATE connections SET models_json = ?2 WHERE id = ?1", rusqlite::params![id, json]);
    }
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

/// `POST /v1beta/models/{model}:generateContent`（Gemini CLI 原生入口）。
/// 模型 id 允许带斜杠（`<slug>/<model>`），所以用 catch-all 路径参数。
async fn gemini_generate(
    State(deps): State<Arc<ProxyDeps>>,
    axum::extract::Path(call): axum::extract::Path<String>,
    request: Request,
) -> Response {
    let Some((model, method)) = call.rsplit_once(':') else {
        return crate::gateway::adapter::gemini::error_response(
            StatusCode::NOT_FOUND,
            "路径应为 /v1beta/models/{model}:generateContent",
        );
    };
    let method = match method {
        "generateContent" => crate::gateway::adapter::gemini::GeminiMethod::Generate,
        "streamGenerateContent" => crate::gateway::adapter::gemini::GeminiMethod::Stream,
        "countTokens" => crate::gateway::adapter::gemini::GeminiMethod::Count,
        other => {
            return crate::gateway::adapter::gemini::error_response(
                StatusCode::NOT_FOUND,
                &format!("不支持的方法：{other}"),
            );
        }
    };
    let body = match axum::body::to_bytes(request.into_body(), BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(error) => {
            return crate::gateway::adapter::gemini::error_response(
                StatusCode::BAD_REQUEST,
                &format!("读取请求体失败：{error}"),
            );
        }
    };
    crate::gateway::adapter::gemini::handle(deps, model, method, body).await
}

/// `GET /v1beta/models`：Gemini 格式的模型清单（只读缓存，不在这里打上游）。
async fn gemini_models(State(deps): State<Arc<ProxyDeps>>) -> Response {
    let mut pool = {
        let guard = match deps.pool_index() {
            Ok(g) => g,
            Err(_) => return error_json(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用"),
        };
        crate::gateway::route::pool_snapshot(&deps.db, guard)
    };
    if let Ok(conn) = deps.db.try_lock() {
        let order = crate::gateway::groups::source_order(&conn);
        pool.sort_by_key(|entry| order.get(&entry.connection_id).copied().unwrap_or((i64::MAX, i64::MAX)));
    }
    let now = now_ms();
    let mut models: Vec<serde_json::Value> = Vec::new();
    for entry in &pool {
        let list: Vec<String> = if entry.kind == "api" {
            MODEL_CACHE
                .lock()
                .ok()
                .and_then(|cache| {
                    cache
                        .get(&entry.connection_id)
                        .filter(|(at, _)| now - *at < MODEL_CACHE_TTL_MS)
                        .map(|(_, list)| list.clone())
                })
                .or_else(|| load_models_json(&deps.db, &entry.connection_id))
                .or_else(|| entry.model.clone().map(|model| vec![model]))
                .unwrap_or_default()
        } else {
            builtin_subscription_models(&entry.platform).iter().map(|m| m.to_string()).collect()
        };
        for model in list {
            models.push(serde_json::json!({
                "name": format!("models/{}/{}", entry.slug, model),
                "displayName": format!("{} · {model}", entry.name),
                "supportedGenerationMethods": ["generateContent", "streamGenerateContent"],
            }));
        }
    }
    json_response(StatusCode::OK, serde_json::json!({"models": models}).to_string())
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
    async fn models_endpoint_answers_empty_list_when_pool_empty() {
        let base = spawn_router(crate::proxy::test_support::bare_deps()).await;
        let body = reqwest::Client::new()
            .get(format!("{base}/v1/models"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["object"], "list");
        assert_eq!(value["data"].as_array().map(Vec::len), Some(0));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn workbuddy_menu_does_not_fabricate_default_for_an_unavailable_source() {
        let deps = crate::proxy::test_support::bare_deps();
        let sealed = crate::provider_browser_auth::seal("wb-menu", &crate::provider_browser_auth::Credential::Workbuddy {
            edition: "cn".into(), session: crate::workbuddy_browser_auth::Session {
                token: "fixture-token".into(), domain: "www.workbuddy.cn".into(), user_id: "fixture-user".into(),
                enterprise_id: None, email: None, expires_at: None,
            },
        }).unwrap();
        deps.db.lock().unwrap().execute("INSERT INTO connections(id,platform,kind,name,secret,status,serve_enabled,created_ms,slug,models_json) VALUES('wb-menu','workbuddy','auth','WorkBuddy',?1,'invalid',1,1,'wb-menu','[\"default\"]')", [sealed]).unwrap();
        let response = models(State(deps)).await;
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(value["data"].as_array().unwrap().is_empty(), "默认别名不能当作已查询的账号模型");
        assert_eq!(value["unavailable"][0], "wb-menu");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn trae_menu_uses_bound_catalog_and_reports_invalid_auth_instead_of_handwritten_model() {
        use sha2::{Digest, Sha256};
        let deps = crate::proxy::test_support::bare_deps();
        let secret = crate::provider_browser_auth::seal("trae-menu", &crate::provider_browser_auth::Credential::Trae {
            edition: "cn".into(), session: crate::trae_browser_auth::Session {
                token: "fixture-menu".into(), host: "https://api.trae.cn".into(),
                expires_at: "2099-01-01T00:00:00Z".into(), account: None, user_id: "menu-user".into(),
            },
        }).unwrap();
        let cached = serde_json::json!({"credential_hash":format!("{:x}", Sha256::digest(secret.as_bytes())),"edition":"cn",
            "fetched_ms":chrono::Utc::now().timestamp_millis(),"models":["live-model"]}).to_string();
        deps.db.lock().unwrap().execute("INSERT INTO connections(id,platform,kind,name,secret,status,serve_enabled,created_ms,slug,model,models_json) VALUES('trae-menu','trae','auth','Trae',?1,'connected',1,1,'trae-menu','handwritten',?2)", rusqlite::params![secret,cached]).unwrap();
        let response = models(State(deps.clone())).await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["data"][0]["id"], "trae-menu/live-model");
        deps.db.lock().unwrap().execute("UPDATE connections SET secret='invalid-fixture' WHERE id='trae-menu'", []).unwrap();
        let response = models(State(deps)).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("fixture-menu"));
    }

    #[tokio::test]
    async fn models_endpoint_reports_workbuddy_authorization_error() {
        let deps = crate::proxy::test_support::bare_deps();
        deps.db.lock().unwrap().execute(
            "INSERT INTO connections(id,platform,kind,name,secret,status,serve_enabled,created_ms,slug)
             VALUES('wb-auth','workbuddy','auth','WorkBuddy','invalid-fixture','connected',1,1,'wb-auth')",
            [],
        ).unwrap();
        let response = models(State(deps)).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["message"], "浏览器授权无法解锁，请使用原 Windows 用户重新授权");
    }

    /// models 端点真实清单：API 连接调上游 /v1/models（mock），订阅连接用内置清单。
    #[tokio::test]
    async fn models_endpoint_lists_pool_with_prefix_and_reports_unavailable() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let upstream_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = socket.read(&mut chunk).await.unwrap();
                if n == 0 { break; }
                raw.extend_from_slice(&chunk[..n]);
                if String::from_utf8_lossy(&raw).contains("\r\n\r\n") { break; }
            }
            let body = "{\"data\":[{\"id\":\"kimi-k2\"},{\"id\":\"kimi-k2-turbo\"}]}";
            let head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
            socket.flush().await.unwrap();
        });

        let deps = crate::proxy::test_support::bare_deps();
        {
            let conn = deps.db.lock().unwrap();
            conn.execute(
                "INSERT INTO connections(id,platform,kind,name,secret,masked,base_url,protocol,serve_enabled,created_ms,slug)
                 VALUES('c-kimi','custom','api','Kimi','sk-test','****',?1,'anthropic',1,1,'kimi')",
                [format!("http://{upstream_addr}")],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO connections(id,platform,kind,name,secret,masked,serve_enabled,created_ms,slug)
                 VALUES('c-claude','claude','auth','Claude 订阅','auth-x','****',1,2,'claude')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO connections(id,platform,kind,name,secret,masked,base_url,protocol,serve_enabled,created_ms,slug)
                 VALUES('c-dead','custom','api','Dead','sk-dead','****','http://127.0.0.1:1','openai',1,3,'dead')",
                [],
            )
            .unwrap();
        }
        // 清缓存保证走真实拉取
        MODEL_CACHE.lock().unwrap().clear();

        let base = spawn_router(deps).await;
        let body = reqwest::Client::builder().no_proxy().build().unwrap()
            .get(format!("{base}/v1/models"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        let ids: Vec<&str> = value["data"].as_array().unwrap()
            .iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"kimi/kimi-k2"), "{ids:?}");
        assert!(ids.contains(&"kimi/kimi-k2-turbo"), "{ids:?}");
        assert!(ids.contains(&"claude/claude-sonnet-5"), "订阅连接用内置清单：{ids:?}");
        let unavailable = value["unavailable"].as_array().unwrap();
        assert_eq!(unavailable.len(), 1, "拉取失败的连接要单独标出：{body}");
        assert_eq!(unavailable[0], "dead");
    }

    /// 暂停的来源仍在菜单里，带上状态，并用上次成功的模型清单——已存的 id 不能消失。
    #[tokio::test]
    async fn paused_source_stays_in_menu_with_status_and_last_models() {
        let deps = crate::proxy::test_support::bare_deps();
        {
            let conn = deps.db.lock().unwrap();
            conn.execute(
                "INSERT INTO connections(id,platform,kind,name,secret,masked,base_url,protocol,serve_enabled,status,created_ms,slug,models_json)
                 VALUES('c-paused-menu','custom','api','Kimi','sk-test','****','http://127.0.0.1:1','anthropic',1,'paused',1,'kimi','[\"kimi-k2\"]')",
                [],
            )
            .unwrap();
        }
        let base = spawn_router(deps).await;
        let body = reqwest::Client::builder().no_proxy().build().unwrap()
            .get(format!("{base}/v1/models"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        let ids: Vec<&str> = value["data"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["kimi/kimi-k2"], "{body}");
        let item = &value["data"][0];
        assert_eq!(item["status"], "paused");
        assert!(item["display_name"].as_str().unwrap().contains("已暂停"), "{body}");
    }

    /// 浏览器跨站请求（带 Origin）在进入任何 handler 之前就被拒绝。
    #[tokio::test]
    async fn cross_site_browser_requests_are_rejected() {
        let base = spawn_router(crate::proxy::test_support::bare_deps()).await;
        let res = reqwest::Client::builder().no_proxy().build().unwrap()
            .get(format!("{base}/v1/models"))
            .header("origin", "https://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), reqwest::StatusCode::FORBIDDEN);
        let res = reqwest::Client::builder().no_proxy().build().unwrap()
            .get(format!("{base}/v1/models"))
            .header("host", "evil.example:12731")
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), reqwest::StatusCode::FORBIDDEN, "非回环 Host 必须拒绝");
    }

    #[test]
    fn loopback_host_accepts_only_local_names() {
        for ok in ["127.0.0.1:12731", "localhost:12731", "[::1]:12731", "127.0.0.1"] {
            assert!(loopback_host(ok), "{ok}");
        }
        for bad in ["evil.example:12731", "10.0.0.2:12731", "127.0.0.1.evil.example", ""] {
            assert!(!loopback_host(bad), "{bad}");
        }
    }

    /// Codex 保留 OAuth 时走 `/backend-api/codex/responses`，必须与 `/v1/responses` 同一处理链。
    #[tokio::test]
    async fn codex_backend_path_reaches_the_gateway() {
        let base = spawn_router(crate::proxy::test_support::bare_deps()).await;
        let res = reqwest::Client::new()
            .post(format!("{base}/backend-api/codex/responses"))
            .header("content-type", "application/json")
            .body(r#"{"model":"kimi/kimi-k2","input":[]}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);
        let body = res.text().await.unwrap();
        assert!(body.contains("模型池为空"), "{body}");
    }

    /// Gemini CLI 原生端点：请求翻成 OpenAI Chat，响应再翻回 Gemini 形状。
    #[tokio::test]
    async fn gemini_generate_translates_to_openai_chat_end_to_end() {
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
            let request = String::from_utf8_lossy(&raw).to_string();
            let body = r#"{"choices":[{"message":{"content":"hi from openai"},"finish_reason":"stop"}],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}"#;
            let head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
            socket.flush().await.unwrap();
            request.to_lowercase()
        });

        let deps = crate::proxy::test_support::bare_deps();
        {
            let conn = deps.db.lock().unwrap();
            conn.execute(
                "INSERT INTO connections(id,platform,kind,name,secret,masked,base_url,protocol,serve_enabled,created_ms,slug)
                 VALUES('c-gem','custom','api','Gem','sk-gem','****',?1,'openai',1,1,'gem')",
                [format!("http://{upstream_addr}")],
            )
            .unwrap();
        }
        let base = spawn_router(deps).await;
        let response = reqwest::Client::builder().no_proxy().build().unwrap()
            .post(format!("{base}/v1beta/models/gem/gemini-2.5-pro:generateContent"))
            .header("content-type", "application/json")
            .body(r#"{"contents":[{"role":"user","parts":[{"text":"hello"}]}],"generationConfig":{"temperature":0.1}}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(body["candidates"][0]["content"]["parts"][0]["text"], "hi from openai");
        assert_eq!(body["candidates"][0]["finishReason"], "STOP");
        assert_eq!(body["usageMetadata"]["totalTokenCount"], 10);

        let request = upstream.await.unwrap();
        assert!(request.starts_with("post /v1/chat/completions "), "{request}");
        assert!(request.contains("authorization: bearer sk-gem"), "{request}");
        assert!(request.contains("\"model\":\"gemini-2.5-pro\""), "slug 必须被去掉：{request}");
        assert!(request.contains("\"stream\":false"), "{request}");
        assert!(request.contains("\"role\":\"user\""), "{request}");
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
                "INSERT INTO connections(id,platform,kind,name,secret,masked,base_url,protocol,serve_enabled,created_ms,slug)
                 VALUES('c-kimi','custom','api','Kimi','sk-upstream-key','****','anthropic-upstream-placeholder','anthropic',1,1,'kimi')",
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
