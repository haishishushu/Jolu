//! Trae 账号目录选路与 llm_utils_chat SSE 转发。
//! 仅使用所选连接已有授权；真实订阅调用尚未验收。
use crate::{
    gateway::{
        route::PoolEntry,
        wire::{error_response, CallerProtocol},
    },
    provider_browser_auth::{self, Credential},
    proxy::ProxyDeps,
    trae_browser_auth::Session,
};
#[cfg(test)]
use axum::body::Body;
use axum::{
    body::Bytes,
    http::StatusCode,
    response::Response,
};
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) fn bound(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<(String, String, Session), String> {
    let sealed: String = conn
        .query_row(
            "SELECT secret FROM connections WHERE id=?1 AND platform='trae' AND kind='auth'",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| "所选 Trae 账号缺少完整授权，请重新登录")?;
    let Credential::Trae { edition, session } = provider_browser_auth::load(id, "trae", &sealed)?
    else {
        return Err("Trae 授权类型不匹配".into());
    };
    model_host(&edition, &session.host)?;
    if session.token.trim().is_empty()
        || session.token.len() >= 65536
        || session.token.chars().any(char::is_control)
        || session.user_id.trim().is_empty()
        || session.user_id.len() > 320
        || session.user_id == "0"
        || session.user_id.chars().any(char::is_control)
        || !chrono::DateTime::parse_from_rfc3339(&session.expires_at)
            .is_ok_and(|v| v > chrono::Utc::now())
    {
        return Err("Trae 授权身份不完整或已过期，请重新登录此账号".into());
    }
    Ok((sealed, edition, session))
}
pub(super) fn model_host(edition: &str, host: &str) -> Result<&'static str, String> {
    // product.json bootConfig.agent.trae；登录地区只映射到官方模型域名。
    match (edition, host) {
        ("cn", "https://api.trae.cn") => Ok("https://trae-api-cn.mchost.guru"),
        ("global", "https://growsg-normal.trae.ai") => Ok("https://coresg-normal.trae.ai"),
        ("global", "https://grow-normal.traeapi.us") => Ok("https://core-normal.traeapi.us"),
        _ => Err("Trae 账号地区与授权地址不匹配，请重新登录".into()),
    }
}
pub(super) fn current(
    deps: &Arc<ProxyDeps>,
    entry: &PoolEntry,
    secret: &str,
) -> Result<(), String> {
    if !crate::gateway::route::source_is_current(deps, entry) {
        return Err("Trae 来源已关闭或配置变化".into());
    }
    let conn = deps.db.lock().map_err(|_| "无法复查 Trae 授权")?;
    if !bound(&conn, &entry.connection_id).is_ok_and(|v| v.0 == secret) {
        return Err("Trae 授权变化，已丢弃旧账号回复".into());
    }
    Ok(())
}

pub(crate) async fn forward(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    protocol: CallerProtocol,
    body: Bytes,
    session: &str,
) -> Response {
    forward_inner(deps, entry, protocol, body, None, Some(session)).await
}
pub(super) async fn read_json(
    mut response: reqwest::Response,
    max: usize,
) -> Result<Value, String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Trae 上游响应读取失败")?
    {
        if bytes.len() + chunk.len() > max {
            return Err("Trae 上游响应超过大小限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "Trae 返回了无效 JSON".into())
}
#[cfg(test)]
async fn forward_at(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    protocol: CallerProtocol,
    body: Bytes,
    test_base: Option<&str>,
) -> Response {
    forward_inner(deps, entry, protocol, body, test_base, None).await
}
async fn forward_inner(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    protocol: CallerProtocol,
    body: Bytes,
    test_base: Option<&str>,
    caller_session: Option<&str>,
) -> Response {
    if protocol != CallerProtocol::Chat {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Trae 使用 Chat 协议；Messages 由网关转换，暂不支持 Responses",
        );
    }
    let value: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Trae 请求不是有效 JSON"),
    };
    let prefix = format!("{}/", entry.slug);
    let Some(model) = value["model"]
        .as_str()
        .and_then(|s| s.strip_prefix(&prefix))
        .map(str::to_owned)
    else {
        return error_response(StatusCode::BAD_REQUEST, "Trae 模型来源无效");
    };
    let streaming = value["stream"].as_bool().unwrap_or(false);
    if let Err(e) = super::trae_models::prepare_route(&deps, entry, &model, test_base).await {
        return error_response(
            if crate::gateway::route::source_is_current(&deps, entry) {
                StatusCode::BAD_GATEWAY
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            },
            &e,
        );
    }
    let guard = crate::gateway::subscription::lock(&entry.connection_id).await;
    // 请求可能在账号锁上排队；此时 wire 层校验的来源快照可能已失效。
    if !crate::gateway::route::source_is_current(&deps, entry) {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "来源已关闭或配置已变化，请刷新后重试",
        );
    }
    let selected = deps
        .db
        .lock()
        .map_err(|_| "无法读取 Trae 授权".to_string())
        .and_then(|conn| bound(&conn, &entry.connection_id));
    let (sealed, edition, session) = match selected {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::UNAUTHORIZED, &e),
    };
    let route = match deps
        .db
        .lock()
        .ok()
        .and_then(|conn| super::trae_models::cached_route(&conn, &entry.connection_id, &model))
    {
        Some(v) => v,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "Trae 模型路由已过期或授权变化，请刷新目录",
            )
        }
    };
    let session_id = match super::trae_chat::conversation(
        &entry.connection_id,
        &sealed,
        &value.to_string(),
        caller_session,
    ) {
        Ok(s) => s,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    let request = match super::trae_chat::request(&body, &entry.slug, &route, &session_id) {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &e),
    };
    let model_origin = match model_host(&edition, &session.host) {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::UNAUTHORIZED, &e),
    };
    // 遵循 Jolu 联网方式；凭据请求绝不跟随服务器重定向。
    let builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(180));
    let builder = if cfg!(test) && test_base.is_some() {
        builder.no_proxy()
    } else {
        match crate::net::current_mode() {
            crate::net::NetMode::Direct => builder.no_proxy(),
            crate::net::NetMode::Manual(url) => match reqwest::Proxy::all(url) {
                Ok(proxy) => builder.proxy(proxy),
                Err(_) => return error_response(StatusCode::BAD_GATEWAY, "Trae 代理配置无效"),
            },
            crate::net::NetMode::System => match crate::net::system_proxy_url() {
                Some(url) => match reqwest::Proxy::all(url) {
                    Ok(proxy) => builder.proxy(proxy),
                    Err(_) => return error_response(StatusCode::BAD_GATEWAY, "Trae 系统代理无效"),
                },
                None => builder,
            },
        }
    };
    let client = match builder.build() {
        Ok(v) => v,
        Err(_) => return error_response(StatusCode::BAD_GATEWAY, "无法创建 Trae 上游连接"),
    };
    let test_base = if cfg!(test) { test_base } else { None };
    let identity_origin = test_base.unwrap_or(&session.host);
    let model_origin = test_base.unwrap_or(model_origin);
    let identity = client
        .post(format!(
            "{identity_origin}/cloudide/api/v3/trae/GetUserInfo"
        ))
        .header("x-cloudide-token", &session.token)
        .json(&json!({"ReqSource":"IDE","IDEVersion":if edition=="cn"{"3.4.4"}else{"3.6.4"}}))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await;
    let identity = match identity {
        Ok(v) if v.status().is_success() => v,
        _ => {
            return error_response(
                StatusCode::UNAUTHORIZED,
                "Trae 所选账号身份核实失败，请重新登录",
            )
        }
    };
    let identity = match read_json(identity, 128 * 1024).await {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::UNAUTHORIZED, &e),
    };
    let user = identity["Result"]["UserID"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| identity["Result"]["UserID"].as_u64().map(|v| v.to_string()));
    if identity
        .pointer("/ResponseMetadata/Error")
        .is_some_and(|v| !v.is_null())
        || user.as_deref() != Some(&session.user_id)
    {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "Trae 返回账号与所选连接不一致，已停止转发",
        );
    }
    let current = deps
        .db
        .lock()
        .map_err(|_| "无法复查 Trae 授权".to_string())
        .and_then(|conn| bound(&conn, &entry.connection_id));
    if !current.is_ok_and(|v| v.0 == sealed) {
        return error_response(StatusCode::UNAUTHORIZED, "Trae 授权已变化，请重新发起请求");
    }
    // 身份核实期间也可关闭来源，实际消耗模型额度前必须再次复查。
    if !crate::gateway::route::source_is_current(&deps, entry) {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "来源已关闭或配置已变化，请刷新后重试",
        );
    }
    let upstream = client
        .post(format!("{model_origin}/api/agent/v3/llm_utils_chat"))
        .header("authorization", format!("Cloud-IDE-JWT {}", session.token))
        .json(&request)
        .send()
        .await;
    let upstream = match upstream {
        Ok(v) => v,
        Err(_) => return error_response(StatusCode::BAD_GATEWAY, "Trae 模型请求失败"),
    };
    let status = upstream.status();
    // 请求期间可能重新登录。旧响应不得污染新授权的状态或用量。
    let current = deps
        .db
        .lock()
        .map_err(|_| "无法复查 Trae 授权".to_string())
        .and_then(|conn| bound(&conn, &entry.connection_id));
    if !current.is_ok_and(|v| v.0 == sealed) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "Trae 授权已变化，已丢弃旧账号响应",
        );
    }
    if status.is_redirection() {
        return error_response(
            StatusCode::BAD_GATEWAY,
            "Trae 上游返回重定向，已拒绝转发授权",
        );
    }
    if !status.is_success() {
        // 只返回状态及安全提示，不能回显可能含授权的上游错误正文。
        let mut response = error_response(
            status,
            &format!("Trae 模型请求被拒绝（HTTP {}）", status.as_u16()),
        );
        if let Some(value) = upstream
            .headers()
            .get("retry-after")
            .filter(|v| v.to_str().is_ok_and(|s| s.parse::<u64>().is_ok()))
        {
            response.headers_mut().insert("retry-after", value.clone());
        }
        return response;
    }
    if !upstream
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| {
            s.split(';')
                .next()
                .is_some_and(|s| s.trim().eq_ignore_ascii_case("text/event-stream"))
        })
    {
        return error_response(StatusCode::BAD_GATEWAY, "Trae 未返回 SSE 对话协议");
    }
    let tools = if value["tool_choice"] == "none" {
        Vec::new()
    } else {
        value["tools"].as_array().cloned().unwrap_or_default()
    };
    super::trae_stream::response(
        super::trae_stream::Reader::new(
            upstream,
            deps,
            entry.clone(),
            sealed,
            guard,
            value["model"].as_str().unwrap_or("").into(),
            &tools,
        ),
        streaming,
    )
    .await
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use axum::{http::HeaderMap, routing::post, Router};
    use std::sync::Mutex;
    fn session(host: &str, id: &str) -> Session {
        Session {
            token: format!("fixture-{id}"),
            host: host.into(),
            expires_at: "2099-01-01T00:00:00Z".into(),
            account: None,
            user_id: id.into(),
        }
    }
    fn insert(deps: &Arc<ProxyDeps>, id: &str, edition: &str, session: Session) {
        let secret = provider_browser_auth::seal(
            id,
            &Credential::Trae {
                edition: edition.into(),
                session,
            },
        )
        .unwrap();
        deps.db.lock().unwrap().execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug,serve_enabled,status,account_fp) VALUES(?1,'trae','auth',?1,?2,1,?1,1,'connected',NULL)",rusqlite::params![id,secret]).unwrap();
    }
    fn entry(id: &str) -> PoolEntry {
        PoolEntry {
            connection_id: id.into(),
            slug: id.into(),
            name: id.into(),
            platform: "trae".into(),
            kind: "auth".into(),
            protocol: "openai".into(),
            base_url: None,
            secret: None,
            account_fp: None,
            model: None,
            status: "connected".into(),
        }
    }
    fn body(id: &str) -> Bytes {
        Bytes::from(json!({"model":format!("{id}/explicit-model"),"messages":[{"role":"user","content":"fixture"}]}).to_string())
    }
    #[tokio::test]
    async fn selected_catalog_route_streams_and_collects_chat_instead_of_old_sdk() {
        let deps = crate::proxy::test_support::bare_deps();
        insert(
            &deps,
            "new-trae",
            "cn",
            session("https://api.trae.cn", "new-trae"),
        );
        let app = Router::new()
            .route("/cloudide/api/v3/trae/GetUserInfo", post(||async{json!({"Result":{"UserID":"new-trae"}}).to_string()}))
            .route("/api/ide/v1/batch_get_detail_param", post(||async{json!({"function_configs":[{"function":"solo_agent","config_info_list":[{"config_name":"explicit-model","model_detail_list":[{"model_name":"model__dev","max_tokens":8192}]}]}]}).to_string()}))
            .route("/api/agent/v3/llm_utils_chat", post(|headers:HeaderMap, body:String|async move {
                assert_eq!(headers["authorization"], "Cloud-IDE-JWT fixture-new-trae");
                let v:Value=serde_json::from_str(&body).unwrap();
                assert_eq!(v["function"],"solo_agent");assert_eq!(v["model_name"],"model__dev");assert_eq!(v["stream"],true);
                Response::builder().header("content-type","text/event-stream").body(Body::from("event: output\ndata: {\"response\":\"你好\"}\n\nevent: TokenUsage\ndata: {\"prompt_tokens\":0,\"completion_tokens\":1}\n\nevent: done\ndata: {}\n\n")).unwrap()
            }));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        for streaming in [false, true] {
            let mut request: Value = serde_json::from_slice(&body("new-trae")).unwrap();
            request["stream"] = json!(streaming);
            let response = forward_at(
                deps.clone(),
                &entry("new-trae"),
                CallerProtocol::Chat,
                Bytes::from(request.to_string()),
                Some(&base),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            if streaming {
                assert!(text.contains("你好"));
                assert!(text.ends_with("data: [DONE]\n\n"));
            } else {
                let v: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(v["choices"][0]["message"]["content"], "你好");
                assert_eq!(v["usage"]["prompt_tokens"], 0);
                assert!(v["usage"].get("total_tokens").is_none());
            }
        }
        // 本地上游回环覆盖 Messages 请求和回复转换，不发送真实账号请求。
        for streaming in [false, true] {
            let messages = json!({"model":"new-trae/explicit-model","max_tokens":100,"stream":streaming,"messages":[{"role":"user","content":"fixture"}]});
            let chat =
                crate::gateway::translate::anthropic_request(messages.to_string().as_bytes())
                    .unwrap();
            let response = forward_at(
                deps.clone(),
                &entry("new-trae"),
                CallerProtocol::Chat,
                chat,
                Some(&base),
            )
            .await;
            let response =
                crate::gateway::translate::anthropic_response(response, "new-trae/explicit-model")
                    .await;
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            if streaming {
                assert!(text.contains("event: message_stop") && text.contains("你好"));
            } else {
                let value: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(value["content"][0]["text"], "你好");
                assert_eq!(value["usage"]["input_tokens"], 0);
                assert_eq!(value["usage"]["output_tokens"], 1);
            }
        }
        task.abort();
    }
    #[tokio::test]
    async fn dropping_stream_releases_account_lock_and_eof_never_sends_success() {
        use futures_util::StreamExt;
        let deps = crate::proxy::test_support::bare_deps();
        let id = "trae-drop";
        insert(&deps, id, "cn", session("https://api.trae.cn", id));
        let app=Router::new()
            .route("/api/ide/v1/batch_get_detail_param",post(||async{json!({"function_configs":[{"function":"chat_v3","config_info_list":[{"config_name":"explicit-model"}]}]}).to_string()}))
            .route("/cloudide/api/v3/trae/GetUserInfo",post(||async{json!({"Result":{"UserID":"trae-drop"}}).to_string()}))
            .route("/api/agent/v3/llm_utils_chat",post(||async{
                Response::builder().header("content-type","text/event-stream").body(Body::from("event: output\ndata: {\"response\":\"partial\"}\n\n")).unwrap()
            }));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut request: Value = serde_json::from_slice(&body(id)).unwrap();
        request["stream"] = json!(true);
        let response = forward_at(
            deps.clone(),
            &entry(id),
            CallerProtocol::Chat,
            Bytes::from(request.to_string()),
            Some(&base),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let mut stream = response.into_body().into_data_stream();
        assert!(
            String::from_utf8_lossy(&stream.next().await.unwrap().unwrap()).contains("partial")
        );
        drop(stream);
        let guard = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            crate::gateway::subscription::lock(id),
        )
        .await
        .expect("取消流必须释放账号锁");
        drop(guard);
        let response = forward_at(
            deps.clone(),
            &entry(id),
            CallerProtocol::Chat,
            Bytes::from(request.to_string()),
            Some(&base),
        )
        .await;
        let text = String::from_utf8(
            axum::body::to_bytes(response.into_body(), 65536)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(text.contains("\"error\""));
        assert!(!text.contains("[DONE]"));
        assert!(!text.contains("\"finish_reason\":\"stop\""));
        let response = forward_at(
            deps,
            &entry(id),
            CallerProtocol::Chat,
            body(id),
            Some(&base),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        server.abort();
    }
    #[tokio::test]
    async fn queued_request_does_not_contact_a_disabled_source() {
        use std::future::Future;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::task::Poll;
        let id = "queued-trae";
        let deps = crate::proxy::test_support::bare_deps();
        insert(&deps, id, "cn", session("https://api.trae.cn", id));
        let selected = entry(id);
        assert!(crate::gateway::route::source_is_current(&deps, &selected));
        let calls = Arc::new(AtomicUsize::new(0));
        let received = calls.clone();
        let app=Router::new().fallback(move || {let received=received.clone();async move {
            received.fetch_add(1,Ordering::Relaxed);
            json!({"Result":{"UserID":"queued-trae"},"choices":[{"message":{"role":"assistant","content":"fixture"}}]}).to_string()
        }});
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let held = crate::gateway::subscription::lock(id).await;
        let mut pending = Box::pin(forward_at(
            deps.clone(),
            &selected,
            CallerProtocol::Chat,
            body(id),
            Some(&base),
        ));
        // 一次真实 poll 将请求推进到账号锁，避免依赖 sleep 猜测排队时间。
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx).is_pending())).await
        );
        deps.db
            .lock()
            .unwrap()
            .execute("UPDATE connections SET serve_enabled=0 WHERE id=?1", [id])
            .unwrap();
        drop(held);
        let response = pending.await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        server.abort();
    }
    #[test]
    fn official_region_mapping_never_accepts_an_arbitrary_or_cross_edition_host() {
        assert_eq!(
            model_host("cn", "https://api.trae.cn").unwrap(),
            "https://trae-api-cn.mchost.guru"
        );
        assert_eq!(
            model_host("global", "https://growsg-normal.trae.ai").unwrap(),
            "https://coresg-normal.trae.ai"
        );
        assert_eq!(
            model_host("global", "https://grow-normal.traeapi.us").unwrap(),
            "https://core-normal.traeapi.us"
        );
        for (edition, host) in [
            ("global", "https://api.trae.cn"),
            ("cn", "https://growsg-normal.trae.ai"),
            ("cn", "https://api.trae.cn.evil.invalid"),
            ("cn", "https://api.trae.cn/"),
        ] {
            assert!(model_host(edition, host).is_err());
        }
    }
    #[test]
    fn selected_encrypted_connection_cannot_borrow_identity_or_expired_token() {
        let deps = crate::proxy::test_support::bare_deps();
        insert(&deps, "a", "cn", session("https://api.trae.cn", "a"));
        insert(
            &deps,
            "b",
            "global",
            session("https://growsg-normal.trae.ai", "b"),
        );
        let mut expired = session("https://api.trae.cn", "old");
        expired.expires_at = "2000-01-01T00:00:00Z".into();
        insert(&deps, "old", "cn", expired);
        let conn = deps.db.lock().unwrap();
        assert_eq!(bound(&conn, "a").unwrap().2.user_id, "a");
        assert_eq!(bound(&conn, "b").unwrap().2.user_id, "b");
        assert!(bound(&conn, "old").is_err());
        assert!(bound(&conn, "missing").is_err());
        conn.execute("UPDATE connections SET secret=(SELECT secret FROM connections WHERE id='b') WHERE id='a'",[]).unwrap();
        assert!(bound(&conn, "a").is_err());
    }
    #[tokio::test]
    async fn mock_http_uses_only_selected_account_and_known_sdk_path() {
        let deps = crate::proxy::test_support::bare_deps();
        insert(&deps, "a", "cn", session("https://api.trae.cn", "a"));
        insert(
            &deps,
            "b",
            "global",
            session("https://growsg-normal.trae.ai", "b"),
        );
        let captures = Arc::new(Mutex::new(Vec::<Value>::new()));
        let captured = captures.clone();
        let app=Router::new()
        .route("/api/ide/v1/batch_get_detail_param",post(||async{json!({"function_configs":[{"function":"chat_v3","config_info_list":[{"config_name":"explicit-model"}]}]}).to_string()}))
        .route("/cloudide/api/v3/trae/GetUserInfo",post(|headers:HeaderMap| async move {
            let id=headers["x-cloudide-token"].to_str().unwrap().strip_prefix("fixture-").unwrap();
            json!({"Result":{"UserID":id}}).to_string()
        })).route("/api/agent/v3/llm_utils_chat",post(move |headers:HeaderMap,body:String|{let captured=captured.clone();async move {
            assert_eq!(headers["authorization"],"Cloud-IDE-JWT fixture-a");assert!(!headers.contains_key("x-cloudide-token"));
            let value:Value=serde_json::from_str(&body).unwrap();assert_eq!(value["model"],"explicit-model");captured.lock().unwrap().push(value);
            Response::builder().header("content-type","text/event-stream").body(Body::from("event: output\ndata: {\"response\":\"ok\"}\n\nevent: done\ndata: {}\n\n")).unwrap()
        }}));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        for _ in 0..2 {
            let result = forward_at(
                deps.clone(),
                &entry("a"),
                CallerProtocol::Chat,
                body("a"),
                Some(&base),
            )
            .await;
            assert_eq!(result.status(), StatusCode::OK);
        }
        let captures = captures.lock().unwrap();
        assert_eq!(captures.len(), 2);
        assert_eq!(captures[0]["session_id"], captures[1]["session_id"]);
        server.abort();
    }
    #[tokio::test]
    async fn wrong_identity_unknown_envelope_and_redirect_fail_closed() {
        for mode in [
            "identity",
            "envelope",
            "redirect",
            "relogin",
            "response_relogin",
            "upstream_error",
        ] {
            let deps = crate::proxy::test_support::bare_deps();
            insert(&deps, "a", "cn", session("https://api.trae.cn", "a"));
            let shared = deps.clone();
            let after = deps.clone();
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let count = calls.clone();
            let app = Router::new()
                .route("/api/ide/v1/batch_get_detail_param",post(||async{json!({"function_configs":[{"function":"chat_v3","config_info_list":[{"config_name":"explicit-model"}]}]}).to_string()}))
                .route(
                    "/cloudide/api/v3/trae/GetUserInfo",
                    post(move || {
                        let shared = shared.clone();
                        async move {
                            if mode == "relogin" {
                                let secret = provider_browser_auth::seal(
                                    "a",
                                    &Credential::Trae {
                                        edition: "cn".into(),
                                        session: session("https://api.trae.cn", "new-user"),
                                    },
                                )
                                .unwrap();
                                shared
                                    .db
                                    .lock()
                                    .unwrap()
                                    .execute(
                                        "UPDATE connections SET secret=?1 WHERE id='a'",
                                        [secret],
                                    )
                                    .unwrap();
                            }
                            json!({"Result":{"UserID":if mode=="identity"{"wrong-user"}else{"a"}}})
                                .to_string()
                        }
                    }),
                )
                .route(
                    "/api/agent/v3/llm_utils_chat",
                    post(move || {
                        let count = count.clone();
                        let after = after.clone();
                        async move {
                            count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if mode == "response_relogin" {
                                let secret = provider_browser_auth::seal(
                                    "a",
                                    &Credential::Trae {
                                        edition: "cn".into(),
                                        session: session("https://api.trae.cn", "new-user"),
                                    },
                                )
                                .unwrap();
                                after
                                    .db
                                    .lock()
                                    .unwrap()
                                    .execute(
                                        "UPDATE connections SET secret=?1 WHERE id='a'",
                                        [secret],
                                    )
                                    .unwrap();
                            }
                            if mode == "redirect" {
                                Response::builder()
                                    .status(302)
                                    .header("location", "/must-not-follow")
                                    .body(Body::empty())
                                    .unwrap()
                            } else if mode == "upstream_error" {
                                Response::builder()
                                    .status(429)
                                    .header("retry-after", "7")
                                    .body(Body::from("fixture-a raw credential must not leak"))
                                    .unwrap()
                            } else {
                                Response::new(Body::from("{\"data\":{\"choices\":[]}}"))
                            }
                        }
                    }),
                )
                .route(
                    "/must-not-follow",
                    axum::routing::get(|| async {
                        panic!("must not follow an upstream redirect");
                        #[allow(unreachable_code)]
                        ""
                    }),
                );
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let result = forward_at(
                deps.clone(),
                &entry("a"),
                CallerProtocol::Chat,
                body("a"),
                Some(&base),
            )
            .await;
            let expected = match mode {
                "identity" | "relogin" | "response_relogin" => StatusCode::UNAUTHORIZED,
                "upstream_error" => StatusCode::TOO_MANY_REQUESTS,
                _ => StatusCode::BAD_GATEWAY,
            };
            assert_eq!(result.status(), expected, "mode={mode}");
            let bytes = axum::body::to_bytes(result.into_body(), 4096)
                .await
                .unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("fixture-a"));
            if mode == "identity" || mode == "relogin" {
                assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 0);
            }
            server.abort();
        }
    }
}
