//! Google 登录订阅使用 Code Assist 协议，凭据只取所选连接。
use crate::{
    gateway::{route::PoolEntry, wire::error_response},
    provider_browser_auth::{self, Credential},
    proxy::ProxyDeps,
};
use axum::{
    body::{Body, Bytes},
    http::StatusCode,
    response::Response,
};
use serde_json::{json, Value};
use std::sync::Arc;
const BASE: &str = "https://cloudcode-pa.googleapis.com/v1internal";

pub(crate) fn bound(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<(String, crate::cli_browser_auth::Session), String> {
    let secret: String = conn
        .query_row(
            "SELECT secret FROM connections WHERE id=?1 AND platform='gemini' AND kind='auth'",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| "所选 Gemini 账号缺少完整授权，请重新登录")?;
    let Credential::Cli { session } = provider_browser_auth::load(id, "gemini", &secret)? else {
        return Err("Gemini 授权类型不匹配".into());
    };
    if session
        .account
        .as_deref()
        .is_none_or(|v| v.trim().is_empty())
        || !["access_token", "refresh_token"].iter().any(|k| {
            session.credentials[k]
                .as_str()
                .is_some_and(|v| !v.is_empty())
        })
    {
        return Err("Gemini 授权身份不完整，请重新登录".into());
    }
    Ok((secret, session))
}

async fn token(deps: &Arc<ProxyDeps>, id: &str) -> Result<String, String> {
    let _guard = crate::gateway::subscription::lock(id).await;
    let (secret, mut session) = {
        let conn = deps.db.lock().map_err(|_| "无法读取 Gemini 授权")?;
        bound(&conn, id)?
    };
    let fresh = session.credentials["access_token"]
        .as_str()
        .filter(|v| !v.is_empty())
        .filter(|_| {
            session.credentials["expiry_date"]
                .as_i64()
                .is_none_or(|at| at > chrono::Utc::now().timestamp_millis() + 60_000)
        });
    if let Some(value) = fresh {
        return Ok(value.into());
    }
    let refresh = session.credentials["refresh_token"]
        .as_str()
        .ok_or("Gemini 授权已过期，请重新登录")?;
    let (client_id, client_secret) = crate::gemini_quota::installed_client()
        .ok_or("需要安装官方 Gemini CLI 以刷新此账号授权")?;
    let response = deps
        .client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("refresh_token", refresh),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|_| "Gemini 授权刷新失败")?;
    let value = read_json(response).await?;
    let access = value["access_token"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or("Gemini 刷新缺少访问凭据")?
        .to_owned();
    let response = deps
        .client
        .get("https://www.googleapis.com/oauth2/v2/userinfo")
        .bearer_auth(&access)
        .send()
        .await
        .map_err(|_| "Gemini 刷新身份查询失败")?;
    let identity = read_json(response).await?;
    if identity["email"].as_str() != session.account.as_deref() {
        return Err("Gemini 刷新账号发生变化，已停止转发".into());
    }
    session.credentials["access_token"] = json!(access);
    if let Some(refresh) = value["refresh_token"].as_str().filter(|v| !v.is_empty()) {
        session.credentials["refresh_token"] = json!(refresh);
    }
    session.credentials["expiry_date"] = json!(value["expires_in"]
        .as_i64()
        .filter(|n| *n > 0)
        .and_then(|n| n.checked_mul(1000))
        .and_then(|n| chrono::Utc::now().timestamp_millis().checked_add(n)));
    let conn = deps.db.lock().map_err(|_| "无法保存 Gemini 授权")?;
    provider_browser_auth::refresh_bound(
        &conn,
        id,
        "gemini",
        &secret,
        &Credential::Cli { session },
    )?;
    Ok(access)
}

async fn read_json(mut response: reqwest::Response) -> Result<Value, String> {
    if !response.status().is_success() {
        return Err(format!(
            "Gemini 上游请求失败（HTTP {}）",
            response.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "Gemini 响应读取失败")? {
        if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
            return Err("Gemini 响应超过大小限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "Gemini 响应格式无效".into())
}

async fn project(deps: &Arc<ProxyDeps>, access: &str, base: &str) -> Result<String, String> {
    let response = deps.client.post(format!("{base}:loadCodeAssist")).bearer_auth(access).json(&json!({"metadata":{"ideType":"IDE_UNSPECIFIED","platform":"PLATFORM_UNSPECIFIED","pluginType":"GEMINI"}})).send().await.map_err(|_| "Gemini 项目查询失败")?;
    let value = read_json(response).await?;
    value["cloudaicompanionProject"]
        .as_str()
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .ok_or("此 Gemini 账号尚未就绪，请先在官方 CLI 完成开通".into())
}

pub(crate) async fn models(deps: &Arc<ProxyDeps>, id: &str) -> Result<Vec<String>, String> {
    let access = token(deps, id).await?;
    let project = project(deps, &access, BASE).await?;
    let response = deps
        .client
        .post(format!("{BASE}:retrieveUserQuota"))
        .bearer_auth(access)
        .json(&json!({"project":project}))
        .send()
        .await
        .map_err(|_| "Gemini 模型查询失败")?;
    let value = read_json(response).await?;
    let mut models: Vec<String> = value["buckets"]
        .as_array()
        .ok_or("Gemini 未返回模型额度列表")?
        .iter()
        .filter_map(|v| v["modelId"].as_str())
        .filter(|v| !v.is_empty() && v.len() <= 256)
        .map(str::to_owned)
        .collect();
    models.sort();
    models.dedup();
    Ok(models)
}

pub(crate) async fn forward(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    model: &str,
    method: super::gemini::GeminiMethod,
    body: Bytes,
) -> Response {
    forward_at(deps, entry, model, method, body, BASE).await
}

async fn forward_at(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    model: &str,
    method: super::gemini::GeminiMethod,
    body: Bytes,
    base: &str,
) -> Response {
    if method == super::gemini::GeminiMethod::Stream {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Gemini 订阅当前支持非流式 generateContent",
        );
    }
    let request: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "请求格式无效"),
    };
    if !request.is_object() || !request["contents"].is_array() {
        return error_response(StatusCode::BAD_REQUEST, "请求缺少 contents");
    }
    let access = match token(&deps, &entry.connection_id).await {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::UNAUTHORIZED, &e),
    };
    let project = match project(&deps, &access, base).await {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::BAD_GATEWAY, &e),
    };
    let started = std::time::Instant::now();
    let response = match deps.client.post(format!("{base}:generateContent")).bearer_auth(access).json(&json!({"model":model,"project":project,"user_prompt_id":format!("jolu-{}",chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()),"request":request})).send().await {
        Ok(v) => v, Err(_) => return error_response(StatusCode::BAD_GATEWAY, "Gemini 模型请求失败")
    };
    super::record_upstream_signal(&deps, entry, &response);
    let value = match read_json(response).await {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::BAD_GATEWAY, &e),
    };
    let Some(mut result) = value.get("response").filter(|v| v.is_object()).cloned() else {
        return error_response(StatusCode::BAD_GATEWAY, "Gemini 未返回模型响应");
    };
    if let Some(id) = value.get("traceId") {
        result["responseId"] = id.clone();
    }
    let usage = &result["usageMetadata"];
    crate::gateway::subscription::record_usage(
        &deps,
        entry,
        &json!({"id":result["responseId"],"model":model,"usage":{"input_tokens":usage["promptTokenCount"],"output_tokens":usage["candidatesTokenCount"],"total_tokens":usage["totalTokenCount"],"cache_read_input_tokens":usage["cachedContentTokenCount"]}}),
        started.elapsed().as_millis() as i64,
    );
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Body::from(result.to_string()))
        .unwrap()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use axum::{http::HeaderMap, routing::post};

    #[tokio::test]
    async fn native_request_uses_selected_token_and_project_and_records_usage() {
        let deps = crate::proxy::test_support::bare_deps();
        for id in ["a", "b"] {
            let session = crate::cli_browser_auth::Session {
                platform: "gemini".into(),
                credentials: json!({"access_token":format!("{id}-token"),"expiry_date":4102444800000i64}),
                account: Some(format!("{id}@example.invalid")),
            };
            let sealed = provider_browser_auth::seal(id, &Credential::Cli { session }).unwrap();
            deps.db.lock().unwrap().execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug) VALUES(?1,'gemini','auth',?1,?2,1,?1)",rusqlite::params![id,sealed]).unwrap();
        }
        let app = axum::Router::new().route("/v1internal:loadCodeAssist",post(|headers:HeaderMap| async move {
            assert_eq!(headers["authorization"],"Bearer a-token"); json!({"cloudaicompanionProject":"project-a"}).to_string()
        })).route("/v1internal:generateContent",post(|headers:HeaderMap, body:String| async move {
            let body: Value = serde_json::from_str(&body).unwrap();
            assert_eq!(headers["authorization"],"Bearer a-token"); assert_eq!(body["project"],"project-a"); assert_eq!(body["model"],"fixture-model");
            assert_eq!(body["request"]["contents"][0]["parts"][0]["text"],"test-only");
            json!({"traceId":"fixture-response","response":{"candidates":[{"content":{"parts":[{"text":"ok"}]}}],"usageMetadata":{"promptTokenCount":2,"candidatesTokenCount":3,"totalTokenCount":5}}}).to_string()
        }));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}/v1internal", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let entry = PoolEntry {
            connection_id: "a".into(),
            slug: "a".into(),
            name: "A".into(),
            platform: "gemini".into(),
            kind: "auth".into(),
            protocol: "openai".into(),
            base_url: None,
            secret: None,
            account_fp: None,
            model: None,
            status: "connected".into(),
        };
        let response = forward_at(
            deps.clone(),
            &entry,
            "fixture-model",
            super::super::gemini::GeminiMethod::Generate,
            Bytes::from(
                json!({"contents":[{"role":"user","parts":[{"text":"test-only"}]}]}).to_string(),
            ),
            &base,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let conn = deps.db.lock().unwrap();
        let (id,total):(String,i64) = conn.query_row("SELECT connection_id,total_tokens FROM requests WHERE source='gateway-subscription'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!((id, total), ("a".into(), 5));
        assert_eq!(
            bound(&conn, "b").unwrap().1.credentials["access_token"],
            "b-token"
        );
        assert!(bound(&conn, "missing").is_err());
        server.abort();
    }
}
