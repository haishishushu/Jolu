//! 会话线程页（Jolu.pen 模块 D，画板 40–57）：本机会话列表、只读消息记录与「在原工具继续」。
//!
//! - 会话来自统计库里带 session_id 的请求；标题与生命周期来自 sessions 表；
//! - 源文件位置来自采集器的 scan_state（path ↔ parser_session_id），项目目录从源文件里读 `cwd`；
//! - 消息只读解析源文件，不改写、不删除；来源没保留正文时如实返回「仅用量」，不拼造对话；
//! - 继续会话只对有可靠命令行入口的来源开放（Claude Code `claude --resume`、Codex `codex resume`）。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use rusqlite::{types::Value, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

pub const UNATTRIBUTED: &str = crate::usage_stats::UNATTRIBUTED;

/// 单条消息正文与工具输出的长度上限，避免超长日志拖垮界面
const TEXT_LIMIT: usize = 20_000;
const TOOL_OUTPUT_LIMIT: usize = 1_200;
/// 正文搜索跳过过大的源文件
const SEARCH_FILE_LIMIT: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SessionFilter {
    #[serde(default)]
    pub platform: Option<String>,
    /// 连接 id 或 [`UNATTRIBUTED`]
    #[serde(default)]
    pub connection: Option<String>,
    /// 项目目录（cwd 原值）；空字符串表示「未识别项目」
    #[serde(default)]
    pub project: Option<String>,
    /// 只看最后活跃时间不早于此刻（UTC 毫秒）
    #[serde(default)]
    pub since: Option<i64>,
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionItem {
    pub platform: String,
    pub session_id: String,
    pub title: Option<String>,
    /// 项目目录；源文件不可读或没写 cwd 时为 None（归入「未识别项目」）
    pub cwd: Option<String>,
    pub first_ts: i64,
    pub last_ts: i64,
    pub requests: i64,
    pub fresh_input: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    pub output: Option<i64>,
    /// 只有全部请求归到同一连接时才有值；含未归属请求时为 None
    pub connection_id: Option<String>,
    /// running / done / failed：来自来源生命周期事件；recent：只有写入时间；usage_only：没有可读源文件
    pub status: String,
    pub has_source: bool,
}

#[derive(Debug, Serialize)]
pub struct SessionList {
    pub sessions: Vec<SessionItem>,
    /// 筛选内去重来源文件数
    pub files: i64,
}

/// 源文件定位：会话 → 主记录文件（不含子代理文件）
pub(crate) fn source_files(conn: &Connection) -> rusqlite::Result<HashMap<(String, String), PathBuf>> {
    let mut stmt = conn.prepare("SELECT path, parser_session_id FROM scan_state")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?;
    let mut out: HashMap<(String, String), PathBuf> = HashMap::new();
    for row in rows {
        let (path, parsed) = row?;
        let lower = path.replace('\\', "/").to_ascii_lowercase();
        let platform = if lower.contains("/.claude/") {
            "claude"
        } else if lower.contains("/sessions/") && lower.contains("rollout-") {
            "codex"
        } else {
            continue;
        };
        if lower.contains("/subagents/") {
            continue;
        }
        let file = PathBuf::from(&path);
        let id = parsed.or_else(|| (platform == "claude").then(|| file.file_stem()?.to_str().map(String::from)).flatten());
        if let Some(id) = id {
            out.entry((platform.into(), id)).or_insert(file);
        }
    }
    Ok(out)
}

/// 项目目录缓存：按文件修改时间失效
static CWD_CACHE: LazyLock<Mutex<HashMap<PathBuf, (u64, Option<String>)>>> = LazyLock::new(Default::default);

fn mtime(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// 读源文件开头找 cwd：Claude 每行顶层带 cwd，Codex 在 session_meta.payload.cwd
pub(crate) fn read_cwd(path: &Path) -> Option<String> {
    let stamp = mtime(path)?;
    if let Some((cached, cwd)) = CWD_CACHE.lock().ok()?.get(path) {
        if *cached == stamp {
            return cwd.clone();
        }
    }
    let file = std::fs::File::open(path).ok()?;
    let cwd = BufReader::new(file)
        .lines()
        .take(400)
        .map_while(Result::ok)
        .find_map(|line| {
            let v: Json = serde_json::from_str(&line).ok()?;
            v.get("cwd").or_else(|| v.get("payload")?.get("cwd"))?.as_str().map(String::from)
        });
    if let Ok(mut cache) = CWD_CACHE.lock() {
        cache.insert(path.to_path_buf(), (stamp, cwd.clone()));
    }
    cwd
}

fn file_contains(path: &Path, needle: &str) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if meta.len() > SEARCH_FILE_LIMIT {
        return false;
    }
    let mut text = String::new();
    std::fs::File::open(path).and_then(|mut f| f.read_to_string(&mut text)).is_ok() && text.to_lowercase().contains(needle)
}

pub fn list(conn: &Connection, f: &SessionFilter) -> rusqlite::Result<SessionList> {
    let mut sql = String::from(
        "SELECT r.platform, r.session_id, MIN(r.ts), MAX(r.ts), COUNT(*),
                SUM(CASE r.input_semantics WHEN 'includes_cache' THEN max(r.input_tokens - r.cache_read_tokens, 0) WHEN 'excludes_cache' THEN r.input_tokens ELSE 0 END),
                MIN(CASE r.input_semantics WHEN 'includes_cache' THEN r.input_known * r.cache_read_known WHEN 'excludes_cache' THEN r.input_known ELSE 0 END),
                SUM(r.cache_read_tokens), MIN(r.cache_read_known), SUM(r.cache_write_tokens), MIN(r.cache_write_known),
                SUM(r.output_tokens), MIN(r.output_known),
                MIN(r.connection_id), MAX(r.connection_id), SUM(r.connection_id IS NULL),
                s.title, s.activity_state
         FROM requests r LEFT JOIN sessions s ON s.platform = r.platform AND s.session_id = r.session_id
         WHERE r.session_id IS NOT NULL AND r.session_id <> ''",
    );
    let mut args: Vec<Value> = Vec::new();
    if let Some(p) = &f.platform {
        sql.push_str(" AND r.platform = ?");
        args.push(Value::Text(p.clone()));
    }
    sql.push_str(" GROUP BY r.platform, r.session_id");
    let files = source_files(conn)?;
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(args), |r| {
        let known = |sum: usize, flag: usize| -> rusqlite::Result<Option<i64>> {
            Ok(if r.get::<_, Option<i64>>(flag)?.unwrap_or(0) != 0 { r.get(sum)? } else { None })
        };
        let platform: String = r.get(0)?;
        let min_conn: Option<String> = r.get(13)?;
        let max_conn: Option<String> = r.get(14)?;
        let nulls: i64 = r.get(15)?;
        Ok((
            SessionItem {
                session_id: r.get(1)?,
                first_ts: r.get(2)?,
                last_ts: r.get(3)?,
                requests: r.get(4)?,
                fresh_input: known(5, 6)?,
                cache_read: known(7, 8)?,
                cache_write: known(9, 10)?,
                output: known(11, 12)?,
                connection_id: if nulls == 0 && min_conn == max_conn { min_conn } else { None },
                title: r.get::<_, Option<String>>(16)?.filter(|t| !t.trim().is_empty()),
                cwd: None,
                status: String::new(),
                has_source: false,
                platform,
            },
            r.get::<_, Option<String>>(17)?,
        ))
    })?;
    let needle = f.search.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_lowercase);
    let mut used = std::collections::HashSet::new();
    let mut sessions = Vec::new();
    for row in rows {
        let (mut item, activity) = row?;
        if f.since.is_some_and(|since| item.last_ts < since) {
            continue;
        }
        match f.connection.as_deref() {
            Some(UNATTRIBUTED) if item.connection_id.is_some() => continue,
            Some(id) if id != UNATTRIBUTED && item.connection_id.as_deref() != Some(id) => continue,
            _ => {}
        }
        let file = files.get(&(item.platform.clone(), item.session_id.clone()));
        item.has_source = file.is_some_and(|p| p.exists());
        item.cwd = file.filter(|_| item.has_source).and_then(|p| read_cwd(p));
        if let Some(project) = &f.project {
            if item.cwd.as_deref().unwrap_or("") != project {
                continue;
            }
        }
        item.status = if !item.has_source {
            if crate::platforms::native(&item.platform) { "recent".into() } else { "usage_only".into() }
        } else {
            match activity.as_deref() {
                Some("running") => "running".into(),
                Some("failed") => "failed".into(),
                Some("done") => "done".into(),
                _ => "recent".into(),
            }
        };
        if let Some(q) = &needle {
            let hit = item.session_id.to_lowercase().contains(q)
                || item.title.as_deref().is_some_and(|t| t.to_lowercase().contains(q))
                || file.is_some_and(|p| file_contains(p, q));
            if !hit {
                continue;
            }
        }
        if let Some(p) = file.filter(|_| item.has_source) {
            used.insert(p.clone());
        }
        sessions.push(item);
    }
    sessions.sort_by(|a, b| b.last_ts.cmp(&a.last_ts));
    Ok(SessionList { files: used.len() as i64, sessions })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ToolCall {
    pub name: String,
    /// 调用摘要：文件路径、命令等；取不到时为空
    pub summary: String,
    /// 工具输出片段；来源没记录输出时为 None，不补写结果
    pub output: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Message {
    /// "user" | "assistant"
    pub role: String,
    pub ts: Option<i64>,
    pub text: String,
    pub tools: Vec<ToolCall>,
}

#[derive(Debug, Serialize)]
pub struct MessagePage {
    /// false：没有可读源文件（仅用量记录）
    pub available: bool,
    pub total: usize,
    /// 本页第一条在全部消息里的序号
    pub offset: usize,
    pub messages: Vec<Message>,
    pub models: Vec<String>,
    /// 已计价请求的估算费用（USD）与计价条数
    pub cost: f64,
    pub priced_requests: i64,
    pub requests: i64,
}

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push_str("…");
    out
}

fn parse_ts(v: &Json) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(v.get("timestamp")?.as_str()?).ok().map(|d| d.timestamp_millis())
}

/// 工具调用摘要：优先取路径、命令、模式等常见字段
fn tool_summary(input: &Json) -> String {
    let input = match input {
        Json::String(s) => serde_json::from_str::<Json>(s).unwrap_or(Json::String(s.clone())),
        other => other.clone(),
    };
    if let Json::String(s) = &input {
        return clip(s.lines().next().unwrap_or(""), 120);
    }
    for key in ["file_path", "path", "command", "cmd", "pattern", "url", "query", "description"] {
        if let Some(v) = input.get(key) {
            let text = match v {
                Json::String(s) => s.clone(),
                Json::Array(items) => items.iter().filter_map(Json::as_str).collect::<Vec<_>>().join(" "),
                _ => continue,
            };
            return clip(text.lines().next().unwrap_or(""), 120);
        }
    }
    String::new()
}

fn tool_output(v: &Json) -> Option<String> {
    let text = match v {
        Json::String(s) => s.clone(),
        Json::Array(items) => items.iter().filter_map(|b| b.get("text").and_then(Json::as_str)).collect::<Vec<_>>().join("\n"),
        Json::Object(_) => v.get("output").and_then(Json::as_str).map(String::from).unwrap_or_else(|| v.to_string()),
        _ => return None,
    };
    (!text.trim().is_empty()).then(|| clip(text.trim(), TOOL_OUTPUT_LIMIT))
}

/// 注入给模型的系统性内容（环境、指令、命令回显）不是用户写的话，不显示
fn is_injected(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with('<') || t.starts_with("# AGENTS.md") || t.is_empty()
}

fn push_assistant(out: &mut Vec<Message>, ts: Option<i64>, text: Option<String>, tool: Option<ToolCall>) {
    let reuse = out.last().is_some_and(|m| m.role == "assistant");
    if !reuse {
        out.push(Message { role: "assistant".into(), ts, text: String::new(), tools: Vec::new() });
    }
    let last = out.last_mut().expect("刚放入");
    if let Some(t) = text.filter(|t| !t.trim().is_empty()) {
        if !last.text.is_empty() {
            last.text.push_str("\n\n");
        }
        last.text.push_str(&clip(t.trim(), TEXT_LIMIT));
    }
    if let Some(tool) = tool {
        last.tools.push(tool);
    }
    if ts.is_some() {
        last.ts = ts;
    }
}

fn attach_output(out: &mut [Message], pending: &mut HashMap<String, (usize, usize)>, id: &str, output: Option<String>) {
    if let Some((m, t)) = pending.remove(id) {
        if let Some(tool) = out.get_mut(m).and_then(|msg| msg.tools.get_mut(t)) {
            tool.output = output;
        }
    }
}

pub fn parse_claude<I: IntoIterator<Item = String>>(lines: I) -> Vec<Message> {
    let mut out = Vec::new();
    let mut pending: HashMap<String, (usize, usize)> = HashMap::new();
    for line in lines {
        let Ok(v) = serde_json::from_str::<Json>(&line) else { continue };
        let kind = v.get("type").and_then(Json::as_str).unwrap_or("");
        if !matches!(kind, "user" | "assistant") || v.get("isSidechain").and_then(Json::as_bool) == Some(true) || v.get("isMeta").and_then(Json::as_bool) == Some(true) {
            continue;
        }
        if v.get("isCompactSummary").and_then(Json::as_bool) == Some(true) {
            continue;
        }
        let ts = parse_ts(&v);
        let Some(content) = v.get("message").and_then(|m| m.get("content")) else { continue };
        if kind == "user" {
            match content {
                Json::String(s) if !is_injected(s) => out.push(Message { role: "user".into(), ts, text: clip(s.trim(), TEXT_LIMIT), tools: Vec::new() }),
                Json::Array(blocks) => {
                    let mut texts = Vec::new();
                    for b in blocks {
                        match b.get("type").and_then(Json::as_str) {
                            Some("text") => {
                                if let Some(t) = b.get("text").and_then(Json::as_str).filter(|t| !is_injected(t)) {
                                    texts.push(t.trim().to_string());
                                }
                            }
                            Some("tool_result") => {
                                if let Some(id) = b.get("tool_use_id").and_then(Json::as_str) {
                                    attach_output(&mut out, &mut pending, id, b.get("content").and_then(tool_output));
                                }
                            }
                            _ => {}
                        }
                    }
                    if !texts.is_empty() {
                        out.push(Message { role: "user".into(), ts, text: clip(&texts.join("\n\n"), TEXT_LIMIT), tools: Vec::new() });
                    }
                }
                _ => {}
            }
        } else if let Json::Array(blocks) = content {
            for b in blocks {
                match b.get("type").and_then(Json::as_str) {
                    Some("text") => push_assistant(&mut out, ts, b.get("text").and_then(Json::as_str).map(String::from), None),
                    Some("tool_use") => {
                        let tool = ToolCall {
                            name: b.get("name").and_then(Json::as_str).unwrap_or("工具").to_string(),
                            summary: b.get("input").map(tool_summary).unwrap_or_default(),
                            output: None,
                        };
                        push_assistant(&mut out, ts, None, Some(tool));
                        if let Some(id) = b.get("id").and_then(Json::as_str) {
                            let m = out.len() - 1;
                            pending.insert(id.to_string(), (m, out[m].tools.len() - 1));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out.retain(|m| !(m.text.is_empty() && m.tools.is_empty()));
    out
}

pub fn parse_codex<I: IntoIterator<Item = String>>(lines: I) -> Vec<Message> {
    let mut out = Vec::new();
    let mut pending: HashMap<String, (usize, usize)> = HashMap::new();
    for line in lines {
        let Ok(v) = serde_json::from_str::<Json>(&line) else { continue };
        if v.get("type").and_then(Json::as_str) != Some("response_item") {
            continue;
        }
        let ts = parse_ts(&v);
        let Some(p) = v.get("payload") else { continue };
        match p.get("type").and_then(Json::as_str) {
            Some("message") => {
                let role = p.get("role").and_then(Json::as_str).unwrap_or("");
                let text = p
                    .get("content")
                    .and_then(Json::as_array)
                    .map(|blocks| blocks.iter().filter_map(|b| b.get("text").and_then(Json::as_str)).collect::<Vec<_>>().join("\n"))
                    .unwrap_or_default();
                match role {
                    "user" if !is_injected(&text) => out.push(Message { role: "user".into(), ts, text: clip(text.trim(), TEXT_LIMIT), tools: Vec::new() }),
                    "assistant" => push_assistant(&mut out, ts, Some(text), None),
                    _ => {}
                }
            }
            Some("function_call") | Some("custom_tool_call") | Some("local_shell_call") => {
                let input = p.get("arguments").or_else(|| p.get("input")).or_else(|| p.get("action")).cloned().unwrap_or(Json::Null);
                let tool = ToolCall {
                    name: p.get("name").and_then(Json::as_str).unwrap_or("shell").to_string(),
                    summary: tool_summary(&input),
                    output: None,
                };
                push_assistant(&mut out, ts, None, Some(tool));
                if let Some(id) = p.get("call_id").and_then(Json::as_str) {
                    let m = out.len() - 1;
                    pending.insert(id.to_string(), (m, out[m].tools.len() - 1));
                }
            }
            Some("function_call_output") | Some("custom_tool_call_output") => {
                if let Some(id) = p.get("call_id").and_then(Json::as_str) {
                    attach_output(&mut out, &mut pending, id, p.get("output").and_then(tool_output));
                }
            }
            _ => {}
        }
    }
    out.retain(|m| !(m.text.is_empty() && m.tools.is_empty()));
    out
}

fn read_lines(path: &Path) -> Vec<String> {
    std::fs::File::open(path).map(|f| BufReader::new(f).lines().map_while(Result::ok).collect()).unwrap_or_default()
}

/// 消息分页：`before` 为 None 取最后 `limit` 条，否则取序号 `before` 之前的 `limit` 条
pub fn messages(conn: &Connection, platform: &str, session_id: &str, before: Option<usize>, limit: usize) -> rusqlite::Result<MessagePage> {
    let files = source_files(conn)?;
    let file = files.get(&(platform.to_string(), session_id.to_string())).filter(|p| p.exists()).cloned();
    let all = match (&file, platform) {
        (Some(p), "claude") => parse_claude(read_lines(p)),
        (Some(p), "codex") => parse_codex(read_lines(p)),
        _ => Vec::new(),
    };
    let total = all.len();
    let end = before.unwrap_or(total).min(total);
    let start = end.saturating_sub(limit);
    let (cost, priced_requests, requests, models) = session_cost(conn, platform, session_id)?;
    Ok(MessagePage {
        available: file.is_some(),
        total,
        offset: start,
        messages: all[start..end].to_vec(),
        models,
        cost,
        priced_requests,
        requests,
    })
}

fn session_cost(conn: &Connection, platform: &str, session_id: &str) -> rusqlite::Result<(f64, i64, i64, Vec<String>)> {
    let mut stmt = conn.prepare(
        "SELECT r.model, r.input_tokens, r.output_tokens, r.cache_write_tokens, r.cache_read_tokens,
                r.input_known * r.output_known * r.cache_write_known * r.cache_read_known, m.reasoning, m.tool
         FROM requests r LEFT JOIN request_metrics m ON m.request_id = r.id
         WHERE r.platform = ?1 AND r.session_id = ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![platform, session_id], |r| {
        Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?, r.get::<_, i64>(5)? != 0, r.get::<_, Option<i64>>(6)?, r.get::<_, Option<i64>>(7)?))
    })?;
    let (mut cost, mut priced, mut count) = (0.0, 0i64, 0i64);
    let mut models: Vec<String> = Vec::new();
    for row in rows {
        let (model, input, output, cache_write, cache_read, known, reasoning, tool) = row?;
        count += 1;
        if known {
            if let Some(c) = crate::pricing::estimate_usage(Some(platform), model.as_deref(), crate::pricing::Usage {
                input, output, cache_write, cache_read,
                reasoning: reasoning.unwrap_or(0), tool: tool.unwrap_or(0),
                long_prompt: input > crate::pricing::GEMINI_LONG_PROMPT_TOKENS,
            }) {
                cost += c;
                priced += 1;
            }
        }
        if let Some(m) = model.filter(|m| !models.contains(m)) {
            models.push(m);
        }
    }
    Ok((cost, priced, count, models))
}

/* ------------------------------ 在原工具继续 ------------------------------ */

#[derive(Debug, Serialize)]
pub struct ContinueCheck {
    /// 该来源是否有可靠的命令行续聊入口
    pub supported: bool,
    pub tool_found: bool,
    pub cwd: Option<String>,
    pub cwd_accessible: bool,
}

fn tool_command(platform: &str) -> Option<(&'static str, &'static [&'static str])> {
    match platform {
        "claude" => Some(("claude", &["--resume"])),
        "codex" => Some(("codex", &["resume"])),
        _ => None,
    }
}

/// 在 PATH 里找可执行文件（Windows 按 PATHEXT 补扩展名）
pub(crate) fn find_executable(name: &str) -> Option<PathBuf> {
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into()).split(';').map(|e| e.to_ascii_lowercase()).collect()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| {
        exts.iter().map(|ext| dir.join(format!("{name}{ext}"))).find(|p| p.is_file())
    })
}

/// 会话 ID 会拼进新控制台的命令行，只接受 UUID 一类的安全字符
pub(crate) fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub fn continue_check(conn: &Connection, platform: &str, session_id: &str) -> rusqlite::Result<ContinueCheck> {
    let files = source_files(conn)?;
    let cwd = files.get(&(platform.to_string(), session_id.to_string())).filter(|p| p.exists()).and_then(|p| read_cwd(p));
    let supported = tool_command(platform).is_some() && safe_id(session_id);
    Ok(ContinueCheck {
        supported,
        tool_found: tool_command(platform).is_some_and(|(name, _)| find_executable(name).is_some()),
        cwd_accessible: cwd.as_deref().is_some_and(|d| Path::new(d).is_dir()),
        cwd,
    })
}

/// 打开新的控制台窗口，在项目目录里执行续聊命令；后续消息由原工具处理
pub fn continue_session(conn: &Connection, platform: &str, session_id: &str) -> Result<(), String> {
    let check = continue_check(conn, platform, session_id).map_err(|e| e.to_string())?;
    let (name, args) = tool_command(platform).filter(|_| check.supported).ok_or("这个来源暂不支持在原工具继续")?;
    if !check.tool_found {
        return Err(format!("未检测到 {name} 命令，请安装或配置后重试"));
    }
    let dir = check.cwd.filter(|_| check.cwd_accessible).ok_or("项目目录不可访问")?;
    let mut command = std::process::Command::new("cmd");
    command.arg("/K").arg(name).args(args).arg(session_id).current_dir(&dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        command.creation_flags(CREATE_NEW_CONSOLE);
    }
    command.spawn().map(|_| ()).map_err(|e| format!("打开 {name} 失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn claude_messages_merge_blocks_and_attach_tool_output() {
        let msgs = parse_claude(lines(&[
            r#"{"type":"user","timestamp":"2026-10-21T04:32:00Z","message":{"role":"user","content":"把列表和详情放在同一页"}}"#,
            r#"{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#,
            r#"{"type":"assistant","timestamp":"2026-10-21T04:33:00Z","message":{"content":[{"type":"thinking","thinking":"x"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"已整理为左右分栏"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"SessionList.tsx"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"已读取 128 行"}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"子代理"}]}}"#,
        ]));
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].text, "已整理为左右分栏");
        assert_eq!(msgs[1].tools[0].summary, "SessionList.tsx");
        assert_eq!(msgs[1].tools[0].output.as_deref(), Some("已读取 128 行"));
    }

    #[test]
    fn codex_messages_skip_injected_context_and_pair_calls() {
        let msgs = parse_codex(lines(&[
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>cwd</environment_context>"}]}}"#,
            r#"{"type":"response_item","timestamp":"2026-10-21T04:32:00Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"优化会话界面"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"rg\",\"Session\"]}","call_id":"c1"}}"#,
            r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"3 matches"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"完成"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"规则"}]}}"#,
        ]));
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].text, "优化会话界面");
        assert_eq!(msgs[1].tools[0].summary, "rg Session");
        assert_eq!(msgs[1].tools[0].output.as_deref(), Some("3 matches"));
        assert_eq!(msgs[1].text, "完成");
    }

    #[test]
    fn unsafe_session_ids_are_rejected() {
        assert!(safe_id("0f8e2c5a-1b2c-4d5e-8f90-123456789abc"));
        assert!(!safe_id("x & calc"));
        assert!(!safe_id(""));
    }
}
