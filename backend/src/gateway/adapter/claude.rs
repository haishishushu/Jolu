//! Claude 订阅适配器：驱动真实本机 `claude` 二进制（执行文档 §6）。
//!
//! 使用选中连接的完整授权创建一次性配置，认证与 endpoint 覆盖环境不继承。
//! 当前提供 Anthropic Messages 非流式文本桥接；流式、图片和工具调用明确拒绝。
//! 每次提供完整对话；生产请求不恢复原工具会话、不改原工具登录。
//! 测试可注入假程序，验证隔离身份、环境与兼容行为。

use std::sync::Arc;

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;

use crate::gateway::route::PoolEntry;
use crate::proxy::ProxyDeps;

/// 单次生成的墙钟上限：claude 长任务也要有个界，超时明确报错而不是挂死调用方。
const TURN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// 生成请求的规范化中间表示（本适配器只用得到文本）。
struct Turn {
    prompt: String,
    model: String,
    system: Option<String>,
    max_tokens: Option<u64>,
}

/// 从 Anthropic 请求体提取对话文本与模型名。
/// 任何非文本内容（图片 / 工具块）都会导致显式拒绝——诚实的边界好过静默丢内容。
fn parse_turn(body: &serde_json::Value) -> Result<Turn, String> {
    for field in ["temperature", "top_p", "top_k", "stop_sequences", "thinking", "output_config", "tool_choice"] {
        if body.get(field).is_some_and(|v| !v.is_null()) {
            return Err(format!("Claude 订阅文本桥接暂不支持 {field}，请移除该选项"));
        }
    }
    if body.get("stream").and_then(|v| v.as_bool()) == Some(true) {
        return Err("Claude 订阅当前仅支持非流式文本，请设置 stream=false".into());
    }
    let model = body.get("model").and_then(|m| m.as_str()).unwrap_or_default().to_string();
    if body.get("tools").and_then(|t| t.as_array()).is_some_and(|t| !t.is_empty()) {
        return Err("当前版本 Claude 订阅经网关仅支持纯文本对话；工具调用桥接已在网关就绪，待真机验证后开放".into());
    }
    let messages = body
        .get("messages")
        .and_then(|m| m.as_array())
        .ok_or("缺少 messages 字段")?;
    let mut lines: Vec<String> = Vec::new();
    let mut system_prompt = None;
    if let Some(system) = body.get("system") {
        let text = match system {
            serde_json::Value::String(text) => text.clone(),
            serde_json::Value::Array(blocks) => blocks.iter().map(|b| {
                if b.get("type").and_then(|v| v.as_str()) != Some("text") { return Err("system 仅支持文本".to_string()); }
                b.get("text").and_then(|v| v.as_str()).map(str::to_owned).ok_or_else(|| "system 文本无效".to_string())
            }).collect::<Result<Vec<_>, _>>()?.join("\n"),
            _ => return Err("system 仅支持文本".into()),
        };
        system_prompt = Some(text);
    }
    for message in messages {
        let role = message.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        if !matches!(role, "user" | "assistant") { return Err("Messages 只支持 user / assistant 角色，系统提示词请放入 system".into()); }
        match message.get("content") {
            Some(serde_json::Value::String(text)) => {
                lines.push(format!("{role}: {text}"));
            }
            Some(serde_json::Value::Array(blocks)) => {
                for block in blocks {
                    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or_default();
                    match block_type {
                        "text" => {
                            let text = block.get("text").and_then(|t| t.as_str()).unwrap_or_default();
                            lines.push(format!("{role}: {text}"));
                        }
                        "tool_use" | "tool_result" | "image" => {
                            return Err("当前版本 Claude 订阅经网关仅支持纯文本对话，消息含工具或图片内容".into());
                        }
                        _ => return Err(format!("不支持的消息块类型「{block_type}」")),
                    }
                }
            }
            _ => return Err("messages 里存在无法解析的内容".into()),
        }
    }
    if lines.is_empty() {
        return Err("对话内容为空".into());
    }
    let max_tokens = match body.get("max_tokens") {
        None => None,
        Some(value) => Some(value.as_u64().filter(|n| *n > 0 && *n <= u32::MAX as u64).ok_or("max_tokens 必须是正整数")?),
    };
    Ok(Turn { prompt: lines.join("\n\n"), model, system: system_prompt, max_tokens })
}

/// stream-json 事件流聚合出的结果。
#[derive(Default, Debug)]
struct TurnOutput {
    session_id: Option<String>,
    texts: Vec<String>,
    final_text: Option<String>,
    failed: bool,
    usage: Option<serde_json::Value>,
}

/// 网关会话键 → claude 的 session_id（用于 `--resume`）。
/// `claude -p` 每轮都是一个新进程；延续同一场会话靠 resume 而不是复用进程。
pub(crate) struct ClaudeSessions {
    entries: std::collections::HashMap<String, (String, i64)>,
}

/// 会话表上限：超过后淘汰最久未用的条目。
pub(crate) const SESSION_CAP: usize = 200;
/// 会话空闲上限：超过则丢弃（下一轮当作新会话，不再 attempt resume）。
pub(crate) const SESSION_IDLE: std::time::Duration = std::time::Duration::from_secs(10 * 60);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl ClaudeSessions {
    pub(crate) fn new() -> Self {
        Self { entries: std::collections::HashMap::new() }
    }

    /// 取可 resume 的 session_id：空闲超时的不返回（并顺手清掉）。
    pub(crate) fn resume_for(&mut self, session: &str) -> Option<String> {
        let (id, used) = self.entries.get(session)?.clone();
        if now_ms() - used > SESSION_IDLE.as_millis() as i64 {
            self.entries.remove(session);
            return None;
        }
        Some(id)
    }

    /// 记下本轮拿到的 session_id（心跳续期）。超上限时淘汰最久未用。
    pub(crate) fn bind(&mut self, session: &str, claude_session: &str) {
        if self.entries.len() >= SESSION_CAP && !self.entries.contains_key(session) {
            if let Some(oldest) = self.entries.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| k.clone()) {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(session.to_string(), (claude_session.to_string(), now_ms()));
    }

    /// 会话失效（resume 失败或进程崩溃）时丢弃，下一轮重新建立。
    pub(crate) fn forget(&mut self, session: &str) {
        self.entries.remove(session);
    }

    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// 聚合 claude 的 stdout JSONL：init 拿 session_id，assistant 累积文本，result 是最终答案。
fn fold_event(output: &mut TurnOutput, line: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else { return };
    match value.get("type").and_then(|t| t.as_str()) {
        Some("system") => {
            if output.session_id.is_none() {
                output.session_id = value.get("session_id").and_then(|s| s.as_str()).map(str::to_string);
            }
        }
        Some("assistant") => {
            if let Some(blocks) = value.pointer("/message/content").and_then(|c| c.as_array()) {
                for block in blocks {
                    if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            if !text.trim().is_empty() {
                                output.texts.push(text.to_string());
                            }
                        }
                    }
                }
            }
        }
        Some("result") => {
            output.failed = value.get("is_error").and_then(|v| v.as_bool()) == Some(true)
                || value.get("subtype").and_then(|v| v.as_str()).is_some_and(|v| v.starts_with("error"));
            output.final_text = value.get("result").and_then(|r| r.as_str()).map(str::to_string);
            output.usage = value.get("usage").filter(|v| v.is_object()).cloned();
        }
        _ => {}
    }
}

/// Anthropic 非流式响应。usage 无真实数据时不编造（省略字段）。
fn anthropic_response(model: &str, text: &str, stop_reason: &str) -> serde_json::Value {
    serde_json::json!({
        "id": format!("msg_jolu_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)),
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [{"type": "text", "text": text}],
        "stop_reason": stop_reason,
        "stop_sequence": serde_json::Value::Null,
    })
}

/// 入口：检查登录 → 解析 → spawn 真实 claude → 聚合 stream-json → Anthropic 响应。
/// `command` 与 `skip_auth_check` 仅供测试注入假二进制；生产走 [`forward`]。
pub(crate) async fn forward(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    session: &str,
    body: axum::body::Bytes,
) -> Response {
    forward_at(deps, entry, session, body, None, false).await
}

pub(crate) async fn forward_at(
    deps: Arc<ProxyDeps>,
    entry: &PoolEntry,
    session: &str,
    body: axum::body::Bytes,
    command: Option<Vec<String>>,
    skip_auth_check: bool,
) -> Response {
    // 生产请求绝不读取当前 CLI 登录，测试只有显式注入假程序才能跳过授权。
    let isolated = !(cfg!(test) && skip_auth_check && command.is_some());
    let _account_guard = if isolated { Some(crate::gateway::subscription::lock(&entry.connection_id).await) } else { None };
    let bound = if isolated {
        let loaded = deps.db.lock().map_err(|_| "网关状态暂不可用".to_string())
            .and_then(|conn| crate::gateway::subscription::load(&conn, &entry.connection_id, "claude"));
        match loaded {
            Ok(value) => Some(value),
            Err(reason) => return crate::gateway::wire::error_response(StatusCode::UNAUTHORIZED, &reason),
        }
    } else { None };
    let profile = match bound.as_ref().map(prepare_profile).transpose() {
        Ok(profile) => profile,
        Err(reason) => return crate::gateway::wire::error_response(StatusCode::SERVICE_UNAVAILABLE, &reason),
    };
    let scoped_session = format!("{}:{}:{session}", entry.connection_id.len(), entry.connection_id);
    let session = scoped_session.as_str();
    let started = std::time::Instant::now();
    let parsed: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => {
            return crate::gateway::wire::error_response(StatusCode::BAD_REQUEST, &format!("请求体不是有效 JSON：{error}"));
        }
    };
    let turn = match parse_turn(&parsed) {
        Ok(turn) => turn,
        Err(reason) => return crate::gateway::wire::error_response(StatusCode::BAD_REQUEST, &reason),
    };
    // 模型名取 slug 后半段（claude/claude-sonnet-5 → claude-sonnet-5）
    let model = crate::gateway::route::split_model_id(&turn.model).map(|(_, rest)| rest.to_string()).unwrap_or(turn.model);

    // 会话延续：本轮若能 resume 上一轮的 session_id，就复用同一场对话（prompt cache 可命中）
    let resume = (!isolated).then_some(()).and_then(|_| deps
        .claude_sessions
        .as_ref()
        .and_then(|table| table.lock().ok().and_then(|mut guard| guard.resume_for(session))));

    let mut args = program_args(command.clone(), &model);
    if let Some(system) = &turn.system { args.extend(["--system-prompt".into(), system.clone()]); }
    let raw_result = match run_turn(&deps, &args, &turn.prompt, resume.as_deref(), profile.as_ref(), turn.max_tokens).await {
        Ok(raw) => Ok(raw),
        Err(failure) => {
            if resume.is_some() {
                if let Some(table) = deps.claude_sessions.as_ref() {
                    if let Ok(mut guard) = table.lock() { guard.forget(session); }
                }
                run_turn(&deps, &args, &turn.prompt, None, profile.as_ref(), turn.max_tokens).await
            } else { Err(failure) }
        }
    };
    // CLI 可能先刷新授权再生成失败；失败路径也必须保全该账号的新授权。

    if let (Some(bound), Some(profile)) = (&bound, &profile) {
        if let Err(reason) = save_profile_refresh(&deps, entry, bound, profile) {
            return crate::gateway::wire::error_response(StatusCode::CONFLICT, &reason);
        }
    }
    let raw = match raw_result { Ok(raw) => raw, Err(failure) => return failure.into_response() };
    let mut output = TurnOutput::default();
    for line in raw.lines() {
        fold_event(&mut output, line);
    }
    if output.failed { return crate::gateway::wire::error_response(StatusCode::BAD_GATEWAY, "Claude 未能完成本次生成，请检查所选账号授权与额度"); }
    let text = output
        .final_text
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| output.texts.join("

"));
    if text.trim().is_empty() {
        return crate::gateway::wire::error_response(
            StatusCode::BAD_GATEWAY,
            "Claude 订阅未产生文本输出；请检查所选账号授权后重试",
        );
    }
    // 记住本轮的 claude session_id，下一轮同一会话键就能 resume
    if let (Some(table), Some(session_id)) = (deps.claude_sessions.as_ref(), output.session_id.as_deref()) {
        if let Ok(mut guard) = table.lock() {
            guard.bind(session, session_id);
        }
    }
    let elapsed_ms = started.elapsed().as_millis() as i64;
    // 归账（执行文档 §9）：订阅路径没有 HTTP 响应流，用 claude 的 session_id 作去重键。
    // 首字时刻对"一次性进程返回"没有意义——传 0 表示未测到，不编造数值。
    if let Some(dedup_key) = output.session_id.as_deref().filter(|_| !isolated) {
        crate::gateway::adapter::settle_subscription_timing(
            &deps,
            &entry.connection_id,
            dedup_key,
            0,
            elapsed_ms,
            200,
        );
    }
    let mut response = anthropic_response(&model, &text, "end_turn");
    if let Some(usage) = output.usage { response["usage"] = usage; }
    if isolated { crate::gateway::subscription::record_usage(&deps, entry, &response, elapsed_ms); }
    let json = serde_json::to_string(&response).unwrap_or_default();
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .header("x-jolu-gateway", entry.slug.as_str())
        .body(Body::from(json))
        .unwrap_or_else(|error| crate::gateway::wire::error_response(StatusCode::BAD_GATEWAY, &format!("响应构造失败：{error}")))
}

/// 为所选账号准备一次性配置，生命周期结束后清理。
fn prepare_profile(bound: &crate::gateway::subscription::BoundAuth) -> Result<crate::cli_browser_auth::Profile, String> {
    let profile = crate::cli_browser_auth::Profile::new()?;
    let credentials = serde_json::json!({"claudeAiOauth": bound.bundle["claudeAiOauth"]});
    let settings = serde_json::json!({"oauthAccount": bound.bundle["oauthAccount"], "hasCompletedOnboarding": true});
    std::fs::write(profile.path.join(".credentials.json"), credentials.to_string()).map_err(|_| "无法准备隔离授权")?;
    std::fs::write(profile.path.join(".claude.json"), settings.to_string()).map_err(|_| "无法准备隔离账号")?;
    Ok(profile)
}

fn save_profile_refresh(deps: &Arc<ProxyDeps>, entry: &PoolEntry, bound: &crate::gateway::subscription::BoundAuth, profile: &crate::cli_browser_auth::Profile) -> Result<(), String> {
    let credentials: serde_json::Value = serde_json::from_slice(&std::fs::read(profile.path.join(".credentials.json")).map_err(|_| "无法读取隔离授权结果")?).map_err(|_| "隔离授权结果无效")?;
    let account: serde_json::Value = serde_json::from_slice(&std::fs::read(profile.path.join(".claude.json")).map_err(|_| "无法读取隔离账号结果")?).map_err(|_| "隔离账号结果无效")?;
    let mut next = bound.bundle.clone();
    next["claudeAiOauth"] = credentials["claudeAiOauth"].clone();
    next["oauthAccount"] = account["oauthAccount"].clone();
    if next == bound.bundle { return Ok(()); }
    let conn = deps.db.lock().map_err(|_| "无法保存刷新后的订阅授权")?;
    crate::gateway::subscription::save_refresh(&conn, &entry.connection_id, "claude", bound, &next)
}

fn program_args(command: Option<Vec<String>>, model: &str) -> Vec<String> {
    match command {
        Some(injected) if !injected.is_empty() => injected,
        _ => vec![
            crate::session_threads::find_executable("claude")
                .filter(|p| !cfg!(windows) || p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")))
                .or_else(|| crate::cli_browser_auth::npm_prefix("claude").map(|p| p.join("node_modules/@anthropic-ai/claude-code/bin/claude.exe")).filter(|p| p.is_file()))
                .map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "claude".into()),
            "-p".into(),
            "--verbose".into(),
            "--tools".into(), "".into(),
            "--strict-mcp-config".into(),
            "--mcp-config".into(), "{\"mcpServers\":{}}".into(),
            "--setting-sources".into(), "user".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--model".into(),
            model.into(),
        ],
    }
}

/// 一轮生成的失败描述（用于组装对调用方的错误响应）。
struct TurnFailure {
    status: StatusCode,
    message: String,
}

impl TurnFailure {
    fn into_response(self) -> Response {
        crate::gateway::wire::error_response(self.status, &self.message)
    }
}

/// 跑一轮 claude：写 prompt、读 stream-json。返回原始 stdout 或明确的失败原因。
/// `resume` 为 Some 时带上 `--resume`（恢复上一轮的会话）。
async fn run_turn(
    _deps: &Arc<ProxyDeps>,
    args: &[String],
    prompt: &str,
    resume: Option<&str>,
    profile: Option<&crate::cli_browser_auth::Profile>,
    max_tokens: Option<u64>,
) -> Result<String, TurnFailure> {
    let mut full_args: Vec<String> = args.to_vec();
    if let Some(session_id) = resume {
        full_args.push("--resume".into());
        full_args.push(session_id.to_string());
    }
    let (program, rest) = match full_args.split_first() {
        Some((program, rest)) => (program.clone(), rest.to_vec()),
        None => {
            return Err(TurnFailure {
                status: StatusCode::BAD_GATEWAY,
                message: "claude 启动参数为空".into(),
            });
        }
    };

        let mut base = std::process::Command::new(&program);
        if let Some(profile) = profile { crate::cli_browser_auth::configure_environment(&mut base, profile); }
        else { base.current_dir(std::env::temp_dir()); }
        if let Some(max_tokens) = max_tokens { base.env("CLAUDE_CODE_MAX_OUTPUT_TOKENS", max_tokens.to_string()); }
        let mut command = tokio::process::Command::from(base);
        let mut child = match command.args(&rest).kill_on_drop(true)
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn()
        {
            Ok(child) => child,
            Err(error) => {
                return Err(TurnFailure { status: StatusCode::SERVICE_UNAVAILABLE, message: format!("无法启动 Claude Code（{error}），请先安装官方客户端") });
            }
        };
    let attempt = tokio::time::timeout(TURN_TIMEOUT, async {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        {
            let mut stdin = match child.stdin.take() {
                Some(stdin) => stdin,
                None => return Err("claude 进程未提供 stdin".into()),
            };
            if stdin.write_all(prompt.as_bytes()).await.is_err() {
                return Err("向 claude 写入输入失败".into());
            }
        }
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => return Err("claude 进程未提供 stdout".into()),
        };
        let mut raw = String::new();
        if stdout.take(16 * 1024 * 1024 + 1).read_to_string(&mut raw).await.is_err() {
            return Err("读取 claude 输出失败".into());
        }
        if raw.len() > 16 * 1024 * 1024 { return Err("Claude 响应超过大小限制".into()); }
        let status = child.wait().await;
        // 非零退出且无任何输出 = 明确失败（resume 失效时 claude 会这样报错）
        if status.as_ref().is_ok_and(|status| status.success()) && !raw.trim().is_empty() {
            return Ok(raw);
        }
        match status {
            Ok(status) if !status.success() => Err(format!("claude 退出码 {}", status.code().unwrap_or(-1))),
            _ => Ok(raw),
        }
    })
    .await;

    if !matches!(&attempt, Ok(Ok(_))) { let _ = child.kill().await; let _ = child.wait().await; }
    match attempt {
        Ok(Ok(raw)) => Ok(raw),
        Ok(Err(reason)) => Err(TurnFailure {
            status: StatusCode::BAD_GATEWAY,
            message: format!("Claude 订阅生成失败：{reason}"),
        }),
        Err(_) => Err(TurnFailure {
            status: StatusCode::GATEWAY_TIMEOUT,
            message: "Claude 订阅生成超时（10 分钟）；请稍后重试".into(),
        }),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[tokio::test]
    async fn selected_claude_accounts_use_separate_profiles_and_clean_up() {
        let deps = deps();
        {
            let conn = deps.db.lock().unwrap();
            for id in ["a", "b"] {
                crate::gateway::subscription::tests::insert(&conn, id, "claude", id, &crate::gateway::subscription::tests::claude_bundle(id, id));
            }
        }
        let script = r#"const fs=require('fs'),p=require('path');const dir=process.env.CLAUDE_CONFIG_DIR;
const v=JSON.parse(fs.readFileSync(p.join(dir,'.credentials.json')));
for(const key of ['ANTHROPIC_API_KEY','ANTHROPIC_AUTH_TOKEN','ANTHROPIC_BASE_URL','CLAUDE_CODE_OAUTH_TOKEN'])if(process.env[key])process.exit(2);
console.log(JSON.stringify({type:'result',result:JSON.stringify({account:v.claudeAiOauth.accessToken,dir}),usage:{input_tokens:3,output_tokens:2}}));"#;
        for id in ["a", "b"] {
            let mut selected = entry(); selected.connection_id = id.into();
            let response = forward_at(deps.clone(), &selected, "same-session", axum::body::Bytes::from_static(br#"{"model":"source/model","messages":[{"role":"user","content":"hi"}]}"#), Some(vec!["node".into(), "-e".into(), script.into()]), false).await;
            assert_eq!(response.status(), StatusCode::OK);
            let body: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), 1024*1024).await.unwrap()).unwrap();
            let result: serde_json::Value = serde_json::from_str(body["content"][0]["text"].as_str().unwrap()).unwrap();
            assert_eq!(result["account"], id);
            assert!(!std::path::Path::new(result["dir"].as_str().unwrap()).exists());
            assert_eq!(body["usage"]["input_tokens"], 3);
        }
        {
            let conn = deps.db.lock().unwrap();
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM requests WHERE source='gateway-subscription' AND connection_id IN ('a','b') AND input_tokens=3 AND input_known=1 AND total_known=0", [], |r| r.get(0)).unwrap();
            assert_eq!(count, 2, "真实提供的输入要归账，缺失总量不能冒充零");
        }
        let rotate_then_fail = r#"const fs=require('fs'),p=require('path').join(process.env.CLAUDE_CONFIG_DIR,'.credentials.json');const v=JSON.parse(fs.readFileSync(p));v.claudeAiOauth.accessToken='a-new';fs.writeFileSync(p,JSON.stringify(v));process.exit(1);"#;
        let mut selected = entry(); selected.connection_id = "a".into();
        let failure = forward_at(deps.clone(), &selected, "same-session", axum::body::Bytes::from_static(br#"{"model":"source/model","messages":[{"role":"user","content":"hi"}]}"#), Some(vec!["node".into(), "-e".into(), rotate_then_fail.into()]), false).await;
        assert_eq!(failure.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(crate::gateway::subscription::load(&deps.db.lock().unwrap(), "a", "claude").unwrap().secret, "a-new", "生成失败也不能丢失已轮换的授权");
        let mut missing = entry(); missing.connection_id = "missing".into();
        let response = forward_at(deps, &missing, "same-session", axum::body::Bytes::new(), Some(vec!["must-not-run".into()]), false).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn production_command_and_protocol_are_explicit() {
        let args = program_args(None, "claude-model");
        assert_ne!(args[0], "-p", "可执行文件不能是参数");
        assert!(args.iter().any(|a| a == "--verbose"));
        assert!(args.windows(2).any(|a| a[0] == "--tools" && a[1].is_empty()));
        assert!(parse_turn(&serde_json::json!({"stream":true,"messages":[{"role":"user","content":"hi"}]})).is_err(), "不能把非流式响应冒充流式");
        let turn = parse_turn(&serde_json::json!({"system":"answer briefly", "messages":[{"role":"user","content":"hi"}]})).unwrap();
        assert_eq!(turn.system.as_deref(), Some("answer briefly"), "system 必须传给系统提示词参数");
    }

    fn entry() -> PoolEntry {
        PoolEntry {
            connection_id: "c-claude".into(),
            slug: "claude".into(),
            name: "Claude 订阅".into(),
            platform: "claude".into(),
            kind: "auth".into(),
            protocol: "anthropic".into(),
            base_url: None,
            secret: None,
            account_fp: None,
            model: None,
            status: "connected".into(),
        }
    }

    fn deps() -> Arc<ProxyDeps> {
        crate::proxy::test_support::bare_deps()
    }

    #[test]
    fn parse_turn_rejects_tools_and_non_text_blocks() {
        let with_tools = serde_json::json!({"model":"claude/claude-sonnet-5","messages":[],"tools":[{"name":"bash"}]});
        assert!(parse_turn(&with_tools).is_err());
        let with_image = serde_json::json!({"model":"m","messages":[
            {"role":"user","content":[{"type":"image","source":{}}]}
        ]});
        assert!(parse_turn(&with_image).is_err());
        let ok = serde_json::json!({"model":"claude/claude-sonnet-5","messages":[
            {"role":"user","content":"你好"},
            {"role":"assistant","content":"请讲"},
            {"role":"user","content":[{"type":"text","text":"继续"}]}
        ]});
        let turn = parse_turn(&ok).unwrap();
        assert!(turn.prompt.contains("user: 你好"));
        assert!(turn.prompt.contains("assistant: 请讲"));
        assert!(turn.prompt.contains("user: 继续"));
    }

    #[test]
    fn fold_event_collects_session_text_and_result() {
        let mut output = TurnOutput::default();
        fold_event(&mut output, r#"{"type":"system","subtype":"init","session_id":"ses-1"}"#);
        fold_event(&mut output, r#"{"type":"assistant","message":{"content":[{"type":"text","text":"部分"}]}}"#);
        fold_event(&mut output, r#"{"type":"result","result":"最终答案"}"#);
        assert_eq!(output.session_id.as_deref(), Some("ses-1"));
        assert_eq!(output.texts, vec!["部分"]);
        assert_eq!(output.final_text.as_deref(), Some("最终答案"));
    }

    /// 端到端：假 claude 二进制（node）输出 stream-json → 网关聚合为 Anthropic 响应。
    #[tokio::test]
    async fn claude_text_turn_end_to_end_with_fake_binary() {
        // node 在本仓库开发环境必在（前端与测试都依赖）；假二进制回显固定 stream-json
        let script = r#"
const chunks = [
  JSON.stringify({type:"system",subtype:"init",session_id:"ses-test"}),
  JSON.stringify({type:"assistant",message:{content:[{type:"text",text:"思考中"}]}}),
  JSON.stringify({type:"result",result:"来自假 claude 的回答"}),
];
process.stdout.write(chunks.join("\n") + "\n");
"#;
        let command = vec!["node".to_string(), "-e".to_string(), script.to_string()];
        let body = axum::body::Bytes::from(
            r#"{"model":"claude/claude-sonnet-5","messages":[{"role":"user","content":"你好"}]}"#,
        );
        let response = forward_at(deps(), &entry(), "user:s1", body, Some(command), true).await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["content"][0]["text"], "来自假 claude 的回答");
        assert_eq!(value["model"], "claude-sonnet-5", "模型名要去掉 slug 前缀");
        assert_eq!(value["stop_reason"], "end_turn");
        assert!(value["usage"].is_null(), "无真实用量时不编造 usage");
    }

    /// 假二进制立即退出且无输出 → 明确错误（不静默降级到别的连接）。
    #[tokio::test]
    async fn claude_failure_reports_clear_error() {
        let script = "process.exit(1);";
        let command = vec!["node".to_string(), "-e".to_string(), script.to_string()];
        let body = axum::body::Bytes::from(
            r#"{"model":"claude/claude-sonnet-5","messages":[{"role":"user","content":"你好"}]}"#,
        );
        let response = forward_at(deps(), &entry(), "user:s1", body, Some(command), true).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let body = String::from_utf8_lossy(&bytes);
        assert!(
            body.contains("退出码") || body.contains("未产生文本输出"),
            "失败要给明确原因（退出码或空输出）：{body}"
        );
    }

    #[test]
    fn session_table_binds_resumes_and_expires_idle() {
        let mut table = ClaudeSessions::new();
        assert!(table.resume_for("user:s1").is_none(), "未绑定的会话没有可 resume 的 id");
        table.bind("user:s1", "claude-sess-1");
        assert_eq!(table.resume_for("user:s1").as_deref(), Some("claude-sess-1"));
        // 过期（人为把时间推前）后不再 resume
        table.entries.insert("stale".into(), ("old".into(), now_ms() - SESSION_IDLE.as_millis() as i64 - 1));
        assert!(table.resume_for("stale").is_none(), "空闲超时的会话要丢弃");
        assert!(!table.entries.contains_key("stale"), "过期条目应被清掉");
        // forget 主动丢弃
        table.forget("user:s1");
        assert!(table.resume_for("user:s1").is_none());
    }

    #[test]
    fn session_table_evicts_oldest_beyond_cap() {
        let mut table = ClaudeSessions::new();
        for i in 0..SESSION_CAP {
            table.bind(&format!("s{i}"), &format!("c{i}"));
        }
        table.bind("newcomer", "c-new");
        assert!(table.len() <= SESSION_CAP);
        assert!(table.resume_for("newcomer").is_some(), "新会话必须保留");
    }

    /// resume 端到端：第二轮带上 --resume 并复用同一 session_id。
    #[tokio::test]
    async fn second_turn_resumes_previous_claude_session() {
        let dir = std::env::temp_dir();
        let stamp = format!("{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos());
        let script_path = dir.join(format!("jolu-claude-resume-{stamp}.js"));
        let log_path = dir.join(format!("jolu-claude-argv-{stamp}.log"));
        let script = [
            "const fs = require('fs');",
            "const args = process.argv.slice(2);",
            "if (process.env.JOLU_TEST_ARGV_LOG) fs.appendFileSync(process.env.JOLU_TEST_ARGV_LOG, JSON.stringify(args) + String.fromCharCode(10));",
            "const resumed = args.includes('--resume');",
            "console.log(JSON.stringify({type:'system',subtype:'init',session_id: resumed ? 'ses-resumed' : 'ses-first'}));",
            "console.log(JSON.stringify({type:'result',result: resumed ? '第二轮' : '第一轮'}));",
        ].join("
");
        std::fs::write(&script_path, script).unwrap();
        std::env::set_var("JOLU_TEST_ARGV_LOG", &log_path);
        let command = vec!["node".to_string(), script_path.display().to_string()];

        let deps = deps();
        let body1 = axum::body::Bytes::from(r#"{"model":"claude/claude-sonnet-5","messages":[{"role":"user","content":"第一"}]}"#);
        let first = forward_at(deps.clone(), &entry(), "user:resume-test", body1, Some(command.clone()), true).await;
        assert_eq!(first.status(), StatusCode::OK);

        let body2 = axum::body::Bytes::from(r#"{"model":"claude/claude-sonnet-5","messages":[{"role":"user","content":"第二"}]}"#);
        let second = forward_at(deps.clone(), &entry(), "user:resume-test", body2, Some(command), true).await;
        assert_eq!(second.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(second.into_body(), 1024 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["content"][0]["text"], "第二轮", "第二轮应走 resume 分支");

        let log = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = log.trim().lines().collect();
        assert_eq!(lines.len(), 2, "{log}");
        assert!(!lines[0].contains("--resume"), "首轮不该 resume：{}", lines[0]);
        assert!(
            lines[1].contains("--resume") && lines[1].contains("ses-first"),
            "第二轮应 resume 首轮的 session_id：{}", lines[1]
        );
        std::env::remove_var("JOLU_TEST_ARGV_LOG");
        let _ = std::fs::remove_file(&log_path);
        let _ = std::fs::remove_file(&script_path);
    }

    /// resume 失败（会话已失效）→ 丢弃记忆并以新会话重试一次。
    #[tokio::test]
    async fn failed_resume_retries_as_new_session() {
        let dir = std::env::temp_dir();
        let stamp = format!("{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos());
        let script_path = dir.join(format!("jolu-claude-stale-{stamp}.js"));
        let script = [
            "const args = process.argv.slice(2);",
            "if (args.includes('--resume')) { process.exit(1); }",
            "console.log(JSON.stringify({type:'system',subtype:'init',session_id:'ses-new'}));",
            "console.log(JSON.stringify({type:'result',result:'重试成功'}));",
        ].join("
");
        std::fs::write(&script_path, script).unwrap();
        let command = vec!["node".to_string(), script_path.display().to_string()];

        let deps = deps();
        if let Some(table) = deps.claude_sessions.as_ref() {
            table.lock().unwrap().bind(&format!("{}:{}:user:stale", entry().connection_id.len(), entry().connection_id), "ses-gone");
        }
        let body = axum::body::Bytes::from(r#"{"model":"claude/claude-sonnet-5","messages":[{"role":"user","content":"hi"}]}"#);
        let response = forward_at(deps.clone(), &entry(), "user:stale", body, Some(command), true).await;
        assert_eq!(response.status(), StatusCode::OK, "resume 失败要能自动重试为新会话");
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["content"][0]["text"], "重试成功");
        let _ = std::fs::remove_file(&script_path);
    }
}
