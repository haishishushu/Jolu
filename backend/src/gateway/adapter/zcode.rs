//! BigModel 个人 Coding Plan：登录态 → 项目短期令牌 → Anthropic 兼容模型接口。
//! 依据 zai-org/zcode 的 project-access-token-client.ts；不扫描其他账号/团队。
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
    http::{HeaderMap, Method, StatusCode},
    response::Response,
};
use serde_json::{json, Value};
use std::sync::Arc;
const ORIGIN: &str = "https://bigmodel.cn";
const MODEL_BASE: &str = "https://open.bigmodel.cn/api/anthropic";

pub(crate) fn bound(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<(String, crate::zcode_browser_auth::Session), String> {
    let secret: String = conn
        .query_row(
            "SELECT secret FROM connections WHERE id=?1 AND platform='zcode' AND kind='auth'",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| "所选 ZCode 账号缺少完整授权，请重新登录")?;
    let Credential::Zcode { session } = provider_browser_auth::load(id, "zcode", &secret)? else {
        return Err("ZCode 授权类型不匹配".into());
    };
    if session.user_id.is_empty() || session.access_token.is_empty() {
        return Err("ZCode 授权身份不完整".into());
    }
    Ok((secret, session))
}

async fn data(mut response: reqwest::Response) -> Result<Value, String> {
    if !response.status().is_success() {
        return Err(format!(
            "ZCode 换取调用凭据失败（HTTP {}），请检查此账号登录或套餐",
            response.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "ZCode 响应读取失败")? {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("ZCode 响应超过大小限制".into());
        }
        bytes.extend_from_slice(&chunk)
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "ZCode 响应格式无效")?;
    if !matches!(value["code"].as_i64(), Some(0 | 200))
        && !matches!(value["code"].as_str(), Some("0" | "200"))
    {
        return Err("ZCode 拒绝换取调用凭据，请检查此账号的套餐权限".into());
    }
    value
        .get("data")
        .cloned()
        .ok_or("ZCode 未返回调用凭据数据".into())
}

fn segment(value: &Value, key: &str) -> Result<String, String> {
    let value = value[key]
        .as_str()
        .filter(|v| !v.trim().is_empty())
        .ok_or("ZCode 项目身份不完整")?;
    Ok(url::form_urlencoded::byte_serialize(value.as_bytes())
        .collect::<String>()
        .replace('+', "%20"))
}

async fn material_at(deps: &Arc<ProxyDeps>, id: &str, origin: &str) -> Result<String, String> {
    let _guard = crate::gateway::subscription::lock(id).await;
    let (secret, session) = {
        let conn = deps.db.lock().map_err(|_| "无法读取 ZCode 授权")?;
        bound(&conn, id)?
    };
    let builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(30));
    let builder = if cfg!(test) && origin != ORIGIN {
        builder.no_proxy()
    } else {
        match crate::net::current_mode() {
            crate::net::NetMode::Direct => builder.no_proxy(),
            crate::net::NetMode::Manual(url) => {
                builder.proxy(reqwest::Proxy::all(url).map_err(|_| "ZCode 代理配置无效")?)
            }
            crate::net::NetMode::System => match crate::net::system_proxy_url() {
                Some(url) => {
                    builder.proxy(reqwest::Proxy::all(url).map_err(|_| "ZCode 系统代理无效")?)
                }
                None => builder,
            },
        }
    };
    let client = builder.build().map_err(|_| "无法创建 ZCode 授权连接")?;
    let request = |method, url: String| {
        client
            .request(method, url)
            .header("authorization", &session.access_token)
    };
    let customer = data(
        request(
            Method::GET,
            format!("{origin}/api/biz/customer/getCustomerInfo"),
        )
        .send()
        .await
        .map_err(|_| "ZCode 项目查询失败")?,
    )
    .await?;
    let organizations = customer["organizations"]
        .as_array()
        .ok_or("ZCode 未返回个人项目")?;
    let mut candidates = Vec::new();
    for org in organizations {
        if let Some(projects) = org["projects"].as_array() {
            for project in projects {
                if project["projectType"] == 2 || project["projectType"] == "2" {
                    continue;
                }
                candidates.push((org, project));
            }
        }
    }
    let defaults: Vec<_> = candidates
        .iter()
        .copied()
        .filter(|(o, p)| {
            o["organizationName"]
                .as_str()
                .is_some_and(|s| s.contains("默认机构"))
                && p["projectName"]
                    .as_str()
                    .is_some_and(|s| s.contains("默认项目"))
        })
        .collect();
    let (org, project) = if defaults.len() == 1 {
        defaults[0]
    } else if candidates.len() == 1 {
        candidates[0]
    } else {
        return Err(
            "ZCode 个人项目无法唯一确定，请在官方工具确认默认个人项目；不会自动改用团队项目".into(),
        );
    };
    let url = format!(
        "{origin}/api/biz/v1/organization/{}/projects/{}/api_keys",
        segment(org, "organizationId")?,
        segment(project, "projectId")?
    );
    let keys = data(
        request(Method::GET, url.clone())
            .send()
            .await
            .map_err(|_| "ZCode 项目凭据查询失败")?,
    )
    .await?;
    let keys = keys.as_array().ok_or("ZCode 项目凭据列表无效")?;
    // 模型入口只使用已有项目 Key；创建长期凭据是额外远端副作用，不能隐式执行。
    let key = keys
        .iter()
        .find(|k| k["name"] == "zcode-api-key")
        .ok_or("ZCode 缺少已有个人项目凭据，请先在官方工具配置；Jolu 不会自动创建 API Key")?;
    let token_url = format!("{url}/{}/access_tokens", segment(&key, "apiKey")?);
    let value = data(
        client
            .post(token_url)
            .bearer_auth(&session.access_token)
            .json(&json!({"clientType":"zcode","clientVersion":"3.17.1"}))
            .send()
            .await
            .map_err(|_| "ZCode 短期凭据签发失败")?,
    )
    .await?;
    if value["enable"] == false
        || value["tokenType"] != "Bearer"
        || value["expiresIn"].as_f64().is_none_or(|v| v <= 0.0)
        || value["expiresAt"]
            .as_f64()
            .is_none_or(|v| v <= chrono::Utc::now().timestamp() as f64)
    {
        return Err("ZCode 项目调用权限未开启或短期凭据已过期".into());
    }
    let token = value["accessToken"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or("ZCode 未返回短期调用凭据")?
        .to_owned();
    let conn = deps.db.lock().map_err(|_| "无法复查 ZCode 授权")?;
    if bound(&conn, id)?.0 != secret {
        return Err("ZCode 账号授权已变化，请重试；不会使用旧账号结果".into());
    }
    Ok(token)
}

pub(crate) async fn models(deps: &Arc<ProxyDeps>, id: &str) -> Result<Vec<String>, String> {
    let token = material_at(deps, id, ORIGIN).await?;
    tokio::task::spawn_blocking(move || {
        crate::connections::fetch_models("zcode", Some("anthropic"), MODEL_BASE, &token)
    })
    .await
    .map_err(|_| "ZCode 模型查询失败")?
}

pub(crate) async fn forward(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    protocol: CallerProtocol,
    body: Bytes,
) -> Response {
    if protocol != CallerProtocol::Anthropic {
        return error_response(
            StatusCode::BAD_REQUEST,
            "ZCode 订阅使用 Anthropic Messages 协议",
        );
    }
    let token = match material_at(&deps, &entry.connection_id, ORIGIN).await {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::UNAUTHORIZED, &e),
    };
    let value: Value = match serde_json::from_slice(&body) {
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
    target.secret = Some(token);
    target.base_url = Some(MODEL_BASE.into());
    target.protocol = "anthropic".into();
    super::direct::forward(
        deps,
        &target,
        model.into(),
        Method::POST,
        "/v1/messages",
        &HeaderMap::new(),
        body,
    )
    .await
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn absent_project_key_never_creates_a_remote_key() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let deps = crate::proxy::test_support::bare_deps();
        let session = crate::zcode_browser_auth::Session {
            access_token: "fixture-login".into(),
            refresh_token: None,
            user_id: "fixture-user".into(),
            account: None,
            zcode_token: "fixture-session".into(),
        };
        let sealed = provider_browser_auth::seal("no-key", &Credential::Zcode { session }).unwrap();
        deps.db.lock().unwrap().execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug) VALUES('no-key','zcode','auth','fixture',?1,1,'no-key')", [sealed]).unwrap();
        let creates = Arc::new(AtomicUsize::new(0));
        let count = creates.clone();
        let app = axum::Router::new()
            .route("/api/biz/customer/getCustomerInfo", axum::routing::get(|| async {json!({"code":200,"data":{"organizations":[{"organizationId":"org","projects":[{"projectId":"personal","projectType":1}]}]}}).to_string()}))
            .route("/api/biz/v1/organization/org/projects/personal/api_keys", axum::routing::get(||async{json!({"code":200,"data":[]}).to_string()}).post(move || {let count=count.clone();async move {count.fetch_add(1,Ordering::SeqCst);json!({"code":200,"data":{"apiKey":"created"}}).to_string()}}));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let result = material_at(&deps, "no-key", &base).await;
        assert_eq!(
            creates.load(Ordering::SeqCst),
            0,
            "模型查询不能隐式创建远端 API Key"
        );
        assert!(result.unwrap_err().contains("已有"));
        server.abort();
    }

    #[tokio::test]
    async fn credential_exchange_rejects_redirects_without_contacting_destination() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let deps = crate::proxy::test_support::bare_deps();
        let session = crate::zcode_browser_auth::Session {
            access_token: "fixture-login".into(),
            refresh_token: None,
            user_id: "fixture-user".into(),
            account: None,
            zcode_token: "fixture-session".into(),
        };
        let sealed =
            provider_browser_auth::seal("redirect", &Credential::Zcode { session }).unwrap();
        deps.db.lock().unwrap().execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug) VALUES('redirect','zcode','auth','fixture',?1,1,'redirect')",[sealed]).unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let count = hits.clone();
        let app = axum::Router::new()
            .route(
                "/api/biz/customer/getCustomerInfo",
                axum::routing::get(|| async { axum::response::Redirect::temporary("/unexpected") }),
            )
            .route(
                "/unexpected",
                axum::routing::get(move || {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        "{}"
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        assert!(material_at(&deps, "redirect", &base).await.is_err());
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "携带凭据的业务查询不得跟随重定向"
        );
        server.abort();
    }

    #[tokio::test]
    async fn personal_project_token_uses_only_selected_account() {
        let deps = crate::proxy::test_support::bare_deps();
        for id in ["a", "b"] {
            let session = crate::zcode_browser_auth::Session {
                access_token: format!("{id}-login"),
                refresh_token: None,
                user_id: id.into(),
                account: None,
                zcode_token: format!("{id}-session"),
            };
            let sealed = provider_browser_auth::seal(id, &Credential::Zcode { session }).unwrap();
            deps.db.lock().unwrap().execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug) VALUES(?1,'zcode','auth',?1,?2,1,?1)",rusqlite::params![id,sealed]).unwrap();
        }
        let app=axum::Router::new()
            .route("/api/biz/customer/getCustomerInfo",axum::routing::get(|h:HeaderMap|async move{
                assert_eq!(h["authorization"],"a-login");json!({"code":200,"data":{"organizations":[{"organizationId":"org-a","projects":[{"projectId":"team","projectType":2},{"projectId":"personal-a","projectType":1}]}]}}).to_string()
            }))
            .route("/api/biz/v1/organization/org-a/projects/personal-a/api_keys",axum::routing::get(|h:HeaderMap|async move{
                assert_eq!(h["authorization"],"a-login");json!({"code":200,"data":[{"name":"zcode-api-key","apiKey":"key-a"}]}).to_string()
            }))
            .route("/api/biz/v1/organization/org-a/projects/personal-a/api_keys/key-a/access_tokens",axum::routing::post(|h:HeaderMap|async move{
                assert_eq!(h["authorization"],"Bearer a-login");json!({"code":200,"data":{"accessToken":"a-short-lived","tokenType":"Bearer","expiresIn":3600,"expiresAt":4102444800i64}}).to_string()
            }));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        assert_eq!(
            material_at(&deps, "a", &base).await.unwrap(),
            "a-short-lived"
        );
        assert_eq!(
            bound(&deps.db.lock().unwrap(), "b").unwrap().1.access_token,
            "b-login"
        );
        assert!(material_at(&deps, "missing", &base).await.is_err());
        server.abort();
    }
}
