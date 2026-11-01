//! Codex 订阅适配器：用本机 `~/.codex/auth.json` 的登录调 ChatGPT 后端。
//!
//! 依据执行文档 §7：
//! - 凭证**现场重读**，不缓存到进程；`last_refresh` 超过 [`REFRESH_AFTER`] 才刷新，
//!   刷新后回写 auth.json——回写前比对 mtime，外部（Codex CLI 自己）已改则以外部为准。
//! - ChatGPT 后端只接受流式：非流式请求转流式采集，聚合 `response.completed` 携带的
//!   完整 response 对象返回。
//! - 后端拒绝的参数（temperature 等）剥掉，被剥清单放 `x-jolu-dropped-params` 响应头
//!   （诊断信息不进日志正文）。
//! - 401（刷新后仍失败）→ 连接 `invalid`；429 → 冷却（复用 direct 的信号处理）。
//!
//! auth.json 结构按 Codex CLI（codex-rs）公开格式：`tokens.{access_token,refresh_token,account_id}` + `last_refresh`。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;

use crate::gateway::route::PoolEntry;
use crate::gateway::wire::CallerProtocol;
use crate::proxy::ProxyDeps;

const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
/// Codex CLI（codex-rs）公开源码中的固定 client_id；设备流与刷新共用。
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const BACKEND_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
/// Codex CLI 每天刷新一次；跟随该节奏（提前量 4 小时，避免长会话中途过期）。
const REFRESH_AFTER_SECS: i64 = 20 * 3600;

/// ChatGPT 后端会拒绝的请求参数（Responses 语义之外的传统采样参数）。
const DROPPED_PARAMS: &[&str] = &[
    "temperature", "top_p", "user", "logprobs", "logit_bias", "n",
    "presence_penalty", "frequency_penalty",
];

pub(crate) struct CodexAuth {
    pub access_token: String,
    pub account_id: Option<String>,
    refresh_token: Option<String>,
}

/// 从 auth.json 的 JSON 值提取凭证（只取字段，不校验 JWT 本身——上游会校验）。
fn extract_tokens(value: &serde_json::Value) -> Option<CodexAuth> {
    let tokens = value.get("tokens")?;
    Some(CodexAuth {
        access_token: tokens.get("access_token")?.as_str()?.trim().to_string(),
        account_id: tokens.get("account_id").and_then(|v| v.as_str()).map(str::to_string),
        refresh_token: tokens.get("refresh_token").and_then(|v| v.as_str()).map(str::to_string),
    })
}

/// `last_refresh`（RFC3339）距今是否已超过刷新间隔；缺失或解析失败按需要刷新处理。
fn needs_refresh(value: &serde_json::Value, now: chrono::DateTime<chrono::Utc>) -> bool {
    match value.get("last_refresh").and_then(|v| v.as_str()) {
        Some(text) => match chrono::DateTime::parse_from_rfc3339(text) {
            Ok(at) => now.signed_duration_since(at.with_timezone(&chrono::Utc)).num_seconds() > REFRESH_AFTER_SECS,
            Err(_) => true,
        },
        None => true,
    }
}

/// 用 refresh_token 换新令牌。失败返回 None（调用方沿用旧令牌继续，由上游 401 做最终判决）。
async fn refresh_at(client: &reqwest::Client, oauth_url: &str, refresh_token: &str) -> Option<serde_json::Value> {
    let response = client
        .post(oauth_url)
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": CODEX_CLIENT_ID,
        }))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let fresh = response.json::<serde_json::Value>().await.ok()?;
    fresh.get("access_token")?.as_str()?;
    Some(fresh)
}

/// 回写 auth.json：更新 tokens 与 last_refresh。文件 mtime 在读取后被外部改过则放弃
/// 回写、返回重读后的外部值（「外部意图优先」，与 proxy_config.rs 还原纪律一致）。
fn write_back(auth_path: &Path, original: serde_json::Value, fresh: &serde_json::Value, observed: std::time::SystemTime) -> Option<serde_json::Value> {
    let Ok(metadata_now) = std::fs::metadata(auth_path) else { return None };
    let modified = metadata_now.modified().ok()?;
    if modified != observed {
        // 外部已改：以磁盘上的新内容为准（可能 Codex CLI 自己刚刷新过）
        return std::fs::read_to_string(auth_path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok());
    }
    let mut next = original;
    let tokens = next.get_mut("tokens")?;
    if let Some(object) = tokens.as_object_mut() {
        for key in ["access_token", "refresh_token", "id_token", "account_id"] {
            if let Some(value) = fresh.get(key) {
                object.insert(key.into(), value.clone());
            }
        }
    }
    next["last_refresh"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());
    let bytes = serde_json::to_vec_pretty(&next).ok()?;
    let temp = auth_path.with_extension("json.jolu-tmp");
    std::fs::write(&temp, &bytes).ok()?;
    std::fs::rename(&temp, auth_path).ok()?;
    Some(next)
}

/// 端点可注入的凭证加载（测试指向 mock 服务器）。
/// `auth_dir = None` 用真实 `~/.codex`（隔离验收模式不读）。返回 None = 本机没有可用的 Codex 登录。
pub(crate) async fn load_auth_at(client: &reqwest::Client, auth_dir: Option<&Path>, oauth_url: &str) -> Option<CodexAuth> {
    let dir: PathBuf = match auth_dir {
        Some(dir) => dir.to_path_buf(),
        None => {
            if crate::validation::isolated() {
                return None;
            }
            crate::creds::codex_dir()?
        }
    };
    let auth_path = dir.join("auth.json");
    let text = std::fs::read_to_string(&auth_path).ok()?;
    let mut value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let observed = std::fs::metadata(&auth_path).and_then(|m| m.modified()).ok();
    let mut auth = extract_tokens(&value)?;
    if needs_refresh(&value, chrono::Utc::now()) {
        if let Some(refresh_token) = auth.refresh_token.clone() {
            if let Some(fresh) = refresh_at(client, oauth_url, &refresh_token).await {
                if let Some(observed) = observed {
                    if let Some(merged) = write_back(&auth_path, value.clone(), &fresh, observed) {
                        value = merged;
                    }
                }
                if let Some(refreshed) = extract_tokens(&value) {
                    auth = refreshed;
                } else {
                    auth.access_token = fresh.get("access_token").and_then(|v| v.as_str()).unwrap_or(&auth.access_token).to_string();
                }
            }
        }
    }
    Some(auth)
}

/// 剥后端拒绝的参数并强制流式。返回 (改写后 body, 被剥参数名)。
pub(crate) fn sanitize_request(body: &serde_json::Value) -> (serde_json::Value, Vec<String>) {
    let mut next = body.clone();
    let mut dropped = Vec::new();
    if let Some(object) = next.as_object_mut() {
        for name in DROPPED_PARAMS {
            if object.remove(*name).is_some() {
                dropped.push((*name).to_string());
            }
        }
        object.insert("stream".into(), serde_json::Value::Bool(true));
    }
    (next, dropped)
}

/// Chat 请求 → Responses 请求的基础转换（文本往返；工具调用显式报不支持）。
/// system 消息并进 instructions，其余消息逐条映射为 input 项。
pub(crate) fn chat_to_responses(body: &serde_json::Value) -> Result<serde_json::Value, String> {
    let messages = body
        .get("messages")
        .and_then(|v| v.as_array())
        .ok_or("缺少 messages 字段")?;
    if messages.iter().any(|m| m.get("tool_calls").is_some()) || body.get("tools").is_some() {
        return Err("Chat 工具调用暂不支持经 Codex 订阅转发，请改用 Responses 协议的工具格式".into());
    }
    let mut instructions: Vec<String> = Vec::new();
    let mut input: Vec<serde_json::Value> = Vec::new();
    for message in messages {
        let role = message.get("role").and_then(|v| v.as_str()).unwrap_or("user");
        let content = message
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        match role {
            "system" | "developer" => instructions.push(content),
            "assistant" => input.push(serde_json::json!({"role": "assistant", "content": content})),
            _ => input.push(serde_json::json!({"role": "user", "content": content})),
        }
    }
    let mut request = serde_json::json!({"model": body.get("model").cloned().unwrap_or_default(), "input": input});
    if !instructions.is_empty() {
        request["instructions"] = serde_json::Value::String(instructions.join("\n\n"));
    }
    Ok(request)
}

/// Responses 完整对象 → Chat completions 响应（文本部分）。
fn responses_to_chat(response: &serde_json::Value) -> serde_json::Value {
    let text = response
        .pointer("/output")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let content = item.get("content")?.as_array()?;
                    content
                        .iter()
                        .filter(|c| c.get("type").and_then(|t| t.as_str()) == Some("output_text"))
                        .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                        .collect::<String>()
                        .into()
                })
                .collect::<Vec<String>>()
                .join("")
        })
        .unwrap_or_default();
    let mut chat = serde_json::json!({
        "id": response.get("id").cloned().unwrap_or_default(),
        "object": "chat.completion",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": text}, "finish_reason": "stop"}],
    });
    if let Some(usage) = response.get("usage") {
        chat["usage"] = usage.clone();
    }
    chat
}

/// 入口：读取凭证 → 请求改写 → 转发 ChatGPT 后端 → 流式透传或聚合成非流式。
pub(crate) async fn forward(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    caller: CallerProtocol,
    body: axum::body::Bytes,
) -> Response {
    forward_at(deps, entry, caller, body, BACKEND_RESPONSES_URL, None).await
}

/// 端点与凭证目录可注入的转发（测试指向 mock 服务器与临时 auth.json）。
pub(crate) async fn forward_at(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    caller: CallerProtocol,
    body: axum::body::Bytes,
    backend_url: &str,
    auth_dir: Option<&Path>,
) -> Response {
    let Some(auth) = load_auth_at(&deps.client, auth_dir, OAUTH_TOKEN_URL).await else {
        return crate::gateway::wire::error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "本机没有可用的 Codex 登录：请先在 Codex CLI 完成 login，再回到网关重试",
        );
    };
    let caller_wants_stream = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
        .unwrap_or(false);

    let parsed: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => {
            return crate::gateway::wire::error_response(
                StatusCode::BAD_REQUEST,
                &format!("请求体不是有效 JSON：{error}"),
            );
        }
    };
    let (request_body, dropped) = match caller {
        // Responses 入口：直接在原请求上剥参数并强制流式
        CallerProtocol::Responses => sanitize_request(&parsed),
        // Chat 入口：协议转换会丢弃它不认识的字段——被剥清单必须在**原始请求**上统计，
        // 否则 temperature 之类会在转换中静默消失，诊断头就漏报了
        CallerProtocol::Chat => {
            let (_, dropped) = sanitize_request(&parsed);
            match chat_to_responses(&parsed) {
                Ok(converted) => (sanitize_request(&converted).0, dropped),
                Err(reason) => return crate::gateway::wire::error_response(StatusCode::BAD_REQUEST, &reason),
            }
        }
        CallerProtocol::Anthropic => {
            return crate::gateway::wire::error_response(
                StatusCode::BAD_REQUEST,
                "Codex 订阅只服务 OpenAI 协议请求",
            );
        }
    };

    let mut request = deps
        .client
        .post(backend_url)
        .bearer_auth(&auth.access_token)
        .header("Accept", "text/event-stream")
        .json(&request_body);
    if let Some(account) = auth.account_id.as_deref() {
        request = request.header("chatgpt-account-id", account);
    }

    let upstream = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            return crate::gateway::wire::error_response(
                StatusCode::BAD_GATEWAY,
                &format!("Codex 订阅上游不可达：{error}"),
            );
        }
    };
    let status = upstream.status();
    crate::gateway::adapter::record_upstream_signal(&deps, entry, &upstream);

    // 非成功状态：原样透传（状态码 + 响应体）。调用方的退避/重授权逻辑依赖真实状态码，
    // 也不能把错误体当 SSE 去聚合。
    if !status.is_success() {
        let mut builder = Response::builder().status(status).header("x-jolu-gateway", entry.slug.as_str());
        if !dropped.is_empty() {
            builder = builder.header("x-jolu-dropped-params", dropped.join(","));
        }
        return builder
            .body(Body::from_stream(upstream.bytes_stream()))
            .unwrap_or_else(|error| crate::gateway::wire::error_response(StatusCode::BAD_GATEWAY, &format!("响应构造失败：{error}")));
    }

    if caller_wants_stream {
        // 流式：SSE 原样透传
        let mut out = HeaderMap::new();
        out.insert("content-type", axum::http::HeaderValue::from_static("text/event-stream"));
        out.insert("cache-control", axum::http::HeaderValue::from_static("no-cache"));
        let mut builder = Response::builder().status(status);
        if let Some(slot) = builder.headers_mut() {
            *slot = out;
        }
        if !dropped.is_empty() {
            builder = builder.header("x-jolu-dropped-params", dropped.join(","));
        }
        builder = builder.header("x-jolu-gateway", entry.slug.as_str());
        return builder
            .body(Body::from_stream(upstream.bytes_stream()))
            .unwrap_or_else(|error| {
                crate::gateway::wire::error_response(StatusCode::BAD_GATEWAY, &format!("响应构造失败：{error}"))
            });
    }

    // 非流式：采集 SSE 到 response.completed，取事件携带的完整 response 对象
    let mut collected: Vec<u8> = Vec::new();
    let mut stream = upstream.bytes_stream();
    use futures_util::StreamExt;
    let mut completed: Option<serde_json::Value> = None;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(bytes) => bytes,
            Err(error) => {
                return crate::gateway::wire::error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("Codex 订阅响应中断：{error}"),
                );
            }
        };
        collected.extend_from_slice(&chunk);
        // SSE 事件以空行分隔；只在缓冲里找 completed 行，避免全量解析
        let text = String::from_utf8_lossy(&collected);
        for event in text.split("\n\n") {
            let Some(data) = event.strip_prefix("data: ") else { continue };
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
                if value.get("type").and_then(|t| t.as_str()) == Some("response.completed") {
                    completed = value.get("response").cloned();
                }
            }
        }
        if completed.is_some() {
            break;
        }
    }
    let Some(response_value) = completed else {
        return crate::gateway::wire::error_response(
            StatusCode::BAD_GATEWAY,
            "Codex 订阅流式响应未包含完成事件；请重试或改用流式请求",
        );
    };
    let out_body = match caller {
        CallerProtocol::Chat => responses_to_chat(&response_value),
        _ => response_value,
    };
    let json = serde_json::to_string(&out_body).unwrap_or_default();
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .header("x-jolu-gateway", entry.slug.as_str());
    if !dropped.is_empty() {
        builder = builder.header("x-jolu-dropped-params", dropped.join(","));
    }
    builder
        .body(Body::from(json))
        .unwrap_or_else(|error| crate::gateway::wire::error_response(StatusCode::BAD_GATEWAY, &format!("响应构造失败：{error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn temp_auth_dir(last_refresh: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "jolu-codex-auth-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("auth.json");
        std::fs::write(
            &path,
            format!(r#"{{"tokens":{{"access_token":"tok-old","refresh_token":"r1","account_id":"acc-1"}},"last_refresh":"{last_refresh}"}}"#),
        )
        .unwrap();
        (dir, path)
    }

    /// 读满 HTTP 请求（含 body）再返回原文——Windows 上未读完就 close 会 RST。
    async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
        let mut raw = Vec::new();
        let mut chunk = [0u8; 8192];
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
        String::from_utf8_lossy(&raw).to_string()
    }

    async fn write_response(socket: &mut tokio::net::TcpStream, status_line: &str, headers: &[(&str, &str)], body: &str) {
        let mut head = format!("{status_line}\r\nContent-Length: {}\r\n", body.len());
        for (k, v) in headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("Connection: close\r\n\r\n");
        socket.write_all(head.as_bytes()).await.unwrap();
        socket.write_all(body.as_bytes()).await.unwrap();
        socket.flush().await.unwrap();
    }

    #[test]
    fn needs_refresh_requires_it_only_after_interval() {
        let fresh = serde_json::json!({"last_refresh": chrono::Utc::now().to_rfc3339()});
        let stale = serde_json::json!({"last_refresh": (chrono::Utc::now() - chrono::Duration::hours(25)).to_rfc3339()});
        let missing = serde_json::json!({});
        let bad = serde_json::json!({"last_refresh": "not-a-date"});
        assert!(!needs_refresh(&fresh, chrono::Utc::now()));
        assert!(needs_refresh(&stale, chrono::Utc::now()));
        assert!(needs_refresh(&missing, chrono::Utc::now()));
        assert!(needs_refresh(&bad, chrono::Utc::now()));
    }

    #[test]
    fn sanitize_drops_rejected_params_and_forces_stream() {
        let body = serde_json::json!({"model":"gpt-5.5","input":[],"stream":false,"temperature":0.7,"top_p":0.9,"max_output_tokens":512});
        let (clean, dropped) = sanitize_request(&body);
        assert_eq!(clean["stream"], true);
        assert!(clean.get("temperature").is_none());
        assert!(clean.get("top_p").is_none());
        assert_eq!(clean["max_output_tokens"], 512, "后端支持的参数要保留");
        assert_eq!(dropped, vec!["temperature", "top_p"]);
    }

    #[test]
    fn chat_to_responses_maps_messages_and_rejects_tools() {
        let body = serde_json::json!({"model":"x","messages":[
            {"role":"system","content":"你是助手"},
            {"role":"user","content":"你好"},
            {"role":"assistant","content":"请说"},
            {"role":"user","content":"继续"}
        ]});
        let converted = chat_to_responses(&body).unwrap();
        assert_eq!(converted["instructions"], "你是助手");
        assert_eq!(converted["input"].as_array().unwrap().len(), 3);
        assert_eq!(converted["input"][0]["role"], "user");
        assert_eq!(converted["input"][1]["role"], "assistant");

        let with_tools = serde_json::json!({"model":"x","messages":[],"tools":[{"type":"function"}]});
        assert!(chat_to_responses(&with_tools).is_err());
    }

    #[test]
    fn responses_to_chat_extracts_text_and_usage() {
        let response = serde_json::json!({
            "id": "resp_1",
            "output": [
                {"type":"reasoning","content":[]},
                {"type":"message","content":[{"type":"output_text","text":"你好"},{"type":"refusal","text":"x"}]}
            ],
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });
        let chat = responses_to_chat(&response);
        assert_eq!(chat["choices"][0]["message"]["content"], "你好");
        assert_eq!(chat["usage"]["output_tokens"], 5);
        assert_eq!(chat["object"], "chat.completion");
    }

    /// 刷新成功：新令牌回写 auth.json。
    #[tokio::test]
    async fn refresh_writes_back_new_token() {
        let (dir, path) = temp_auth_dir(&(chrono::Utc::now() - chrono::Duration::hours(30)).to_rfc3339());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await.to_lowercase();
            assert!(request.contains("refresh_token"), "{request}");
            write_response(&mut socket, "HTTP/1.1 200 OK", &[("Content-Type", "application/json")],
                r#"{"access_token":"tok-new","refresh_token":"r2","account_id":"acc-1"}"#).await;
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let auth = load_auth_at(&client, Some(&dir), &format!("http://{addr}/oauth/token")).await.unwrap();
        server.await.unwrap();
        assert_eq!(auth.access_token, "tok-new");
        assert_eq!(auth.account_id.as_deref(), Some("acc-1"));
        let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["tokens"]["access_token"], "tok-new");
        assert_eq!(written["tokens"]["refresh_token"], "r2");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// mtime 冲突：读取后外部改了文件 → 放弃回写，用外部值。
    #[tokio::test]
    async fn writeback_conflict_keeps_external_value() {
        let (dir, path) = temp_auth_dir(&(chrono::Utc::now() - chrono::Duration::hours(30)).to_rfc3339());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut socket).await;
            // 外部（Codex CLI）在网关读取之后、回写之前改了文件
            std::fs::write(&path, r#"{"tokens":{"access_token":"tok-external","refresh_token":"r9"},"last_refresh":"fresh"}"#).unwrap();
            write_response(&mut socket, "HTTP/1.1 200 OK", &[("Content-Type", "application/json")],
                r#"{"access_token":"tok-ours"}"#).await;
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let auth = load_auth_at(&client, Some(&dir), &format!("http://{addr}/oauth/token")).await.unwrap();
        server.await.unwrap();
        assert_eq!(auth.access_token, "tok-external", "外部意图优先：改用外部值而不是覆盖");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 刷新失败：沿用旧令牌继续（由上游 401 做最终判决）。
    #[tokio::test]
    async fn failed_refresh_falls_back_to_old_token() {
        let (dir, _path) = temp_auth_dir(&(chrono::Utc::now() - chrono::Duration::hours(30)).to_rfc3339());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut socket).await;
            write_response(&mut socket, "HTTP/1.1 400 Bad Request", &[("Content-Type", "application/json")], "{}").await;
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let auth = load_auth_at(&client, Some(&dir), &format!("http://{addr}/oauth/token")).await.unwrap();
        server.await.unwrap();
        assert_eq!(auth.access_token, "tok-old");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 端到端：非流式 Chat 请求 → 后端收到流式请求与正确凭证 → SSE 聚合成 chat.completion。
    #[tokio::test]
    async fn forward_chat_nonstream_aggregates_sse() {
        let (dir, _path) = temp_auth_dir(&chrono::Utc::now().to_rfc3339());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await.to_lowercase();
            assert!(request.contains("bearer tok-old"), "{request}");
            assert!(request.contains("chatgpt-account-id: acc-1"), "{request}");
            assert!(request.contains("\"stream\":true"), "后端只收流式，请求必须已改写：{request}");
            assert!(!request.contains("temperature"), "被拒参数不能发给后端：{request}");
            let sse = "data: {\"type\":\"response.created\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_9\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"你好\"}]}],\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}}\n\n";
            let head = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", sse.len());
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(sse.as_bytes()).await.unwrap();
            socket.flush().await.unwrap();
        });

        let deps = crate::proxy::test_support::bare_deps();
        let entry = PoolEntry {
            connection_id: "c-codex".into(),
            slug: "codex".into(),
            platform: "codex".into(),
            kind: "auth".into(),
            protocol: "openai".into(),
            base_url: None,
            secret: None,
            model: None,
            status: "connected".into(),
        };
        let body = axum::body::Bytes::from(
            r#"{"model":"codex/gpt-5.5","stream":false,"temperature":0.5,"messages":[{"role":"user","content":"你好"}]}"#,
        );
        let response = forward_at(deps.clone(), &entry, CallerProtocol::Chat, body, &format!("http://{addr}/backend/codex/responses"), Some(&dir)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("x-jolu-dropped-params").unwrap(), "temperature");
        let text = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024).await.unwrap();
        let chat: serde_json::Value = serde_json::from_slice(&text).unwrap();
        assert_eq!(chat["choices"][0]["message"]["content"], "你好");
        assert_eq!(chat["usage"]["output_tokens"], 2);
        server.await.unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 后端 429 → 连接进入冷却；401 → 连接标记 invalid。
    #[tokio::test]
    async fn upstream_signals_update_cooldown_and_status() {
        for (status_line, code) in [("HTTP/1.1 429 Too Many Requests", 429u16), ("HTTP/1.1 401 Unauthorized", 401)] {
            let (dir, _path) = temp_auth_dir(&chrono::Utc::now().to_rfc3339());
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let _ = read_request(&mut socket).await;
                write_response(&mut socket, status_line, &[("Retry-After", "7")], "{}").await;
            });
            let deps = crate::proxy::test_support::bare_deps();
            deps.db.lock().unwrap().execute(
                "INSERT INTO connections(id,platform,kind,name,created_ms) VALUES('c-codex','codex','auth','Codex 订阅',1)",
                [],
            ).unwrap();
            let entry = PoolEntry {
                connection_id: "c-codex".into(),
                slug: "codex".into(),
                platform: "codex".into(),
                kind: "auth".into(),
                protocol: "openai".into(),
                base_url: None,
                secret: None,
                model: None,
                status: "connected".into(),
            };
            let body = axum::body::Bytes::from(r#"{"model":"codex/gpt-5.5","stream":false,"messages":[]}"#);
            let response = forward_at(deps.clone(), &entry, CallerProtocol::Chat, body, &format!("http://{addr}/b"), Some(&dir)).await;
            assert_eq!(response.status().as_u16(), code, "上游状态码要原样回传");
            server.await.unwrap();
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
            if code == 429 {
                let cooldowns = deps.cooldowns.as_ref().unwrap().lock().unwrap();
                assert!(cooldowns.remaining_ms("c-codex", now) > 0, "429 后必须进入冷却");
            } else {
                let status: String = deps.db.lock().unwrap()
                    .query_row("SELECT status FROM connections WHERE id='c-codex'", [], |r| r.get(0)).unwrap();
                assert_eq!(status, "invalid", "401 要标记 invalid");
            }
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }
}
