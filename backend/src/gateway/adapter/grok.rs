//! Grok Build OAuth 走官方 cli-chat-proxy；不得改用公共 API Key 端点。
use crate::provider_browser_auth::{self, Credential};
use crate::{
    gateway::{
        route::PoolEntry,
        wire::{error_response, CallerProtocol},
    },
    proxy::ProxyDeps,
};
use axum::{
    body::Bytes,
    http::{HeaderMap, Method, StatusCode},
    response::Response,
};
use std::sync::Arc;

const BASE: &str = "https://cli-chat-proxy.grok.com/v1";
const ISSUER: &str = "https://auth.x.ai";

pub(crate) fn bound(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<(String, crate::grok_browser_auth::Session), String> {
    let secret: String = conn
        .query_row(
            "SELECT secret FROM connections WHERE id=?1 AND platform='grok' AND kind='auth'",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| "所选 Grok 账号缺少授权，请重新登录")?;
    let Credential::Grok { session } = provider_browser_auth::load(id, "grok", &secret)? else {
        return Err("Grok 授权类型不匹配".into());
    };
    if session.user_id.trim().is_empty() || session.access_token.trim().is_empty() {
        return Err("Grok 授权身份不完整，请重新登录".into());
    }
    Ok((secret, session))
}

async fn token_at(deps: &Arc<ProxyDeps>, id: &str, issuer: &str) -> Result<String, String> {
    let _guard = crate::gateway::subscription::lock(id).await;
    let (secret, mut session) = {
        let conn = deps.db.lock().map_err(|_| "无法读取 Grok 授权")?;
        bound(&conn, id)?
    };
    if session
        .expires_at
        .is_some_and(|at| at <= chrono::Utc::now().timestamp_millis() + 60_000)
    {
        let refresh = session
            .refresh_token
            .as_deref()
            .ok_or("Grok 授权已过期，请重新登录")?;
        let response = deps
            .client
            .post(format!("{issuer}/oauth2/token"))
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh),
                ("client_id", "b1a00492-073a-47ea-816f-4c329264a828"),
            ])
            .send()
            .await
            .map_err(|_| "Grok 授权刷新失败")?;
        if !response.status().is_success() {
            return Err("Grok 授权刷新被拒绝，请重新登录".into());
        }
        let value: serde_json::Value = response.json().await.map_err(|_| "Grok 刷新响应无效")?;
        let access = value["access_token"]
            .as_str()
            .filter(|v| !v.trim().is_empty())
            .ok_or("Grok 刷新缺少访问凭据")?;
        let response = deps
            .client
            .get(format!("{issuer}/oauth2/userinfo"))
            .bearer_auth(access)
            .send()
            .await
            .map_err(|_| "无法核实 Grok 刷新身份")?;
        if !response.status().is_success() {
            return Err("无法核实 Grok 刷新身份".into());
        }
        let identity: serde_json::Value = response.json().await.map_err(|_| "Grok 身份响应无效")?;
        if identity["sub"].as_str() != Some(session.user_id.as_str()) {
            return Err("Grok 刷新账号发生变化，已停止转发".into());
        }
        session.access_token = access.into();
        if let Some(refresh) = value["refresh_token"].as_str().filter(|v| !v.is_empty()) {
            session.refresh_token = Some(refresh.into());
        }
        session.expires_at = value["expires_in"]
            .as_i64()
            .filter(|n| *n > 0)
            .and_then(|n| n.checked_mul(1000))
            .and_then(|n| chrono::Utc::now().timestamp_millis().checked_add(n));
        let conn = deps.db.lock().map_err(|_| "无法保存 Grok 授权")?;
        provider_browser_auth::refresh_bound(
            &conn,
            id,
            "grok",
            &secret,
            &Credential::Grok {
                session: session.clone(),
            },
        )?;
    }
    Ok(session.access_token)
}

pub(crate) async fn models(deps: &Arc<ProxyDeps>, id: &str) -> Result<Vec<String>, String> {
    let token = token_at(deps, id, ISSUER).await?;
    let response = deps
        .client
        .get(format!("{BASE}/models"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| "Grok 模型查询失败")?;
    if !response.status().is_success() {
        return Err("Grok 模型查询被拒绝".into());
    }
    let value: serde_json::Value = response.json().await.map_err(|_| "Grok 模型列表无效")?;
    let list = value["data"]
        .as_array()
        .ok_or("Grok 模型列表格式不受支持")?;
    Ok(list
        .iter()
        .filter_map(|v| v["id"].as_str())
        .filter(|v| !v.is_empty() && v.len() <= 256)
        .map(str::to_owned)
        .collect())
}

pub(crate) async fn forward(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    protocol: CallerProtocol,
    body: Bytes,
) -> Response {
    if protocol == CallerProtocol::Anthropic {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Grok 订阅支持 Chat / Responses 协议",
        );
    }
    let token = match token_at(&deps, &entry.connection_id, ISSUER).await {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::UNAUTHORIZED, &e),
    };
    let value: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "请求格式无效"),
    };
    let Some(model) = value["model"]
        .as_str()
        .and_then(|m| m.split_once('/').map(|(_, m)| m))
    else {
        return error_response(StatusCode::BAD_REQUEST, "请选择来源和模型");
    };
    let mut target = entry.clone();
    target.base_url = Some(BASE.into());
    target.secret = Some(token);
    target.protocol = "openai".into();
    let path = if protocol == CallerProtocol::Responses {
        "/responses"
    } else {
        "/chat/completions"
    };
    super::direct::forward(
        deps,
        &target,
        model.into(),
        Method::POST,
        path,
        &HeaderMap::new(),
        body,
    )
    .await
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use axum::routing::{get, post};
    use serde_json::json;

    #[tokio::test]
    async fn refresh_is_serial_and_bound_to_selected_identity() {
        let deps = crate::proxy::test_support::bare_deps();
        for id in ["a", "b"] {
            let session = crate::grok_browser_auth::Session {
                access_token: format!("{id}-old"),
                refresh_token: Some(format!("{id}-refresh")),
                expires_at: Some(1),
                account: Some(format!("{id}@example.invalid")),
                user_id: id.into(),
            };
            let sealed = provider_browser_auth::seal(id, &Credential::Grok { session }).unwrap();
            deps.db.lock().unwrap().execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug) VALUES(?1,'grok','auth',?1,?2,1,?1)", rusqlite::params![id,sealed]).unwrap();
        }
        let app = axum::Router::new()
            .route(
                "/oauth2/token",
                post(|body: String| async move {
                    assert!(body.contains("a-refresh"));
                    assert!(!body.contains("b-refresh"));
                    json!({"access_token":"a-new","refresh_token":"a-next","expires_in":3600})
                        .to_string()
                }),
            )
            .route(
                "/oauth2/userinfo",
                get(|headers: HeaderMap| async move {
                    assert_eq!(headers["authorization"], "Bearer a-new");
                    json!({"sub":"a"}).to_string()
                }),
            );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let (one, two) = tokio::join!(token_at(&deps, "a", &base), token_at(&deps, "a", &base));
        assert_eq!(one.unwrap(), "a-new");
        assert_eq!(two.unwrap(), "a-new");
        assert_eq!(
            bound(&deps.db.lock().unwrap(), "b").unwrap().1.access_token,
            "b-old"
        );
        assert!(token_at(&deps, "missing", &base).await.is_err());
        server.abort();
    }
}
