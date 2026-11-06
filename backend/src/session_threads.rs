//! 会话线程页（Jolu.pen 模块 D，画板 40–57）：本机会话列表、只读消息记录与「在原工具继续」。
//!
//! - 会话来自统计库里带 session_id 的请求；标题与生命周期来自 sessions 表；
//! - 源文件位置来自采集器的 scan_state（path ↔ parser_session_id），项目目录从源文件里读 `cwd`；
//! - 消息只读解析源文件，不改写、不删除；来源没保留正文时如实返回「仅用量」，不拼造对话；
//! - ZCode 是库型来源：正文在 `~/.zcode/cli/db/db.sqlite` 的 `message` / `part` 表里，
//!   以 `mode=ro` 等价方式（`SQLITE_OPEN_READ_ONLY`）打开，结构漂移就降级为「无正文」；
//! - 继续会话只对有可靠命令行入口的来源开放（Claude Code `claude --resume`、Codex `codex resume`）。

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use rusqlite::{types::Value, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

pub const UNATTRIBUTED: &str = crate::usage_stats::UNATTRIBUTED;

/// 单条消息正文与工具输出的长度上限，避免超长日志拖垮界面
const TEXT_LIMIT: usize = 20_000;
const TOOL_OUTPUT_LIMIT: usize = 1_200;
/// 交接用的详细解析：工具输出保留首尾共这么多字（「最后一次验证」要取末尾 15 行），命令保留原文
const DETAIL_OUTPUT_LIMIT: usize = 40_000;
const DETAIL_SUMMARY_LIMIT: usize = 2_000;
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
    /// 只查这些原始会话 ID（统一网关页定向取数用；前端筛选不会传）
    #[serde(skip)]
    pub session_ids: Option<Vec<String>>,
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
    // 隔离库也可能导入过真实路径；不能因路径已在数据库里就信任它。
    if crate::validation::isolated() { return Ok(HashMap::new()); }
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
    if crate::validation::isolated() { return None; }
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
    if crate::validation::isolated() { return false; }
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
                s.title, s.activity_state, s.cwd
         FROM requests r LEFT JOIN sessions s ON s.platform = r.platform AND s.session_id = r.session_id
         WHERE r.session_id IS NOT NULL AND r.session_id <> ''",
    );
    let mut args: Vec<Value> = Vec::new();
    if let Some(p) = &f.platform {
        sql.push_str(" AND r.platform = ?");
        args.push(Value::Text(p.clone()));
    }
    if let Some(ids) = &f.session_ids {
        if ids.is_empty() {
            return Ok(SessionList { sessions: Vec::new(), files: 0 });
        }
        sql.push_str(&format!(" AND r.session_id IN ({})", vec!["?"; ids.len()].join(",")));
        args.extend(ids.iter().cloned().map(Value::Text));
    }
    sql.push_str(" GROUP BY r.platform, r.session_id");
    let files = source_files(conn)?;
    // ZCode 的子代理会话不当作主会话展示（规则见 `zcode_child_sessions`）；
    // 库不可读时不加这层过滤，宁可多显示也不要吞掉真实会话
    let zcode_children = zcode_child_sessions();
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
                // 库里的 cwd 优先（ZCode 这类库型来源没有源文件可读）；
                // 源文件能读到时会覆盖它（Claude / Codex 的 cwd 更权威）
                cwd: r.get::<_, Option<String>>(18)?.filter(|c| !c.trim().is_empty()),
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
        if zcode_child_session(zcode_children.as_ref(), &item.platform, &item.session_id) {
            continue;
        }
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
        if let Some(from_file) = file.filter(|_| item.has_source).and_then(|p| read_cwd(p)) {
            item.cwd = Some(from_file);
        }
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

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ToolCall {
    pub name: String,
    /// 调用摘要：文件路径、命令等；取不到时为空
    pub summary: String,
    /// 工具输出片段；来源没记录输出时为 None，不补写结果
    pub output: Option<String>,
    /// 工具结果是否失败：Claude `tool_result.is_error`；Codex 取退出码、`CommandExecution.status`
    /// 或 `exec` 输出首行 `Script failed|completed`。来源没有记录时为 None，不推断
    pub failed: Option<bool>,
    /// 退出码：Claude 失败结果开头的 `Exit code N`、Codex `exit_code` / `Process exited with code N`；取不到为 None
    pub exit_code: Option<i32>,
    /// 调用涉及的文件路径（`file_path` / `notebook_path` / `path`、apply_patch 的文件头、Codex FileChange 的键），只供交接事实区使用
    #[serde(skip)]
    pub paths: Vec<String>,
}

/// 库型来源（ZCode）记录的单条 token 用量：缺哪个字段就是 None，不补 0
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct TokenUsage {
    pub total: Option<i64>,
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub reasoning: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Message {
    /// "user" | "assistant"
    pub role: String,
    pub ts: Option<i64>,
    pub text: String,
    pub tools: Vec<ToolCall>,
    /// 库型来源（ZCode）的推理片段；源文件来源的解析器不填
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// 库型来源记录的 `modelId`；源文件来源不填
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 库型来源记录的 `providerId`（如 `account:zai-start-plan`）；源文件来源不填
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// 本条消息的 token 用量；只有库型来源提供
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenUsage>,
    /// 本条消息的费用（来源原样透传）；只有库型来源提供
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

#[derive(Debug, Default, Serialize)]
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
    /// 正文不完整时给界面的说明（截断 / 有坏记录被跳过）；完整时为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push_str("…");
    out
}

/// 保留首尾：交接要看输出末尾（测试结果、报错），中间省略
fn clip_middle(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_string();
    }
    let head = limit / 4;
    let tail = limit - head;
    let start: String = text.chars().take(head).collect();
    let end: String = text.chars().skip(count - tail).collect();
    format!("{start}\n…［中间省略 {} 字］…\n{end}", count - head - tail)
}

/// 解析的详细程度：会话页只要片段；交接要更长的输出、完整命令和 Codex 的命令事件
#[derive(Debug, Clone, Copy)]
struct Limits {
    text: usize,
    output: usize,
    summary: usize,
    detail: bool,
}

const VIEW: Limits = Limits { text: TEXT_LIMIT, output: TOOL_OUTPUT_LIMIT, summary: 120, detail: false };
const DETAIL: Limits = Limits { text: TEXT_LIMIT, output: DETAIL_OUTPUT_LIMIT, summary: DETAIL_SUMMARY_LIMIT, detail: true };

impl Limits {
    fn summary(&self, text: &str) -> String {
        if self.detail { clip(text.trim(), self.summary) } else { clip(text.lines().next().unwrap_or(""), self.summary) }
    }
    fn output(&self, text: &str) -> String {
        if self.detail { clip_middle(text, self.output) } else { clip(text, self.output) }
    }
}

fn parse_ts(v: &Json) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(v.get("timestamp")?.as_str()?).ok().map(|d| d.timestamp_millis())
}

fn parse_input(input: &Json) -> Json {
    match input {
        Json::String(s) => serde_json::from_str::<Json>(s).unwrap_or(Json::String(s.clone())),
        other => other.clone(),
    }
}

/// 工具调用摘要：优先取路径、命令、模式等常见字段
fn tool_summary(input: &Json, limits: Limits) -> String {
    let input = parse_input(input);
    if let Json::String(s) = &input {
        return limits.summary(s);
    }
    for key in ["file_path", "path", "command", "cmd", "pattern", "url", "query", "description"] {
        if let Some(v) = input.get(key) {
            let text = match v {
                Json::String(s) => s.clone(),
                Json::Array(items) => items.iter().filter_map(Json::as_str).collect::<Vec<_>>().join(" "),
                _ => continue,
            };
            return limits.summary(&text);
        }
    }
    String::new()
}

/// 调用涉及的文件：常见路径字段，以及 apply_patch 补丁里的文件头
fn tool_paths(input: &Json) -> Vec<String> {
    let input = parse_input(input);
    let mut out = Vec::new();
    let patch = match &input {
        Json::String(s) => Some(s.as_str()),
        other => other.get("patch").or_else(|| other.get("input")).and_then(Json::as_str),
    };
    if let Some(patch) = patch.filter(|p| p.contains("*** Begin Patch")) {
        for line in patch.lines() {
            for head in ["*** Add File: ", "*** Update File: ", "*** Delete File: ", "*** Move to: "] {
                if let Some(path) = line.strip_prefix(head).map(str::trim).filter(|p| !p.is_empty()) {
                    out.push(path.to_string());
                }
            }
        }
    }
    for key in ["file_path", "notebook_path", "path"] {
        if let Some(path) = input.get(key).and_then(Json::as_str).map(str::trim).filter(|p| !p.is_empty()) {
            out.push(path.to_string());
        }
    }
    out
}

fn output_text(v: &Json) -> Option<String> {
    let text = match v {
        Json::String(s) => s.clone(),
        Json::Array(items) => items.iter().filter_map(|b| b.get("text").and_then(Json::as_str)).collect::<Vec<_>>().join("\n"),
        Json::Object(_) => v.get("output").and_then(Json::as_str).map(String::from).unwrap_or_else(|| v.to_string()),
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// 工具结果：输出片段、失败标记与退出码
#[derive(Debug, Default)]
struct ToolResult {
    output: Option<String>,
    failed: Option<bool>,
    exit_code: Option<i32>,
}

/// 文本里行首 `<前缀> N` 形式的退出码
fn exit_line(text: &str, prefixes: &[&str]) -> Option<i32> {
    text.lines().find_map(|line| {
        let line = line.trim();
        prefixes.iter().find_map(|p| line.strip_prefix(p)).and_then(|rest| {
            let digits: String = rest.trim_start_matches([':', ' ']).chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
            digits.parse().ok()
        })
    })
}

/// Claude `tool_result`：`is_error` 是失败标记，失败时正文以 `Exit code N` 开头
fn claude_result(block: &Json, limits: Limits) -> ToolResult {
    let text = block.get("content").and_then(output_text);
    let exit_code = text.as_deref().and_then(|t| t.lines().next()).and_then(|first| exit_line(first, &["Exit code"]));
    ToolResult {
        output: text.map(|t| limits.output(&t)),
        failed: block.get("is_error").and_then(Json::as_bool),
        exit_code,
    }
}

/// Codex 输出没有错误字段：旧版 shell 的 JSON `metadata.exit_code`、`exec_command` 的 `Process exited with code N`、
/// `exec` 脚本首行 `Script failed|completed`；都没有时为 None
fn codex_result(output: Option<&Json>, limits: Limits) -> ToolResult {
    let Some(text) = output.and_then(output_text) else { return ToolResult::default() };
    let json_exit = serde_json::from_str::<Json>(&text).ok().and_then(|j| j.pointer("/metadata/exit_code").and_then(Json::as_i64)).map(|n| n as i32);
    let exit_code = json_exit.or_else(|| exit_line(&text, &["Process exited with code", "Exit code"]));
    let script = match text.lines().next().map(str::trim) {
        Some(l) if l.starts_with("Script failed") => Some(true),
        Some(l) if l.starts_with("Script completed") => Some(false),
        _ => None,
    };
    ToolResult { failed: exit_code.map(|c| c != 0).or(script), exit_code, output: Some(limits.output(&text)) }
}

/// 注入给模型的系统性内容（环境、指令、命令回显）不是用户写的话，不显示
fn is_injected(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with('<') || t.starts_with("# AGENTS.md") || t.is_empty()
}

fn push_assistant(out: &mut Vec<Message>, ts: Option<i64>, text: Option<String>, tool: Option<ToolCall>, limit: usize) {
    let reuse = out.last().is_some_and(|m| m.role == "assistant");
    if !reuse {
        out.push(Message { role: "assistant".into(), ts, text: String::new(), tools: Vec::new(), ..Message::default() });
    }
    let last = out.last_mut().expect("刚放入");
    if let Some(t) = text.filter(|t| !t.trim().is_empty()) {
        if !last.text.is_empty() {
            last.text.push_str("\n\n");
        }
        last.text.push_str(&clip(t.trim(), limit));
    }
    if let Some(tool) = tool {
        last.tools.push(tool);
    }
    if ts.is_some() {
        last.ts = ts;
    }
}

fn attach_output(out: &mut [Message], pending: &mut HashMap<String, (usize, usize)>, id: &str, result: ToolResult) {
    if let Some((m, t)) = pending.remove(id) {
        if let Some(tool) = out.get_mut(m).and_then(|msg| msg.tools.get_mut(t)) {
            tool.output = result.output;
            tool.failed = result.failed;
            tool.exit_code = result.exit_code;
        }
    }
}

pub fn parse_claude<I: IntoIterator<Item = String>>(lines: I) -> Vec<Message> {
    parse_claude_with(lines, VIEW)
}

fn parse_claude_with<I: IntoIterator<Item = String>>(lines: I, limits: Limits) -> Vec<Message> {
    let mut out = Vec::new();
    let mut pending: HashMap<String, (usize, usize)> = HashMap::new();
    // 压缩后 Claude Code 会把保留段按原 uuid 再写一遍；按 uuid 去重，避免消息重复、序号错位
    let mut seen: HashSet<String> = HashSet::new();
    for line in lines {
        let Ok(v) = serde_json::from_str::<Json>(&line) else { continue };
        let kind = v.get("type").and_then(Json::as_str).unwrap_or("");
        if matches!(kind, "user" | "assistant") {
            if let Some(uuid) = v.get("uuid").and_then(Json::as_str) {
                if !seen.insert(uuid.to_string()) {
                    continue;
                }
            }
        }
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
                Json::String(s) if !is_injected(s) => out.push(Message { role: "user".into(), ts, text: clip(s.trim(), limits.text), tools: Vec::new(), ..Message::default() }),
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
                                    attach_output(&mut out, &mut pending, id, claude_result(b, limits));
                                }
                            }
                            _ => {}
                        }
                    }
                    if !texts.is_empty() {
                        out.push(Message { role: "user".into(), ts, text: clip(&texts.join("\n\n"), limits.text), tools: Vec::new(), ..Message::default() });
                    }
                }
                _ => {}
            }
        } else if let Json::Array(blocks) = content {
            for b in blocks {
                match b.get("type").and_then(Json::as_str) {
                    Some("text") => push_assistant(&mut out, ts, b.get("text").and_then(Json::as_str).map(String::from), None, limits.text),
                    Some("tool_use") => {
                        let input = b.get("input").cloned().unwrap_or(Json::Null);
                        let tool = ToolCall {
                            name: b.get("name").and_then(Json::as_str).unwrap_or("工具").to_string(),
                            summary: tool_summary(&input, limits),
                            paths: tool_paths(&input),
                            ..ToolCall::default()
                        };
                        push_assistant(&mut out, ts, None, Some(tool), limits.text);
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
    parse_codex_with(lines, VIEW)
}

/// Codex 新版把真实命令记在 `event_msg` → `item_completed`（`CommandExecution`：`command` 数组、`exit_code`、`status`），
/// 文件修改记在 `FileChange.changes`（以路径为键）。只有交接的详细解析读取，会话页显示保持不变
fn codex_event_tool(item: &Json, limits: Limits) -> Option<ToolCall> {
    match item.get("type").and_then(Json::as_str)? {
        "CommandExecution" | "command_execution" => {
            let command = match item.get("command")? {
                Json::Array(parts) => parts.last().and_then(Json::as_str).unwrap_or("").to_string(),
                Json::String(s) => s.clone(),
                _ => String::new(),
            };
            let exit_code = item.get("exit_code").and_then(Json::as_i64).map(|n| n as i32);
            let status_failed = item.get("status").and_then(Json::as_str).map(|s| s == "failed");
            Some(ToolCall {
                name: "command_execution".into(),
                summary: limits.summary(&command),
                output: item.get("aggregated_output").and_then(output_text).map(|t| limits.output(&t)),
                failed: match exit_code {
                    Some(code) => Some(code != 0 || status_failed == Some(true)),
                    None => status_failed,
                },
                exit_code,
                paths: Vec::new(),
            })
        }
        "FileChange" | "file_change" => {
            let paths: Vec<String> = match item.get("changes")? {
                Json::Object(map) => map.keys().cloned().collect(),
                Json::Array(items) => items.iter().filter_map(|c| c.get("path").and_then(Json::as_str).map(String::from)).collect(),
                _ => Vec::new(),
            };
            (!paths.is_empty()).then(|| ToolCall {
                name: "apply_patch".into(),
                summary: limits.summary(&paths.join(", ")),
                failed: item.get("status").and_then(Json::as_str).map(|s| s == "failed"),
                paths,
                ..ToolCall::default()
            })
        }
        _ => None,
    }
}

fn parse_codex_with<I: IntoIterator<Item = String>>(lines: I, limits: Limits) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();
    let mut pending: HashMap<String, (usize, usize)> = HashMap::new();
    for line in lines {
        let Ok(v) = serde_json::from_str::<Json>(&line) else { continue };
        if limits.detail && v.get("type").and_then(Json::as_str) == Some("event_msg") {
            let payload = v.get("payload");
            if payload.and_then(|p| p.get("type")).and_then(Json::as_str) == Some("item_completed") {
                // 只附加到已有的最后一条消息上，不新建消息，保证 `#n` 与会话页一致
                if let (Some(tool), Some(last)) = (payload.and_then(|p| p.get("item")).and_then(|item| codex_event_tool(item, limits)), out.last_mut()) {
                    last.tools.push(tool);
                }
            }
            continue;
        }
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
                    "user" if !is_injected(&text) => out.push(Message { role: "user".into(), ts, text: clip(text.trim(), limits.text), tools: Vec::new(), ..Message::default() }),
                    "assistant" => push_assistant(&mut out, ts, Some(text), None, limits.text),
                    _ => {}
                }
            }
            Some("function_call") | Some("custom_tool_call") | Some("local_shell_call") => {
                let input = p.get("arguments").or_else(|| p.get("input")).or_else(|| p.get("action")).cloned().unwrap_or(Json::Null);
                let tool = ToolCall {
                    name: p.get("name").and_then(Json::as_str).unwrap_or("shell").to_string(),
                    summary: tool_summary(&input, limits),
                    paths: tool_paths(&input),
                    ..ToolCall::default()
                };
                push_assistant(&mut out, ts, None, Some(tool), limits.text);
                if let Some(id) = p.get("call_id").and_then(Json::as_str) {
                    let m = out.len() - 1;
                    pending.insert(id.to_string(), (m, out[m].tools.len() - 1));
                }
            }
            Some("function_call_output") | Some("custom_tool_call_output") => {
                if let Some(id) = p.get("call_id").and_then(Json::as_str) {
                    attach_output(&mut out, &mut pending, id, codex_result(p.get("output"), limits));
                }
            }
            _ => {}
        }
    }
    out.retain(|m| !(m.text.is_empty() && m.tools.is_empty()));
    out
}

pub(crate) fn read_lines(path: &Path) -> Result<Vec<String>, String> {
    if crate::validation::isolated() { return Err("隔离验收不读取用户会话文件".into()); }
    let file = std::fs::File::open(path).map_err(|error| format!("无法打开会话文件：{error}"))?;
    // 任一行读取失败都不能把此前内容当作完整会话，更不能据此生成交接。
    BufReader::new(file).lines().collect::<std::io::Result<Vec<_>>>()
        .map_err(|error| format!("读取会话文件失败：{error}"))
}

/// 会话主记录文件；源文件不存在时为 None
pub(crate) fn source_path(conn: &Connection, platform: &str, session_id: &str) -> rusqlite::Result<Option<PathBuf>> {
    Ok(source_files(conn)?.remove(&(platform.to_string(), session_id.to_string())).filter(|p| p.exists()))
}

/// 交接用的详细解析：消息切分与去重规则和会话页相同，所以 `#n` 序号一致；
/// 但保留更长的工具输出与完整命令，并读取 Codex 的命令与文件修改事件（附在当前助手消息里，不新增消息）
pub(crate) fn parse_detail(platform: &str, lines: &[String]) -> Vec<Message> {
    match platform {
        "claude" => parse_claude_with(lines.iter().cloned(), DETAIL),
        "codex" => parse_codex_with(lines.iter().cloned(), DETAIL),
        _ => Vec::new(),
    }
}

/* ------------------------------ ZCode 库型来源 ------------------------------ */

/// 单会话最多解析的 part 条数。本机 477 MB 的库里单个会话最多有 14,404 条 part，
/// 一次读全会把整个会话 JSON 拉进内存；超出后只解析最近的部分，并在页面上标注截断。
const ZCODE_PART_LIMIT: usize = 2_000;
/// 库型来源的等待上限：原工具可能正在写 WAL，宁可稍等也不认为会话正文不可读
const ZCODE_BUSY_MS: u64 = 400;

/// ZCode 会话库：`~/.zcode/cli/db/db.sqlite`。缺失时返回 None，由调用方降级。
fn zcode_db_path() -> Option<PathBuf> {
    if crate::validation::isolated() { return None; }
    Some(crate::creds::home()?.join(".zcode").join("cli").join("db").join("db.sqlite"))
}

/// 只读打开 ZCode 库。`SQLITE_OPEN_READ_ONLY` 与 `mode=ro` 等价：连接不持有写锁、
/// 不会创建或修改 `.zcode` 下任何文件（WAL 的 `-shm` 已由原工具创建，只读取用）。
fn open_zcode_db() -> Option<Connection> {
    let path = zcode_db_path().filter(|p| p.is_file())?;
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()?;
    let _ = conn.busy_timeout(std::time::Duration::from_millis(ZCODE_BUSY_MS));
    Some(conn)
}

/// 结构探测：原工具升级会让列漂移，探测不到就当作「没有正文」，绝不猜列名或 mock 数据。
fn zcode_schema_ok(conn: &Connection) -> bool {
    conn.prepare("SELECT id, session_id, sequence, time_created, data FROM message LIMIT 0").is_ok()
        && conn.prepare("SELECT id, message_id, sequence, time_created, data FROM part LIMIT 0").is_ok()
}

/// 子会话 id 集合（`parent_id` 非空）。ZCode 把子代理会话也写进同一张 `session` 表：
/// 171 条里 146 条 `parent_id` 非空（workflow_child 100 / subagent_child 41 / fork 2 /
/// selection_side_chat 3），只有 25 条是用户真正打开的根会话。
///
/// 列表规则：只把根会话当主会话展示（子会话不进列表，因此也不会刷屏）；子会话不折叠展示，
/// 因为它的正文只是父会话里一次 Agent 调用的展开，父会话正文已包含该调用的结果。
/// 子会话自己的正文仍可按 id 只读查询（例如从别处拿到 id），只是不会出现在列表里。
fn zcode_child_sessions() -> Option<HashSet<String>> {
    let conn = open_zcode_db()?;
    let mut stmt = conn.prepare("SELECT id FROM session WHERE parent_id IS NOT NULL AND parent_id <> ''").ok()?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).ok()?;
    let out: HashSet<String> = rows.filter_map(Result::ok).collect();
    (!out.is_empty()).then_some(out)
}

/// Jolu 的请求表把 ZCode 会话记成 `{来源 id}:{原始会话 id}`，例如
/// `native:zcode:main:sess_0a15…`；查原库前必须还原成裸 id，否则一条 message 都匹配不上。
/// 没有 `native:` 前缀（含直接传裸 id）时按原样使用。
fn zcode_raw_session_id(session_id: &str) -> &str {
    if session_id.starts_with("native:") {
        session_id.rsplit(':').next().filter(|id| !id.is_empty()).unwrap_or(session_id)
    } else {
        session_id
    }
}

/// 列表里的这一条是否是 ZCode 子代理会话（不在主列表展示）。子会话集合不可用时一律当作根会话。
fn zcode_child_session(children: Option<&HashSet<String>>, platform: &str, session_id: &str) -> bool {
    platform == "zcode" && children.is_some_and(|children| children.contains(zcode_raw_session_id(session_id)))
}

/// 一条消息的重建结果：`Ok(None)` 表示这条不该展示（系统注入 / 模型专用），`Err(())` 表示 data 坏了
fn zcode_message(data: &str, parts: &[String], limits: Limits) -> Result<Option<Message>, ()> {
    let Ok(meta) = serde_json::from_str::<Json>(data) else { return Err(()) };
    // 注入给模型、没写进用户可见记录的内容不显示（与 Claude 的 isMeta / isInjected 同一口径）
    let hidden = meta.get("synthetic").and_then(Json::as_bool) == Some(true)
        || meta.pointer("/metadata/visibility").and_then(Json::as_str) == Some("model-only")
        || matches!(meta.pointer("/semantics/uiVisibility").and_then(Json::as_str), Some("hidden"))
        || matches!(meta.pointer("/semantics/transcriptVisibility").and_then(Json::as_str), Some("hidden"));
    if hidden { return Ok(None); }
    let role = meta.get("role").and_then(Json::as_str).unwrap_or("");
    if !matches!(role, "user" | "assistant") { return Ok(None); }

    let ts = meta.pointer("/time/created").and_then(Json::as_i64);
    let mut texts: Vec<String> = Vec::new();
    let mut thoughts: Vec<String> = Vec::new();
    let mut tools: Vec<ToolCall> = Vec::new();
    // step-finish 是每条模型调用的结算节点，消息级 tokens 缺失时用它累计
    let mut step_tokens = TokenUsage::default();
    let mut step_cost = 0.0f64;
    let mut step_seen = false;
    for raw in parts {
        let Ok(part) = serde_json::from_str::<Json>(raw) else { continue };
        match part.get("type").and_then(Json::as_str).unwrap_or("") {
            "text" => {
                if let Some(text) = part.get("text").and_then(Json::as_str).map(str::trim).filter(|t| !t.is_empty()) {
                    texts.push(text.to_string());
                }
            }
            "reasoning" => {
                if let Some(text) = part.get("text").and_then(Json::as_str).map(str::trim).filter(|t| !t.is_empty()) {
                    thoughts.push(text.to_string());
                }
            }
            "tool" => {
                if let Some(tool) = zcode_tool(&part, limits) { tools.push(tool); }
            }
            "step-finish" => {
                step_seen = true;
                step_cost += part.get("cost").and_then(Json::as_f64).unwrap_or(0.0);
                zcode_add_tokens(&mut step_tokens, part.get("tokens"));
            }
            // step-start：只有计时，没有内容；file / timeline / compaction：附件与分隔标记，
            // 不属于消息正文（附件路径已在工具调用摘要里出现），不拼进对话
            _ => {}
        }
    }

    let text = clip(&texts.join("\n\n"), limits.text);
    let tokens = zcode_tokens(meta.get("tokens")).or(step_seen.then(|| step_tokens.clone()));
    let cost = meta.get("cost").and_then(Json::as_f64).or(step_seen.then_some(step_cost));
    let reasoning = (!thoughts.is_empty()).then(|| clip(&thoughts.join("\n\n"), limits.text));
    if text.is_empty() && tools.is_empty() && reasoning.is_none() {
        return Ok(None);
    }
    Ok(Some(Message {
        role: role.into(),
        ts,
        text,
        tools,
        reasoning,
        model: meta.get("modelId").and_then(Json::as_str).map(String::from),
        provider: meta.get("providerId").and_then(Json::as_str).map(String::from),
        tokens,
        cost,
    }))
}

/// `part.type == "tool"`：摘要、输出、状态都在 `state` 里，调用与结果是同一条记录
fn zcode_tool(part: &Json, limits: Limits) -> Option<ToolCall> {
    let state = part.get("state")?;
    let input = state.get("input").cloned().unwrap_or(Json::Null);
    let summary = {
        let from_input = tool_summary(&input, limits);
        if from_input.is_empty() {
            state.get("title").and_then(Json::as_str).map(|t| limits.summary(t)).unwrap_or_default()
        } else {
            from_input
        }
    };
    // `state.output` 只在工具结束时才有；运行中没有输出就返回 None，不补写结果
    let output = state.get("output").and_then(output_text).map(|t| limits.output(&t));
    let failed = match state.get("status").and_then(Json::as_str) {
        Some("error") => Some(true),
        Some("completed") => Some(false),
        _ => None,
    };
    Some(ToolCall {
        name: part.get("tool").and_then(Json::as_str).unwrap_or("工具").to_string(),
        summary,
        output,
        failed,
        // ZCode 不单独记退出码，输出文本里也不保证有；不推断
        exit_code: None,
        paths: tool_paths(&input),
    })
}

fn zcode_tokens(value: Option<&Json>) -> Option<TokenUsage> {
    let value = value?;
    let number = |v: Option<&Json>| v.and_then(Json::as_i64);
    let usage = TokenUsage {
        total: number(value.get("total")),
        input: number(value.get("input")),
        output: number(value.get("output")),
        reasoning: number(value.get("reasoning")),
        cache_read: number(value.pointer("/cache/read")),
        cache_write: number(value.pointer("/cache/write")),
    };
    (usage.total.is_some() || usage.input.is_some() || usage.output.is_some()).then_some(usage)
}

fn zcode_add_tokens(into: &mut TokenUsage, value: Option<&Json>) {
    let Some(usage) = zcode_tokens(value) else { return };
    let acc = |slot: &mut Option<i64>, add: Option<i64>| {
        if let Some(add) = add { *slot = Some(slot.unwrap_or(0) + add); }
    };
    acc(&mut into.total, usage.total);
    acc(&mut into.input, usage.input);
    acc(&mut into.output, usage.output);
    acc(&mut into.reasoning, usage.reasoning);
    acc(&mut into.cache_read, usage.cache_read);
    acc(&mut into.cache_write, usage.cache_write);
}

#[derive(Default)]
struct ZcodeStream {
    messages: Vec<Message>,
    /// 达到 part 上限、只解析了最近的部分
    truncated: bool,
    /// `data` 坏了被跳过的消息条数
    skipped: usize,
}

/// 按会话 id 走索引重建消息流。
///
/// 索引：`message(session_id, sequence, ...)`（`message_session_sequence_idx`）与
/// `part(message_id, sequence, ...)`（`part_message_sequence_idx`），不使用全表扫描。
/// 方向：先按 `sequence` 倒序取消息、再倒序还原，保证被截断时留下的是最近的对话；
/// `sequence` 可能为 NULL（旧记录），用最大值兜底让它排在末尾。
fn zcode_load(conn: &Connection, session_id: &str, limits: Limits, part_budget: usize) -> Result<ZcodeStream, String> {
    let mut messages = conn.prepare(
        "SELECT id, data FROM message WHERE session_id = ?1
         ORDER BY COALESCE(sequence, 9223372036854775807) DESC, time_created DESC, id DESC",
    ).map_err(|error| error.to_string())?;
    let mut parts = conn.prepare(
        "SELECT data FROM part WHERE message_id = ?1
         ORDER BY COALESCE(sequence, 9223372036854775807), time_created, id",
    ).map_err(|error| error.to_string())?;
    let rows = messages.query_map([session_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|error| error.to_string())?;
    let mut out = ZcodeStream::default();
    let mut used = 0usize;
    for row in rows {
        let (message_id, data) = row.map_err(|error| error.to_string())?;
        if used >= part_budget {
            out.truncated = true;
            break;
        }
        let rows: Vec<String> = parts.query_map([&message_id], |r| r.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .collect();
        // 单条消息自己就超过上限时也要留下内容（否则最新一次对话会整段消失）：
        // 只保留它最近的 `part_budget` 条 part，并标记截断
        let rows = if rows.len() > part_budget && out.messages.is_empty() {
            out.truncated = true;
            rows[rows.len() - part_budget..].to_vec()
        } else {
            rows
        };
        if used + rows.len() > part_budget {
            out.truncated = true;
            break;
        }
        used += rows.len();
        match zcode_message(&data, &rows, limits) {
            Ok(Some(message)) => out.messages.push(message),
            Ok(None) => {}
            // 单条 data 解析失败只跳过该条，不牵连整页
            Err(()) => out.skipped += 1,
        }
    }
    out.messages.reverse();
    Ok(out)
}

/// 库型来源的分页：正文来自 ZCode 库，`models` / 费用仍来自 Jolu 自己的请求表
fn zcode_page(conn: &Connection, zdb: Option<&Connection>, session_id: &str, before: Option<usize>, limit: usize) -> Result<MessagePage, String> {
    zcode_page_with(conn, zdb, session_id, before, limit, ZCODE_PART_LIMIT)
}

fn zcode_page_with(conn: &Connection, zdb: Option<&Connection>, session_id: &str, before: Option<usize>, limit: usize, part_budget: usize) -> Result<MessagePage, String> {
    // 费用与模型来自 Jolu 自己的请求表（用带前缀的原始 session_id），正文来自 ZCode 的裸 id
    let (cost, priced_requests, requests, models) = session_cost(conn, "zcode", session_id).map_err(|error| error.to_string())?;
    let raw_id = zcode_raw_session_id(session_id);
    let (stream, available) = match zdb {
        Some(zdb) if zcode_schema_ok(zdb) => (zcode_load(zdb, raw_id, VIEW, part_budget)?, true),
        _ => (ZcodeStream::default(), false),
    };
    let mut note = Vec::new();
    if stream.truncated {
        note.push(format!("只解析了本会话最近 {ZCODE_PART_LIMIT} 条记录，更早的正文请在原工具查看"));
    }
    if stream.skipped > 0 {
        note.push(format!("{} 条记录格式无法解析，已跳过", stream.skipped));
    }
    let total = stream.messages.len();
    let end = before.unwrap_or(total).min(total);
    let start = end.saturating_sub(limit);
    Ok(MessagePage {
        available,
        total,
        offset: start,
        messages: stream.messages[start..end].to_vec(),
        models,
        cost,
        priced_requests,
        requests,
        note: (!note.is_empty()).then(|| note.join("；")),
    })
}

/// 消息分页：`before` 为 None 取最后 `limit` 条，否则取序号 `before` 之前的 `limit` 条
pub fn messages(conn: &Connection, platform: &str, session_id: &str, before: Option<usize>, limit: usize) -> Result<MessagePage, String> {
    // 库型来源：正文在 ZCode 自己的 sqlite 里，打不开/结构不符时照常返回「仅用量」，不报错
    if platform == "zcode" {
        let zdb = open_zcode_db();
        return zcode_page(conn, zdb.as_ref(), session_id, before, limit);
    }
    let files = source_files(conn).map_err(|error| error.to_string())?;
    let file = files.get(&(platform.to_string(), session_id.to_string())).filter(|p| p.exists()).cloned();
    let all = match (&file, platform) {
        (Some(p), "claude") => parse_claude(read_lines(p)?),
        (Some(p), "codex") => parse_codex(read_lines(p)?),
        _ => Vec::new(),
    };
    let total = all.len();
    let end = before.unwrap_or(total).min(total);
    let start = end.saturating_sub(limit);
    let (cost, priced_requests, requests, models) = session_cost(conn, platform, session_id).map_err(|error| error.to_string())?;
    Ok(MessagePage {
        available: file.is_some(),
        total,
        offset: start,
        messages: all[start..end].to_vec(),
        models,
        cost,
        priced_requests,
        requests,
        note: None,
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

    #[test]
    fn message_read_failure_is_not_reported_as_partial_success() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("jolu-message-read-{}-{nonce}", std::process::id()));
        let folder = root.join(".claude");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("broken-session.jsonl");
        let mut bytes = b"{\"type\":\"user\",\"message\":{\"content\":\"valid first line\"}}\n".to_vec();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(read_lines(&path).unwrap().len(), 1);
        bytes.extend_from_slice(&[0xff, b'\n']);
        std::fs::write(&path, bytes).unwrap();
        let conn = crate::db::open(Path::new(":memory:")).unwrap();
        conn.execute("INSERT INTO scan_state(path, offset, mtime, parser_session_id) VALUES(?1, 0, 0, 'broken-session')", [path.to_string_lossy().as_ref()]).unwrap();
        let result = messages(&conn, "claude", "broken-session", None, 40);
        std::fs::remove_file(&path).unwrap();
        assert!(read_lines(&path).is_err(), "missing files must not become an empty successful read");
        std::fs::remove_dir(&folder).unwrap();
        std::fs::remove_dir(&root).unwrap();
        assert!(result.is_err(), "a file read failure must not return a successful partial conversation");
    }

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

    const CLAUDE_FIXTURE: &str = include_str!("../tests/fixtures/handoff/claude-compacted.jsonl");
    const CODEX_FIXTURE: &str = include_str!("../tests/fixtures/handoff/codex-commands.jsonl");

    fn fixture(text: &str) -> Vec<String> {
        text.lines().map(String::from).collect()
    }

    #[test]
    fn claude_tool_failure_exit_code_and_uuid_dedupe() {
        let msgs = parse_claude(fixture(CLAUDE_FIXTURE));
        // 压缩后按原 uuid 重写的 u1 被去重；两条摘要记录不进入消息
        assert_eq!(msgs.iter().map(|m| m.role.as_str()).collect::<Vec<_>>(), ["user", "assistant", "user", "assistant", "user"]);
        assert_eq!(msgs.iter().filter(|m| m.text == "占位：用户第一条要求").count(), 1);
        let bash = &msgs[1].tools[0];
        assert_eq!((bash.failed, bash.exit_code), (Some(true), Some(1)));
        let edit = &msgs[1].tools[1];
        assert_eq!(edit.paths, vec!["D:/placeholder/project/src/a.ts".to_string()]);
        assert_eq!((edit.failed, edit.exit_code), (None, None), "没有 is_error 时不推断");
        let ls = &msgs[3].tools[0];
        assert_eq!((ls.failed, ls.exit_code), (Some(false), None));
        // 详细解析与会话页序号一致
        assert_eq!(parse_detail("claude", &fixture(CLAUDE_FIXTURE)).len(), msgs.len());
    }

    #[test]
    fn codex_detail_reads_command_events() {
        let view = parse_codex(fixture(CODEX_FIXTURE));
        let detail = parse_detail("codex", &fixture(CODEX_FIXTURE));
        assert_eq!(view.len(), 2);
        assert_eq!(detail.len(), view.len(), "命令事件不新增消息");
        // 会话页显示不变：只有 exec 与 exec_command 两个调用
        assert_eq!(view[1].tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["exec", "exec_command"]);
        let tools = &detail[1].tools;
        assert_eq!(tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["exec", "command_execution", "apply_patch", "exec_command"]);
        assert_eq!((tools[0].failed, tools[0].exit_code), (Some(true), None), "Script failed");
        assert_eq!(tools[0].summary, "占位脚本第一行\n占位脚本第二行");
        assert_eq!(view[1].tools[0].summary, "占位脚本第一行");
        assert_eq!((tools[1].summary.as_str(), tools[1].failed, tools[1].exit_code), ("pnpm test", Some(true), Some(1)));
        assert_eq!(tools[2].paths, vec!["src/a.ts".to_string()]);
        assert_eq!((tools[3].summary.as_str(), tools[3].failed, tools[3].exit_code), ("cargo check", Some(false), Some(0)));
    }

    #[test]
    fn detail_output_keeps_head_and_tail() {
        let long: String = (0..10_000).map(|i| format!("行{i}\n")).collect();
        let clipped = DETAIL.output(&long);
        assert!(clipped.starts_with("行0\n") && clipped.trim_end().ends_with("行9999"));
        assert!(clipped.contains("中间省略"));
        assert!(VIEW.output(&long).ends_with('…'));
    }

    #[test]
    fn unsafe_session_ids_are_rejected() {
        assert!(safe_id("0f8e2c5a-1b2c-4d5e-8f90-123456789abc"));
        assert!(!safe_id("x & calc"));
        assert!(!safe_id(""));
    }

    /* ------------------------------ ZCode 库型来源 ------------------------------ */

    const ZSESSION: &str = "session_fixture";

    /// 内存里的小 sqlite：只建 ZCode 的 session / message / part 三张表和两条索引，
    /// 断言不依赖本机那份 477 MB 的库。
    fn zcode_fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, parent_id TEXT, title TEXT NOT NULL DEFAULT '', directory TEXT NOT NULL DEFAULT '');
             CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL, sequence INTEGER);
             CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT NOT NULL, session_id TEXT NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL, sequence INTEGER);
             CREATE INDEX message_session_sequence_idx ON message(session_id, sequence, time_created, id);
             CREATE INDEX part_message_sequence_idx ON part(message_id, sequence, time_created, id);",
        ).unwrap();
        conn
    }

    fn zcode_message_row(conn: &Connection, id: &str, sequence: i64, ts: i64, data: &str) {
        conn.execute(
            "INSERT INTO message(id, session_id, time_created, time_updated, data, sequence) VALUES(?1, ?2, ?3, ?3, ?4, ?5)",
            rusqlite::params![id, ZSESSION, ts, data, sequence],
        ).unwrap();
    }

    fn zcode_part_row(conn: &Connection, id: &str, message: &str, sequence: i64, data: &str) {
        conn.execute(
            "INSERT INTO part(id, message_id, session_id, time_created, time_updated, data, sequence) VALUES(?1, ?2, ?3, 0, 0, ?4, ?5)",
            rusqlite::params![id, message, ZSESSION, data, sequence],
        ).unwrap();
    }

    fn jolu_store() -> Connection {
        crate::db::open(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn zcode_stream_keeps_text_reasoning_tools_and_hides_injected_messages() {
        let zdb = zcode_fixture();
        zcode_message_row(&zdb, "m1", 0, 1_000, r#"{"role":"user","time":{"created":1000},"semantics":{"uiVisibility":"visible","transcriptVisibility":"visible"}}"#);
        zcode_part_row(&zdb, "m1p1", "m1", 0, r#"{"type":"text","text":"把列表和详情放在同一页"}"#);
        // 系统提醒：模型可见、记录里不可见，不显示
        zcode_message_row(&zdb, "m2", 1, 1_100, r#"{"role":"user","time":{"created":1100},"synthetic":true,"metadata":{"visibility":"model-only"},"semantics":{"uiVisibility":"hidden","transcriptVisibility":"hidden"}}"#);
        zcode_part_row(&zdb, "m2p1", "m2", 0, r#"{"type":"text","text":"agent_listing_delta"}"#);
        zcode_message_row(&zdb, "m3", 2, 1_200, r#"{"role":"assistant","time":{"created":1200,"completed":1500},"modelId":"GLM-5.3","providerId":"account:zai-start-plan","cost":0.5,"tokens":{"total":120,"input":100,"output":20,"reasoning":5,"cache":{"read":30,"write":0}}}"#);
        zcode_part_row(&zdb, "m3p1", "m3", 0, r#"{"type":"step-start"}"#);
        zcode_part_row(&zdb, "m3p2", "m3", 1, r#"{"type":"reasoning","text":"先看现状再改"}"#);
        zcode_part_row(&zdb, "m3p3", "m3", 2, r#"{"type":"text","text":"已整理为左右分栏"}"#);
        zcode_part_row(&zdb, "m3p4", "m3", 3, r#"{"type":"tool","callID":"call_1","tool":"Bash","state":{"status":"completed","input":{"command":"rg SessionList"},"output":"3 matches"}}"#);
        zcode_part_row(&zdb, "m3p5", "m3", 4, r#"{"type":"tool","callID":"call_2","tool":"Read","state":{"status":"error","input":{"file_path":"src/a.ts"},"output":"ENOENT"}}"#);

        let stream = zcode_load(&zdb, ZSESSION, VIEW, ZCODE_PART_LIMIT).unwrap();
        assert!(!stream.truncated);
        assert_eq!(stream.skipped, 0);
        assert_eq!(stream.messages.iter().map(|m| m.role.as_str()).collect::<Vec<_>>(), ["user", "assistant"]);
        assert_eq!(stream.messages[0].text, "把列表和详情放在同一页");
        let assistant = &stream.messages[1];
        assert_eq!(assistant.ts, Some(1_200));
        assert_eq!(assistant.text, "已整理为左右分栏");
        assert_eq!(assistant.reasoning.as_deref(), Some("先看现状再改"));
        assert_eq!(assistant.model.as_deref(), Some("GLM-5.3"));
        assert_eq!(assistant.provider.as_deref(), Some("account:zai-start-plan"));
        assert_eq!(assistant.cost, Some(0.5));
        let tokens = assistant.tokens.as_ref().unwrap();
        assert_eq!((tokens.total, tokens.input, tokens.cache_read), (Some(120), Some(100), Some(30)));
        let tools = &assistant.tools;
        assert_eq!(tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Bash", "Read"]);
        assert_eq!((tools[0].summary.as_str(), tools[0].output.as_deref(), tools[0].failed), ("rg SessionList", Some("3 matches"), Some(false)));
        assert_eq!((tools[1].failed, tools[1].output.as_deref()), (Some(true), Some("ENOENT")));
        assert_eq!(tools[1].paths, vec!["src/a.ts".to_string()]);
    }

    #[test]
    fn zcode_tokens_fall_back_to_step_finish_parts() {
        let zdb = zcode_fixture();
        // 消息级没有 tokens / cost 时用 step-finish 结算节点累加，仍缺失就是 None
        zcode_message_row(&zdb, "m1", 0, 1_000, r#"{"role":"assistant","modelId":"GLM-5.3"}"#);
        zcode_part_row(&zdb, "m1p1", "m1", 0, r#"{"type":"text","text":"第一段"}"#);
        zcode_part_row(&zdb, "m1p2", "m1", 1, r#"{"type":"step-finish","cost":0.25,"tokens":{"total":10,"input":8,"output":2,"cache":{"read":4,"write":0}}}"#);
        zcode_part_row(&zdb, "m1p3", "m1", 2, r#"{"type":"step-finish","cost":0.75,"tokens":{"total":20,"input":16,"output":4,"cache":{"read":0,"write":0}}}"#);
        let stream = zcode_load(&zdb, ZSESSION, VIEW, ZCODE_PART_LIMIT).unwrap();
        let message = &stream.messages[0];
        let tokens = message.tokens.as_ref().expect("step-finish 应能累加 tokens");
        assert_eq!((tokens.total, tokens.input, tokens.output, tokens.cache_read), (Some(30), Some(24), Some(6), Some(4)));
        assert_eq!(message.cost, Some(1.0));
    }

    #[test]
    fn zcode_part_budget_keeps_the_newest_messages_and_flags_truncation() {
        let zdb = zcode_fixture();
        for i in 0..3i64 {
            let id = format!("m{i}");
            zcode_message_row(&zdb, &id, i, 1_000 + i, r#"{"role":"user"}"#);
            zcode_part_row(&zdb, &format!("{id}p0"), &id, 0, &format!(r#"{{"type":"text","text":"消息 {i} 前半"}}"#));
            zcode_part_row(&zdb, &format!("{id}p1"), &id, 1, &format!(r#"{{"type":"text","text":"消息 {i} 后半"}}"#));
        }
        let stream = zcode_load(&zdb, ZSESSION, VIEW, 2).unwrap();
        assert!(stream.truncated, "超过 part 上限要标记截断");
        assert_eq!(stream.messages.len(), 1, "截断时保留最近的会话尾巴");
        assert_eq!(stream.messages[0].text, "消息 2 前半\n\n消息 2 后半");

        let jolu = jolu_store();
        let page = zcode_page_with(&jolu, Some(&zdb), ZSESSION, None, 40, 2).unwrap();
        assert!(page.available);
        assert_eq!(page.messages.len(), 1);
        assert!(page.note.as_deref().is_some_and(|note| note.contains("只解析了本会话最近")), "截断必须在页面上标注");
    }

    #[test]
    fn zcode_single_oversized_message_still_shows_its_latest_parts() {
        let zdb = zcode_fixture();
        // 一条消息自己就超过上限：留下它最近的 part，而不是让最新一次对话整段消失
        zcode_message_row(&zdb, "m1", 0, 1_000, r#"{"role":"user"}"#);
        for i in 0..4i64 {
            zcode_part_row(&zdb, &format!("p{i}"), "m1", i, &format!(r#"{{"type":"text","text":"段 {i}"}}"#));
        }
        let stream = zcode_load(&zdb, ZSESSION, VIEW, 2).unwrap();
        assert!(stream.truncated);
        assert_eq!(stream.messages.len(), 1);
        assert_eq!(stream.messages[0].text, "段 2\n\n段 3");
    }

    #[test]
    fn zcode_malformed_rows_degrade_instead_of_failing_the_page() {
        let zdb = zcode_fixture();
        zcode_message_row(&zdb, "bad", 0, 1_000, "{not json");
        zcode_message_row(&zdb, "m1", 1, 1_100, r#"{"role":"user"}"#);
        zcode_part_row(&zdb, "m1p1", "m1", 0, r#"{"type":"text","text":"还能打开"}"#);
        zcode_message_row(&zdb, "m2", 2, 1_200, r#"{"role":"user"}"#);
        // 单条 part 坏掉只丢这一条，不影响同一条消息的其他内容和整页
        zcode_part_row(&zdb, "m2p1", "m2", 0, "not json either");
        zcode_part_row(&zdb, "m2p2", "m2", 1, r#"{"type":"text","text":"好的"}"#);

        let jolu = jolu_store();
        let page = zcode_page_with(&jolu, Some(&zdb), ZSESSION, None, 40, ZCODE_PART_LIMIT).unwrap();
        assert!(page.available);
        assert_eq!(page.total, 2);
        assert_eq!(page.messages[0].text, "还能打开");
        assert_eq!(page.messages[1].text, "好的");
        assert!(page.note.as_deref().is_some_and(|note| note.contains("格式无法解析")));
    }

    #[test]
    fn zcode_without_a_readable_database_reports_usage_only() {
        let jolu = jolu_store();
        // 库不存在（未安装 / 已删除）：降级为「仅用量」，不是错误页
        let page = zcode_page(&jolu, None, "session_missing", None, 40).unwrap();
        assert!(!page.available);
        assert_eq!(page.total, 0);
        assert!(page.messages.is_empty());
        assert!(page.note.is_none());
        // 结构漂移（旧版本库缺少列）：同样降级，不 panic、不猜列名
        let odd = Connection::open_in_memory().unwrap();
        odd.execute_batch("CREATE TABLE message (id TEXT PRIMARY KEY)").unwrap();
        let page = zcode_page(&jolu, Some(&odd), "session_missing", None, 40).unwrap();
        assert!(!page.available);
    }

    /// 真实库的只读冒烟：只在显式执行时跑，断言不依赖具体对话内容，也不写任何文件
    #[test]
    #[ignore = "需要本机 ZCode 会话库；仅显式只读执行"]
    fn zcode_local_database_readonly_smoke() {
        assert!(zcode_db_path().is_some_and(|path| path.is_file()), "未找到 ZCode 会话库");
        let zdb = open_zcode_db().expect("ZCode 会话库应能以只读方式打开");
        assert!(zcode_schema_ok(&zdb), "本机库结构应与探测一致");
        let raw: String = zdb
            .query_row("SELECT id FROM session WHERE parent_id IS NULL ORDER BY time_updated DESC LIMIT 1", [], |r| r.get(0))
            .expect("本机应有根会话");
        // 走真实入口：Jolu 的 session_id 带 `native:zcode:main:` 前缀，必须能还原成原库的裸 id
        let jolu = jolu_store();
        let session_id = format!("native:zcode:main:{raw}");
        let started = std::time::Instant::now();
        let page = messages(&jolu, "zcode", &session_id, None, 40).unwrap();
        let elapsed = started.elapsed().as_millis();
        println!("zcode read-only: available={} total={} page={} note={} elapsed_ms={elapsed}",
            page.available, page.total, page.messages.len(), page.note.is_some());
        assert!(page.available, "本机库应可只读解析");
        assert!(!page.messages.is_empty(), "最近的根会话应有可读正文");
        // 477 MB 的库单会话最多 14,404 条 part；按会话 id 走索引解析必须远低于一次全量扫描
        assert!(elapsed < 5_000, "单会话解析耗时过长：{elapsed} ms");
    }

    #[test]
    fn zcode_session_ids_keep_the_collector_prefix_but_query_by_raw_id() {
        assert_eq!(zcode_raw_session_id("native:zcode:main:sess_0a15"), "sess_0a15");
        assert_eq!(zcode_raw_session_id("native:zcode:ai:session_6b90"), "session_6b90");
        // 已经是裸 id 时不改动
        assert_eq!(zcode_raw_session_id("sess_0a15"), "sess_0a15");
        let children: HashSet<String> = ["sess_child".to_string()].into_iter().collect();
        assert!(zcode_child_session(Some(&children), "zcode", "native:zcode:main:sess_child"));
    }

    #[test]
    fn zcode_child_sessions_are_not_listed_as_main_sessions() {
        let children: HashSet<String> = ["session_child".to_string()].into_iter().collect();
        assert!(zcode_child_session(Some(&children), "zcode", "session_child"));
        assert!(!zcode_child_session(Some(&children), "zcode", "session_root"));
        // 只按平台过滤，其他来源的同名 id 不受影响
        assert!(!zcode_child_session(Some(&children), "claude", "session_child"));
        // 库不可读时不过滤，宁可多显示也不要吞掉真实会话
        assert!(!zcode_child_session(None, "zcode", "session_child"));
    }
}
