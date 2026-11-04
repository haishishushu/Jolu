//! WorkBuddy 官方客户端 ModelProvider：{endpoint}/v2/chat/completions。
//! 地区、账号和企业头均来自所选连接；不继承调用方的账号头。
use crate::{
    gateway::{
        route::PoolEntry,
        wire::{error_response, CallerProtocol},
    },
    provider_browser_auth::{self, Credential},
    proxy::ProxyDeps,
};
use axum::{
    body::Bytes,
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::Response,
};
use std::sync::Arc;

pub(crate) fn bound(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<(String, crate::workbuddy_browser_auth::Session), String> {
    let secret: String = conn
        .query_row(
            "SELECT secret FROM connections WHERE id=?1 AND platform='workbuddy' AND kind='auth'",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| "所选 WorkBuddy 账号缺少完整授权，请重新登录")?;
    let Credential::Workbuddy { edition, session } =
        provider_browser_auth::load(id, "workbuddy", &secret)?
    else {
        return Err("WorkBuddy 授权类型不匹配".into());
    };
    let allowed = match edition.as_str() {
        "cn" => matches!(
            session.domain.as_str(),
            "www.workbuddy.cn"
                | "workbuddy.cn"
                | "www.codebuddy.cn"
                | "codebuddy.cn"
                | "copilot.tencent.com"
                | "wb.tencentbuddy.com"
        ),
        "ai" => matches!(
            session.domain.as_str(),
            "www.workbuddy.ai" | "workbuddy.ai" | "www.codebuddy.ai" | "codebuddy.ai"
        ),
        _ => false,
    };
    if !allowed || session.user_id.is_empty() || session.token.is_empty() {
        return Err("WorkBuddy 账号或地区不完整，请重新登录".into());
    }
    if session
        .expires_at
        .is_some_and(|v| v <= chrono::Utc::now().timestamp_millis())
    {
        return Err("WorkBuddy 登录已过期，请重新登录此账号".into());
    }
    Ok((edition, session))
}

// 两个版本的官方 API 与桌面客户端标识，依据 magpie-community/plugins 的
// packages/workbuddy（MIT）SITES：国内版走 copilot.tencent.com，国际版走
// www.workbuddy.ai，请求头按 WorkBuddy 5.9.3 / CLI 2.165.0 发出。
// UA 不是 WorkBuddy 客户端的请求会被拒（10085；带 X-User-Id 时 12403）。
struct Site {
    endpoint: &'static str,
    app: &'static str,
    cli: &'static str,
    app_name: &'static str,
    runtime: &'static str,
    /// 账号目录读不到时的内置清单（magpie CN_MODELS / AI_MODELS）
    models: &'static [&'static str],
}

const CN: Site = Site {
    endpoint: "https://copilot.tencent.com",
    app: "5.9.3",
    cli: "2.165.0",
    app_name: "WorkBuddy",
    runtime: "local-agent",
    models: &[
        "auto", "hy4-preview-f", "hy3", "hy3-x", "deepseek-v4.1-flash", "glm-5.3", "glm-5.3-flash",
        "glm-5.2", "glm-5.1", "glm-5v-turbo", "minimax-m3", "kimi-k3-1", "kimi-k2.7", "kimi-k2.6",
        "deepseek-v4-pro",
    ],
};

const AI: Site = Site {
    endpoint: "https://www.workbuddy.ai",
    app: "5.9.3",
    cli: "2.165.0",
    app_name: "workbuddy-ai",
    runtime: "local-agent",
    models: &[
        "default-model", "fast-model", "balanced-model", "primary-model", "deep-model", "gpt-5.5",
        "gpt-5.4", "gpt-5.3-codex", "gemini-3.1-pro", "gemini-3.5-flash", "glm-5.3", "glm-5.2", "hy3",
        "kimi-k3", "kimi-k2.6", "minimax-m3",
    ],
};

fn site(edition: &str) -> &'static Site {
    if edition == "ai" { &AI } else { &CN }
}

/// 官方 CLI 的 User-Agent：平台名/版本、应用名/版本、CLI/版本。
fn cli_agent(site: &Site) -> String {
    format!("WorkBuddy/{0} {1}/{0} CLI/{2}", site.app, site.app_name, site.cli)
}

/// magpie sign：账号头、客户端头与使用端归属头。Authorization 由调用方按各自客户端设置。
fn sign(site: &Site, session: &crate::workbuddy_browser_auth::Session, headers: &mut HeaderMap) -> Result<(), String> {
    // 官方协议使用 encodeURIComponent 编码账号 ID；URL form 的 '+' 不适用于此请求头。
    let user = url::form_urlencoded::byte_serialize(session.user_id.as_bytes())
        .collect::<String>()
        .replace('+', "%20");
    let invalid = || "WorkBuddy 账号信息含无效字符，请重新登录".to_string();
    for (name, value) in [
        ("x-user-id", user),
        ("x-domain", session.domain.clone()),
        ("x-product", "SaaS".into()),
        ("x-ide-type", "WorkBuddy".into()),
        ("user-agent", cli_agent(site)),
        ("x-requested-with", "XMLHttpRequest".into()),
        ("x-agent-intent", "craft".into()),
        ("x-agent-purpose", "conversation".into()),
        ("x-ide-name", "WorkBuddy".into()),
        ("x-ide-version", site.app.into()),
    ] {
        headers.insert(name, HeaderValue::from_str(&value).map_err(|_| invalid())?);
    }
    // 企业账号带 X-Enterprise-Id / X-Tenant-Id，否则 WorkBuddy 按个人账号作答
    if let Some(enterprise) = session.enterprise_id.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        let value = HeaderValue::from_str(enterprise).map_err(|_| "WorkBuddy 企业身份无效".to_string())?;
        headers.insert("x-enterprise-id", value.clone());
        headers.insert("x-tenant-id", value);
    }
    Ok(())
}

/// magpie chatHeaders：官方 CLI 只在模型请求上加的 agent 与 OpenAI SDK 头。
fn chat_headers(site: &Site, headers: &mut HeaderMap) {
    let os = match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "MacOS",
        "linux" => "Linux",
        _ => "Unknown",
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "x32",
        _ => "unknown",
    };
    for (name, value) in [
        ("x-agent-type", "main"),
        ("x-agent-runtime", site.runtime),
        ("accept", "application/json"),
        ("x-stainless-retry-count", "0"),
        ("x-stainless-timeout", "1200"),
        ("x-stainless-lang", "js"),
        ("x-stainless-package-version", "6.25.0"),
        ("x-stainless-os", os),
        ("x-stainless-arch", arch),
        ("x-stainless-runtime", "node"),
        ("x-stainless-runtime-version", "v22.21.1"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
}

const MODEL_TTL_MS: i64 = 300_000;

fn credential_hash(secret: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(secret.as_bytes()))
}

/// 本适配器为当前授权保存的目录：(获取时间, 模型)。换过授权的旧缓存不可信。
fn stored(conn: &rusqlite::Connection, id: &str) -> Option<(i64, Vec<String>)> {
    let (secret, text): (String, String) = conn.query_row(
        "SELECT secret,models_json FROM connections WHERE id=?1 AND platform='workbuddy' AND kind='auth'",
        [id], |r| Ok((r.get(0)?,r.get(1)?)),
    ).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    if value["credential_hash"].as_str()? != credential_hash(&secret) {
        return None;
    }
    let models: Vec<String> = serde_json::from_value(value["models"].clone()).ok()?;
    Some((value["fetched_ms"].as_i64()?, models))
}

fn is_fresh(fetched_ms: i64) -> bool {
    chrono::Utc::now()
        .timestamp_millis()
        .checked_sub(fetched_ms)
        .is_some_and(|age| (0..MODEL_TTL_MS).contains(&age))
}

/// 已读到的账号目录，否则内置清单；授权不可用时为 None。
fn known_models(conn: &rusqlite::Connection, id: &str) -> Option<(String, Option<(i64, Vec<String>)>)> {
    let (edition, _) = bound(conn, id).ok()?;
    Some((edition, stored(conn, id).filter(|(_, list)| !list.is_empty())))
}

fn builtin(edition: &str) -> Vec<String> {
    site(edition).models.iter().map(|m| m.to_string()).collect()
}

/// 模型池展示用：上次读到的账号目录，没有时为内置清单（与 `/v1/models` 一致）。
pub(crate) fn cached_models(conn: &rusqlite::Connection, id: &str) -> Option<Vec<String>> {
    let (edition, last) = known_models(conn, id)?;
    Some(last.map(|(_, list)| list).unwrap_or_else(|| builtin(&edition)))
}

/// magpie liveModels：只取 /v3/config 中 `cli` agent 声明的模型，保持原顺序。
fn cli_models(cloud: &serde_json::Value) -> Result<Vec<String>, String> {
    let mut result: Vec<String> = Vec::new();
    for agent in cloud["agents"].as_array().into_iter().flatten().filter(|agent| agent["name"] == "cli") {
        for id in agent["models"].as_array().into_iter().flatten() {
            let id = id
                .as_str()
                .filter(|id| !id.trim().is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
                .ok_or("WorkBuddy 模型目录含无效 ID")?;
            if !result.iter().any(|v| v == id) {
                result.push(id.to_string());
            }
        }
    }
    Ok(result)
}

/// 账号模型目录：5 分钟内用缓存；账号目录读不到或为空时，与 magpie 一样沿用上次
/// 读到的清单，再没有则用内置清单。授权本身不可用时返回错误。
pub(crate) async fn models(deps: &Arc<ProxyDeps>, id: &str) -> Result<Vec<String>, String> {
    models_or_known(deps, id, None).await
}

async fn models_or_known(
    deps: &Arc<ProxyDeps>,
    id: &str,
    test_base: Option<&str>,
) -> Result<Vec<String>, String> {
    let (edition, last) = {
        let conn = deps.db.lock().map_err(|_| "无法读取 WorkBuddy 授权")?;
        bound(&conn, id)?;
        known_models(&conn, id).ok_or("所选 WorkBuddy 账号缺少完整授权，请重新登录")?
    };
    if let Some((at, list)) = &last {
        if is_fresh(*at) {
            return Ok(list.clone());
        }
    }
    match models_at(deps, id, test_base).await {
        Ok(list) if !list.is_empty() => Ok(list),
        _ => Ok(last.map(|(_, list)| list).unwrap_or_else(|| builtin(&edition))),
    }
}

async fn models_at(
    deps: &Arc<ProxyDeps>,
    id: &str,
    test_base: Option<&str>,
) -> Result<Vec<String>, String> {
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        models_fetch(deps, id, test_base),
    )
    .await
    .map_err(|_| "WorkBuddy 模型目录获取超时，请重试".to_string())?
}

async fn models_fetch(
    deps: &Arc<ProxyDeps>,
    id: &str,
    test_base: Option<&str>,
) -> Result<Vec<String>, String> {
    let (edition, session, sealed) = {
        let conn = deps.db.lock().map_err(|_| "无法读取 WorkBuddy 授权")?;
        let (edition, session) = bound(&conn, id)?;
        let secret: String = conn
            .query_row("SELECT secret FROM connections WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .map_err(|_| "无法读取 WorkBuddy 授权")?;
        (edition, session, secret)
    };
    let site = site(&edition);
    let builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(15));
    let builder = if cfg!(test) && test_base.is_some() {
        builder.no_proxy()
    } else {
        match crate::net::current_mode() {
            crate::net::NetMode::Direct => builder.no_proxy(),
            crate::net::NetMode::Manual(url) => {
                builder.proxy(reqwest::Proxy::all(url).map_err(|_| "WorkBuddy 代理配置无效")?)
            }
            crate::net::NetMode::System => match crate::net::system_proxy_url() {
                Some(url) => {
                    builder.proxy(reqwest::Proxy::all(url).map_err(|_| "WorkBuddy 系统代理无效")?)
                }
                None => builder,
            },
        }
    };
    let client = builder
        .build()
        .map_err(|_| "无法创建 WorkBuddy 模型目录连接")?;
    let base = if cfg!(test) {
        test_base.map(str::to_owned)
    } else {
        None
    }
    .unwrap_or_else(|| site.endpoint.to_string());
    let mut headers = HeaderMap::new();
    sign(site, &session, &mut headers)?;
    headers.insert("accept", HeaderValue::from_static("application/json"));
    headers.insert("connection", HeaderValue::from_static("close"));
    let mut response = client
        .get(format!("{base}/v3/config"))
        .headers(headers)
        .bearer_auth(&session.token)
        .send()
        .await
        .map_err(|_| "WorkBuddy 模型目录请求失败")?;
    if !response.status().is_success() {
        if matches!(response.status().as_u16(), 401 | 403) {
            return Err("WorkBuddy 拒绝了此账号的目录授权，请重新登录并重试".into());
        }
        return Err(format!(
            "WorkBuddy 模型目录获取失败（HTTP {}）",
            response.status().as_u16()
        ));
    }
    let mut data = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "WorkBuddy 模型目录读取失败")?
    {
        if data.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("WorkBuddy 模型目录超过大小限制".into());
        }
        data.extend_from_slice(&chunk);
    }
    let value: serde_json::Value =
        serde_json::from_slice(&data).map_err(|_| "WorkBuddy 模型目录不是有效 JSON")?;
    if matches!(value["code"].as_u64(), Some(401 | 403)) {
        return Err("WorkBuddy 拒绝了此账号的目录授权，请重新登录并重试".into());
    }
    if let Some(code) = value["code"].as_u64().filter(|code| *code != 0) {
        return Err(format!("WorkBuddy 模型目录请求未成功（业务状态 {code}）"));
    }
    let models = cli_models(&value["data"])?;
    let cache=serde_json::json!({"credential_hash":credential_hash(&sealed),"fetched_ms":chrono::Utc::now().timestamp_millis(),"models":models}).to_string();
    let conn = deps.db.lock().map_err(|_| "无法保存 WorkBuddy 模型目录")?;
    bound(&conn, id)?;
    let changed=conn.execute("UPDATE connections SET models_json=?3 WHERE id=?1 AND platform='workbuddy' AND kind='auth' AND secret=?2",rusqlite::params![id,sealed,cache]).map_err(|_| "无法保存 WorkBuddy 模型目录")?;
    if changed != 1 {
        return Err("WorkBuddy 授权已变化，已丢弃旧账号目录".into());
    }
    Ok(models)
}

pub(crate) async fn forward(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    protocol: CallerProtocol,
    body: Bytes,
) -> Response {
    forward_at(deps, entry, protocol, body, None).await
}

async fn forward_at(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    protocol: CallerProtocol,
    body: Bytes,
    test_base: Option<&str>,
) -> Response {
    if protocol != CallerProtocol::Chat {
        return error_response(
            StatusCode::BAD_REQUEST,
            "WorkBuddy 订阅使用 OpenAI Chat Completions 协议",
        );
    }
    let selected = deps
        .db
        .lock()
        .map_err(|_| "无法读取 WorkBuddy 授权".to_string())
        .and_then(|conn| bound(&conn, &entry.connection_id));
    let (edition, session) = match selected {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::UNAUTHORIZED, &e),
    };
    let mut value: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "请求格式无效"),
    };
    super::workbuddy_request::normalize(&mut value);
    let Some(model) = value["model"]
        .as_str()
        .and_then(|m| m.split_once('/').map(|(_, m)| m))
    else {
        return error_response(StatusCode::BAD_REQUEST, "请选择来源和模型");
    };
    let site = site(&edition);
    let mut headers = HeaderMap::new();
    if let Err(e) = sign(site, &session, &mut headers) {
        return error_response(StatusCode::UNAUTHORIZED, &e);
    }
    chat_headers(site, &mut headers);
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    let mut target = entry.clone();
    target.secret = Some(session.token);
    target.protocol = "openai".into();
    target.base_url = Some(
        if cfg!(test) {
            test_base.map(str::to_owned)
        } else {
            None
        }
        .unwrap_or_else(|| format!("{}/v2", site.endpoint)),
    );
    super::direct::forward(
        deps,
        &target,
        model.into(),
        Method::POST,
        "/chat/completions",
        &headers,
        Bytes::from(value.to_string()),
    )
    .await
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn catalog_takes_cli_agent_models_in_order() {
        let cloud = json!({
            "models":[{"id":"not-declared","tags":["chat"]}],
            "agents":[
                {"name":"craft","models":["craft-only"]},
                {"name":"cli","models":["auto","glm-5.3","hy3","glm-5.3"]}]});
        assert_eq!(cli_models(&cloud).unwrap(), ["auto", "glm-5.3", "hy3"]);
        assert!(cli_models(&json!({"agents":null,"models":null})).unwrap().is_empty());
        assert!(cli_models(&json!({"agents":[{"name":"cli","models":[""]}]})).is_err());
        assert_eq!(
            cli_agent(site("cn")),
            "WorkBuddy/5.9.3 WorkBuddy/5.9.3 CLI/2.165.0"
        );
        assert_eq!(
            cli_agent(site("ai")),
            "WorkBuddy/5.9.3 workbuddy-ai/5.9.3 CLI/2.165.0"
        );
    }
    fn account(deps: &Arc<ProxyDeps>, id: &str, edition: &str, enterprise: Option<&str>) -> String {
        let credential = Credential::Workbuddy {
            edition: edition.into(),
            session: crate::workbuddy_browser_auth::Session {
                token: format!("{id}-token"),
                domain: if edition == "cn" {
                    "www.codebuddy.cn"
                } else {
                    "www.workbuddy.ai"
                }
                .into(),
                user_id: id.into(),
                enterprise_id: enterprise.map(str::to_owned),
                email: None,
                expires_at: None,
            },
        };
        let secret = provider_browser_auth::seal(id, &credential).unwrap();
        deps.db.lock().unwrap().execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug,serve_enabled) VALUES(?1,'workbuddy','auth',?1,?2,1,?1,1)",rusqlite::params![id,secret]).unwrap();
        secret
    }
    #[tokio::test]
    async fn model_catalog_is_signed_as_workbuddy_cli_and_cache_tracks_secret() {
        let deps = crate::proxy::test_support::bare_deps();
        account(&deps, "a", "cn", Some("enterprise-a"));
        account(&deps, "b", "ai", None);
        let app = axum::Router::new().route(
            "/v3/config",
            axum::routing::get(|headers: HeaderMap| async move {
                let id = headers["x-user-id"].to_str().unwrap();
                assert_eq!(headers["authorization"], format!("Bearer {id}-token"));
                assert_eq!(headers["x-product"], "SaaS");
                assert_eq!(headers["x-ide-type"], "WorkBuddy");
                assert_eq!(headers["x-ide-version"], "5.9.3");
                if id == "a" {
                    assert_eq!(headers["user-agent"], "WorkBuddy/5.9.3 WorkBuddy/5.9.3 CLI/2.165.0");
                    assert_eq!(headers["x-domain"], "www.codebuddy.cn");
                    assert_eq!(headers["x-enterprise-id"], "enterprise-a");
                    assert_eq!(headers["x-tenant-id"], "enterprise-a");
                } else {
                    assert_eq!(headers["user-agent"], "WorkBuddy/5.9.3 workbuddy-ai/5.9.3 CLI/2.165.0");
                    assert_eq!(headers["x-domain"], "www.workbuddy.ai");
                    assert!(!headers.contains_key("x-enterprise-id"));
                    assert!(!headers.contains_key("x-tenant-id"));
                }
                json!({"code":0,"data":{"agents":[{"name":"cli","models":[format!("model-{id}")]}]}})
                    .to_string()
            }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        assert_eq!(models_at(&deps, "a", Some(&base)).await.unwrap(), ["model-a"]);
        assert_eq!(models_at(&deps, "b", Some(&base)).await.unwrap(), ["model-b"]);
        let conn = deps.db.lock().unwrap();
        assert_eq!(cached_models(&conn, "a").unwrap(), ["model-a"]);
        let menu = crate::gateway::catalog::list(&conn).unwrap();
        assert_eq!(
            menu.iter()
                .filter(|m| m.slug == "a")
                .map(|m| m.model.as_str())
                .collect::<Vec<_>>(),
            ["model-a"]
        );
        let text: String = conn
            .query_row("SELECT models_json FROM connections WHERE id='a'", [], |r| r.get(0))
            .unwrap();
        assert!(!text.contains("a-token"));
        conn.execute("UPDATE connections SET secret='changed' WHERE id='a'", [])
            .unwrap();
        assert!(stored(&conn, "a").is_none());
        assert!(cached_models(&conn, "a").is_none());
        let text: String = conn
            .query_row("SELECT models_json FROM connections WHERE id='b'", [], |r| r.get(0))
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["fetched_ms"] = json!(chrono::Utc::now().timestamp_millis() - MODEL_TTL_MS - 1);
        conn.execute("UPDATE connections SET models_json=?1 WHERE id='b'", [value.to_string()])
            .unwrap();
        let (at, list) = stored(&conn, "b").unwrap();
        assert!(!is_fresh(at));
        assert_eq!(list, ["model-b"]);
        server.abort();
    }
    #[tokio::test]
    async fn unreadable_catalog_falls_back_to_last_list_then_builtin() {
        let deps = crate::proxy::test_support::bare_deps();
        account(&deps, "cn", "cn", None);
        // 从未读到过目录：模型池与 /v1/models 都给内置清单
        {
            let conn = deps.db.lock().unwrap();
            assert_eq!(cached_models(&conn, "cn").unwrap(), builtin("cn"));
            assert_eq!(builtin("cn")[0], "auto");
        }
        let app = axum::Router::new()
            .route("/v3/config", axum::routing::get(|| async { (StatusCode::BAD_REQUEST, r#"{"code":12403}"#) }));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        assert_eq!(models_or_known(&deps, "cn", Some(&base)).await.unwrap(), builtin("cn"));
        // 读到过的账号目录优先于内置清单，即使已过期
        let sealed: String = deps.db.lock().unwrap().query_row("SELECT secret FROM connections WHERE id='cn'", [], |r| r.get(0)).unwrap();
        let old = json!({"credential_hash":credential_hash(&sealed),"fetched_ms":1,"models":["last-good"]}).to_string();
        deps.db.lock().unwrap().execute("UPDATE connections SET models_json=?1 WHERE id='cn'", [old]).unwrap();
        assert_eq!(models_or_known(&deps, "cn", Some(&base)).await.unwrap(), ["last-good"]);
        server.abort();
        deps.db
            .lock()
            .unwrap()
            .execute("UPDATE connections SET secret='broken' WHERE id='cn'", [])
            .unwrap();
        assert!(super::models(&deps, "cn").await.is_err(), "授权不可用时必须报错，不回退内置清单");
    }
    #[tokio::test]
    async fn changed_credentials_reject_late_catalog_and_redirects_are_not_followed() {
        let deps = crate::proxy::test_support::bare_deps();
        let old = account(&deps, "late", "cn", None);
        let change = deps.clone();
        let app = axum::Router::new().route(
            "/v3/config",
            axum::routing::get(move || {
                let change = change.clone();
                async move {
                    change
                        .db
                        .lock()
                        .unwrap()
                        .execute(
                            "UPDATE connections SET secret='new-login' WHERE id='late'",
                            [],
                        )
                        .unwrap();
                    json!({"data":{"agents":[{"name":"cli","models":["late-model"]}]}}).to_string()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        assert!(models_at(&deps, "late", Some(&base)).await.is_err());
        assert!(cached_models(&deps.db.lock().unwrap(), "late").is_none());
        server.abort();
        deps.db
            .lock()
            .unwrap()
            .execute("UPDATE connections SET secret=?1 WHERE id='late'", [old])
            .unwrap();
        let app = axum::Router::new()
            .route(
                "/v3/config",
                axum::routing::get(|| async {
                    (StatusCode::FOUND, [("location", "/must-not-follow")])
                }),
            )
            .route(
                "/must-not-follow",
                axum::routing::get(|| async {
                    panic!("redirect must not be followed");
                    #[allow(unreachable_code)]
                    ""
                }),
            );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        assert!(models_at(&deps, "late", Some(&base)).await.is_err());
        server.abort();
    }
    #[tokio::test]
    async fn chat_is_signed_for_selected_account_only() {
        let deps = crate::proxy::test_support::bare_deps();
        account(&deps, "a", "cn", Some("enterprise-a"));
        account(&deps, "b", "ai", None);
        let app = axum::Router::new().route(
            "/v2/chat/completions",
            axum::routing::post(|headers: HeaderMap, body: String| async move {
                assert_eq!(headers["authorization"], "Bearer a-token");
                assert_eq!(headers["x-user-id"], "a");
                assert_eq!(headers["x-enterprise-id"], "enterprise-a");
                assert_eq!(headers["x-domain"], "www.codebuddy.cn");
                assert_eq!(headers["user-agent"], "WorkBuddy/5.9.3 WorkBuddy/5.9.3 CLI/2.165.0");
                assert_eq!(headers["x-agent-type"], "main");
                assert_eq!(headers["x-agent-runtime"], "local-agent");
                assert_eq!(headers["x-stainless-package-version"], "6.25.0");
                let sent: serde_json::Value = serde_json::from_str(&body).unwrap();
                // Claude Code 的身份提示不能原样发往 WorkBuddy（magpie #182）。
                assert_eq!(sent["messages"][0]["content"], "You are Claude Code, Anthropic's CLI for Claude.\n保留中文任务约束。");
                assert_eq!(sent["messages"][1]["content"], "Main branch (usually the base for PRs): main");
                assert_eq!(sent["stream"], true);
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&body).unwrap()["model"],
                    "glm-5.3"
                );
                json!({"id":"fixture","choices":[{"message":{"role":"assistant","content":"ok"}}]})
                    .to_string()
            }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}/v2", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let entry = PoolEntry {
            connection_id: "a".into(),
            slug: "a".into(),
            name: "A".into(),
            platform: "workbuddy".into(),
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
            CallerProtocol::Chat,
            Bytes::from(
                crate::gateway::translate::anthropic_request(
                    &serde_json::to_vec(&json!({
                        "model":"a/glm-5.3", "stream":true,
                        "system":"x-anthropic-billing-header: cc_version=2.1.318;\nYou are Claude Code, Anthropic's official CLI for Claude.\n保留中文任务约束。",
                        "messages":[{"role":"user","content":"Main branch (you will usually use this for PRs): main"}]
                    })).unwrap()
                ).unwrap(),
            ),
            Some(&base),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let _ = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let conn = deps.db.lock().unwrap();
        assert!(bound(&conn, "missing").is_err());
        server.abort();
    }

    /// 显式手动验收：两次低输出上限请求；真实数据库只读，授权仅在内存中使用。
    #[tokio::test]
    #[ignore = "requires JOLU_WORKBUDDY_PROBE_DB and JOLU_WORKBUDDY_PROBE_SLUG; consumes model usage"]
    async fn local_channel_prompt_probe() {
        let path = std::env::var("JOLU_WORKBUDDY_PROBE_DB").expect("explicit database path required");
        let slug = std::env::var("JOLU_WORKBUDDY_PROBE_SLUG").expect("explicit connection slug required");
        let source = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let (id, sealed): (String, String) = source.query_row(
            "SELECT id,secret FROM connections WHERE slug=?1 AND platform='workbuddy' AND kind='auth'",
            [&slug], |row| Ok((row.get(0)?, row.get(1)?)),
        ).expect("selected connection must exist");
        drop(source);
        let deps = crate::proxy::test_support::bare_deps();
        deps.db.lock().unwrap().execute(
            "INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug) VALUES(?1,'workbuddy','auth','probe',?2,1,?3)",
            rusqlite::params![id,sealed,slug],
        ).unwrap();
        let (edition, session) = bound(&deps.db.lock().unwrap(), &id).expect("selected authorization must be valid");
        let site = site(&edition);
        let mut headers = HeaderMap::new();
        sign(site, &session, &mut headers).unwrap();
        chat_headers(site, &mut headers);
        let baseline = json!({"model":"hy3","stream":true,"max_tokens":32,"messages":[
            {"role":"system","content":"You are Claude Code, Anthropic's official CLI for Claude."},
            {"role":"user","content":"Reply with OK."}
        ]});
        let response = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(45)).build().unwrap()
            .post(format!("{}/v2/chat/completions", site.endpoint)).headers(headers)
            .bearer_auth(&session.token).json(&baseline).send().await.expect("baseline network request failed");
        let before_status = response.status();
        let before: serde_json::Value = response.json().await.expect("baseline must return a JSON rejection");
        eprintln!("baseline: HTTP {}, business code {}", before_status.as_u16(), before["code"].as_i64().unwrap_or_default());
        assert_eq!(before_status, StatusCode::BAD_REQUEST);
        assert_eq!(before["code"].as_i64(), Some(11128));
        let entry = PoolEntry {
            connection_id:id, slug:slug.clone(), name:"probe".into(), platform:"workbuddy".into(),
            kind:"auth".into(), protocol:"openai".into(), base_url:None, secret:None,
            account_fp:None, model:None, status:"connected".into(),
        };
        let mut body = baseline;
        body["model"] = json!(format!("{slug}/hy3"));
        let (status, bytes) = tokio::time::timeout(std::time::Duration::from_secs(60), async {
            let response = forward(deps, &entry, CallerProtocol::Chat, Bytes::from(body.to_string())).await;
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            (status, bytes)
        }).await.expect("normalized request timed out");
        let choices = String::from_utf8_lossy(&bytes).lines().filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|data| serde_json::from_str::<serde_json::Value>(data).ok())
            .filter(|v| v["choices"].as_array().is_some_and(|a| !a.is_empty())).count();
        eprintln!("normalized: HTTP {}, SSE choice chunks {}", status.as_u16(), choices);
        assert_eq!(status, StatusCode::OK);
        assert!(choices > 0, "normalized response must contain model choices");
    }
}
