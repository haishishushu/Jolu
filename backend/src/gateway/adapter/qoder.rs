//! Qoder 所选订阅的模型入口。按 magpie-community/plugins `packages/qoder`（MIT）直连
//! Qoder 模型接口：设备令牌换任务令牌（国内版换不到时用设备令牌本身），COSY 签名，
//! 请求体编码后发往 agent_chat_generation，SSE 回复转回 OpenAI Chat Completions。
use super::qoder_chat::{self, Event, Failure};
use super::qoder_cosy::{self, Signer};
use crate::{
    gateway::{
        route::PoolEntry,
        wire::{error_response, CallerProtocol},
    },
    provider_browser_auth::{self, Credential},
    proxy::ProxyDeps,
    qoder_desktop::Session,
};
use axum::{
    body::{Body, Bytes},
    http::{HeaderValue, StatusCode},
    response::Response,
};
use futures_util::{Stream, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    pin::Pin,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

const CLIENT_ID: &str = "732aef47-9cf2-46a2-95fe-4cebb5d0d1fa";
const CHAT_PATH: &str = "/algo/api/v2/service/pro/sse/agent_chat_generation";
const CHAT_QUERY: &str = "?FetchKeys=llm_model_result&AgentId=agent_common&Encode=1";
const MODELS_PATH: &str = "/algo/api/v2/model/list";
const MODELS_QUERY: &str = "?Encode=1";
const MODEL_TTL_MS: i64 = 5 * 60_000;
/// 任务令牌离到期不足此时长就重新换取。
const TOKEN_LEAD_MS: i64 = 5 * 60_000;

/// 两个站点的账号、模型接口与读不到列表时的内置清单（magpie SITES / MODELS / CN_MODELS）。
struct Site {
    openapi: &'static str,
    api: &'static str,
    /// 国内版 CLI 不换任务令牌：换票被拒（4xx）时用设备令牌签名
    device_chat: bool,
    models: &'static [&'static str],
}

const GLOBAL: Site = Site {
    openapi: "https://openapi.qoder.sh",
    api: "https://api3.qoder.sh",
    device_chat: false,
    models: &[
        "ultimate", "fast", "performance", "efficient", "smodel", "cmodel", "qmodel_38max", "qfmodel",
        "qmodel_latest", "qmodel", "kmodel_latest", "kmodel", "gmodel", "gfmodel", "dmodel", "dfmodel", "mmodel",
    ],
};

const CN: Site = Site {
    openapi: "https://openapi.qoder.com.cn",
    api: "https://gateway.qoder.com.cn",
    device_chat: true,
    models: &["ultimate", "performance", "efficient", "lite"],
};

fn site(edition: &str) -> Result<&'static Site, String> {
    match edition {
        "cn" => Ok(&CN),
        "global" => Ok(&GLOBAL),
        _ => Err("Qoder 地区无效".into()),
    }
}

fn token(value: &Value, key: &str) -> Result<String, String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 65536 && !s.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or("Qoder 返回的凭据不完整".into())
}

fn check_identity(value: &Value, expected: &str) -> Result<(), String> {
    let actual = ["id", "user_id", "uid"].iter().find_map(|key| value[*key].as_str());
    if !expected.is_empty() && actual == Some(expected) {
        Ok(())
    } else {
        Err("Qoder 返回的账号与所选连接不一致".into())
    }
}

pub(crate) fn bound(conn: &rusqlite::Connection, id: &str) -> Result<(String, String, Session), String> {
    let sealed: String = conn
        .query_row(
            "SELECT secret FROM connections WHERE id=?1 AND platform='qoder' AND kind='auth'",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| "所选 Qoder 账号缺少授权")?;
    let Credential::Qoder { edition, session } = provider_browser_auth::load(id, "qoder", &sealed)? else {
        return Err("Qoder 授权类型不匹配".into());
    };
    site(&edition)?;
    if session.token.is_empty() || session.user_id.as_deref().is_none_or(|s| s.is_empty()) {
        return Err("Qoder 授权身份不完整，请重新登录".into());
    }
    Ok((sealed, edition, session))
}

/// 设备授权时上报的 machine_id；早于此版本的授权没有保存，按连接与账号派生一个稳定值。
fn machine_id(id: &str, session: &Session) -> String {
    if let Some(m) = session.machine_id.as_deref().filter(|m| !m.is_empty()) {
        return m.into();
    }
    use sha2::{Digest, Sha256};
    let h: String = Sha256::digest(format!("jolu-qoder-machine\0{id}\0{}", session.user_id.as_deref().unwrap_or_default()).as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("{}-{}-4{}-8{}-{}", &h[..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
}

/// 仅固定官方端点；重定向禁止，不能把 Bearer 转交给另一站点。代理跟随 Jolu 网络设置。
fn http_client(test: bool) -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15));
    let builder = if test {
        builder.no_proxy()
    } else {
        match crate::net::current_mode() {
            crate::net::NetMode::Direct => builder.no_proxy(),
            crate::net::NetMode::Manual(url) => builder.proxy(reqwest::Proxy::all(url).map_err(|_| "Qoder 代理配置无效")?),
            crate::net::NetMode::System => match crate::net::system_proxy_url() {
                Some(url) => builder.proxy(reqwest::Proxy::all(url).map_err(|_| "Qoder 系统代理无效")?),
                None => builder,
            },
        }
    };
    builder.build().map_err(|_| "无法创建 Qoder 请求".into())
}

/// 读取有大小上限的 JSON 响应，返回状态码与内容。
async fn read_json(response: reqwest::Response, limit: usize) -> Result<(u16, Value), String> {
    let status = response.status().as_u16();
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "Qoder 响应读取失败")? {
        if bytes.len() + chunk.len() > limit {
            return Err("Qoder 响应过大".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((status, serde_json::from_slice(&bytes).unwrap_or(Value::Null)))
}

async fn ok_json(request: reqwest::RequestBuilder, what: &str) -> Result<Value, String> {
    let response = request.timeout(Duration::from_secs(30)).send().await.map_err(|_| format!("{what}失败"))?;
    let (status, value) = read_json(response, 1024 * 1024).await?;
    if !(200..300).contains(&status) {
        return Err(format!("{what}被拒绝（HTTP {status}）"));
    }
    Ok(value)
}

async fn identity(client: &reqwest::Client, base: &str, access: &str, expected: &str) -> Result<(), String> {
    let value = ok_json(client.get(format!("{base}/api/v1/userinfo")).bearer_auth(access), "Qoder 账号核实").await?;
    check_identity(&value, expected)
}

/// magpie expiresAt：expire_time / expires_at（日期或秒、毫秒时间戳），再 expires_in（毫秒），否则一天。
fn expires_at(value: &Value) -> i64 {
    let now = chrono::Utc::now().timestamp_millis();
    for key in ["expire_time", "expires_at"] {
        let at = match &value[key] {
            Value::Number(n) => n.as_i64().map(|n| if n < 1_000_000_000_000 { n * 1000 } else { n }).unwrap_or(0),
            Value::String(s) => s.trim().parse::<i64>().map(|n| if n < 1_000_000_000_000 { n * 1000 } else { n })
                .or_else(|_| chrono::DateTime::parse_from_rfc3339(s.trim()).map(|d| d.timestamp_millis())).unwrap_or(0),
            _ => 0,
        };
        if at > now {
            return at;
        }
    }
    match value["expires_in"].as_i64().filter(|n| *n > 0) {
        Some(ms) => now + ms,
        None => now + 24 * 60 * 60_000,
    }
}

/// 签名所需的一切：授权密文快照、地区、会话与签名令牌。
struct Chat {
    sealed: String,
    edition: String,
    session: Session,
    machine: String,
    token: String,
}

impl Chat {
    fn signer(&self) -> Signer<'_> {
        let account = self.session.account.as_deref().unwrap_or_default();
        Signer {
            uid: self.session.user_id.as_deref().unwrap_or_default(),
            name: "",
            email: if account.contains('@') { account } else { "" },
            access: &self.token,
            machine_id: &self.machine,
        }
    }
}

/// 连接 → (授权密文快照, 签名令牌, 到期毫秒)。仅存内存，不持久化。
static TOKENS: LazyLock<Mutex<HashMap<String, (String, String, i64)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// 锁覆盖设备令牌刷新与任务令牌换取；返回前再查一次数据库快照，避免切号沿用旧结果。
async fn chat_token(deps: &Arc<ProxyDeps>, entry: &PoolEntry, test_base: Option<&str>, renew: bool) -> Result<Chat, String> {
    let id = entry.connection_id.as_str();
    let _guard = crate::gateway::subscription::lock(id).await;
    // 等待账号锁期间来源可能已关闭、暂停或被重新配置；不得沿用排队前快照换票。
    if !crate::gateway::route::source_is_current(deps, entry) {
        return Err("Qoder 来源已停用或发生变化，请刷新后重试".into());
    }
    let (mut sealed, edition, mut session) = {
        let conn = deps.db.lock().map_err(|_| "无法读取 Qoder 授权")?;
        bound(&conn, id)?
    };
    let machine = machine_id(id, &session);
    let now = chrono::Utc::now().timestamp_millis();
    if !renew {
        let cached = TOKENS.lock().ok().and_then(|t| t.get(id).cloned());
        if let Some((at, token, expires)) = cached.filter(|(at, _, expires)| *at == sealed && *expires - now > TOKEN_LEAD_MS) {
            let _ = (at, expires);
            return Ok(Chat { sealed, edition, session, machine, token });
        }
    }
    let site = site(&edition)?;
    let base = if cfg!(test) { test_base.unwrap_or(site.openapi) } else { site.openapi };
    let client = http_client(cfg!(test) && test_base.is_some())?;
    let expires = session
        .expires_at
        .as_deref()
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .ok_or("Qoder 授权有效期未知，请重新登录")?;
    let uid = session.user_id.clone().unwrap_or_default();
    if expires.timestamp_millis() <= now + 60_000 {
        let refresh = session.refresh_token.as_deref().filter(|s| !s.is_empty()).ok_or("Qoder 授权已过期，请重新登录")?;
        let value = ok_json(
            client.post(format!("{base}/api/v1/deviceToken/refresh")).json(&json!({"refresh_token":refresh})),
            "Qoder 授权刷新",
        )
        .await?;
        session.token = token(&value, "device_token").or_else(|_| token(&value, "token"))?;
        session.refresh_token = Some(token(&value, "refresh_token")?);
        session.expires_at = if let Some(at) = value["expires_at"].as_str() {
            Some(chrono::DateTime::parse_from_rfc3339(at).map_err(|_| "Qoder 刷新有效期无效")?.to_rfc3339())
        } else {
            value["expires_in"]
                .as_i64()
                .filter(|v| *v > 0 && *v <= 365 * 86400)
                .map(|seconds| (chrono::Utc::now() + chrono::Duration::seconds(seconds)).to_rfc3339())
        };
        if session.expires_at.is_none() {
            return Err("Qoder 刷新缺少有效期".into());
        }
        identity(&client, base, &session.token, &uid).await?;
        let conn = deps.db.lock().map_err(|_| "无法保存 Qoder 授权")?;
        provider_browser_auth::refresh_bound(&conn, id, "qoder", &sealed, &Credential::Qoder { edition: edition.clone(), session: session.clone() })?;
        sealed = bound(&conn, id)?.0;
    }
    let response = client
        .post(format!("{base}/api/v1/me/jobToken"))
        .bearer_auth(&session.token)
        .json(&json!({"clientId":CLIENT_ID}))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| "Qoder 任务凭据换取失败")?;
    let (status, value) = read_json(response, 1024 * 1024).await?;
    let (chat_token, chat_expires) = if (200..300).contains(&status) {
        let job = token(&value, "token")?;
        identity(&client, base, &job, &uid).await?;
        (job, expires_at(&value))
    } else if site.device_chat && (400..500).contains(&status) {
        // 国内版 CLI 直接用设备令牌签名模型请求
        identity(&client, base, &session.token, &uid).await?;
        let device_expires = session.expires_at.as_deref().and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok()).map(|d| d.timestamp_millis()).unwrap_or(now);
        (session.token.clone(), device_expires)
    } else {
        return Err(format!("Qoder 任务凭据换取被拒绝（HTTP {status}）"));
    };
    let conn = deps.db.lock().map_err(|_| "无法复查 Qoder 授权")?;
    if bound(&conn, id)?.0 != sealed {
        return Err("Qoder 账号授权已变化，请重新发起请求".into());
    }
    if let Ok(mut tokens) = TOKENS.lock() {
        tokens.insert(id.into(), (sealed.clone(), chat_token.clone(), chat_expires));
    }
    Ok(Chat { sealed, edition, session, machine, token: chat_token })
}

fn forget_token(id: &str) {
    if let Ok(mut tokens) = TOKENS.lock() {
        tokens.remove(id);
    }
}

fn credential_hash(secret: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(secret.as_bytes()))
}

/// app 场景的模型（Qoder 应用显示的列表），再补上 chat 场景里 app 已不列出的模型；
/// 只要启用的、有具体模型键的（auto / default 是 Qoder 内部路由）。
fn scene_entries(listing: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = listing["app"].as_array().cloned().unwrap_or_default();
    let mut after: isize = -1;
    for m in listing["chat"].as_array().into_iter().flatten() {
        if let Some(i) = out.iter().position(|x| x["key"] == m["key"]) {
            after = i as isize;
            continue;
        }
        after += 1;
        out.insert(after as usize, m.clone());
    }
    out.into_iter()
        .filter(|m| m["enable"] == true && !["", "auto", "default"].contains(&m["key"].as_str().unwrap_or_default().trim()))
        .map(|mut m| {
            // 条目里的密钥与地址字段不落库
            if let Some(o) = m.as_object_mut() {
                for key in ["api_key", "apiKey", "url"] {
                    o.remove(key);
                }
            }
            m
        })
        .collect()
}

/// 本适配器为当前授权保存的列表条目：(获取时间, 条目)。
fn stored(conn: &rusqlite::Connection, id: &str) -> Option<(i64, Vec<Value>)> {
    let (secret, text): (String, String) = conn
        .query_row(
            "SELECT secret,models_json FROM connections WHERE id=?1 AND platform='qoder' AND kind='auth'",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    if value["credential_hash"].as_str()? != credential_hash(&secret) {
        return None;
    }
    Some((value["fetched_ms"].as_i64()?, value["entries"].as_array()?.clone()))
}

fn keys(entries: &[Value]) -> Vec<String> {
    entries.iter().filter_map(|m| m["key"].as_str().map(str::to_owned)).collect()
}

/// 模型池展示用：上次读到的账号列表，没有时为内置清单（与 `/v1/models` 一致）。
pub(crate) fn cached_models(conn: &rusqlite::Connection, id: &str) -> Option<Vec<String>> {
    let (_, edition, _) = bound(conn, id).ok()?;
    let listed = stored(conn, id).map(|(_, entries)| keys(&entries)).filter(|k| !k.is_empty());
    Some(listed.unwrap_or_else(|| site(&edition).map(|s| s.models.iter().map(|m| m.to_string()).collect()).unwrap_or_default()))
}

async fn fetch_entries(deps: &Arc<ProxyDeps>, entry: &PoolEntry, chat: &Chat, test_base: Option<&str>) -> Result<Vec<Value>, String> {
    let site = site(&chat.edition)?;
    let api = if cfg!(test) { test_base.unwrap_or(site.api) } else { site.api };
    let client = http_client(cfg!(test) && test_base.is_some())?;
    let mut request = client.get(format!("{api}{MODELS_PATH}{MODELS_QUERY}")).timeout(Duration::from_secs(15));
    for (name, value) in qoder_cosy::cosy_headers(MODELS_PATH, &chat.signer(), "")? {
        request = request.header(name, value);
    }
    let response = request.send().await.map_err(|_| "Qoder 模型列表请求失败")?;
    let (status, listing) = read_json(response, 4 * 1024 * 1024).await?;
    if !(200..300).contains(&status) {
        return Err(format!("Qoder 模型列表获取失败（HTTP {status}）"));
    }
    let entries = scene_entries(&listing);
    let cache = json!({"credential_hash":credential_hash(&chat.sealed),"fetched_ms":chrono::Utc::now().timestamp_millis(),"entries":entries}).to_string();
    let conn = deps.db.lock().map_err(|_| "无法保存 Qoder 模型列表")?;
    let changed = conn
        .execute(
            "UPDATE connections SET models_json=?3 WHERE id=?1 AND platform='qoder' AND kind='auth' AND secret=?2",
            rusqlite::params![entry.connection_id, chat.sealed, cache],
        )
        .map_err(|_| "无法保存 Qoder 模型列表")?;
    if changed != 1 {
        return Err("Qoder 授权已变化，已丢弃旧账号的模型列表".into());
    }
    Ok(entries)
}

/// 5 分钟内用缓存的账号列表，否则重新读取。
async fn entries(deps: &Arc<ProxyDeps>, entry: &PoolEntry, chat: &Chat, test_base: Option<&str>, fresh: bool) -> Result<Vec<Value>, String> {
    if !fresh {
        let cached = deps.db.lock().ok().and_then(|conn| stored(&conn, &entry.connection_id));
        if let Some((at, list)) = cached.filter(|(at, list)| !list.is_empty() && (0..MODEL_TTL_MS).contains(&(chrono::Utc::now().timestamp_millis() - at))) {
            let _ = at;
            return Ok(list);
        }
    }
    fetch_entries(deps, entry, chat, test_base).await
}

/// 账号模型列表；读不到或为空时与 magpie 一样沿用上次列表，再没有则用内置清单。
/// 授权本身不可用时返回错误。
pub(crate) async fn models(deps: &Arc<ProxyDeps>, entry: &PoolEntry) -> Result<Vec<String>, String> {
    models_at(deps, entry, None).await
}

async fn models_at(deps: &Arc<ProxyDeps>, entry: &PoolEntry, test_base: Option<&str>) -> Result<Vec<String>, String> {
    let fallback = {
        let conn = deps.db.lock().map_err(|_| "无法读取 Qoder 授权")?;
        cached_models(&conn, &entry.connection_id).ok_or("所选 Qoder 账号缺少授权")?
    };
    let chat = match chat_token(deps, entry, test_base, false).await {
        Ok(chat) => chat,
        Err(e) if e.contains("登录") || e.contains("授权") => return Err(e),
        Err(_) => return Ok(fallback),
    };
    match entries(deps, entry, &chat, test_base, false).await {
        Ok(list) if !list.is_empty() => Ok(keys(&list)),
        _ => Ok(fallback),
    }
}

/// 上游 SSE 的逐行读取与事件队列。
struct Reader {
    upstream: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
    buf: Vec<u8>,
    events: qoder_chat::Events,
    queue: VecDeque<Event>,
    ended: bool,
}

impl Reader {
    fn new(response: reqwest::Response, tools: &[Value]) -> Self {
        Self {
            upstream: Box::pin(response.bytes_stream()),
            buf: Vec::new(),
            events: qoder_chat::Events::new(tools),
            queue: VecDeque::new(),
            ended: false,
        }
    }

    fn lines(&mut self) {
        while let Some(n) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=n).collect();
            let text = String::from_utf8_lossy(&line);
            let events = self.events.line(text.trim());
            self.queue.extend(events);
            if self.events.done {
                self.ended = true;
                return;
            }
        }
    }

    async fn next(&mut self) -> Option<Event> {
        loop {
            if let Some(e) = self.queue.pop_front() {
                return Some(e);
            }
            if self.ended {
                return None;
            }
            match self.upstream.next().await {
                Some(Ok(chunk)) => {
                    self.buf.extend_from_slice(&chunk);
                    self.lines();
                }
                Some(Err(_)) => {
                    self.queue.push_back(Event::Error(Failure { status: 502, message: "Qoder 回复中断".into(), retry_after: None, lapsed: false, nameless: false }));
                    self.ended = true;
                }
                None => {
                    if !self.buf.is_empty() {
                        self.buf.push(b'\n');
                        self.lines();
                    }
                    if !self.events.done {
                        let events = self.events.finish();
                        self.queue.extend(events);
                    }
                    self.ended = true;
                }
            }
        }
    }
}

fn failed(f: &Failure) -> Response {
    let mut response = error_response(StatusCode::from_u16(f.status).unwrap_or(StatusCode::BAD_GATEWAY), &f.message);
    if let Some(seconds) = f.retry_after {
        response.headers_mut().insert("retry-after", HeaderValue::from(seconds));
    }
    response
}

/// 用量按所选连接归账；只记录上游真实给出的数字。
struct Meter {
    deps: Arc<ProxyDeps>,
    entry: PoolEntry,
    started: std::time::Instant,
    id: String,
    model: String,
}

impl Meter {
    fn record(&self, usage: &Value) {
        crate::gateway::subscription::record_usage(
            &self.deps,
            &self.entry,
            &json!({"id":self.id,"model":self.model,"usage":qoder_chat::metered(usage)}),
            self.started.elapsed().as_millis() as i64,
        );
    }
}

/// 非流式：读完整个回复再组成 chat.completion。
async fn complete(mut reader: Reader, first: Event, meter: Meter) -> Response {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut calls: Vec<Value> = Vec::new();
    let mut stop = "stop";
    let mut usage: Option<Value> = None;
    let mut next = Some(first);
    while let Some(e) = next {
        match e {
            Event::Error(f) => return failed(&f),
            Event::Text(t) => content.push_str(&t),
            Event::Reasoning(r) => reasoning.push_str(&r),
            Event::Tool { id, name, args, .. } => calls.push(json!({"id":id,"type":"function","function":{"name":name,"arguments":args}})),
            Event::Args { index, text } => {
                if let Some(call) = calls.get_mut(index) {
                    let joined = format!("{}{text}", call["function"]["arguments"].as_str().unwrap_or_default());
                    call["function"]["arguments"] = json!(joined);
                }
            }
            Event::Stop { reason, usage: u } => {
                stop = reason;
                usage = u;
            }
        }
        next = reader.next().await;
    }
    let mut message = json!({"role":"assistant","content":content});
    if !reasoning.is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    if !calls.is_empty() {
        message["tool_calls"] = Value::Array(calls);
    }
    let mut body = json!({"id":meter.id,"object":"chat.completion","created":chrono::Utc::now().timestamp(),"model":meter.model,
        "choices":[{"index":0,"message":message,"finish_reason":stop}]});
    if let Some(u) = &usage {
        meter.record(u);
        body["usage"] = u.clone();
    }
    Response::builder().status(StatusCode::OK).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()
}

struct Streaming {
    reader: Reader,
    first: Option<Event>,
    began: bool,
    closed: bool,
    created: i64,
    meter: Meter,
}

fn sse(value: &Value) -> Bytes {
    Bytes::from(format!("data: {value}\n\n"))
}

/// 流式：逐个事件转成 chat.completion.chunk。
fn stream(reader: Reader, first: Event, meter: Meter) -> Response {
    let state = Streaming { reader, first: Some(first), began: false, closed: false, created: chrono::Utc::now().timestamp(), meter };
    let body = futures_util::stream::unfold(state, |mut s| async move {
        if s.closed {
            return None;
        }
        let chunk = |s: &Streaming, delta: Value, finish: Option<&str>, extra: Option<Value>| {
            let mut v = json!({"id":s.meter.id,"object":"chat.completion.chunk","created":s.created,"model":s.meter.model,
                "choices":[{"index":0,"delta":delta,"finish_reason":finish}]});
            if let Some(u) = extra {
                v["usage"] = u;
            }
            sse(&v)
        };
        let mut out = Vec::new();
        if !s.began {
            s.began = true;
            out.push(chunk(&s, json!({"role":"assistant","content":""}), None, None));
        }
        let event = match s.first.take() {
            Some(e) => Some(e),
            None => s.reader.next().await,
        };
        match event {
            None => {
                out.push(Bytes::from_static(b"data: [DONE]\n\n"));
                s.closed = true;
            }
            Some(Event::Text(t)) => out.push(chunk(&s, json!({"content":t}), None, None)),
            Some(Event::Reasoning(r)) => out.push(chunk(&s, json!({"reasoning_content":r}), None, None)),
            Some(Event::Tool { index, id, name, args }) => out.push(chunk(&s, json!({"tool_calls":[{"index":index,"id":id,"type":"function","function":{"name":name,"arguments":args}}]}), None, None)),
            Some(Event::Args { index, text }) => out.push(chunk(&s, json!({"tool_calls":[{"index":index,"function":{"arguments":text}}]}), None, None)),
            Some(Event::Stop { reason, usage }) => {
                if let Some(u) = &usage {
                    s.meter.record(u);
                }
                out.push(chunk(&s, json!({}), Some(reason), usage));
            }
            Some(Event::Error(f)) => {
                out.push(sse(&json!({"error":{"message":f.message,"code":f.status}})));
                s.closed = true;
            }
        }
        let bytes: Vec<u8> = out.concat();
        Some((Ok::<Bytes, std::convert::Infallible>(Bytes::from(bytes)), s))
    });
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(Body::from_stream(body))
        .unwrap()
}

pub(crate) async fn forward(deps: Arc<ProxyDeps>, entry: &PoolEntry, protocol: CallerProtocol, body: Bytes) -> Response {
    forward_at(deps, entry, protocol, body, None).await
}

async fn forward_at(deps: Arc<ProxyDeps>, entry: &PoolEntry, protocol: CallerProtocol, body: Bytes, test_base: Option<&str>) -> Response {
    let started = std::time::Instant::now();
    if protocol != CallerProtocol::Chat {
        return error_response(StatusCode::BAD_REQUEST, "Qoder 订阅使用 OpenAI Chat Completions 协议");
    }
    let mut chat: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "Qoder 请求 JSON 无效"),
    };
    let prefix = format!("{}/", entry.slug);
    let Some(model) = chat["model"].as_str().and_then(|m| m.strip_prefix(&prefix)).map(str::to_owned) else {
        return error_response(StatusCode::BAD_REQUEST, "请选择来源和模型");
    };
    chat["model"] = json!(model);
    let unavailable = |e: String| {
        error_response(
            if crate::gateway::route::source_is_current(&deps, entry) { StatusCode::UNAUTHORIZED } else { StatusCode::SERVICE_UNAVAILABLE },
            &e,
        )
    };
    let mut auth = match chat_token(&deps, entry, test_base, false).await {
        Ok(v) => v,
        Err(e) => return unavailable(e),
    };
    let mut relogged = false;
    // 账号列表里没有（或已关闭）这个模型：重新换票再读一次列表，仍没有才报未知模型
    let mut listed = match entries(&deps, entry, &auth, test_base, false).await {
        Ok(list) => list.into_iter().find(|m| m["key"] == model.as_str()),
        Err(e) => return error_response(StatusCode::BAD_GATEWAY, &e),
    };
    if listed.is_none() {
        relogged = true;
        auth = match chat_token(&deps, entry, test_base, true).await {
            Ok(v) => v,
            Err(e) => return unavailable(e),
        };
        listed = match entries(&deps, entry, &auth, test_base, true).await {
            Ok(list) => list.into_iter().find(|m| m["key"] == model.as_str()),
            Err(e) => return error_response(StatusCode::BAD_GATEWAY, &e),
        };
    }
    let Some(listed) = listed else {
        return error_response(StatusCode::BAD_REQUEST, &format!("Qoder 账号没有可用的模型「{model}」"));
    };
    let info = qoder_chat::model_info(&listed);
    let task = qoder_chat::task_of(&qoder_chat::session_of(&chat), &chat);
    let mut request = match qoder_chat::qoder_body(&chat, &info, &task) {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &e),
    };
    let tools: Vec<Value> = if chat["tool_choice"] == "none" {
        Vec::new()
    } else {
        chat["tools"].as_array().into_iter().flatten().filter(|t| t["type"] == "function").cloned().collect()
    };
    let client = match http_client(cfg!(test) && test_base.is_some()) {
        Ok(c) => c,
        Err(e) => return error_response(StatusCode::BAD_GATEWAY, &e),
    };
    let mut renamed = false;
    for attempt in 0.. {
        if attempt > 0 {
            let id = qoder_cosy::random_uuid();
            request["request_id"] = json!(id);
            request["chat_record_id"] = json!(id);
        }
        let api = match site(&auth.edition) {
            Ok(s) if cfg!(test) => test_base.unwrap_or(s.api),
            Ok(s) => s.api,
            Err(e) => return error_response(StatusCode::BAD_REQUEST, &e),
        };
        let wire = qoder_cosy::encode_body(&request.to_string());
        let headers = match qoder_cosy::chat_headers(CHAT_PATH, &auth.signer(), &wire, &info.key, &info.source) {
            Ok(h) => h,
            Err(e) => return error_response(StatusCode::UNAUTHORIZED, &e),
        };
        let mut call = client.post(format!("{api}{CHAT_PATH}{CHAT_QUERY}")).body(wire).timeout(Duration::from_secs(1200));
        for (name, value) in headers {
            call = call.header(name, value);
        }
        // 网络往返期间来源可能已停用；发出模型请求前再核实一次。
        if !crate::gateway::route::source_is_current(&deps, entry) {
            return error_response(StatusCode::SERVICE_UNAVAILABLE, "Qoder 来源已停用或发生变化，请刷新后重试");
        }
        let response = match call.send().await {
            Ok(r) => r,
            Err(_) => return error_response(StatusCode::BAD_GATEWAY, "Qoder 模型接口不可达"),
        };
        let (fail, opened) = if response.status().is_success() {
            let mut reader = Reader::new(response, &tools);
            match reader.next().await {
                Some(Event::Error(f)) => (Some(f), None),
                Some(first) => (None, Some((reader, first))),
                None => (Some(Failure { status: 502, message: "Qoder 没有返回回答".into(), retry_after: None, lapsed: false, nameless: false }), None),
            }
        } else {
            let status = response.status().as_u16();
            let mut response = response;
            let mut text = Vec::new();
            while let Ok(Some(chunk)) = response.chunk().await {
                if text.len() + chunk.len() > 1024 * 1024 {
                    break;
                }
                text.extend_from_slice(&chunk);
            }
            (Some(qoder_chat::failure(status, &String::from_utf8_lossy(&text))), None)
        };
        if let Some(f) = fail {
            // 任务令牌被拒：换一次票再发
            if f.lapsed && !relogged {
                relogged = true;
                forget_token(&entry.connection_id);
                auth = match chat_token(&deps, entry, test_base, true).await {
                    Ok(v) => v,
                    Err(e) => return unavailable(e),
                };
                continue;
            }
            // 只回了一个没名字的原生工具调用：换新 request_id 再问一次
            if f.nameless && !renamed {
                renamed = true;
                continue;
            }
            return failed(&f);
        }
        let Some((reader, first)) = opened else { break };
        qoder_chat::task_answered(&task.key);
        let meter = Meter {
            deps: deps.clone(),
            entry: entry.clone(),
            started,
            id: format!("chatcmpl-{}", qoder_cosy::random_uuid().replace('-', "")),
            model: info.key.clone(),
        };
        return if chat["stream"] == true { stream(reader, first, meter) } else { complete(reader, first, meter).await };
    }
    error_response(StatusCode::BAD_GATEWAY, "Qoder 没有返回回答")
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use axum::{
        http::HeaderMap,
        routing::{get, post},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn session(id: &str, expires: &str) -> Session {
        Session {
            token: format!("{id}-device"),
            refresh_token: Some(format!("{id}-refresh")),
            expires_at: Some(expires.into()),
            user_id: Some(id.into()),
            account: Some(format!("{id}@example.com")),
            machine_id: Some(format!("{id}-machine")),
        }
    }

    fn insert(deps: &Arc<ProxyDeps>, id: &str, edition: &str, session: Session) -> PoolEntry {
        let sealed = provider_browser_auth::seal(id, &Credential::Qoder { edition: edition.into(), session }).unwrap();
        deps.db.lock().unwrap().execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug,status,serve_enabled) VALUES(?1,'qoder','auth',?1,?2,1,'source-'||?1,'connected',1)", rusqlite::params![id, sealed]).unwrap();
        crate::connections::refresh_account_fp(&deps.db.lock().unwrap(), id).unwrap();
        PoolEntry {
            connection_id: id.into(),
            slug: format!("source-{id}"),
            name: id.into(),
            platform: "qoder".into(),
            kind: "auth".into(),
            protocol: "openai".into(),
            base_url: None,
            secret: None,
            account_fp: deps.db.lock().unwrap().query_row("SELECT account_fp FROM connections WHERE id=?1", [id], |r| r.get(0)).unwrap(),
            model: None,
            status: "connected".into(),
        }
    }

    async fn serve(app: axum::Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        (base, tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }))
    }

    fn listing() -> String {
        json!({"app":[{"key":"auto","enable":true},{"key":"gmodel","display_name":"GLM-5.3","source":"system","enable":true,"api_key":"secret-key",
            "thinking_config":{"enabled":{"efforts":{"high":{"is_default":true}}}}},{"key":"off","enable":false}],
            "chat":[{"key":"dmodel","enable":true,"source":"system"}]}).to_string()
    }

    fn sse_body(chunks: &[Value]) -> String {
        let mut out = String::new();
        for c in chunks {
            out.push_str(&format!("data: {}\n\n", json!({"statusCodeValue":200,"body":c.to_string()})));
        }
        out.push_str(&format!("data: {}\n\n", json!({"statusCodeValue":200,"body":"[DONE]"})));
        out
    }

    #[tokio::test]
    async fn queued_request_disabled_blocks_qoder_auth_network() {
        let deps = crate::proxy::test_support::bare_deps();
        let entry = insert(&deps, "queued", "cn", session("queued", "2099-01-01T00:00:00Z"));
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let (base, server) = serve(axum::Router::new().fallback(move || {
            let observed = observed.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                StatusCode::UNAUTHORIZED
            }
        }))
        .await;
        let held = crate::gateway::subscription::lock("queued").await;
        let queued_deps = deps.clone();
        let queued_entry = entry.clone();
        let queued = tokio::spawn(async move {
            forward_at(queued_deps, &queued_entry, CallerProtocol::Chat, Bytes::from(json!({"model":"source-queued/gmodel","messages":[{"role":"user","content":"x"}]}).to_string()), Some(&base)).await
        });
        tokio::task::yield_now().await;
        assert!(!queued.is_finished());
        deps.db.lock().unwrap().execute("UPDATE connections SET serve_enabled=0 WHERE id='queued'", []).unwrap();
        drop(held);
        assert_eq!(queued.await.unwrap().status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "关闭来源后，不得发起授权请求");
        server.abort();
    }

    #[tokio::test]
    async fn device_refresh_job_exchange_listing_and_chat_for_selected_account() {
        let deps = crate::proxy::test_support::bare_deps();
        let entry = insert(&deps, "a", "global", session("a", "2000-01-01T00:00:00Z"));
        insert(&deps, "b", "global", session("b", "2099-01-01T00:00:00Z"));
        let before_b = bound(&deps.db.lock().unwrap(), "b").unwrap().0;
        let chats = Arc::new(AtomicUsize::new(0));
        let seen = chats.clone();
        let app = axum::Router::new()
            .route("/api/v1/deviceToken/refresh", post(|body: String| async move {
                assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["refresh_token"], "a-refresh");
                json!({"token":"a-new","refresh_token":"a-refresh-new","expires_in":3600}).to_string()
            }))
            .route("/api/v1/userinfo", get(|headers: HeaderMap| async move {
                assert!(matches!(headers["authorization"].to_str().unwrap(), "Bearer a-new" | "Bearer a-job"));
                json!({"id":"a"}).to_string()
            }))
            .route("/api/v1/me/jobToken", post(|headers: HeaderMap, body: String| async move {
                assert_eq!(headers["authorization"], "Bearer a-new");
                assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["clientId"], CLIENT_ID);
                json!({"token":"a-job","expires_in":3_600_000}).to_string()
            }))
            .route(MODELS_PATH, get(|headers: HeaderMap| async move {
                assert!(headers["authorization"].to_str().unwrap().starts_with("Bearer COSY."));
                assert_eq!(headers["cosy-machineid"], "a-machine");
                assert_eq!(headers["cosy-user"], "a");
                listing()
            }))
            .route(CHAT_PATH, post(move |headers: HeaderMap, body: String| {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(headers["x-model-key"], "gmodel");
                    assert_eq!(headers["accept"], "text/event-stream");
                    assert!(!body.starts_with('{'), "请求体必须编码");
                    sse_body(&[
                        json!({"choices":[{"delta":{"reasoning_content":"想"}}]}),
                        json!({"choices":[{"delta":{"content":"答案"}}]}),
                        json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
                        json!({"usage":{"prompt_tokens":3,"completion_tokens":2}}),
                    ])
                }
            }));
        let (base, server) = serve(app).await;
        assert_eq!(models_at(&deps, &entry, Some(&base)).await.unwrap(), ["dmodel", "gmodel"]);
        {
            let conn = deps.db.lock().unwrap();
            assert_eq!(bound(&conn, "a").unwrap().2.token, "a-new");
            assert_eq!(bound(&conn, "b").unwrap().0, before_b, "其他账号不受影响");
            let text: String = conn.query_row("SELECT models_json FROM connections WHERE id='a'", [], |r| r.get(0)).unwrap();
            assert!(!text.contains("secret-key") && !text.contains("a-job"));
            assert_eq!(cached_models(&conn, "a").unwrap(), ["dmodel", "gmodel"]);
            assert_eq!(cached_models(&conn, "b").unwrap(), GLOBAL.models.iter().map(|m| m.to_string()).collect::<Vec<_>>());
        }
        let response = forward_at(deps.clone(), &entry, CallerProtocol::Chat, Bytes::from(json!({"model":"source-a/gmodel","messages":[{"role":"user","content":"hi"}]}).to_string()), Some(&base)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let value: Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!(value["choices"][0]["message"]["content"], "答案");
        assert_eq!(value["choices"][0]["message"]["reasoning_content"], "想");
        assert_eq!(value["usage"]["total_tokens"], 5);
        assert_eq!(crate::db::attributed_request_count(&deps.db.lock().unwrap(), "a").unwrap(), 1);
        assert_eq!(crate::db::attributed_request_count(&deps.db.lock().unwrap(), "b").unwrap(), 0);
        let response = forward_at(deps.clone(), &entry, CallerProtocol::Chat, Bytes::from(json!({"model":"source-a/gmodel","stream":true,"messages":[{"role":"user","content":"hi"}]}).to_string()), Some(&base)).await;
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        let text = String::from_utf8(axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap();
        assert!(text.contains("\"content\":\"答案\"") && text.contains("\"finish_reason\":\"stop\"") && text.ends_with("data: [DONE]\n\n"));
        assert_eq!(chats.load(Ordering::SeqCst), 2, "任务令牌缓存后不重复换票");
        let response = forward_at(deps.clone(), &entry, CallerProtocol::Chat, Bytes::from(json!({"model":"source-a/missing","messages":[{"role":"user","content":"hi"}]}).to_string()), Some(&base)).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        server.abort();
    }

    #[tokio::test]
    async fn cn_falls_back_to_device_token_and_maps_queue() {
        let deps = crate::proxy::test_support::bare_deps();
        let entry = insert(&deps, "c", "cn", session("c", "2099-01-01T00:00:00Z"));
        let app = axum::Router::new()
            .route("/api/v1/userinfo", get(|| async { json!({"id":"c"}).to_string() }))
            .route("/api/v1/me/jobToken", post(|| async { (StatusCode::FORBIDDEN, "{}") }))
            .route(MODELS_PATH, get(|| async { listing() }))
            .route(CHAT_PATH, post(|| async {
                (StatusCode::FORBIDDEN, json!({"code":"10605","message":"{\"isQueued\":true,\"retryAfterSeconds\":9}"}).to_string())
            }));
        let (base, server) = serve(app).await;
        let chat = chat_token(&deps, &entry, Some(&base), false).await.unwrap();
        assert_eq!(chat.token, "c-device", "国内版换票被拒时用设备令牌签名");
        let response = forward_at(deps.clone(), &entry, CallerProtocol::Chat, Bytes::from(json!({"model":"source-c/gmodel","messages":[{"role":"user","content":"hi"}]}).to_string()), Some(&base)).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()["retry-after"], "9");
        server.abort();
    }

    #[tokio::test]
    async fn identity_mismatch_and_switched_login_fail_closed() {
        let deps = crate::proxy::test_support::bare_deps();
        let entry = insert(&deps, "d", "global", session("d", "2099-01-01T00:00:00Z"));
        let (base, server) = serve(axum::Router::new()
            .route("/api/v1/me/jobToken", post(|| async { json!({"token":"d-job"}).to_string() }))
            .route("/api/v1/userinfo", get(|| async { json!({"id":"other"}).to_string() })))
        .await;
        forget_token("d");
        assert!(chat_token(&deps, &entry, Some(&base), true).await.is_err());
        server.abort();
        let switched = deps.clone();
        let (base, server) = serve(axum::Router::new()
            .route("/api/v1/userinfo", get(|| async { json!({"id":"d"}).to_string() }))
            .route("/api/v1/me/jobToken", post(move || {
                let switched = switched.clone();
                async move {
                    let conn = switched.db.lock().unwrap();
                    let (old, edition, mut session) = bound(&conn, "d").unwrap();
                    session.token = "d-switched".into();
                    provider_browser_auth::refresh_bound(&conn, "d", "qoder", &old, &Credential::Qoder { edition, session }).unwrap();
                    json!({"token":"stale-job"}).to_string()
                }
            })))
        .await;
        let error = chat_token(&deps, &entry, Some(&base), true).await.err().unwrap();
        assert!(error.contains("变化") && !error.contains("stale-job"));
        server.abort();
        let legacy = Session { machine_id: None, ..session("e", "2099-01-01T00:00:00Z") };
        assert_eq!(machine_id("e", &legacy), machine_id("e", &legacy), "旧授权派生稳定的设备标识");
        assert!(site("ai").is_err());
        assert!(check_identity(&json!({"uid":"b"}), "a").is_err());
    }
}
