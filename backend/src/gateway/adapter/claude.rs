//! Claude 订阅适配器：驱动真实本机 `claude` 二进制（执行文档 §6）。
//!
//! 依据：Anthropic 把第三方系统提示词判为第三方流量，注入 OAuth 的转发有风控风险——
//! 所以不经 HTTP 转发，而是 spawn 真实的 `claude -p --output-format stream-json`，
//! 用它自己的登录完成生成；Jolu 不读取、不刷新、不持有订阅令牌。
//!
//! 范围：
//! - 纯文本对话直通：messages 文本拼成 prompt，claude 输出转回 Anthropic 响应
//! - 工具调用显式拒绝：MCP 桥（`gateway/bridge.rs`）已就绪，闭环交互待真机验证后开放
//! - 凭证检查：只确认本机存在 Claude 登录（`creds::read_local_pair`），令牌内容不进本模块
//! - **会话延续**：`claude -p` 是非交互一次性进程，无法常驻复用；延续靠 `--resume <session_id>`
//!   ——首轮拿到 `system.init` 的 session_id 后按会话键记住，后续轮次带上它恢复同一场会话
//!   （prompt cache 因此可命中）。会话表 LRU 上限 [`SESSION_CAP`]，空闲超 [`SESSION_IDLE`] 即丢弃。

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
}

/// 从 Anthropic 请求体提取对话文本与模型名。
/// 任何非文本内容（图片 / 工具块）都会导致显式拒绝——诚实的边界好过静默丢内容。
fn parse_turn(body: &serde_json::Value) -> Result<Turn, String> {
    let model = body.get("model").and_then(|m| m.as_str()).unwrap_or_default().to_string();
    if body.get("tools").and_then(|t| t.as_array()).is_some_and(|t| !t.is_empty()) {
        return Err("当前版本 Claude 订阅经网关仅支持纯文本对话；工具调用桥接已在网关就绪，待真机验证后开放".into());
    }
    let messages = body
        .get("messages")
        .and_then(|m| m.as_array())
        .ok_or("缺少 messages 字段")?;
    let mut lines: Vec<String> = Vec::new();
    for message in messages {
        let role = message.get("role").and_then(|r| r.as_str()).unwrap_or("user");
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
    Ok(Turn { prompt: lines.join("\n\n"), model })
}

/// stream-json 事件流聚合出的结果。
#[derive(Default, Debug)]
struct TurnOutput {
    session_id: Option<String>,
    texts: Vec<String>,
    final_text: Option<String>,
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
            output.final_text = value.get("result").and_then(|r| r.as_str()).map(str::to_string);
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
    if !skip_auth_check {
        if crate::validation::isolated() {
            return crate::gateway::wire::error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "隔离环境不读取本机 Claude 登录",
            );
        }
        if crate::creds::read_local_pair("claude", "auth").is_none() {
            return crate::gateway::wire::error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "本机没有可用的 Claude 登录：请先在 Claude Code 完成 login，再回到网关重试",
            );
        }
    }

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
    let resume = deps
        .claude_sessions
        .as_ref()
        .and_then(|table| table.lock().ok().and_then(|mut guard| guard.resume_for(session)));

    let args = program_args(command.clone(), &model);
    let raw = match run_turn(&deps, &args, &turn.prompt, resume.as_deref()).await {
        Ok(raw) => raw,
        Err(failure) => {
            // resume 失败（会话过期 / daemon 侧丢失）：丢弃记忆，本轮当新会话重来一次
            if resume.is_some() {
                if let Some(table) = deps.claude_sessions.as_ref() {
                    if let Ok(mut guard) = table.lock() {
                        guard.forget(session);
                    }
                }
                match run_turn(&deps, &args, &turn.prompt, None).await {
                    Ok(raw) => raw,
                    Err(failure) => return failure.into_response(),
                }
            } else {
                return failure.into_response();
            }
        }
    };

    let mut output = TurnOutput::default();
    for line in raw.lines() {
        fold_event(&mut output, line);
    }
    let text = output
        .final_text
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| output.texts.join("

"));
    if text.trim().is_empty() {
        return crate::gateway::wire::error_response(
            StatusCode::BAD_GATEWAY,
            "Claude 订阅未产生文本输出；请确认本机登录有效后重试",
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
    if let Some(dedup_key) = output.session_id.as_deref() {
        crate::gateway::adapter::settle_subscription_timing(
            &deps,
            &entry.connection_id,
            dedup_key,
            0,
            elapsed_ms,
            200,
        );
    }
    let response = anthropic_response(&model, &text, "end_turn");
    let json = serde_json::to_string(&response).unwrap_or_default();
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .header("x-jolu-gateway", entry.slug.as_str())
        .body(Body::from(json))
        .unwrap_or_else(|error| crate::gateway::wire::error_response(StatusCode::BAD_GATEWAY, &format!("响应构造失败：{error}")))
}

/// 组装 claude 参数；本轮若可 resume，追加 `--resume <id>` 恢复同一场会话。
fn program_args(command: Option<Vec<String>>, model: &str) -> Vec<String> {
    match command {
        Some(injected) if !injected.is_empty() => injected,
        _ => vec![
            "-p".into(),
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

    let attempt = tokio::time::timeout(TURN_TIMEOUT, async {
        let mut child = match tokio::process::Command::new(&program)
            .args(&rest)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .current_dir(std::env::temp_dir())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                return Err(format!("无法启动 claude 二进制（{error}）：请确认 Claude Code 已安装并在 PATH"));
            }
        };
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
        let mut stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => return Err("claude 进程未提供 stdout".into()),
        };
        let mut raw = String::new();
        if stdout.read_to_string(&mut raw).await.is_err() {
            return Err("读取 claude 输出失败".into());
        }
        let status = child.wait().await;
        // 非零退出且无任何输出 = 明确失败（resume 失效时 claude 会这样报错）
        if !raw.trim().is_empty() {
            return Ok(raw);
        }
        match status {
            Ok(status) if !status.success() => Err(format!("claude 退出码 {}", status.code().unwrap_or(-1))),
            _ => Ok(raw),
        }
    })
    .await;

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
            table.lock().unwrap().bind("user:stale", "ses-gone");
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
