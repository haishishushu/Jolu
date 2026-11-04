//! Trae 账号模型目录；对话转发与目录读取分别验收。
//! 查询形状参考 magpie-community/plugins 的 Trae 插件；不模拟设备或客户端身份。
use crate::{gateway::route::PoolEntry, proxy::ProxyDeps};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};

const FUNCTIONS: &[&str] = &[
    "chat_v3",
    "solo_work_lite",
    "solo_agent",
    "solo_agent_lite",
    "solo_coder",
];
const TTL_MS: i64 = 5 * 60_000;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct ModelRoute {
    pub function: String,
    pub model_name: Option<String>,
    pub max_tokens: Option<u64>,
    pub images: Option<bool>,
    pub reasoning: Vec<String>,
}

fn identifier(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= 512 && !id.chars().any(char::is_control)
}

/// 目录只保留调用所需的白名单字段，不缓存整个上游条目。
fn model_route(function: &str, value: &Value) -> ModelRoute {
    let dev = value["model_detail_list"].as_array().and_then(|entries| {
        entries.iter().find(|d| {
            d["model_name"]
                .as_str()
                .is_some_and(|s| identifier(s) && s.ends_with("__dev"))
        })
    });
    ModelRoute {
        function: function.into(),
        model_name: dev
            .and_then(|d| d["model_name"].as_str())
            .map(str::to_owned),
        max_tokens: dev
            .and_then(|d| d["max_tokens"].as_u64())
            .filter(|n| *n > 0),
        images: value["multimodal"].as_bool().or_else(|| {
            value
                .pointer("/display_config/multimodal")
                .and_then(Value::as_bool)
        }),
        reasoning: value
            .pointer("/reasoning_effort_config/options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|s| ["light", "medium", "high", "extra_high", "max"].contains(s))
            .map(str::to_owned)
            .collect(),
    }
}

pub(super) fn cached_route(
    conn: &rusqlite::Connection,
    id: &str,
    model: &str,
) -> Option<ModelRoute> {
    if !cached_models(conn, id)?.iter().any(|m| m == model) {
        return None;
    }
    let text: String = conn
        .query_row(
            "SELECT models_json FROM connections WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let route: ModelRoute = serde_json::from_value(value["routes"].get(model)?.clone()).ok()?;
    if !FUNCTIONS.contains(&route.function.as_str())
        || route
            .model_name
            .as_deref()
            .is_some_and(|s| !identifier(s) || !s.ends_with("__dev"))
        || route.max_tokens == Some(0)
        || route
            .reasoning
            .iter()
            .any(|s| !["light", "medium", "high", "extra_high", "max"].contains(&s.as_str()))
    {
        return None;
    }
    Some(route)
}

pub(super) async fn prepare_route(
    deps: &Arc<ProxyDeps>,
    entry: &PoolEntry,
    model: &str,
    test_base: Option<&str>,
) -> Result<(), String> {
    load_models(deps, entry, test_base, true).await?;
    let conn = deps.db.lock().map_err(|_| "无法读取 Trae 模型路由")?;
    cached_route(&conn, &entry.connection_id, model)
        .ok_or_else(|| "Trae 所选账号目录没有此模型的有效路由，请刷新模型列表".into())
        .map(|_| ())
}

fn credential_hash(secret: &str) -> String {
    format!("{:x}", Sha256::digest(secret.as_bytes()))
}

/// 仅接受当前授权绑定且未过期的目录；不读其他适配器遗留的裸数组。
pub(crate) fn cached_models(conn: &rusqlite::Connection, id: &str) -> Option<Vec<String>> {
    let (secret, edition, _) = super::trae::bound(conn, id).ok()?;
    let text: String = conn
        .query_row(
            "SELECT models_json FROM connections WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    if value["credential_hash"].as_str()? != credential_hash(&secret)
        || value["edition"].as_str()? != edition
        || !chrono::Utc::now()
            .timestamp_millis()
            .checked_sub(value["fetched_ms"].as_i64()?)
            .is_some_and(|age| (0..TTL_MS).contains(&age))
    {
        return None;
    }
    serde_json::from_value(value["models"].clone()).ok()
}

fn client(test: bool) -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15));
    let builder = if test {
        builder.no_proxy()
    } else {
        match crate::net::current_mode() {
            crate::net::NetMode::Direct => builder.no_proxy(),
            crate::net::NetMode::Manual(url) => {
                builder.proxy(reqwest::Proxy::all(url).map_err(|_| "Trae 代理配置无效")?)
            }
            crate::net::NetMode::System => match crate::net::system_proxy_url() {
                Some(url) => {
                    builder.proxy(reqwest::Proxy::all(url).map_err(|_| "Trae 系统代理无效")?)
                }
                None => builder,
            },
        }
    };
    builder.build().map_err(|_| "无法创建 Trae 目录连接".into())
}

struct QueryError {
    message: String,
    auth: bool,
}
impl QueryError {
    fn other(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            auth: false,
        }
    }
    fn auth() -> Self {
        Self {
            message: "Trae 模型目录授权被拒绝，请重新登录所选账号".into(),
            auth: true,
        }
    }
}

async fn query(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    path: &str,
    body: Value,
) -> Result<Value, QueryError> {
    let response = client
        .post(format!("{base}{path}"))
        .header("authorization", format!("Cloud-IDE-JWT {token}"))
        .json(&body)
        .send()
        .await
        .map_err(|_| QueryError::other("Trae 模型目录请求失败"))?;
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err(QueryError::auth());
    }
    if !response.status().is_success() {
        return Err(QueryError::other(format!(
            "Trae 模型目录被拒绝（HTTP {}）",
            response.status().as_u16()
        )));
    }
    let value = super::trae::read_json(response, 2 * 1024 * 1024)
        .await
        .map_err(QueryError::other)?;
    let error = value
        .pointer("/ResponseMetadata/Error")
        .filter(|v| !v.is_null())
        .or_else(|| value.get("error").filter(|v| !v.is_null() && **v != false));
    let code = error
        .and_then(|v| v.get("code").or_else(|| v.get("Code")))
        .or_else(|| value.get("code"))
        .or_else(|| value.get("Code"));
    let code = code.map(|v| {
        v.as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| v.to_string())
    });
    if code
        .as_deref()
        .is_some_and(|c| ["401", "403", "1001", "20101"].contains(&c))
    {
        return Err(QueryError::auth());
    }
    if error.is_some()
        || code
            .as_deref()
            .is_some_and(|c| c != "0" && c != "null" && !c.is_empty())
    {
        return Err(QueryError::other("Trae 返回模型目录业务错误，未更新缓存"));
    }
    Ok(value)
}

fn chat_model(value: &Value, edition: &str) -> Option<String> {
    let id = value["config_name"].as_str()?;
    if id.trim().is_empty()
        || id.len() > 512
        || id.chars().any(char::is_control)
        || id.to_ascii_lowercase().starts_with("custom_model")
        || value
            .get("usage")
            .is_some_and(|v| !v.is_null() && v != "" && v != "chat_completion")
        || value["config_switch"] == false
        || value["is_custom_model"] == true
        || value.pointer("/display_config/is_custom_model") == Some(&Value::Bool(true))
        || (edition == "global" && value["is_invisible_to_user"] == true)
    {
        return None;
    }
    Some(id.into())
}

fn current(deps: &Arc<ProxyDeps>, entry: &PoolEntry, secret: &str) -> Result<(), String> {
    if !crate::gateway::route::source_is_current(deps, entry) || entry.status != "connected" {
        return Err("Trae 来源已关闭或配置已变化，请刷新后重试".into());
    }
    let conn = deps.db.lock().map_err(|_| "无法复查 Trae 授权")?;
    if !super::trae::bound(&conn, &entry.connection_id).is_ok_and(|v| v.0 == secret) {
        return Err("Trae 授权已变化，已丢弃旧账号目录".into());
    }
    Ok(())
}

pub(crate) async fn models(
    deps: &Arc<ProxyDeps>,
    entry: &PoolEntry,
) -> Result<Vec<String>, String> {
    models_at(deps, entry, None).await
}

async fn models_at(
    deps: &Arc<ProxyDeps>,
    entry: &PoolEntry,
    test_base: Option<&str>,
) -> Result<Vec<String>, String> {
    load_models(deps, entry, test_base, false).await
}

async fn load_models(
    deps: &Arc<ProxyDeps>,
    entry: &PoolEntry,
    test_base: Option<&str>,
    require_routes: bool,
) -> Result<Vec<String>, String> {
    let _guard = crate::gateway::subscription::lock(&entry.connection_id).await;
    let (secret, edition, session) = {
        let conn = deps.db.lock().map_err(|_| "无法读取 Trae 授权")?;
        super::trae::bound(&conn, &entry.connection_id)?
    };
    current(deps, entry, &secret)?;
    let cached = {
        let conn = deps.db.lock().map_err(|_| "无法读取 Trae 模型目录")?;
        cached_models(&conn, &entry.connection_id).filter(|models| {
            !require_routes
                || models
                    .iter()
                    .all(|model| cached_route(&conn, &entry.connection_id, model).is_some())
        })
    };
    if let Some(models) = cached {
        current(deps, entry, &secret)?;
        return Ok(models);
    }
    let test_base = if cfg!(test) { test_base } else { None };
    let client = client(test_base.is_some())?;
    let base = test_base.unwrap_or(super::trae::model_host(&edition, &session.host)?);
    let read = async {
        let batch = query(&client, base, &session.token, "/api/ide/v1/batch_get_detail_param", json!({
            "functions":FUNCTIONS,"agent_type":"","current_config_info":{"config_name":"","is_custom_model":false},
            "mode_type":0,"access_type":0,"ab_force_vids":"","ab_autotest_advanced_mode":0,"show_custom_model":true,
        })).await;
        current(deps, entry, &secret)?;
        let mut lists = std::collections::BTreeMap::<String, Vec<Value>>::new();
        match batch {
            Err(e) if e.auth => return Err(e.message),
            Ok(value) => {
                if let Some(groups) = value
                    .get("function_configs")
                    .or_else(|| value.pointer("/data/function_configs"))
                    .and_then(Value::as_array)
                {
                    for group in groups {
                        if let (Some(function), Some(entries)) = (
                            group["function"].as_str().filter(|f| FUNCTIONS.contains(f)),
                            group["config_info_list"].as_array(),
                        ) {
                            lists
                                .entry(function.into())
                                .or_default()
                                .extend(entries.iter().cloned());
                        }
                    }
                }
            }
            Err(_) => {}
        }
        if lists.is_empty() {
            let mut successes = 0;
            let mut last_error = "Trae 未返回可识别的模型目录".to_string();
            use futures_util::{stream::FuturesUnordered, StreamExt};
            // 最多五个固定目录并发读取；一个慢接口不阻塞其他函数。
            // 不 spawn，授权错误或取消时，剩余请求随集合一起丢弃。
            let mut pending = FuturesUnordered::new();
            for function in FUNCTIONS {
                let client = &client;
                let token = &session.token;
                pending.push(async move {
                    let result = query(client, base, token, "/api/ide/v1/get_detail_param", json!({
                        "function":function,"config_names":null,"need_prompt":false,"current_config_info":null,
                        "poly_prompt":true,"mode_type":null,"agent_type":null,
                    })).await;
                    (function, result)
                });
            }
            while let Some((function, result)) = pending.next().await {
                current(deps, entry, &secret)?;
                match result {
                    Ok(value) => {
                        if let Some(entries) = value
                            .get("config_info_list")
                            .or_else(|| value.pointer("/data/config_info_list"))
                            .and_then(Value::as_array)
                        {
                            successes += 1;
                            lists.insert((*function).into(), entries.clone());
                        }
                    }
                    Err(e) if e.auth => return Err(e.message),
                    Err(e) => last_error = e.message,
                }
            }
            if successes == 0 {
                return Err(last_error);
            }
        }
        let mut models = Vec::new();
        let mut routes = std::collections::BTreeMap::<String, ModelRoute>::new();
        for function in FUNCTIONS {
            for value in lists.get(*function).into_iter().flatten() {
                if let Some(id) = chat_model(value, &edition) {
                    if !models.contains(&id) {
                        models.push(id.clone());
                    }
                    let candidate = model_route(function, value);
                    // 优先使用 __dev 路由，其次保留声明思考档位的条目；同分保持函数顺序。
                    let score = |r: &ModelRoute| {
                        (r.model_name.is_some() as u8) * 4 + (!r.reasoning.is_empty() as u8)
                    };
                    if routes
                        .get(&id)
                        .is_none_or(|old| score(&candidate) > score(old))
                    {
                        routes.insert(id, candidate);
                    }
                }
            }
        }
        if models.is_empty() {
            return Err("Trae 所选账号未返回可选择的对话模型，未更新缓存".into());
        }
        Ok((models, routes))
    };
    let (models, routes) = tokio::time::timeout(Duration::from_secs(45), read)
        .await
        .map_err(|_| "Trae 模型目录查询超时，未更新缓存")??;
    current(deps, entry, &secret)?;
    // 缓存公共模型与白名单路由，不保存原始条目、上游错误或任何认证字段。
    let text = json!({"credential_hash":credential_hash(&secret),"edition":edition,
        "fetched_ms":chrono::Utc::now().timestamp_millis(),"models":models,"routes":routes})
    .to_string();
    let conn = deps.db.lock().map_err(|_| "无法保存 Trae 模型目录")?;
    let changed = conn.execute(
        "UPDATE connections SET models_json=?1 WHERE id=?2 AND secret=?3 AND serve_enabled=1 AND status='connected' AND slug=?4 AND account_fp IS ?5",
        rusqlite::params![text,entry.connection_id,secret,entry.slug,entry.account_fp],
    ).map_err(|_| "无法保存 Trae 模型目录")?;
    if changed != 1 {
        return Err("Trae 授权或来源已变化，已丢弃旧账号目录".into());
    }
    Ok(models)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use axum::{http::HeaderMap, routing::post, Router};
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn insert(deps: &Arc<ProxyDeps>, id: &str) -> PoolEntry {
        let session = crate::trae_browser_auth::Session {
            token: format!("fixture-{id}"),
            host: "https://api.trae.cn".into(),
            expires_at: "2099-01-01T00:00:00Z".into(),
            account: None,
            user_id: id.into(),
        };
        let secret = crate::provider_browser_auth::seal(
            id,
            &crate::provider_browser_auth::Credential::Trae {
                edition: "cn".into(),
                session,
            },
        )
        .unwrap();
        deps.db.lock().unwrap().execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug,serve_enabled,status) VALUES(?1,'trae','auth',?1,?2,1,?1,1,'connected')", rusqlite::params![id, secret]).unwrap();
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
    async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, task)
    }
    fn batch() -> Value {
        json!({"data":{"function_configs":[
            {"function":"solo_agent","config_info_list":[{"config_name":"solo-only","usage":"chat_completion"},{"config_name":"shared","model_detail_list":[{"model_name":"shared__dev","max_tokens":8192}],"token":"must-not-cache"}]},
            {"function":"chat_v3","config_info_list":[{"config_name":"shared"},{"config_name":"disabled","config_switch":false},{"config_name":"custom_model_1"},{"config_name":"helper","usage":"summary"},{"config_name":"custom","is_custom_model":true}]},
            {"function":"unknown_function","config_info_list":[{"config_name":"not-chat"}]}
        ]}})
    }
    #[tokio::test]
    async fn batch_models_are_filtered_ordered_and_cached_for_selected_account() {
        let deps = crate::proxy::test_support::bare_deps();
        let selected = insert(&deps, "trae-list-a");
        insert(&deps, "trae-list-b");
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let (base, task) = serve(Router::new().route(
            "/api/ide/v1/batch_get_detail_param",
            post(move |headers: HeaderMap, body: String| {
                let count = count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(
                        headers["authorization"],
                        "Cloud-IDE-JWT fixture-trae-list-a"
                    );
                    let body: Value = serde_json::from_str(&body).unwrap();
                    assert_eq!(
                        body["functions"],
                        json!([
                            "chat_v3",
                            "solo_work_lite",
                            "solo_agent",
                            "solo_agent_lite",
                            "solo_coder"
                        ])
                    );
                    batch().to_string()
                }
            }),
        ))
        .await;
        assert_eq!(
            models_at(&deps, &selected, Some(&base)).await.unwrap(),
            vec!["shared", "solo-only"]
        );
        assert_eq!(
            models_at(&deps, &selected, Some(&base)).await.unwrap(),
            vec!["shared", "solo-only"]
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let conn = deps.db.lock().unwrap();
        let cached: String = conn
            .query_row(
                "SELECT models_json FROM connections WHERE id='trae-list-a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let cache: Value = serde_json::from_str(&cached).unwrap();
        // shared 必须选有 __dev 的 SOLO 入口，而非先出现但缺少路由信息的 chat_v3。
        assert_eq!(cache["routes"]["shared"]["function"], "solo_agent");
        assert_eq!(cache["routes"]["shared"]["model_name"], "shared__dev");
        assert_eq!(cache["routes"]["shared"]["max_tokens"], 8192);
        assert_eq!(cache["routes"]["solo-only"]["function"], "solo_agent");
        assert!(!cached.contains("must-not-cache"));
        assert!(!cached.contains("fixture-trae-list-a"));
        let other: Option<String> = conn
            .query_row(
                "SELECT models_json FROM connections WHERE id='trae-list-b'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(other.is_none());
        task.abort();
    }
    #[tokio::test]
    async fn absent_batch_lists_use_each_chat_function_but_auth_failure_stops() {
        for rejected in [false, true] {
            let deps = crate::proxy::test_support::bare_deps();
            let selected = insert(
                &deps,
                if rejected {
                    "trae-denied"
                } else {
                    "trae-fallback"
                },
            );
            let calls = Arc::new(AtomicUsize::new(0));
            let count = calls.clone();
            let (base, task) = serve(Router::new()
                .route("/api/ide/v1/batch_get_detail_param", post(move || async move {
                    (if rejected { axum::http::StatusCode::UNAUTHORIZED } else { axum::http::StatusCode::OK }, "{}")
                }))
                .route("/api/ide/v1/get_detail_param", post(move |body: String| { let count = count.clone(); async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    let body: Value = serde_json::from_str(&body).unwrap();
                    if body["function"] == "solo_agent" { json!({"data":{"config_info_list":[{"config_name":"only-solo"}]}}).to_string() }
                    else { json!({"config_info_list":[]}).to_string() }
                }}))).await;
            let result = models_at(&deps, &selected, Some(&base)).await;
            if rejected {
                assert!(result.is_err());
                assert_eq!(calls.load(Ordering::SeqCst), 0);
            } else {
                assert_eq!(result.unwrap(), vec!["only-solo"]);
                assert_eq!(calls.load(Ordering::SeqCst), 5);
            }
            task.abort();
        }
    }
    #[tokio::test]
    async fn failed_refresh_preserves_cache_but_never_reports_it_as_fresh() {
        let deps = crate::proxy::test_support::bare_deps();
        let selected = insert(&deps, "trae-stale");
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let (base, task) = serve(Router::new().route(
            "/api/ide/v1/batch_get_detail_param",
            post(move || {
                let count = count.clone();
                async move {
                    if count.fetch_add(1, Ordering::SeqCst) == 0 {
                        batch().to_string()
                    } else {
                        json!({"error":{"code":1001,"message":"fixture-private-message"}})
                            .to_string()
                    }
                }
            }),
        ))
        .await;
        assert_eq!(
            models_at(&deps, &selected, Some(&base)).await.unwrap(),
            vec!["shared", "solo-only"]
        );
        {
            let conn = deps.db.lock().unwrap();
            let raw: String = conn
                .query_row(
                    "SELECT models_json FROM connections WHERE id='trae-stale'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let mut value: Value = serde_json::from_str(&raw).unwrap();
            value["fetched_ms"] = json!(1);
            conn.execute(
                "UPDATE connections SET models_json=?1 WHERE id='trae-stale'",
                [value.to_string()],
            )
            .unwrap();
        }
        let error = models_at(&deps, &selected, Some(&base)).await.unwrap_err();
        assert!(!error.contains("fixture-private-message"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let raw: String = deps
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT models_json FROM connections WHERE id='trae-stale'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&raw).unwrap()["fetched_ms"],
            1
        );
        deps.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE connections SET serve_enabled=0 WHERE id='trae-stale'",
                [],
            )
            .unwrap();
        assert!(models_at(&deps, &selected, Some(&base)).await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        task.abort();
    }
    #[test]
    fn global_hidden_models_and_invalid_identifiers_are_not_selectable() {
        assert_eq!(
            chat_model(&json!({"config_name":"visible"}), "global"),
            Some("visible".into())
        );
        assert_eq!(
            chat_model(
                &json!({"config_name":"hidden","is_invisible_to_user":true}),
                "global"
            ),
            None
        );
        assert_eq!(
            chat_model(
                &json!({"config_name":"hidden","is_invisible_to_user":true}),
                "cn"
            ),
            Some("hidden".into())
        );
        for value in [
            json!({"config_name":""}),
            json!({"config_name":"\ninvalid"}),
            json!({"config_name":"Custom_Model_9"}),
            json!({"config_name":"custom","display_config":{"is_custom_model":true}}),
            json!({"config_name":"x","usage":"title"}),
        ] {
            assert_eq!(chat_model(&value, "cn"), None);
        }
    }
    #[tokio::test]
    async fn redirects_are_not_followed_and_empty_lists_do_not_replace_history() {
        for redirect in [true, false] {
            let deps = crate::proxy::test_support::bare_deps();
            let selected = insert(&deps, "trae-empty");
            deps.db
                .lock()
                .unwrap()
                .execute(
                    "UPDATE connections SET models_json='[\"history\"]' WHERE id='trae-empty'",
                    [],
                )
                .unwrap();
            let hits = Arc::new(AtomicUsize::new(0));
            let count = hits.clone();
            let (base, task) = serve(Router::new()
                .route("/api/ide/v1/batch_get_detail_param", post(move || async move {
                    if redirect { axum::response::Response::builder().status(302).header("location", "/credential-sink").body(axum::body::Body::empty()).unwrap() }
                    else { axum::response::Response::new(axum::body::Body::from("{\"function_configs\":[{\"function\":\"chat_v3\",\"config_info_list\":[]}] }")) }
                }))
                .route("/credential-sink", axum::routing::get(move || { let count = count.clone(); async move {
                    count.fetch_add(1, Ordering::SeqCst); "{}"
                }}))).await;
            assert!(models_at(&deps, &selected, Some(&base)).await.is_err());
            assert_eq!(hits.load(Ordering::SeqCst), 0);
            let raw: String = deps
                .db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT models_json FROM connections WHERE id='trae-empty'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(raw, "[\"history\"]");
            task.abort();
        }
    }
    #[tokio::test]
    async fn function_fallbacks_progress_independently_when_an_earlier_one_waits() {
        let deps = crate::proxy::test_support::bare_deps();
        let selected = insert(&deps, "trae-parallel");
        let later_started = Arc::new(tokio::sync::Notify::new());
        let notify = later_started.clone();
        let (base, task) = serve(
            Router::new()
                .route(
                    "/api/ide/v1/batch_get_detail_param",
                    post(|| async { "{}" }),
                )
                .route(
                    "/api/ide/v1/get_detail_param",
                    post(move |body: String| {
                        let notify = notify.clone();
                        async move {
                            let body: Value = serde_json::from_str(&body).unwrap();
                            if body["function"] == "solo_work_lite" {
                                let _ =
                                    tokio::time::timeout(Duration::from_secs(2), notify.notified())
                                        .await;
                                json!({"config_info_list":[]}).to_string()
                            } else if body["function"] == "solo_coder" {
                                notify.notify_one();
                                json!({"config_info_list":[{"config_name":"later-model"}]})
                                    .to_string()
                            } else {
                                json!({"config_info_list":[]}).to_string()
                            }
                        }
                    }),
                ),
        )
        .await;
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            models_at(&deps, &selected, Some(&base)),
        )
        .await;
        task.abort();
        assert_eq!(
            result.expect("慢函数不能阻止其他函数开始查询").unwrap(),
            vec!["later-model"]
        );
    }
    #[tokio::test]
    async fn changed_authorization_cannot_publish_old_catalog() {
        let deps = crate::proxy::test_support::bare_deps();
        let selected = insert(&deps, "trae-race");
        let shared = deps.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let (base, task) = serve(Router::new().route(
            "/api/ide/v1/batch_get_detail_param",
            post(move || {
                let shared = shared.clone();
                let count = count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    shared
                        .db
                        .lock()
                        .unwrap()
                        .execute(
                            "UPDATE connections SET secret='replaced' WHERE id='trae-race'",
                            [],
                        )
                        .unwrap();
                    batch().to_string()
                }
            }),
        ))
        .await;
        assert!(models_at(&deps, &selected, Some(&base)).await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let cached: Option<String> = deps
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT models_json FROM connections WHERE id='trae-race'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(cached.is_none());
        task.abort();
    }
}
