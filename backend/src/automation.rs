//! 自动化（Jolu.pen 模块 E，画板 58–73）：任务、运行记录与本机调度。
//!
//! - 执行器：Claude Code `claude -p --output-format stream-json --verbose`，Codex `codex exec --json`。
//!   提示词走标准输入，避免命令行转义；不传任何绕过权限或沙箱的参数，文件修改权限沿用执行器自己的配置。
//! - 执行账号：不切换全局登录、不注入凭证；所选连接必须是该工具当前登录的连接，否则进入「需要处理」。
//! - 额度：复用供应商页的额度查询；检查失败不当作额度耗尽，间隔重试并逐步拉长。
//! - 任务、队列与运行记录全部落库；重启后把中断的运行标为「需要处理」，不自动从头重跑。

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use chrono::{Datelike, Local, NaiveDate, NaiveTime, TimeZone};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use tauri::{AppHandle, Emitter, Manager};

use crate::quota::QuotaState;

type Shared = Arc<Mutex<Connection>>;

/// 前端订阅的变化事件：任务或运行状态变化后发出
pub const CHANGED: &str = "automation-changed";
const TICK: Duration = Duration::from_secs(30);
/// 定时触发超过这个时长才被发现（休眠、关机、Jolu 未运行）时标「已错过」，由用户补跑
const MISSED_GRACE_MS: i64 = 10 * 60_000;
const PAGE: i64 = 6;
const LOG_LIMIT: i64 = 200;
/// 额度恢复后在原会话里继续时追加的指令
const RESUME_PROMPT: &str = "额度已恢复。请从中断处继续完成上一条任务，不要重复已经完成的修改；完成后汇总改动与验证结果。";
const ACTIVE: &str = "('queued','waiting','running','attention')";

fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

fn clip(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit { return text.to_string(); }
    format!("{}…", text.chars().take(limit).collect::<String>())
}

pub fn tool_name(platform: &str) -> Option<&'static str> {
    match platform {
        "claude" => Some("claude"),
        "codex" => Some("codex"),
        _ => None,
    }
}

fn display_name(platform: &str) -> &'static str {
    if platform == "codex" { "Codex" } else { "Claude Code" }
}

/* ─────────────────────────── 数据表 ─────────────────────────── */

pub fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS automation_tasks (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            name          TEXT    NOT NULL,
            platform      TEXT    NOT NULL,
            connection_id TEXT    NOT NULL,
            model         TEXT,
            cwd           TEXT    NOT NULL,
            session_id    TEXT,
            prompt        TEXT    NOT NULL,
            skills        TEXT    NOT NULL DEFAULT '[]',
            launch        TEXT    NOT NULL,
            schedule      TEXT,
            auto_resume   INTEGER NOT NULL DEFAULT 1,
            wait_deadline INTEGER,
            state         TEXT    NOT NULL DEFAULT 'active',
            next_run_at   INTEGER,
            missed_at     INTEGER,
            created_at    INTEGER NOT NULL,
            updated_at    INTEGER NOT NULL,
            serial_queue  INTEGER NOT NULL DEFAULT 1
        );
        CREATE TABLE IF NOT EXISTS automation_runs (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id         INTEGER NOT NULL,
            source          TEXT    NOT NULL,
            status          TEXT    NOT NULL,
            block_kind      TEXT,
            reason          TEXT,
            resets_at       INTEGER,
            last_check_at   INTEGER,
            next_check_at   INTEGER,
            check_failures  INTEGER NOT NULL DEFAULT 0,
            resume_pending  INTEGER NOT NULL DEFAULT 0,
            session_id      TEXT,
            account         TEXT,
            model           TEXT,
            created_at      INTEGER NOT NULL,
            started_at      INTEGER,
            ended_at        INTEGER,
            exec_ms         INTEGER NOT NULL DEFAULT 0,
            wait_ms         INTEGER NOT NULL DEFAULT 0,
            exec_started_at INTEGER,
            wait_started_at INTEGER,
            waits           INTEGER NOT NULL DEFAULT 0,
            input_tokens    INTEGER,
            cache_read_tokens INTEGER,
            output_tokens   INTEGER,
            result          TEXT,
            files           TEXT    NOT NULL DEFAULT '[]',
            error           TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_automation_runs_task ON automation_runs(task_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_automation_runs_created ON automation_runs(created_at DESC);
        CREATE TABLE IF NOT EXISTS automation_events (
            id     INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id INTEGER NOT NULL,
            ts     INTEGER NOT NULL,
            kind   TEXT    NOT NULL,
            title  TEXT    NOT NULL,
            detail TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_automation_events_run ON automation_events(run_id, id);
        "#,
    )?;
    // 任务 2.6：旧库补「同账号排队」列，默认开启
    let has_serial = conn.prepare("SELECT 1 FROM pragma_table_info('automation_tasks') WHERE name = 'serial_queue'")?.exists([])?;
    if !has_serial {
        conn.execute("ALTER TABLE automation_tasks ADD COLUMN serial_queue INTEGER NOT NULL DEFAULT 1", [])?;
    }
    Ok(())
}

/* ─────────────────────────── 结构 ─────────────────────────── */

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Schedule {
    /// once / daily / weekly
    pub freq: String,
    /// HH:MM，按本机时区
    pub time: String,
    /// 1 = 周一 … 7 = 周日，仅 weekly
    pub weekday: Option<u32>,
    /// YYYY-MM-DD，仅 once
    pub date: Option<String>,
    pub timezone: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskInput {
    pub name: String,
    pub platform: String,
    pub connection_id: String,
    pub model: Option<String>,
    pub cwd: String,
    /// 选择已有会话时在原会话追加；为空表示新建会话
    pub session_id: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub skills: Vec<String>,
    /// now / schedule / quota
    pub launch: String,
    pub schedule: Option<Schedule>,
    pub auto_resume: bool,
    pub wait_deadline: Option<i64>,
    /// 同账号排队（D-09），缺省为开启
    #[serde(default = "yes")]
    pub serial_queue: bool,
}

fn yes() -> bool { true }

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: i64,
    pub name: String,
    pub platform: String,
    pub connection_id: String,
    pub model: Option<String>,
    pub cwd: String,
    pub session_id: Option<String>,
    pub prompt: String,
    pub skills: Vec<String>,
    pub launch: String,
    pub schedule: Option<Schedule>,
    pub auto_resume: bool,
    pub wait_deadline: Option<i64>,
    /// active / paused / finished / cancelled
    pub state: String,
    pub next_run_at: Option<i64>,
    pub missed_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    /// 同账号排队（D-09）：关闭后可与同账号任务并行
    pub serial_queue: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Run {
    pub id: i64,
    pub task_id: i64,
    pub task_name: String,
    pub platform: String,
    pub connection_id: String,
    /// now / schedule / quota / manual
    pub source: String,
    /// queued / waiting / running / attention / completed / failed / cancelled
    pub status: String,
    pub block_kind: Option<String>,
    pub reason: Option<String>,
    pub resets_at: Option<i64>,
    pub last_check_at: Option<i64>,
    pub next_check_at: Option<i64>,
    pub check_failures: i64,
    pub resume_pending: bool,
    pub session_id: Option<String>,
    pub account: Option<String>,
    pub model: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub exec_ms: i64,
    pub wait_ms: i64,
    pub exec_started_at: Option<i64>,
    pub wait_started_at: Option<i64>,
    pub waits: i64,
    /// 含缓存的全部输入；执行器没有返回时为 None，不以 0 代替
    pub input_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub result: Option<String>,
    pub files: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub ts: i64,
    pub kind: String,
    pub title: String,
    pub detail: Option<String>,
}

const TASK_COLS: &str = "id, name, platform, connection_id, model, cwd, session_id, prompt, skills, launch, schedule, auto_resume, wait_deadline, state, next_run_at, missed_at, created_at, updated_at, serial_queue";

fn task_from(r: &rusqlite::Row) -> rusqlite::Result<Task> {
    let skills: String = r.get(8)?;
    let schedule: Option<String> = r.get(10)?;
    Ok(Task {
        id: r.get(0)?,
        name: r.get(1)?,
        platform: r.get(2)?,
        connection_id: r.get(3)?,
        model: r.get(4)?,
        cwd: r.get(5)?,
        session_id: r.get(6)?,
        prompt: r.get(7)?,
        skills: serde_json::from_str(&skills).unwrap_or_default(),
        launch: r.get(9)?,
        schedule: schedule.and_then(|s| serde_json::from_str(&s).ok()),
        auto_resume: r.get::<_, i64>(11)? != 0,
        wait_deadline: r.get(12)?,
        state: r.get(13)?,
        next_run_at: r.get(14)?,
        missed_at: r.get(15)?,
        created_at: r.get(16)?,
        updated_at: r.get(17)?,
        serial_queue: r.get::<_, i64>(18)? != 0,
    })
}

const RUN_COLS: &str = "r.id, r.task_id, t.name, t.platform, t.connection_id, r.source, r.status, r.block_kind, r.reason, r.resets_at, r.last_check_at, r.next_check_at, r.check_failures, r.resume_pending, r.session_id, r.account, r.model, r.created_at, r.started_at, r.ended_at, r.exec_ms, r.wait_ms, r.exec_started_at, r.wait_started_at, r.waits, r.input_tokens, r.cache_read_tokens, r.output_tokens, r.result, r.files, r.error";

fn run_from(r: &rusqlite::Row) -> rusqlite::Result<Run> {
    let files: String = r.get(29)?;
    Ok(Run {
        id: r.get(0)?,
        task_id: r.get(1)?,
        task_name: r.get(2)?,
        platform: r.get(3)?,
        connection_id: r.get(4)?,
        source: r.get(5)?,
        status: r.get(6)?,
        block_kind: r.get(7)?,
        reason: r.get(8)?,
        resets_at: r.get(9)?,
        last_check_at: r.get(10)?,
        next_check_at: r.get(11)?,
        check_failures: r.get(12)?,
        resume_pending: r.get::<_, i64>(13)? != 0,
        session_id: r.get(14)?,
        account: r.get(15)?,
        model: r.get(16)?,
        created_at: r.get(17)?,
        started_at: r.get(18)?,
        ended_at: r.get(19)?,
        exec_ms: r.get(20)?,
        wait_ms: r.get(21)?,
        exec_started_at: r.get(22)?,
        wait_started_at: r.get(23)?,
        waits: r.get(24)?,
        input_tokens: r.get(25)?,
        cache_read_tokens: r.get(26)?,
        output_tokens: r.get(27)?,
        result: r.get(28)?,
        files: serde_json::from_str(&files).unwrap_or_default(),
        error: r.get(30)?,
    })
}

fn load_task(conn: &Connection, id: i64) -> rusqlite::Result<Option<Task>> {
    conn.query_row(&format!("SELECT {TASK_COLS} FROM automation_tasks WHERE id = ?1"), [id], task_from).optional()
}

fn load_run(conn: &Connection, id: i64) -> rusqlite::Result<Option<Run>> {
    conn.query_row(
        &format!("SELECT {RUN_COLS} FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id WHERE r.id = ?1"),
        [id],
        run_from,
    )
    .optional()
}

/// 任务当前进行中的运行（排队 / 等待 / 执行中 / 需要处理）
fn active_run(conn: &Connection, task_id: i64) -> rusqlite::Result<Option<Run>> {
    conn.query_row(
        &format!("SELECT {RUN_COLS} FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id WHERE r.task_id = ?1 AND r.status IN {ACTIVE} ORDER BY r.created_at DESC, r.id DESC LIMIT 1"),
        [task_id],
        run_from,
    )
    .optional()
}

fn last_run(conn: &Connection, task_id: i64) -> rusqlite::Result<Option<Run>> {
    conn.query_row(
        &format!("SELECT {RUN_COLS} FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id WHERE r.task_id = ?1 ORDER BY r.created_at DESC, r.id DESC LIMIT 1"),
        [task_id],
        run_from,
    )
    .optional()
}

fn add_event(conn: &Connection, run_id: i64, now: i64, kind: &str, title: &str, detail: Option<&str>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO automation_events (run_id, ts, kind, title, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![run_id, now, kind, title, detail],
    )?;
    Ok(())
}

fn events_of(conn: &Connection, run_id: i64, logs: bool) -> rusqlite::Result<Vec<Event>> {
    let sql = if logs {
        "SELECT ts, kind, title, detail FROM (SELECT * FROM automation_events WHERE run_id = ?1 AND kind = 'log' ORDER BY id DESC LIMIT 60) ORDER BY id"
    } else {
        "SELECT ts, kind, title, detail FROM automation_events WHERE run_id = ?1 AND kind != 'log' ORDER BY id"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([run_id], |r| Ok(Event { ts: r.get(0)?, kind: r.get(1)?, title: r.get(2)?, detail: r.get(3)? }))?;
    rows.collect()
}

fn connection_name(conn: &Connection, id: &str) -> Option<String> {
    conn.query_row("SELECT name FROM connections WHERE id = ?1", [id], |r| r.get::<_, String>(0)).optional().ok().flatten()
}

/* ─────────────────────────── 定时 ─────────────────────────── */

/// 下一次触发时间（毫秒）；`after_ms` 之后最近的一次，单次任务已过去时返回 None
pub fn next_fire<Tz: TimeZone>(tz: &Tz, s: &Schedule, after_ms: i64) -> Option<i64> {
    let time = NaiveTime::parse_from_str(&s.time, "%H:%M").ok()?;
    let at = |date: NaiveDate| tz.from_local_datetime(&date.and_time(time)).earliest().map(|d| d.timestamp_millis());
    match s.freq.as_str() {
        "once" => {
            let date = NaiveDate::parse_from_str(s.date.as_deref()?, "%Y-%m-%d").ok()?;
            at(date).filter(|ms| *ms > after_ms)
        }
        "daily" | "weekly" => {
            let start = tz.timestamp_millis_opt(after_ms).single()?.date_naive();
            (0..=8u64)
                .filter_map(|d| start.checked_add_days(chrono::Days::new(d)))
                .filter(|d| s.freq == "daily" || Some(d.weekday().number_from_monday()) == s.weekday)
                .filter_map(at)
                .find(|ms| *ms > after_ms)
        }
        _ => None,
    }
}

/* ─────────────────────────── 校验与新建 ─────────────────────────── */

fn safe_model(model: &str) -> bool {
    !model.is_empty() && model.len() <= 120 && model.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/' | '[' | ']'))
}

pub fn validate(input: &TaskInput, now: i64) -> Result<(), String> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 80 { return Err("任务名称应为 1 到 80 个字符".into()); }
    if tool_name(&input.platform).is_none() { return Err("该工具尚未接入执行器".into()); }
    if input.connection_id.trim().is_empty() { return Err("请选择账号后再创建任务。".into()); }
    if let Some(model) = input.model.as_deref().filter(|m| !m.is_empty()) {
        if !safe_model(model) { return Err("模型名称包含不支持的字符".into()); }
    }
    if !Path::new(input.cwd.trim()).is_dir() { return Err("项目目录不存在或不可访问".into()); }
    if let Some(id) = input.session_id.as_deref() {
        if !crate::session_threads::safe_id(id) { return Err("会话 ID 无效".into()); }
    }
    if input.prompt.trim().is_empty() { return Err("请填写执行内容".into()); }
    if input.prompt.chars().count() > 20_000 { return Err("执行内容不能超过 20000 个字符".into()); }
    if input.skills.len() > MAX_SKILLS { return Err(format!("最多选择 {MAX_SKILLS} 个 Skills")); }
    if let Some(bad) = input.skills.iter().find(|s| !crate::skills::valid_name(s)) { return Err(format!("Skill 名称无效：{bad}")); }
    if input.skills.iter().collect::<HashSet<_>>().len() != input.skills.len() { return Err("Skills 不能重复选择".into()); }
    match input.launch.as_str() {
        "now" | "quota" => {}
        "schedule" => {
            let s = input.schedule.as_ref().ok_or("请设置执行时间")?;
            NaiveTime::parse_from_str(&s.time, "%H:%M").map_err(|_| "执行时间格式应为 HH:MM")?;
            match s.freq.as_str() {
                "once" => {
                    if next_fire(&Local, s, now).is_none() { return Err("单次执行时间已经过去".into()); }
                }
                "daily" => {}
                "weekly" => {
                    if !s.weekday.is_some_and(|d| (1..=7).contains(&d)) { return Err("请选择每周执行的星期".into()); }
                }
                _ => return Err("不支持的执行频率".into()),
            }
        }
        _ => return Err("不支持的启动方式".into()),
    }
    if input.wait_deadline.is_some_and(|d| d <= now) { return Err("等待截止时间应晚于现在".into()); }
    Ok(())
}

fn insert_run(conn: &Connection, task: &Task, source: &str, status: &str, now: i64) -> rusqlite::Result<i64> {
    let account = connection_name(conn, &task.connection_id);
    let waiting = status == "waiting";
    conn.execute(
        "INSERT INTO automation_runs (task_id, source, status, block_kind, reason, next_check_at, wait_started_at, session_id, account, model, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            task.id, source, status,
            waiting.then_some("quota"),
            waiting.then_some("等待检查额度"),
            waiting.then_some(now),
            waiting.then_some(now),
            task.session_id, account, task.model, now
        ],
    )?;
    let id = conn.last_insert_rowid();
    let (kind, title) = match status {
        "waiting" => ("wait", "创建任务，等待额度"),
        _ => ("queue", "进入执行队列"),
    };
    add_event(conn, id, now, kind, title, None)?;
    Ok(id)
}

pub fn create_task(conn: &Connection, input: &TaskInput, now: i64) -> Result<i64, String> {
    validate(input, now)?;
    let next = match (input.launch.as_str(), input.schedule.as_ref()) {
        ("schedule", Some(s)) => next_fire(&Local, s, now),
        _ => None,
    };
    let e = |e: rusqlite::Error| e.to_string();
    conn.execute(
        "INSERT INTO automation_tasks (name, platform, connection_id, model, cwd, session_id, prompt, skills, launch, schedule, auto_resume, wait_deadline, state, next_run_at, created_at, updated_at, serial_queue)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'active', ?13, ?14, ?14, ?15)",
        params![
            input.name.trim(), input.platform, input.connection_id,
            input.model.as_deref().filter(|m| !m.is_empty()),
            input.cwd.trim(), input.session_id,
            input.prompt.trim(),
            serde_json::to_string(&input.skills).unwrap_or_else(|_| "[]".into()),
            input.launch,
            input.schedule.as_ref().and_then(|s| serde_json::to_string(s).ok()),
            input.auto_resume as i64, input.wait_deadline, next, now, input.serial_queue as i64
        ],
    ).map_err(e)?;
    let id = conn.last_insert_rowid();
    let task = load_task(conn, id).map_err(e)?.ok_or("任务保存失败")?;
    match input.launch.as_str() {
        "now" => { insert_run(conn, &task, "now", "queued", now).map_err(e)?; }
        "quota" => { insert_run(conn, &task, "quota", "waiting", now).map_err(e)?; }
        _ => {}
    }
    Ok(id)
}

/// 编辑只影响后续执行：不改动正在进行的运行
pub fn update_task(conn: &Connection, id: i64, input: &TaskInput, now: i64) -> Result<(), String> {
    validate(input, now)?;
    let task = load_task(conn, id).map_err(|e| e.to_string())?.ok_or("任务不存在")?;
    if task.state == "cancelled" { return Err("任务已取消，不能再编辑".into()); }
    let next = match (input.launch.as_str(), input.schedule.as_ref()) {
        ("schedule", Some(s)) if task.state == "active" => next_fire(&Local, s, now),
        _ => None,
    };
    conn.execute(
        "UPDATE automation_tasks SET name=?2, platform=?3, connection_id=?4, model=?5, cwd=?6, session_id=?7, prompt=?8, skills=?9, launch=?10, schedule=?11, auto_resume=?12, wait_deadline=?13, next_run_at=?14, updated_at=?15, serial_queue=?16 WHERE id=?1",
        params![
            id, input.name.trim(), input.platform, input.connection_id,
            input.model.as_deref().filter(|m| !m.is_empty()),
            input.cwd.trim(), input.session_id, input.prompt.trim(),
            serde_json::to_string(&input.skills).unwrap_or_else(|_| "[]".into()),
            input.launch,
            input.schedule.as_ref().and_then(|s| serde_json::to_string(s).ok()),
            input.auto_resume as i64, input.wait_deadline, next, now, input.serial_queue as i64
        ],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

/* ─────────────────────────── 列表 ─────────────────────────── */

#[derive(Debug, Serialize)]
pub struct TaskItem {
    #[serde(flatten)]
    pub task: Task,
    pub active: Option<Run>,
    pub last: Option<Run>,
    /// 排队中时，同账号前方的任务数
    pub queue_ahead: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct Overview {
    pub tasks: Vec<TaskItem>,
    pub enabled: i64,
    pub running: i64,
    /// 第一个执行中的运行：工具 · 账号
    pub running_note: Option<String>,
    pub waiting: i64,
    /// 等待中最早的预计恢复时间
    pub next_reset: Option<i64>,
    pub done_today: i64,
    pub failed_today: i64,
}

fn day_start(now: i64) -> i64 {
    Local.timestamp_millis_opt(now).single()
        .and_then(|d| Local.from_local_datetime(&d.date_naive().and_hms_opt(0, 0, 0)?).earliest())
        .map(|d| d.timestamp_millis())
        .unwrap_or(now)
}

/// 同账号排在前面的排队任务数；关闭排队的任务不挡排队任务，不计入（D-09）。口径同 [`startable`]：排队与等待只看启用中的任务
fn queue_ahead(conn: &Connection, run: &Run) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id
         WHERE t.connection_id = ?1 AND (t.state = 'active' OR r.status = 'running') AND t.serial_queue = 1 AND r.id != ?2
           AND r.status IN ('queued','waiting','running')
           AND (r.status = 'running' OR r.created_at < ?3 OR (r.created_at = ?3 AND r.id < ?2))",
        params![run.connection_id, run.id, run.created_at],
        |r| r.get(0),
    )
}

fn task_item(conn: &Connection, task: Task) -> rusqlite::Result<TaskItem> {
    let active = active_run(conn, task.id)?;
    let last = last_run(conn, task.id)?;
    // 关闭排队的任务没有队列位置（只受工作目录、并行上限与额度等待限制），不给出前方任务数；显示方式待确认
    let queue_ahead = match &active {
        Some(run) if run.status == "queued" && task.serial_queue => Some(queue_ahead(conn, run)?),
        _ => None,
    };
    Ok(TaskItem { task, active, last, queue_ahead })
}

pub fn overview(conn: &Connection, now: i64) -> rusqlite::Result<Overview> {
    let mut stmt = conn.prepare(&format!("SELECT {TASK_COLS} FROM automation_tasks ORDER BY updated_at DESC, id DESC"))?;
    let tasks: Vec<Task> = stmt.query_map([], task_from)?.collect::<Result<_, _>>()?;
    let tasks: Vec<TaskItem> = tasks.into_iter().map(|t| task_item(conn, t)).collect::<Result<_, _>>()?;
    let status_of = |item: &TaskItem| item.active.as_ref().map(|r| r.status.clone());
    let enabled = tasks.iter().filter(|i| i.task.state == "active" && (i.task.launch == "schedule" || i.active.is_some())).count() as i64;
    let running: Vec<&TaskItem> = tasks.iter().filter(|i| status_of(i).as_deref() == Some("running")).collect();
    let waiting: Vec<&TaskItem> = tasks.iter().filter(|i| status_of(i).as_deref() == Some("waiting")).collect();
    let running_note = running.first().map(|i| {
        let account = i.active.as_ref().and_then(|r| r.account.clone());
        match account {
            Some(a) => format!("{} · {a}", display_name(&i.task.platform)),
            None => display_name(&i.task.platform).to_string(),
        }
    });
    let next_reset = waiting.iter().filter_map(|i| i.active.as_ref().and_then(|r| r.resets_at)).filter(|t| *t > now).min();
    let start = day_start(now);
    let count = |status: &str| -> rusqlite::Result<i64> {
        conn.query_row("SELECT COUNT(*) FROM automation_runs WHERE status = ?1 AND ended_at >= ?2", params![status, start], |r| r.get(0))
    };
    Ok(Overview {
        enabled,
        running: running.len() as i64,
        running_note,
        waiting: waiting.len() as i64,
        next_reset,
        done_today: count("completed")?,
        failed_today: count("failed")?,
        tasks,
    })
}

#[derive(Debug, Serialize)]
pub struct QueueItem {
    pub task_id: i64,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct TaskDetail {
    pub item: TaskItem,
    /// 同账号的队列（执行中 / 等待额度 / 排队中），按先后顺序；只列开启排队的任务（D-09）
    pub queue: Vec<QueueItem>,
    /// 当前（或最近一次）运行的时间线
    pub events: Vec<Event>,
}

pub fn task_detail(conn: &Connection, id: i64) -> Result<TaskDetail, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let task = load_task(conn, id).map_err(e)?.ok_or("任务不存在")?;
    let item = task_item(conn, task).map_err(e)?;
    let mut stmt = conn.prepare(
        "SELECT t.id, t.name, r.status FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id
         WHERE t.connection_id = ?1 AND (t.state = 'active' OR r.status = 'running') AND t.serial_queue = 1 AND r.status IN ('queued','waiting','running')
         ORDER BY CASE r.status WHEN 'running' THEN 0 ELSE 1 END, r.created_at, r.id",
    ).map_err(e)?;
    let queue = stmt.query_map([&item.task.connection_id], |r| Ok(QueueItem { task_id: r.get(0)?, name: r.get(1)?, status: r.get(2)? }))
        .map_err(e)?.collect::<Result<_, _>>().map_err(e)?;
    let run_id = item.active.as_ref().or(item.last.as_ref()).map(|r| r.id);
    let events = match run_id { Some(id) => events_of(conn, id, false).map_err(e)?, None => vec![] };
    Ok(TaskDetail { item, queue, events })
}

#[derive(Debug, Deserialize)]
pub struct RunFilter {
    /// today / 7d / 30d / all
    pub range: String,
    pub status: Option<String>,
    pub connection: Option<String>,
    pub task: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct RunPage {
    pub rows: Vec<Run>,
    pub total: i64,
    pub page: u32,
    pub pages: u32,
    pub completed: i64,
    /// 完成的运行里经历过额度等待的次数
    pub completed_waited: i64,
    pub running: i64,
    pub failed: i64,
    pub failed_note: Option<String>,
    /// 范围内已知 Token 合计；全部未知时为 None
    pub tokens: Option<i64>,
    pub tasks: Vec<QueueItem>,
}

fn range_start(range: &str, now: i64) -> Option<i64> {
    let start = day_start(now);
    match range {
        "today" => Some(start),
        "7d" => Some(start - 6 * 86_400_000),
        "30d" => Some(start - 29 * 86_400_000),
        _ => None,
    }
}

pub fn runs(conn: &Connection, f: &RunFilter, page: u32, now: i64) -> rusqlite::Result<RunPage> {
    let since = range_start(&f.range, now).unwrap_or(i64::MIN);
    let base = "FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id WHERE r.created_at >= ?1 AND (?2 IS NULL OR t.connection_id = ?2) AND (?3 IS NULL OR r.task_id = ?3)";
    let status_clause = "AND (?4 IS NULL OR r.status = ?4)";
    let p = params![since, f.connection, f.task, f.status];
    let total: i64 = conn.query_row(&format!("SELECT COUNT(*) {base} {status_clause}"), p, |r| r.get(0))?;
    let pages = ((total + PAGE - 1) / PAGE).max(1) as u32;
    let page = page.clamp(1, pages);
    let mut stmt = conn.prepare(&format!("SELECT {RUN_COLS} {base} {status_clause} ORDER BY r.created_at DESC, r.id DESC LIMIT {PAGE} OFFSET ?5"))?;
    let rows = stmt.query_map(params![since, f.connection, f.task, f.status, (page as i64 - 1) * PAGE], run_from)?.collect::<Result<_, _>>()?;
    let q = params![since, f.connection, f.task];
    let (completed, completed_waited, running, failed, known, tokens): (i64, i64, i64, i64, i64, i64) = conn.query_row(
        &format!(
            "SELECT COALESCE(SUM(r.status = 'completed'), 0), COALESCE(SUM(r.status = 'completed' AND r.waits > 0), 0),
                    COALESCE(SUM(r.status = 'running'), 0), COALESCE(SUM(r.status = 'failed'), 0),
                    COALESCE(SUM(r.input_tokens IS NOT NULL OR r.output_tokens IS NOT NULL), 0),
                    COALESCE(SUM(COALESCE(r.input_tokens, 0) + COALESCE(r.output_tokens, 0)), 0) {base}"
        ),
        q,
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
    )?;
    let failed_note = conn.query_row(
        &format!("SELECT COALESCE(r.error, r.reason) {base} AND r.status = 'failed' ORDER BY r.created_at DESC LIMIT 1"),
        q,
        |r| r.get::<_, Option<String>>(0),
    ).optional()?.flatten().map(|s| clip(&s, 40));
    let mut stmt = conn.prepare("SELECT id, name, state FROM automation_tasks ORDER BY name")?;
    let tasks = stmt.query_map([], |r| Ok(QueueItem { task_id: r.get(0)?, name: r.get(1)?, status: r.get(2)? }))?.collect::<Result<_, _>>()?;
    Ok(RunPage { rows, total, page, pages, completed, completed_waited, running, failed, failed_note, tokens: (known > 0).then_some(tokens), tasks })
}

#[derive(Debug, Serialize)]
pub struct RunDetail {
    pub run: Run,
    pub task: Option<Task>,
    pub events: Vec<Event>,
    pub logs: Vec<Event>,
}

pub fn run_detail(conn: &Connection, id: i64) -> Result<RunDetail, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let run = load_run(conn, id).map_err(e)?.ok_or("运行记录不存在")?;
    let task = load_task(conn, run.task_id).map_err(e)?;
    Ok(RunDetail { events: events_of(conn, id, false).map_err(e)?, logs: events_of(conn, id, true).map_err(e)?, run, task })
}

/* ─────────────────────────── 额度判定 ─────────────────────────── */

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WindowInfo {
    pub key: String,
    pub name: String,
    pub used_percent: Option<f64>,
    pub resets_at: Option<i64>,
    pub blocking: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct QuotaVerdict {
    /// available / blocked / unknown / reauth / unsupported
    pub state: String,
    pub reason: Option<String>,
    /// 全部阻塞窗口都恢复的预计时间；未知时为 None
    pub resets_at: Option<i64>,
    pub windows: Vec<WindowInfo>,
}

impl QuotaVerdict {
    fn simple(state: &str, reason: impl Into<String>) -> Self {
        Self { state: state.into(), reason: Some(reason.into()), resets_at: None, windows: vec![] }
    }
}

pub fn parse_reset(text: &str) -> Option<i64> {
    let text = text.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(text) { return Some(dt.timestamp_millis()); }
    let n: i64 = text.parse().ok()?;
    Some(if n < 100_000_000_000 { n * 1000 } else { n })
}

pub fn verdict(state: &QuotaState) -> QuotaVerdict {
    match state {
        QuotaState::Ok { windows, .. } => {
            let windows: Vec<WindowInfo> = windows.iter().map(|w| WindowInfo {
                key: w.key.clone(),
                name: w.window_name.clone(),
                used_percent: w.used_percent,
                resets_at: w.resets_at.as_deref().and_then(parse_reset),
                blocking: w.used_percent.is_some_and(|p| p >= 100.0),
            }).collect();
            let blocking: Vec<&WindowInfo> = windows.iter().filter(|w| w.blocking).collect();
            if blocking.is_empty() {
                return QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows };
            }
            let resets_at = if blocking.iter().all(|w| w.resets_at.is_some()) { blocking.iter().filter_map(|w| w.resets_at).max() } else { None };
            let names: Vec<&str> = blocking.iter().map(|w| w.name.as_str()).collect();
            QuotaVerdict { state: "blocked".into(), reason: Some(format!("{}已用完", names.join("、"))), resets_at, windows }
        }
        QuotaState::Unauthorized { reason } => QuotaVerdict::simple("reauth", reason.clone()),
        QuotaState::Unsupported { reason } | QuotaState::Forbidden { reason } => QuotaVerdict::simple("unsupported", reason.clone()),
        QuotaState::RateLimited { reason } | QuotaState::Failed { reason } => QuotaVerdict::simple("unknown", reason.clone()),
    }
}

/// 恢复时间未知或检查失败：5、10、20、40 分钟，之后每 60 分钟
fn backoff(failures: i64) -> i64 {
    let minutes = match failures { ..=1 => 5, 2 => 10, 3 => 20, 4 => 40, _ => 60 };
    minutes * 60_000
}

/// 把一次额度检查的结果写入等待中的运行；返回是否转入队列
pub fn apply_check(conn: &Connection, run_id: i64, v: &QuotaVerdict, now: i64) -> rusqlite::Result<bool> {
    let Some(run) = load_run(conn, run_id)? else { return Ok(false) };
    if run.status != "waiting" { return Ok(false); }
    let waited = now - run.wait_started_at.unwrap_or(now);
    let windows = v.windows.iter()
        .map(|w| format!("{} {}", w.name, w.used_percent.map(|p| format!("{p:.0}%")).unwrap_or_else(|| "未知".into())))
        .collect::<Vec<_>>().join(" · ");
    let set_wait = |kind: &str, reason: &str, resets: Option<i64>, next: i64, failures: i64| -> rusqlite::Result<()> {
        if run.block_kind.as_deref() != Some(kind) || run.reason.as_deref() != Some(reason) {
            add_event(conn, run_id, now, "check", "检查额度", Some(reason))?;
        }
        conn.execute(
            "UPDATE automation_runs SET block_kind=?2, reason=?3, resets_at=?4, last_check_at=?5, next_check_at=?6, check_failures=?7 WHERE id=?1",
            params![run_id, kind, reason, resets, now, next, failures],
        )?;
        Ok(())
    };
    match v.state.as_str() {
        "available" => {
            conn.execute(
                "UPDATE automation_runs SET status='queued', block_kind=NULL, reason=NULL, last_check_at=?2, next_check_at=NULL, check_failures=0, wait_ms=wait_ms+?3, wait_started_at=NULL WHERE id=?1",
                params![run_id, now, waited],
            )?;
            let detail = if windows.is_empty() { "额度已可用".to_string() } else { windows };
            add_event(conn, run_id, now, "check", "检查额度恢复", Some(&detail))?;
            return Ok(true);
        }
        "blocked" => match v.resets_at {
            Some(at) => {
                let next = if at > now { at + 30_000 } else { now + backoff(1) };
                set_wait("quota", v.reason.as_deref().unwrap_or("额度暂不可用"), Some(at), next, 0)?;
            }
            None => {
                let failures = run.check_failures + 1;
                set_wait("quota_unknown", "恢复时间未知", None, now + backoff(failures), failures)?;
            }
        },
        "unknown" => {
            let failures = run.check_failures + 1;
            let reason = format!("检查失败 · {}", v.reason.as_deref().unwrap_or("未知原因"));
            set_wait("check_failed", &reason, run.resets_at, now + backoff(failures), failures)?;
        }
        "reauth" => {
            conn.execute(
                "UPDATE automation_runs SET status='attention', block_kind='reauth', reason=?2, last_check_at=?3, next_check_at=NULL, wait_ms=wait_ms+?4, wait_started_at=NULL WHERE id=?1",
                params![run_id, "需要重新登录：停止额度检查，保留任务；重新认证并检查账号后才能继续。", now, waited],
            )?;
            add_event(conn, run_id, now, "attention", "需要重新登录", v.reason.as_deref())?;
        }
        _ => {
            // 不支持读取额度：执行中耗尽的运行在执行器给出的时间（或间隔重试）直接尝试继续；
            // 额度恢复后启动的任务无法判断，转为需要处理
            if run.resume_pending {
                match run.resets_at {
                    Some(at) if now >= at => {
                        conn.execute(
                            "UPDATE automation_runs SET status='queued', block_kind=NULL, reason=NULL, last_check_at=?2, next_check_at=NULL, wait_ms=wait_ms+?3, wait_started_at=NULL WHERE id=?1",
                            params![run_id, now, waited],
                        )?;
                        add_event(conn, run_id, now, "check", "到达预计恢复时间", Some("该账号不支持读取额度，直接尝试继续"))?;
                        return Ok(true);
                    }
                    Some(at) => set_wait("quota_unknown", "该账号不支持读取额度，将在预计时间尝试继续", Some(at), at + 30_000, run.check_failures)?,
                    None => {
                        conn.execute(
                            "UPDATE automation_runs SET resets_at=?2 WHERE id=?1",
                            params![run_id, now + 30 * 60_000],
                        )?;
                        set_wait("quota_unknown", "恢复时间未知 · 30 分钟后尝试继续", Some(now + 30 * 60_000), now + 30 * 60_000 + 30_000, run.check_failures + 1)?;
                    }
                }
            } else {
                conn.execute(
                    "UPDATE automation_runs SET status='attention', block_kind='unsupported', reason=?2, last_check_at=?3, next_check_at=NULL, wait_ms=wait_ms+?4, wait_started_at=NULL WHERE id=?1",
                    params![run_id, format!("该账号不支持读取额度：{}", v.reason.as_deref().unwrap_or("")), now, waited],
                )?;
                add_event(conn, run_id, now, "attention", "无法读取额度", v.reason.as_deref())?;
            }
        }
    }
    Ok(false)
}

/* ─────────────────────────── 执行器输出 ─────────────────────────── */

#[derive(Debug, Default, Clone)]
pub struct Progress {
    pub session_id: Option<String>,
    pub completed: bool,
    pub failed: bool,
    pub result: Option<String>,
    pub error: Option<String>,
    pub input: Option<i64>,
    pub cache_read: Option<i64>,
    pub output: Option<i64>,
    pub files: Vec<String>,
    /// Claude `rate_limit_event` 报告已到限额（`status: "rejected"`）
    pub rate_limited: bool,
    /// 同一事件给出的限额重置时间（毫秒）
    pub limit_resets_at: Option<i64>,
}

fn add(slot: &mut Option<i64>, v: Option<i64>) {
    if let Some(v) = v { *slot = Some(slot.unwrap_or(0) + v); }
}

fn tool_summary(input: &Json) -> String {
    for key in ["file_path", "command", "pattern", "path", "url", "description"] {
        if let Some(s) = input.get(key).and_then(Json::as_str) { return clip(s, 80); }
    }
    String::new()
}

impl Progress {
    fn file(&mut self, path: &str) {
        let path = path.trim();
        if !path.is_empty() && !self.files.iter().any(|f| f == path) && self.files.len() < 50 { self.files.push(path.to_string()); }
    }

    /// 读一行执行器输出，返回需要写入执行日志的行
    pub fn feed(&mut self, platform: &str, line: &str) -> Vec<String> {
        let Ok(v) = serde_json::from_str::<Json>(line.trim()) else { return vec![] };
        let mut logs = Vec::new();
        let kind = v.get("type").and_then(Json::as_str).unwrap_or("");
        if platform == "claude" {
            match kind {
                "system" => {
                    if let Some(id) = v.get("session_id").and_then(Json::as_str) { self.session_id = Some(id.to_string()); }
                }
                // 限额状态变化；`resetsAt` 为 Unix 秒（官方文档 Agent SDK 的 RateLimitInfo）。
                // 主额度 rejected 但超额用量仍可用（`overageStatus` 为 allowed / allowed_warning）时会话继续，不记为耗尽
                "rate_limit_event" => {
                    let info = v.get("rate_limit_info");
                    let field = |k: &str| info.and_then(|i| i.get(k)).and_then(Json::as_str);
                    let overage_ok = matches!(field("overageStatus").or_else(|| field("overage_status")), Some("allowed" | "allowed_warning"));
                    if field("status") == Some("rejected") && !overage_ok {
                        self.rate_limited = true;
                        let at = info.and_then(|i| i.get("resetsAt").or_else(|| i.get("resets_at"))).and_then(Json::as_i64);
                        if let Some(at) = at.and_then(|n| parse_reset(&n.to_string())) { self.limit_resets_at = Some(at); }
                    }
                }
                "assistant" => {
                    let blocks = v.pointer("/message/content").and_then(Json::as_array).cloned().unwrap_or_default();
                    for b in blocks {
                        match b.get("type").and_then(Json::as_str) {
                            Some("text") => {
                                if let Some(t) = b.get("text").and_then(Json::as_str).and_then(|t| t.lines().find(|l| !l.trim().is_empty())) { logs.push(clip(t, 160)); }
                            }
                            Some("tool_use") => {
                                let name = b.get("name").and_then(Json::as_str).unwrap_or("工具");
                                let input = b.get("input").cloned().unwrap_or(Json::Null);
                                if matches!(name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
                                    if let Some(p) = input.get("file_path").or_else(|| input.get("notebook_path")).and_then(Json::as_str) { self.file(p); }
                                }
                                let summary = tool_summary(&input);
                                logs.push(if summary.is_empty() { format!("调用 {name}") } else { format!("调用 {name} · {summary}") });
                            }
                            _ => {}
                        }
                    }
                }
                "result" => {
                    if let Some(id) = v.get("session_id").and_then(Json::as_str) { self.session_id = Some(id.to_string()); }
                    let is_error = v.get("is_error").and_then(Json::as_bool).unwrap_or(false);
                    let subtype = v.get("subtype").and_then(Json::as_str).unwrap_or("");
                    let text = v.get("result").and_then(Json::as_str).map(str::to_string);
                    if let Some(u) = v.get("usage") {
                        let n = |k: &str| u.get(k).and_then(Json::as_i64);
                        let input = [n("input_tokens"), n("cache_creation_input_tokens"), n("cache_read_input_tokens")];
                        if input.iter().any(Option::is_some) { add(&mut self.input, Some(input.iter().flatten().sum())); }
                        add(&mut self.cache_read, n("cache_read_input_tokens"));
                        add(&mut self.output, n("output_tokens"));
                    }
                    if !is_error && subtype == "success" {
                        self.completed = true;
                        self.result = text;
                    } else {
                        self.failed = true;
                        self.error = text.filter(|t| !t.trim().is_empty()).or_else(|| Some(format!("执行器返回 {subtype}")));
                    }
                }
                _ => {}
            }
        } else {
            match kind {
                "thread.started" => {
                    if let Some(id) = v.get("thread_id").and_then(Json::as_str) { self.session_id = Some(id.to_string()); }
                }
                "item.completed" => {
                    let item = v.get("item").cloned().unwrap_or(Json::Null);
                    match item.get("type").and_then(Json::as_str) {
                        Some("agent_message") => {
                            if let Some(t) = item.get("text").and_then(Json::as_str) {
                                if let Some(first) = t.lines().find(|l| !l.trim().is_empty()) { logs.push(clip(first, 160)); }
                                self.result = Some(t.to_string());
                            }
                        }
                        Some("command_execution") => {
                            if let Some(c) = item.get("command").and_then(Json::as_str) { logs.push(format!("执行命令 · {}", clip(c, 120))); }
                        }
                        Some("file_change") => {
                            let changes = item.get("changes").and_then(Json::as_array).cloned().unwrap_or_default();
                            for c in &changes {
                                if let Some(p) = c.get("path").and_then(Json::as_str) { self.file(p); }
                            }
                            logs.push(format!("修改文件 · {} 个", changes.len()));
                        }
                        Some("error") => {
                            if let Some(m) = item.get("message").and_then(Json::as_str) { logs.push(format!("提示 · {}", clip(m, 120))); }
                        }
                        _ => {}
                    }
                }
                "turn.completed" => {
                    if let Some(u) = v.get("usage") {
                        let n = |k: &str| u.get(k).and_then(Json::as_i64);
                        add(&mut self.input, n("input_tokens"));
                        add(&mut self.cache_read, n("cached_input_tokens"));
                        add(&mut self.output, n("output_tokens"));
                    }
                    self.completed = true;
                }
                "turn.failed" => {
                    self.failed = true;
                    self.error = v.pointer("/error/message").and_then(Json::as_str).map(str::to_string).or_else(|| Some("turn failed".into()));
                }
                "error" => {
                    if let Some(m) = v.get("message").and_then(Json::as_str) { self.error = Some(m.to_string()); }
                }
                _ => {}
            }
        }
        logs
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Completed,
    Cancelled,
    Limit { resets_at: Option<i64>, message: String },
    Reauth(String),
    /// Codex 拒绝在非 Git、未信任的目录执行；保留目录保护，不加 `--skip-git-repo-check`
    UntrustedDir(String),
    Failed(String),
}

const UNTRUSTED_DIR_REASON: &str = "Codex 只能在 Git 仓库或已信任的目录中执行。请把工作目录改为 Git 仓库，或先在 Codex 中信任该目录。";

/// codex-cli 0.162.0 原文：`Not inside a trusted directory and --skip-git-repo-check was not specified.`
pub fn is_untrusted_dir(text: &str) -> bool {
    has_any(text, &["not inside a trusted directory", "--skip-git-repo-check was not specified"])
}

fn has_any(text: &str, words: &[&str]) -> bool {
    let lower = text.to_lowercase();
    words.iter().any(|w| lower.contains(w))
}

/// `ve hit your`：Claude Code 2.1.308 的 `You've hit your session limit / weekly limit / Opus limit …`
/// 与 Codex 的 `You’ve hit your usage limit`（两种撇号）
pub fn is_limit(text: &str) -> bool {
    has_any(text, &["usage limit", "usage_limit", "rate limit", "rate_limit", "ratelimit", "limit reached", "hit your limit", "ve hit your", "quota", "too many requests", "429", "额度"])
}

pub fn is_auth(text: &str) -> bool {
    has_any(text, &["401", "unauthorized", "authentication", "invalid api key", "/login", "not logged in", "log in again", "token expired", "expired token", "invalid_grant", "重新登录"])
}

/// 执行器在限额提示里附带的恢复时间，如 `Claude AI usage limit reached|1761300000`
pub fn limit_reset(text: &str) -> Option<i64> {
    text.split('|').skip(1).find_map(|part| {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        (digits.len() >= 9).then(|| parse_reset(&digits)).flatten()
    })
}

/// Codex 限额提示末尾的 `try again at <时间>`，按本机时区：当天为 `3:05 PM`，跨天为 `Oct 23rd, 2026 3:05 PM`
/// （codex-cli 0.162.0 内置格式 `%-I:%M %p` 与 `%b %-d{st|nd|rd|th}, %Y %-I:%M %p`）
pub fn codex_retry_at<Tz: TimeZone>(tz: &Tz, text: &str, now_ms: i64) -> Option<i64> {
    const KEY: &str = "try again at ";
    let start = text.to_ascii_lowercase().find(KEY)? + KEY.len();
    let rest = text[start..].split(['.', '\n']).next()?.trim();
    let parts: Vec<&str> = rest.split_whitespace().collect();
    let local = match parts.as_slice() {
        [time, ampm] => {
            let time = NaiveTime::parse_from_str(&format!("{time} {ampm}"), "%I:%M %p").ok()?;
            let today = tz.timestamp_millis_opt(now_ms).single()?.date_naive();
            let at = tz.from_local_datetime(&today.and_time(time)).earliest()?;
            // 提示只精确到分钟：比现在早 5 分钟以内仍按当天算，更早才是次日
            if at.timestamp_millis() > now_ms - 5 * 60_000 { at } else { tz.from_local_datetime(&today.succ_opt()?.and_time(time)).earliest()? }
        }
        [month, day, year, time, ampm] => {
            let day: String = day.chars().take_while(char::is_ascii_digit).collect();
            let naive = chrono::NaiveDateTime::parse_from_str(&format!("{month} {day} {year} {time} {ampm}"), "%b %d %Y %I:%M %p").ok()?;
            tz.from_local_datetime(&naive).earliest()?
        }
        _ => return None,
    };
    Some(local.timestamp_millis())
}

pub fn classify(p: &Progress, exit_ok: bool, stderr_tail: &str, stopped: bool) -> Outcome {
    if stopped { return Outcome::Cancelled; }
    if p.completed && !p.failed { return Outcome::Completed; }
    let stderr = stderr_tail.trim();
    let message = p.error.clone()
        .or_else(|| (!stderr.is_empty()).then(|| stderr.to_string()))
        .unwrap_or_else(|| if exit_ok { "执行器没有返回结果".into() } else { "执行器异常退出".into() });
    // 先按文本判断；`rate_limit_event` 的结构化标记只在文本没有给出其他判断时兜底，避免吞掉认证、网络等真实原因
    let limit = |message: String| {
        let resets_at = limit_reset(&message).or(p.limit_resets_at).or_else(|| codex_retry_at(&Local, &message, now_ms()));
        Outcome::Limit { resets_at, message }
    };
    if is_untrusted_dir(&message) {
        Outcome::UntrustedDir(message)
    } else if is_limit(&message) {
        limit(message)
    } else if is_auth(&message) {
        Outcome::Reauth(message)
    } else if p.rate_limited {
        limit(message)
    } else {
        Outcome::Failed(message)
    }
}

pub fn build_args(platform: &str, model: Option<&str>, session: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut push = |s: &str| args.push(s.to_string());
    if platform == "codex" {
        push("exec");
        push("--json");
        if let Some(m) = model { push("-m"); push(m); }
        if let Some(s) = session { push("resume"); push(s); }
        // 提示词从标准输入读取
        push("-");
    } else {
        push("-p");
        push("--output-format");
        push("stream-json");
        push("--verbose");
        if let Some(m) = model { push("--model"); push(m); }
        if let Some(s) = session { push("--resume"); push(s); }
    }
    args
}

/* ─────────────────────────── Skills（D-08） ─────────────────────────── */

/// 一个任务最多选择的 Skills 数
const MAX_SKILLS: usize = 10;

/// 提示词前缀：Codex 在提示词里识别 `$名称`；Claude Code 的 `-p` 模式一次只加载第一个 `/skill`，改用自然语言点名
pub fn skill_prefix(platform: &str, names: &[String]) -> String {
    if names.is_empty() { return String::new(); }
    if platform == "codex" {
        let list: Vec<String> = names.iter().map(|n| format!("${n}")).collect();
        format!("{}\n\n", list.join(" "))
    } else {
        format!("本任务请使用以下 Skills：{}\n\n", names.join("、"))
    }
}

/// 写入执行器标准输入的提示词：首次运行为「前缀 + 任务内容」，续跑为「前缀 + 续跑指令」
pub fn run_prompt(platform: &str, skills: &[String], prompt: &str, resume: bool) -> String {
    let body = if resume { RESUME_PROMPT } else { prompt };
    format!("{}{body}", skill_prefix(platform, skills))
}

/// 任务选中、但目标工具当前没有启用的 Skills，按选择顺序
pub fn missing_skills(selected: &[String], available: &[crate::skills::SkillOption]) -> Vec<String> {
    selected.iter().filter(|s| !available.iter().any(|o| &o.name == *s)).cloned().collect()
}

fn missing_reason(missing: &[String]) -> String {
    let names: Vec<String> = missing.iter().map(|n| format!("「{n}」")).collect();
    format!("Skill{}已停用或不存在", names.join("、"))
}

/// 执行前（首次运行与续跑都经过这里）检查选中的 Skills 仍然启用；缺失时转为「需要处理」并返回 false，不启动进程
pub fn skills_ready(conn: &Connection, run_id: i64, selected: &[String], available: &[crate::skills::SkillOption], now: i64) -> rusqlite::Result<bool> {
    let missing = missing_skills(selected, available);
    if missing.is_empty() { return Ok(true); }
    set_attention(conn, run_id, "skill_missing", &missing_reason(&missing), now)?;
    Ok(false)
}

/* ─────────────────────────── 运行生命周期 ─────────────────────────── */

pub fn finish(conn: &Connection, run_id: i64, outcome: &Outcome, p: &Progress, now: i64) -> rusqlite::Result<()> {
    let Some(run) = load_run(conn, run_id)? else { return Ok(()) };
    let task = load_task(conn, run.task_id)?;
    let exec_add = now - run.exec_started_at.unwrap_or(now);
    let mut input = run.input_tokens;
    let mut cache = run.cache_read_tokens;
    let mut output = run.output_tokens;
    add(&mut input, p.input);
    add(&mut cache, p.cache_read);
    add(&mut output, p.output);
    let mut files = run.files.clone();
    for f in &p.files {
        if !files.contains(f) { files.push(f.clone()); }
    }
    let session = p.session_id.clone().or(run.session_id.clone());
    conn.execute(
        "UPDATE automation_runs SET exec_ms=exec_ms+?2, exec_started_at=NULL, input_tokens=?3, cache_read_tokens=?4, output_tokens=?5, files=?6, session_id=?7, result=COALESCE(?8, result) WHERE id=?1",
        params![run_id, exec_add, input, cache, output, serde_json::to_string(&files).unwrap_or_else(|_| "[]".into()), session, p.result.as_deref().map(|r| clip(r, 4000))],
    )?;
    let auto_resume = task.as_ref().is_some_and(|t| t.auto_resume);
    // 任务已不是启用状态（执行中被取消）：会让运行继续挂着的结果（等待额度、需要处理）一律按已取消收尾，
    // 不留下等待 / 排队的孤儿运行；完成与失败是真实结果，照常记录
    let inactive = task.as_ref().is_none_or(|t| t.state != "active");
    let lingering = match outcome {
        Outcome::Limit { .. } => auto_resume,
        Outcome::Reauth(_) | Outcome::UntrustedDir(_) => true,
        _ => false,
    };
    if inactive && lingering {
        conn.execute("UPDATE automation_runs SET status='cancelled', ended_at=?2, resume_pending=0, block_kind=NULL, reason=NULL WHERE id=?1", params![run_id, now])?;
        add_event(conn, run_id, now, "stop", "任务已取消，运行结束", Some("任务不再启用，不进入等待或需要处理"))?;
        return Ok(());
    }
    match outcome {
        Outcome::Completed => {
            conn.execute("UPDATE automation_runs SET status='completed', ended_at=?2, resume_pending=0, block_kind=NULL, reason=NULL WHERE id=?1", params![run_id, now])?;
            add_event(conn, run_id, now, "done", "完成任务", Some(if session.is_some() { "结果已关联到原会话" } else { "执行器已返回结果" }))?;
            if let Some(t) = &task {
                let one_shot = t.launch != "schedule" || t.schedule.as_ref().is_some_and(|s| s.freq == "once");
                if one_shot && t.state == "active" {
                    conn.execute("UPDATE automation_tasks SET state='finished', updated_at=?2 WHERE id=?1", params![t.id, now])?;
                }
            }
        }
        Outcome::Cancelled => {
            conn.execute("UPDATE automation_runs SET status='cancelled', ended_at=?2 WHERE id=?1", params![run_id, now])?;
            add_event(conn, run_id, now, "stop", "已停止运行", Some("保留运行记录和已经完成的文件修改"))?;
        }
        Outcome::Limit { resets_at, message } if auto_resume && session.is_some() => {
            let next = resets_at.map(|r| r.max(now) + 30_000).unwrap_or(now + backoff(1));
            conn.execute(
                "UPDATE automation_runs SET status='waiting', block_kind='quota', reason='额度耗尽，保留进度', resets_at=?2, next_check_at=?3, check_failures=0, wait_started_at=?4, waits=waits+1, resume_pending=1 WHERE id=?1",
                params![run_id, resets_at, next, now],
            )?;
            add_event(conn, run_id, now, "pause", "额度耗尽，保留进度", Some(&clip(message, 200)))?;
        }
        Outcome::Limit { message, .. } if auto_resume => {
            conn.execute(
                "UPDATE automation_runs SET status='attention', block_kind='resume_unsupported', reason='无法自动续跑 · 需要手动继续：执行器没有返回可续跑的会话' WHERE id=?1",
                params![run_id],
            )?;
            add_event(conn, run_id, now, "attention", "额度耗尽，无法自动续跑", Some(&clip(message, 200)))?;
        }
        Outcome::Limit { message, .. } => {
            conn.execute("UPDATE automation_runs SET status='failed', ended_at=?2, error=?3 WHERE id=?1", params![run_id, now, format!("额度耗尽：{}", clip(message, 1000))])?;
            add_event(conn, run_id, now, "fail", "额度耗尽，运行结束", Some("未开启「等待恢复后自动继续」"))?;
        }
        Outcome::Reauth(message) => {
            conn.execute(
                "UPDATE automation_runs SET status='attention', block_kind='reauth', reason='需要重新登录：停止额度检查，保留任务；重新认证并检查账号后才能继续。', error=?2 WHERE id=?1",
                params![run_id, clip(message, 1000)],
            )?;
            add_event(conn, run_id, now, "attention", "需要重新登录", Some(&clip(message, 200)))?;
        }
        Outcome::UntrustedDir(message) => {
            conn.execute(
                "UPDATE automation_runs SET status='attention', block_kind='cwd_missing', reason=?2, error=?3 WHERE id=?1",
                params![run_id, UNTRUSTED_DIR_REASON, clip(message, 1000)],
            )?;
            add_event(conn, run_id, now, "attention", "需要处理", Some(UNTRUSTED_DIR_REASON))?;
        }
        Outcome::Failed(message) => {
            conn.execute("UPDATE automation_runs SET status='failed', ended_at=?2, error=?3 WHERE id=?1", params![run_id, now, clip(message, 2000)])?;
            add_event(conn, run_id, now, "fail", "运行失败", Some(&clip(message, 200)))?;
        }
    }
    Ok(())
}

/// 只改仍在启动流程里的运行（排队中，或刚认领为执行中）；启动途中被删除或取消的运行不再写入
fn set_attention(conn: &Connection, run_id: i64, kind: &str, reason: &str, now: i64) -> rusqlite::Result<()> {
    let n = conn.execute(
        "UPDATE automation_runs SET status='attention', block_kind=?2, reason=?3 WHERE id=?1 AND status IN ('queued','running')",
        params![run_id, kind, reason],
    )?;
    if n == 0 { return Ok(()); }
    add_event(conn, run_id, now, "attention", "需要处理", Some(reason))
}

/// D-09：每个账号同时运行的任务上限（含排队任务与并行任务）
const MAX_PARALLEL_PER_ACCOUNT: usize = 3;

/// 工作目录比较键：与会话模块一致，小写、`\` 换成 `/`、去掉末尾 `/`
fn cwd_key(path: &str) -> String {
    path.trim().replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// 按账号决定本轮可以启动的运行（D-09）。排队与等待的运行只看启用中的任务（与 [`claim_run`] 一致，暂停、取消、
/// 完成的任务不占队列，也不会被反复放行又认领失败）；执行中的运行无论任务状态都计入工作目录与并行数：
/// ③ 账号下任一运行在等待额度时，整个账号不再启动新任务（正在运行的跑完当前这一轮）；
/// 排队任务（serial）一次一个，先放行排队队首，再按顺序放行关闭排队的任务；
/// ① 与任何账号正在运行（或本轮已放行）的任务工作目录相同则不放行；② 同账号运行数不超过上限
pub fn startable(conn: &Connection) -> rusqlite::Result<Vec<i64>> {
    struct Row { id: i64, status: String, cwd: String, serial: bool }
    let mut stmt = conn.prepare(
        "SELECT r.id, r.status, t.connection_id, t.cwd, t.serial_queue FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id
         WHERE r.status IN ('queued','waiting','running') AND (t.state = 'active' OR r.status = 'running')
         ORDER BY r.created_at, r.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(2)?, Row { id: r.get(0)?, status: r.get(1)?, cwd: cwd_key(&r.get::<_, String>(3)?), serial: r.get::<_, i64>(4)? != 0 }))
    })?;
    let mut accounts: Vec<(String, Vec<Row>)> = Vec::new();
    for row in rows {
        let (connection, row) = row?;
        match accounts.iter_mut().find(|(c, _)| *c == connection) {
            Some((_, list)) => list.push(row),
            None => accounts.push((connection, vec![row])),
        }
    }
    let mut out = Vec::new();
    // ① 同一工作目录跨账号也串行：先放入所有正在运行的目录，本轮放行的也加入
    let mut busy: HashSet<&str> = accounts.iter().flat_map(|(_, rows)| rows.iter()).filter(|r| r.status == "running").map(|r| r.cwd.as_str()).collect();
    for (_, rows) in &accounts {
        if rows.iter().any(|r| r.status == "waiting") { continue; }
        let running: Vec<&Row> = rows.iter().filter(|r| r.status == "running").collect();
        let mut count = running.len();
        let serial_busy = running.iter().any(|r| r.serial);
        let head = rows.iter().find(|r| r.status == "queued" && r.serial).filter(|_| !serial_busy);
        let parallel = rows.iter().filter(|r| r.status == "queued" && !r.serial);
        for r in head.into_iter().chain(parallel) {
            if count >= MAX_PARALLEL_PER_ACCOUNT { break; }
            if !busy.insert(r.cwd.as_str()) { continue; }
            out.push(r.id);
            count += 1;
        }
    }
    Ok(out)
}

/// 到点的定时任务入队；仍在执行时跳过本次触发；发现太晚的触发标「已错过」
pub fn fire_schedules<Tz: TimeZone>(conn: &Connection, tz: &Tz, now: i64) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(&format!("SELECT {TASK_COLS} FROM automation_tasks WHERE state = 'active' AND launch = 'schedule' AND next_run_at IS NOT NULL AND next_run_at <= ?1"))?;
    let due: Vec<Task> = stmt.query_map([now], task_from)?.collect::<Result<_, _>>()?;
    let mut changed = false;
    for task in due {
        let Some(at) = task.next_run_at else { continue };
        let next = task.schedule.as_ref().filter(|s| s.freq != "once").and_then(|s| next_fire(tz, s, now));
        if now - at > MISSED_GRACE_MS {
            conn.execute("UPDATE automation_tasks SET missed_at=?2, next_run_at=?3, updated_at=?4 WHERE id=?1", params![task.id, at, next, now])?;
        } else {
            conn.execute("UPDATE automation_tasks SET next_run_at=?2, updated_at=?3 WHERE id=?1", params![task.id, next, now])?;
            if active_run(conn, task.id)?.is_none() {
                insert_run(conn, &task, "schedule", "queued", now)?;
            }
        }
        changed = true;
    }
    Ok(changed)
}

/// 等待截止已过的运行转为「需要处理」，返回有运行到期需要检查额度的账号（D-09：同账号合并成一次检查）
pub fn due_checks(conn: &Connection, now: i64) -> rusqlite::Result<(Vec<String>, bool)> {
    let mut stmt = conn.prepare(
        "SELECT r.id FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id
         WHERE r.status = 'waiting' AND t.wait_deadline IS NOT NULL AND t.wait_deadline < ?1",
    )?;
    let expired: Vec<i64> = stmt.query_map([now], |r| r.get(0))?.collect::<Result<_, _>>()?;
    for id in &expired {
        conn.execute(
            "UPDATE automation_runs SET status='attention', block_kind='timeout', reason='等待超时：到达截止时间仍不可用，已保留记录，可重新排队', wait_ms=wait_ms+(?2-COALESCE(wait_started_at, ?2)), wait_started_at=NULL WHERE id=?1",
            params![id, now],
        )?;
        add_event(conn, *id, now, "attention", "等待超时", None)?;
    }
    let mut stmt = conn.prepare(
        "SELECT t.connection_id FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id
         WHERE r.status = 'waiting' AND t.state = 'active' AND (r.next_check_at IS NULL OR r.next_check_at <= ?1)
         GROUP BY t.connection_id ORDER BY MIN(r.created_at), MIN(r.id)",
    )?;
    let due = stmt.query_map([now], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok((due, !expired.is_empty()))
}

/// D-09 ③：额度检查按账号合并，一次检查的结果写入该账号全部等待中的运行（暂停的任务除外）；
/// `extra` 是手动检查的那条运行，即使所属任务已暂停也照常写入。
/// `checked_from` 是这次额度查询开始的时间：查询期间才进入等待的运行不使用这次（可能已过时的）结果。
/// 返回是否有运行转入队列
pub fn apply_account_check(conn: &Connection, connection_id: &str, extra: Option<i64>, checked_from: i64, v: &QuotaVerdict, now: i64) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT r.id FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id
         WHERE r.status = 'waiting' AND t.connection_id = ?1 AND (t.state != 'paused' OR r.id = ?2)
           AND COALESCE(r.wait_started_at, 0) <= ?3
         ORDER BY r.created_at, r.id",
    )?;
    let ids: Vec<i64> = stmt.query_map(params![connection_id, extra.unwrap_or(-1), checked_from], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let mut queued = false;
    for id in ids { queued |= apply_check(conn, id, v, now)?; }
    Ok(queued)
}

/// 启动时：上次退出时还在执行的运行标为需要处理（不自动重跑）；等待中的立即重新检查额度
pub fn recover(conn: &Connection, now: i64) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("SELECT id FROM automation_runs WHERE status = 'running'")?;
    let ids: Vec<i64> = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    for id in ids {
        conn.execute(
            "UPDATE automation_runs SET status='attention', block_kind='interrupted', reason='Jolu 退出时运行被中断；已保留进度，不会自动从头重跑，可查看关联会话后手动继续', exec_started_at=NULL WHERE id=?1",
            [id],
        )?;
        add_event(conn, id, now, "attention", "运行被中断", Some("Jolu 退出或重启"))?;
    }
    conn.execute("UPDATE automation_runs SET next_check_at=?1 WHERE status='waiting'", [now])?;
    Ok(())
}

/// 任务操作；返回需要结束的进程所属运行
pub fn act(conn: &Connection, id: i64, action: &str, now: i64) -> Result<Option<i64>, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let task = load_task(conn, id).map_err(e)?.ok_or("任务不存在")?;
    let active = active_run(conn, id).map_err(e)?;
    let running = active.as_ref().filter(|r| r.status == "running").map(|r| r.id);
    match action {
        "pause" => {
            if running.is_some() { return Err("运行中的任务请先停止运行".into()); }
            if task.state != "active" { return Err("只有启用中的任务可以暂停".into()); }
            conn.execute("UPDATE automation_tasks SET state='paused', updated_at=?2 WHERE id=?1", params![id, now]).map_err(e)?;
            if let Some(r) = &active { add_event(conn, r.id, now, "pause", "暂停等待", Some("暂停期间不自动运行")).map_err(e)?; }
            Ok(None)
        }
        "resume" => {
            if task.state != "paused" { return Err("任务没有暂停".into()); }
            let next = match (task.launch.as_str(), task.schedule.as_ref()) {
                ("schedule", Some(s)) => next_fire(&Local, s, now),
                _ => None,
            };
            conn.execute("UPDATE automation_tasks SET state='active', next_run_at=?2, updated_at=?3 WHERE id=?1", params![id, next, now]).map_err(e)?;
            if let Some(r) = &active {
                conn.execute("UPDATE automation_runs SET next_check_at=?2 WHERE id=?1 AND status='waiting'", params![r.id, now]).map_err(e)?;
                add_event(conn, r.id, now, "resume", "恢复任务", None).map_err(e)?;
            }
            Ok(None)
        }
        "cancel" => {
            if task.state == "cancelled" { return Ok(None); }
            conn.execute("UPDATE automation_tasks SET state='cancelled', next_run_at=NULL, updated_at=?2 WHERE id=?1", params![id, now]).map_err(e)?;
            if let Some(r) = &active {
                if r.status != "running" {
                    conn.execute(
                        "UPDATE automation_runs SET status='cancelled', ended_at=?2, wait_ms=wait_ms+(?2-COALESCE(wait_started_at, ?2)), wait_started_at=NULL WHERE id=?1",
                        params![r.id, now],
                    ).map_err(e)?;
                    add_event(conn, r.id, now, "stop", "取消任务", Some("停止后续等待与调度")).map_err(e)?;
                }
            }
            Ok(running)
        }
        "stop" => running.map(Some).ok_or_else(|| "没有执行中的运行".to_string()),
        "run" | "retry" => {
            if task.state == "cancelled" { return Err("任务已取消".into()); }
            if task.state == "paused" { return Err("请先恢复任务".into()); }
            match &active {
                Some(r) if r.status == "attention" => {
                    conn.execute("UPDATE automation_runs SET status='queued', block_kind=NULL, reason=NULL WHERE id=?1", [r.id]).map_err(e)?;
                    add_event(conn, r.id, now, "queue", "重新排队", None).map_err(e)?;
                }
                Some(_) => return Err("任务已有进行中的运行".into()),
                None => {
                    insert_run(conn, &task, "manual", "queued", now).map_err(e)?;
                }
            }
            conn.execute(
                "UPDATE automation_tasks SET missed_at=NULL, state=CASE state WHEN 'finished' THEN 'active' ELSE state END, updated_at=?2 WHERE id=?1",
                params![id, now],
            ).map_err(e)?;
            Ok(None)
        }
        _ => Err("不支持的操作".into()),
    }
}

/// 删除任务（任务 2.9）：在一个事务里删除任务、它的全部运行记录和事件；用量统计（`requests` 等表）不动。
/// 有执行中的运行时拒绝；排队或等待额度的运行先转为已取消再删除。调度线程只认 `status='queued'` 的运行
/// （见 [`claim_run`]），`startable`、`due_checks` 与合并额度检查都按库里现存的运行查询，删除后自然不再出现。
pub fn delete_task(conn: &mut Connection, task_id: i64) -> Result<(), String> {
    let e = |e: rusqlite::Error| e.to_string();
    let tx = conn.transaction().map_err(e)?;
    if load_task(&tx, task_id).map_err(e)?.is_none() { return Err("任务不存在".into()); }
    let running: bool = tx
        .query_row("SELECT EXISTS(SELECT 1 FROM automation_runs WHERE task_id = ?1 AND status = 'running')", [task_id], |r| r.get(0))
        .map_err(e)?;
    if running { return Err("任务正在运行，请先停止".into()); }
    tx.execute("UPDATE automation_runs SET status='cancelled', next_check_at=NULL WHERE task_id = ?1 AND status IN ('queued','waiting')", [task_id]).map_err(e)?;
    tx.execute("DELETE FROM automation_events WHERE run_id IN (SELECT id FROM automation_runs WHERE task_id = ?1)", [task_id]).map_err(e)?;
    tx.execute("DELETE FROM automation_runs WHERE task_id = ?1", [task_id]).map_err(e)?;
    tx.execute("DELETE FROM automation_tasks WHERE id = ?1", [task_id]).map_err(e)?;
    tx.commit().map_err(e)
}

/// 启动前认领运行：只有仍在排队、且所属任务处于启用状态（未暂停、未取消）的运行才转为执行中。
/// 认领发生在拉起进程之前，运行在这之间被删除、取消或任务被暂停时认领失败，不会启动执行器
pub fn claim_run(conn: &Connection, run_id: i64) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "UPDATE automation_runs SET status='running', block_kind=NULL, reason=NULL
         WHERE id=?1 AND status='queued'
           AND EXISTS(SELECT 1 FROM automation_tasks t WHERE t.id = automation_runs.task_id AND t.state = 'active')",
        [run_id],
    )?;
    Ok(n == 1)
}

/// 执行中的进程登记与停止请求。认领到写入进程 ID 之间有一个窗口：这期间收到的停止请求
/// 先留下标记，登记进程时发现标记就立即结束进程，按「已取消」收尾
#[derive(Default)]
struct Procs {
    /// 运行 ID → 进程 ID
    pids: HashMap<i64, u32>,
    /// 已请求停止的运行
    stopping: HashSet<i64>,
}

impl Procs {
    /// 认领前清掉同一运行上一轮遗留的停止标记（续跑的运行沿用原来的运行 ID）
    fn clear_stop(&mut self, run_id: i64) {
        self.stopping.remove(&run_id);
    }

    /// 请求停止：留下标记；进程已登记时返回进程 ID，由调用方结束进程树
    fn request_stop(&mut self, run_id: i64) -> Option<u32> {
        self.stopping.insert(run_id);
        self.pids.get(&run_id).copied()
    }

    /// 登记进程；返回 true 表示登记前已有停止请求，需要立即结束
    fn register(&mut self, run_id: i64, pid: u32) -> bool {
        self.pids.insert(run_id, pid);
        self.stopping.contains(&run_id)
    }

    /// 进程退出：注销，返回是否因停止请求结束
    fn exited(&mut self, run_id: i64) -> bool {
        self.pids.remove(&run_id);
        self.stopping.remove(&run_id)
    }
}

/* ─────────────────────────── 调度器 ─────────────────────────── */

struct Engine {
    wake: Mutex<bool>,
    cv: Condvar,
    /// 执行中的进程与停止请求，同一把锁，避免「先停止、后登记」时漏掉停止请求
    procs: Mutex<Procs>,
}

fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| Engine { wake: Mutex::new(false), cv: Condvar::new(), procs: Mutex::new(Procs::default()) })
}

static SCHEMA_READY: AtomicBool = AtomicBool::new(false);

fn shared(app: &AppHandle) -> Shared {
    app.state::<crate::Db>().0.clone()
}

fn kick() {
    let e = engine();
    *e.wake.lock().unwrap_or_else(|p| p.into_inner()) = true;
    e.cv.notify_all();
}

fn emit(app: &AppHandle) {
    let _ = app.emit(CHANGED, ());
}

fn with_conn<T>(db: &Shared, work: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    if !SCHEMA_READY.load(Ordering::Acquire) {
        ensure_schema(&conn).map_err(|e| e.to_string())?;
        SCHEMA_READY.store(true, Ordering::Release);
    }
    work(&conn)
}

async fn blocking<T: Send + 'static>(app: &AppHandle, work: impl FnOnce(&Connection) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    let db = shared(app);
    tauri::async_runtime::spawn_blocking(move || with_conn(&db, work)).await.map_err(|e| e.to_string())?
}

/// 本机 CLI 当前登录的连接；判断逻辑与 `current_connections` 一致，只比对、不返回凭证
fn current_connection_ids(db: &Shared) -> Vec<String> {
    if crate::validation::isolated() { return vec![]; }
    let mut local = Vec::new();
    for platform in ["claude", "codex"] {
        for kind in ["api", "auth"] {
            if let Some(secret) = crate::creds::read_secret(platform, kind) { local.push((platform, kind, secret)); }
        }
    }
    let Ok(conn) = db.lock() else { return vec![] };
    ["claude", "codex"].iter().filter_map(|platform| {
        local.iter().filter(|(p, _, _)| p == platform)
            .find_map(|(p, kind, secret)| crate::connections::find_by_secret(&conn, p, kind, secret).ok().flatten())
    }).collect()
}

/// 应用初始化后启动：建表、恢复中断的运行，然后每 30 秒（或有操作时）检查一次
pub fn start(app: AppHandle) {
    let db = shared(&app);
    if let Err(error) = with_conn(&db, |conn| recover(conn, now_ms()).map_err(|e| e.to_string())) {
        eprintln!("[自动化] 初始化失败：{error}");
        return;
    }
    let spawned = std::thread::Builder::new().name("jolu-automation".into()).spawn(move || loop {
        tick(&app);
        let e = engine();
        let guard = e.wake.lock().unwrap_or_else(|p| p.into_inner());
        let (mut guard, _) = e.cv.wait_timeout_while(guard, TICK, |woken| !*woken).unwrap_or_else(|p| p.into_inner());
        *guard = false;
    });
    if let Err(error) = spawned { eprintln!("[自动化] 调度线程启动失败：{error}"); }
}

fn tick(app: &AppHandle) {
    let db = shared(app);
    let now = now_ms();
    let due = with_conn(&db, |conn| {
        let fired = fire_schedules(conn, &Local, now).map_err(|e| e.to_string())?;
        let (due, expired) = due_checks(conn, now).map_err(|e| e.to_string())?;
        Ok((due, fired || expired))
    });
    let due = match due {
        Ok((due, changed)) => {
            if changed { emit(app); }
            due
        }
        Err(error) => {
            eprintln!("[自动化] 调度检查失败：{error}");
            return;
        }
    };
    for connection_id in due {
        check_account_blocking(app, &connection_id);
    }
    match with_conn(&db, |conn| startable(conn).map_err(|e| e.to_string())) {
        Ok(runs) => {
            for run_id in runs { start_run(app, run_id); }
        }
        Err(error) => eprintln!("[自动化] 读取队列失败：{error}"),
    }
}

fn check_account_blocking(app: &AppHandle, connection_id: &str) {
    let db = shared(app);
    let checked_from = now_ms();
    let state = tauri::async_runtime::block_on(crate::query_connection_quota(app.clone(), connection_id.to_string(), true));
    let v = match state {
        Ok(state) => verdict(&state),
        Err(error) => QuotaVerdict::simple("unknown", error),
    };
    if with_conn(&db, |conn| apply_account_check(conn, connection_id, None, checked_from, &v, now_ms()).map_err(|e| e.to_string())).is_ok() { emit(app); }
}

fn start_run(app: &AppHandle, run_id: i64) {
    let db = shared(app);
    let now = now_ms();
    let loaded = with_conn(&db, |conn| {
        let run = load_run(conn, run_id).map_err(|e| e.to_string())?.ok_or("运行记录不存在")?;
        let task = load_task(conn, run.task_id).map_err(|e| e.to_string())?.ok_or("任务不存在")?;
        Ok((run, task))
    });
    let Ok((run, task)) = loaded else { return };
    if run.status != "queued" { return; }
    let attention = |kind: &str, reason: String| {
        let _ = with_conn(&db, |conn| set_attention(conn, run_id, kind, &reason, now).map_err(|e| e.to_string()));
        emit(app);
    };
    let Some(tool) = tool_name(&task.platform) else { return attention("tool_missing", "该工具尚未接入执行器".into()) };
    let Some(exe) = crate::session_threads::find_executable(tool) else {
        return attention("tool_missing", format!("未检测到 {}，请安装或配置后重试；已保留进度。", display_name(&task.platform)));
    };
    if !Path::new(&task.cwd).is_dir() {
        return attention("cwd_missing", "项目目录不可访问，无法在原目录执行。".into());
    }
    if !current_connection_ids(&db).contains(&task.connection_id) {
        let name = run.account.clone().unwrap_or_else(|| "所选账号".into());
        return attention(
            "account_mismatch",
            format!("{name} 不是 {} 当前登录的账号；Jolu 不会自动切换账号，请在供应商页启用该账号后重试。", display_name(&task.platform)),
        );
    }
    // D-08：首次运行与续跑前都检查选中的 Skills 仍然启用，不在了就转为需要处理
    if !task.skills.is_empty() {
        let available = match crate::skills::enabled_skills(&task.platform, Some(Path::new(&task.cwd))) {
            Ok(list) => list,
            Err(error) => return attention("skill_missing", format!("无法读取 Skills：{error}")),
        };
        if !matches!(with_conn(&db, |conn| skills_ready(conn, run_id, &task.skills, &available, now).map_err(|e| e.to_string())), Ok(true)) {
            emit(app);
            return;
        }
    }
    // 先认领再拉起进程：认领之后删除会被拒绝（运行中），认领之前被删除或取消则不启动
    engine().procs.lock().unwrap_or_else(|p| p.into_inner()).clear_stop(run_id);
    match with_conn(&db, |conn| claim_run(conn, run_id).map_err(|e| e.to_string())) {
        Ok(true) => {}
        _ => return,
    }
    let session = run.session_id.clone();
    let prompt = run_prompt(&task.platform, &task.skills, &task.prompt, run.resume_pending);
    let args = build_args(&task.platform, task.model.as_deref(), session.as_deref());
    let mut command = Command::new(&exe);
    command.args(&args).current_dir(&task.cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return attention("tool_missing", format!("启动 {} 失败：{error}", display_name(&task.platform))),
    };
    let resumed = run.resume_pending;
    let _ = with_conn(&db, |conn| {
        conn.execute(
            "UPDATE automation_runs SET started_at=COALESCE(started_at, ?2), exec_started_at=?2 WHERE id=?1",
            params![run_id, now],
        ).map_err(|e| e.to_string())?;
        let (title, detail) = if resumed { ("恢复原会话", "继续未完成的任务") } else if session.is_some() { ("开始执行", "在所选会话中追加任务指令") } else { ("开始执行", "新建会话") };
        add_event(conn, run_id, now, "start", title, Some(detail)).map_err(|e| e.to_string())
    });
    // 认领后、登记前收到的停止请求：登记时立即结束进程树，不写入提示词；进程退出后按「已取消」收尾
    let stop_now = engine().procs.lock().unwrap_or_else(|p| p.into_inner()).register(run_id, child.id());
    if stop_now { kill_tree(child.id()); }
    emit(app);

    let app = app.clone();
    let platform = task.platform.clone();
    std::thread::spawn(move || {
        if let Some(mut stdin) = child.stdin.take().filter(|_| !stop_now) {
            let _ = stdin.write_all(prompt.as_bytes());
        }
        let stderr = child.stderr.take().map(|mut err| std::thread::spawn(move || {
            let mut text = String::new();
            let _ = err.read_to_string(&mut text);
            let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            lines[lines.len().saturating_sub(8)..].join("\n")
        }));
        let mut progress = Progress::default();
        let db = shared(&app);
        if let Some(out) = child.stdout.take() {
            let mut logged = 0i64;
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let before = progress.session_id.clone();
                let logs = progress.feed(&platform, &line);
                let session_changed = progress.session_id != before;
                if logs.is_empty() && !session_changed { continue; }
                let now = now_ms();
                let _ = with_conn(&db, |conn| {
                    if session_changed {
                        conn.execute("UPDATE automation_runs SET session_id=?2 WHERE id=?1", params![run_id, progress.session_id]).map_err(|e| e.to_string())?;
                    }
                    for log in &logs {
                        if logged >= LOG_LIMIT { break; }
                        add_event(conn, run_id, now, "log", log, None).map_err(|e| e.to_string())?;
                        logged += 1;
                    }
                    Ok(())
                });
            }
        }
        let exit_ok = child.wait().map(|s| s.success()).unwrap_or(false);
        let stderr_tail = stderr.and_then(|h| h.join().ok()).unwrap_or_default();
        let stopped = engine().procs.lock().unwrap_or_else(|p| p.into_inner()).exited(run_id);
        let outcome = classify(&progress, exit_ok, &stderr_tail, stopped);
        if let Err(error) = with_conn(&db, |conn| finish(conn, run_id, &outcome, &progress, now_ms()).map_err(|e| e.to_string())) {
            eprintln!("[自动化] 记录运行结果失败：{error}");
        }
        emit(&app);
        kick();
    });
}

/// 停止运行：进程已登记时结束整个进程树；还没登记（刚认领、正在拉起）时留下停止标记，登记时再结束
fn kill_run(run_id: i64) {
    let pid = engine().procs.lock().unwrap_or_else(|p| p.into_inner()).request_stop(run_id);
    if let Some(pid) = pid { kill_tree(pid); }
}

/// 结束整个进程树（执行器会再拉起子进程）
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).creation_flags(0x0800_0000).status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).status();
    }
}

/* ─────────────────────────── 命令 ─────────────────────────── */

#[tauri::command]
pub async fn automation_overview(app: AppHandle) -> Result<Overview, String> {
    blocking(&app, |conn| overview(conn, now_ms()).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn automation_task(app: AppHandle, id: i64) -> Result<TaskDetail, String> {
    blocking(&app, move |conn| task_detail(conn, id)).await
}

#[tauri::command]
pub async fn automation_create(app: AppHandle, input: TaskInput) -> Result<i64, String> {
    let id = blocking(&app, move |conn| create_task(conn, &input, now_ms())).await?;
    emit(&app);
    kick();
    Ok(id)
}

#[tauri::command]
pub async fn automation_update(app: AppHandle, id: i64, input: TaskInput) -> Result<(), String> {
    blocking(&app, move |conn| update_task(conn, id, &input, now_ms())).await?;
    emit(&app);
    kick();
    Ok(())
}

/// 删除任务及其运行记录（任务 2.9）；执行中的任务需先停止
#[tauri::command]
pub async fn automation_delete(app: AppHandle, task_id: i64) -> Result<(), String> {
    let db = shared(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = db.lock().map_err(|e| e.to_string())?;
        if !SCHEMA_READY.load(Ordering::Acquire) {
            ensure_schema(&conn).map_err(|e| e.to_string())?;
            SCHEMA_READY.store(true, Ordering::Release);
        }
        delete_task(&mut conn, task_id)
    }).await.map_err(|e| e.to_string())??;
    emit(&app);
    kick();
    Ok(())
}

/// 任务操作：pause / resume / cancel / stop / run / retry / check
#[tauri::command]
pub async fn automation_action(app: AppHandle, id: i64, action: String) -> Result<(), String> {
    if action == "check" {
        let run = blocking(&app, move |conn| active_run(conn, id).map_err(|e| e.to_string())).await?
            .filter(|r| r.status == "waiting")
            .ok_or("当前没有等待额度的运行")?;
        let checked_from = now_ms();
        let v = match crate::query_connection_quota(app.clone(), run.connection_id.clone(), true).await {
            Ok(state) => verdict(&state),
            Err(error) => QuotaVerdict::simple("unknown", error),
        };
        blocking(&app, move |conn| apply_account_check(conn, &run.connection_id, Some(run.id), checked_from, &v, now_ms()).map_err(|e| e.to_string())).await?;
    } else {
        let kill = blocking(&app, move |conn| act(conn, id, &action, now_ms())).await?;
        if let Some(run_id) = kill { kill_run(run_id); }
    }
    emit(&app);
    kick();
    Ok(())
}

#[tauri::command]
pub async fn automation_runs(app: AppHandle, filter: RunFilter, page: u32) -> Result<RunPage, String> {
    blocking(&app, move |conn| runs(conn, &filter, page, now_ms()).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn automation_run(app: AppHandle, id: i64) -> Result<RunDetail, String> {
    blocking(&app, move |conn| run_detail(conn, id)).await
}

#[derive(Debug, Serialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub status: String,
    /// 是否为该工具当前登录的连接；只有当前登录的连接能执行
    pub current: bool,
}

#[tauri::command]
pub async fn automation_accounts(app: AppHandle, platform: String) -> Result<Vec<Account>, String> {
    let db = shared(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let current = current_connection_ids(&db);
        with_conn(&db, |conn| {
            let list = crate::connections::list(conn).map_err(|e| e.to_string())?;
            Ok(list.into_iter().filter(|c| c.platform == platform).map(|c| Account {
                current: current.contains(&c.id),
                id: c.id,
                name: c.name,
                kind: c.kind,
                status: c.status,
            }).collect())
        })
    }).await.map_err(|e| e.to_string())?
}

/// 模型候选：连接保存的默认模型在前，其后是本机记录里最近用过的模型
#[tauri::command]
pub async fn automation_models(app: AppHandle, platform: String, connection_id: Option<String>) -> Result<Vec<String>, String> {
    blocking(&app, move |conn| {
        let mut out: Vec<String> = Vec::new();
        if let Some(id) = connection_id {
            let model: Option<String> = conn.query_row("SELECT model FROM connections WHERE id = ?1", [&id], |r| r.get(0)).optional().map_err(|e| e.to_string())?.flatten();
            if let Some(m) = model.filter(|m| safe_model(m)) { out.push(m); }
        }
        let mut stmt = conn.prepare("SELECT model FROM requests WHERE platform = ?1 AND model IS NOT NULL AND model != '' GROUP BY model ORDER BY MAX(ts) DESC LIMIT 12").map_err(|e| e.to_string())?;
        let seen: Vec<String> = stmt.query_map([&platform], |r| r.get(0)).map_err(|e| e.to_string())?.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        for m in seen {
            if safe_model(&m) && !out.contains(&m) { out.push(m); }
        }
        Ok(out)
    }).await
}

#[tauri::command]
pub async fn automation_quota(app: AppHandle, connection_id: String, force: bool) -> Result<QuotaVerdict, String> {
    Ok(match crate::query_connection_quota(app, connection_id, force).await {
        Ok(state) => verdict(&state),
        Err(error) => QuotaVerdict::simple("unknown", error),
    })
}

/// 「Skills（可选）」候选（D-08）：目标工具已启用的全局 Skills + 工作目录下的项目 Skills
#[tauri::command]
pub async fn automation_skills(_app: AppHandle, platform: String, cwd: Option<String>) -> Result<Vec<crate::skills::SkillOption>, String> {
    if tool_name(&platform).is_none() { return Err("该工具尚未接入执行器".into()); }
    tauri::async_runtime::spawn_blocking(move || {
        let cwd = cwd.map(|c| c.trim().to_string()).filter(|c| !c.is_empty() && Path::new(c).is_dir());
        crate::skills::enabled_skills(&platform, cwd.as_deref().map(Path::new))
    }).await.map_err(|e| e.to_string())?
}

#[derive(Debug, Serialize)]
pub struct ToolInfo {
    pub platform: String,
    pub found: bool,
}

#[tauri::command]
pub async fn automation_tools() -> Vec<ToolInfo> {
    ["claude", "codex"].iter().map(|p| ToolInfo {
        platform: p.to_string(),
        found: tool_name(p).and_then(crate::session_threads::find_executable).is_some(),
    }).collect()
}

#[tauri::command]
pub async fn automation_pick_dir(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let mut dialog = app.dialog().file().set_title("选择项目目录");
    if let Some(window) = app.get_webview_window(crate::MAIN) { dialog = dialog.set_parent(&window); }
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder()).await.map_err(|e| e.to_string())?;
    picked.map(|p| p.into_path().map(|p| p.display().to_string()).map_err(|e| e.to_string())).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn shanghai() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    fn ms(tz: &FixedOffset, y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        tz.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap().timestamp_millis()
    }

    fn sched(freq: &str, time: &str, weekday: Option<u32>, date: Option<&str>) -> Schedule {
        Schedule { freq: freq.into(), time: time.into(), weekday, date: date.map(str::to_string), timezone: "Asia/Shanghai".into() }
    }

    #[test]
    fn next_fire_handles_daily_weekly_and_once() {
        let tz = shanghai();
        // 2026-10-22 是周四
        let now = ms(&tz, 2026, 10, 22, 10, 0);
        assert_eq!(next_fire(&tz, &sched("daily", "09:00", None, None), now), Some(ms(&tz, 2026, 10, 23, 9, 0)));
        assert_eq!(next_fire(&tz, &sched("daily", "18:30", None, None), now), Some(ms(&tz, 2026, 10, 22, 18, 30)));
        assert_eq!(next_fire(&tz, &sched("weekly", "09:00", Some(1), None), now), Some(ms(&tz, 2026, 10, 26, 9, 0)));
        assert_eq!(next_fire(&tz, &sched("weekly", "11:00", Some(4), None), now), Some(ms(&tz, 2026, 10, 22, 11, 0)));
        assert_eq!(next_fire(&tz, &sched("once", "09:00", None, Some("2026-10-22")), now), None);
        assert_eq!(next_fire(&tz, &sched("once", "09:00", None, Some("2026-10-24")), now), Some(ms(&tz, 2026, 10, 24, 9, 0)));
    }

    #[test]
    fn quota_verdict_separates_blocked_unknown_and_reauth() {
        let w = |key: &str, used: Option<f64>, resets: Option<&str>| crate::quota::QuotaWindow {
            key: key.into(), window_name: format!("{key} 窗口"), used_percent: used, amount_text: None, remaining_text: None, resets_at: resets.map(str::to_string),
        };
        let blocked = verdict(&QuotaState::Ok { windows: vec![w("5h", Some(100.0), Some("2026-10-22T10:30:00Z")), w("7d", Some(58.0), None)], plan: None });
        assert_eq!(blocked.state, "blocked");
        assert_eq!(blocked.resets_at, parse_reset("2026-10-22T10:30:00Z"));
        assert!(blocked.windows[0].blocking && !blocked.windows[1].blocking);
        let unknown_reset = verdict(&QuotaState::Ok { windows: vec![w("7d", Some(100.0), None)], plan: None });
        assert_eq!((unknown_reset.state.as_str(), unknown_reset.resets_at), ("blocked", None));
        assert_eq!(verdict(&QuotaState::Ok { windows: vec![w("5h", Some(40.0), None)], plan: None }).state, "available");
        assert_eq!(verdict(&QuotaState::Failed { reason: "网络".into() }).state, "unknown");
        assert_eq!(verdict(&QuotaState::Unauthorized { reason: "过期".into() }).state, "reauth");
        assert_eq!(verdict(&QuotaState::Unsupported { reason: "API Key".into() }).state, "unsupported");
    }

    #[test]
    fn claude_stream_is_parsed_and_classified() {
        let mut p = Progress::default();
        p.feed("claude", r#"{"type":"system","subtype":"init","session_id":"s-1"}"#);
        let logs = p.feed("claude", r#"{"type":"assistant","message":{"content":[{"type":"text","text":"开始检查\n第二行"},{"type":"tool_use","name":"Edit","input":{"file_path":"src/LoginForm.tsx"}}]}}"#);
        assert_eq!(logs, vec!["开始检查".to_string(), "调用 Edit · src/LoginForm.tsx".to_string()]);
        p.feed("claude", r#"{"type":"result","subtype":"success","is_error":false,"result":"任务完成","session_id":"s-1","usage":{"input_tokens":100,"cache_read_input_tokens":900,"cache_creation_input_tokens":0,"output_tokens":50}}"#);
        assert_eq!(p.session_id.as_deref(), Some("s-1"));
        assert_eq!((p.input, p.cache_read, p.output), (Some(1000), Some(900), Some(50)));
        assert_eq!(p.files, vec!["src/LoginForm.tsx".to_string()]);
        assert_eq!(classify(&p, true, "", false), Outcome::Completed);

        let mut limited = Progress::default();
        limited.feed("claude", r#"{"type":"result","subtype":"success","is_error":true,"result":"Claude AI usage limit reached|1761300000"}"#);
        assert_eq!(classify(&limited, false, "", false), Outcome::Limit { resets_at: Some(1_761_300_000_000), message: "Claude AI usage limit reached|1761300000".into() });
        assert_eq!(classify(&limited, false, "", true), Outcome::Cancelled);
    }

    #[test]
    fn codex_events_are_parsed_and_classified() {
        let mut p = Progress::default();
        p.feed("codex", r#"{"type":"thread.started","thread_id":"t-9"}"#);
        p.feed("codex", r#"{"type":"item.completed","item":{"id":"i1","type":"file_change","changes":[{"path":"a.ts","kind":"update"}]}}"#);
        p.feed("codex", r#"{"type":"item.completed","item":{"id":"i2","type":"agent_message","text":"已完成"}}"#);
        p.feed("codex", r#"{"type":"turn.completed","usage":{"input_tokens":1200,"cached_input_tokens":800,"output_tokens":90,"reasoning_output_tokens":10}}"#);
        assert_eq!(p.session_id.as_deref(), Some("t-9"));
        assert_eq!((p.input, p.cache_read, p.output), (Some(1200), Some(800), Some(90)));
        assert_eq!(p.result.as_deref(), Some("已完成"));
        assert_eq!(classify(&p, true, "", false), Outcome::Completed);

        let mut auth = Progress::default();
        auth.feed("codex", r#"{"type":"turn.failed","error":{"message":"401 Unauthorized: please log in again"}}"#);
        assert!(matches!(classify(&auth, false, "", false), Outcome::Reauth(_)));
        let failed = Progress::default();
        assert_eq!(classify(&failed, false, "boom", false), Outcome::Failed("boom".into()));
        assert_eq!(build_args("codex", Some("gpt-5"), Some("t-9")), vec!["exec", "--json", "-m", "gpt-5", "resume", "t-9", "-"]);
        assert_eq!(build_args("claude", None, Some("s-1")), vec!["-p", "--output-format", "stream-json", "--verbose", "--resume", "s-1"]);
    }

    fn input(launch: &str) -> TaskInput {
        TaskInput {
            name: "完善登录页面".into(),
            platform: "claude".into(),
            connection_id: "c1".into(),
            model: Some("claude-sonnet-4-5".into()),
            cwd: std::env::temp_dir().display().to_string(),
            session_id: None,
            prompt: "完善表单校验".into(),
            skills: vec![],
            launch: launch.into(),
            schedule: None,
            auto_resume: true,
            wait_deadline: None,
            serial_queue: true,
        }
    }

    #[test]
    fn run_waits_for_quota_resumes_and_queues_per_account() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let first = create_task(&conn, &input("now"), now).unwrap();
        let second = create_task(&conn, &input("now"), now + 1).unwrap();
        let queue = startable(&conn).unwrap();
        assert_eq!(queue.len(), 1, "同一账号只启动队首");
        let run_id = queue[0];
        conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2 WHERE id=?1", params![run_id, now]).unwrap();
        let p = Progress { session_id: Some("s-1".into()), input: Some(10), output: Some(2), ..Default::default() };
        finish(&conn, run_id, &Outcome::Limit { resets_at: Some(now + 60_000), message: "usage limit".into() }, &p, now + 5_000).unwrap();
        let run = load_run(&conn, run_id).unwrap().unwrap();
        assert_eq!((run.status.as_str(), run.exec_ms, run.waits, run.resume_pending), ("waiting", 5_000, 1, true));
        assert!(startable(&conn).unwrap().is_empty(), "等待额度的运行挡住同账号后续任务");
        assert_eq!(queue_ahead(&conn, &active_run(&conn, second).unwrap().unwrap()).unwrap(), 1);

        let failed = QuotaVerdict::simple("unknown", "网络不可用");
        apply_check(&conn, run_id, &failed, now + 70_000).unwrap();
        let run = load_run(&conn, run_id).unwrap().unwrap();
        assert_eq!((run.status.as_str(), run.block_kind.as_deref()), ("waiting", Some("check_failed")), "检查失败不当作额度耗尽");
        let ok = QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows: vec![] };
        assert!(apply_check(&conn, run_id, &ok, now + 100_000).unwrap());
        let run = load_run(&conn, run_id).unwrap().unwrap();
        assert_eq!((run.status.as_str(), run.wait_ms), ("queued", 95_000));
        assert_eq!(startable(&conn).unwrap(), vec![run_id]);

        act(&conn, first, "cancel", now + 200_000).unwrap();
        assert_eq!(load_run(&conn, run_id).unwrap().unwrap().status, "cancelled");
        assert_eq!(startable(&conn).unwrap().len(), 1, "取消后轮到下一个任务");
    }

    /* ── 任务 2.6：关闭同账号排队（D-09） ── */

    /// 新建一个立即执行的任务，返回它的运行；工作目录直接写库（`startable` 不访问磁盘）
    fn queued(conn: &Connection, connection: &str, cwd: &str, serial: bool, now: i64) -> i64 {
        let mut i = input("now");
        i.connection_id = connection.into();
        i.serial_queue = serial;
        let task = create_task(conn, &i, now).unwrap();
        conn.execute("UPDATE automation_tasks SET cwd=?2 WHERE id=?1", params![task, cwd]).unwrap();
        active_run(conn, task).unwrap().unwrap().id
    }

    fn set_status(conn: &Connection, run: i64, status: &str) {
        conn.execute("UPDATE automation_runs SET status=?2 WHERE id=?1", params![run, status]).unwrap();
    }

    #[test]
    fn startable_skips_serial_for_opt_out() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let a = queued(&conn, "c1", r"D:\work\a", true, now);
        let b = queued(&conn, "c1", r"D:\work\b", true, now + 1);
        let c = queued(&conn, "c1", r"D:\work\c", false, now + 2);
        set_status(&conn, a, "running");
        assert_eq!(startable(&conn).unwrap(), vec![c], "排队任务等 A，关闭排队的 C 直接放行");
        set_status(&conn, c, "running");
        set_status(&conn, a, "completed");
        assert_eq!(startable(&conn).unwrap(), vec![b], "A 完成后轮到排队的 B，不受并行的 C 影响");
        assert_eq!(queue_ahead(&conn, &load_run(&conn, b).unwrap().unwrap()).unwrap(), 0, "队列位置只统计排队任务");
    }

    #[test]
    fn same_cwd_always_serial() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let a = queued(&conn, "c1", r"D:\Work\Proj", true, now);
        let c = queued(&conn, "c1", "d:/work/proj/", false, now + 1);
        let _d = queued(&conn, "c1", "d:/work/proj", false, now + 2);
        set_status(&conn, a, "running");
        assert!(startable(&conn).unwrap().is_empty(), "同一工作目录（大小写、斜杠不同）不并行");
        set_status(&conn, a, "completed");
        assert_eq!(startable(&conn).unwrap(), vec![c], "同一目录一次只放行一个");
    }

    #[test]
    fn parallel_cap_per_account() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        for i in 0..3 {
            let run = queued(&conn, "c1", &format!(r"D:\work\p{i}"), false, now + i);
            set_status(&conn, run, "running");
        }
        let _fourth = queued(&conn, "c1", r"D:\work\p3", false, now + 3);
        let other = queued(&conn, "c2", r"D:\work\p4", false, now + 4);
        assert_eq!(startable(&conn).unwrap(), vec![other], "同账号最多 3 个并行，其他账号不受影响");
    }

    #[test]
    fn quota_wait_blocks_whole_account() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let r1 = queued(&conn, "c1", r"D:\work\r1", false, now);
        let r2 = queued(&conn, "c1", r"D:\work\r2", false, now + 1);
        set_status(&conn, r1, "running");
        conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2 WHERE id=?1", params![r2, now]).unwrap();
        let p = Progress { session_id: Some("s-2".into()), ..Default::default() };
        finish(&conn, r2, &Outcome::Limit { resets_at: Some(now + 60_000), message: "usage limit".into() }, &p, now + 5_000).unwrap();
        assert_eq!(load_run(&conn, r2).unwrap().unwrap().status, "waiting");
        let _serial = queued(&conn, "c1", r"D:\work\s", true, now + 2);
        let _parallel = queued(&conn, "c1", r"D:\work\n", false, now + 3);
        let other = queued(&conn, "c2", r"D:\work\x", false, now + 4);
        assert_eq!(startable(&conn).unwrap(), vec![other], "额度等待挡住同账号全部新任务，其他账号照常");
        assert_eq!(load_run(&conn, r1).unwrap().unwrap().status, "running", "正在运行的任务跑完当前这一轮");
    }

    #[test]
    fn recovery_releases_serial_head_first() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let n1 = queued(&conn, "c1", r"D:\work\n1", false, now);
        let mut waiting = input("quota");
        waiting.connection_id = "c1".into();
        let head_task = create_task(&conn, &waiting, now + 1).unwrap();
        let head = active_run(&conn, head_task).unwrap().unwrap().id;
        let _b = queued(&conn, "c1", r"D:\work\b", true, now + 2);
        let n2 = queued(&conn, "c1", r"D:\work\n2", false, now + 3);
        let _n3 = queued(&conn, "c1", r"D:\work\n3", false, now + 4);
        assert!(startable(&conn).unwrap().is_empty(), "等待额度期间整个账号不放行");
        let ok = QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows: vec![] };
        assert!(apply_check(&conn, head, &ok, now + 10_000).unwrap());
        assert_eq!(startable(&conn).unwrap(), vec![head, n1, n2], "先放行排队队首，再放行并行任务，总数不超过 3");
    }

    #[test]
    fn quota_checks_merge_per_account() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let mut w = input("quota");
        w.serial_queue = false;
        let w1 = active_run(&conn, create_task(&conn, &w, now).unwrap()).unwrap().unwrap().id;
        let w2 = active_run(&conn, create_task(&conn, &w, now + 1).unwrap()).unwrap().unwrap().id;
        w.connection_id = "c2".into();
        let w3 = active_run(&conn, create_task(&conn, &w, now + 2).unwrap()).unwrap().unwrap().id;
        let (due, _) = due_checks(&conn, now + 10).unwrap();
        assert_eq!(due, vec!["c1".to_string(), "c2".to_string()], "同账号只检查一次");
        let ok = QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows: vec![] };
        assert!(apply_account_check(&conn, "c1", None, now + 10, &ok, now + 20).unwrap());
        let status = |id| load_run(&conn, id).unwrap().unwrap().status;
        assert_eq!((status(w1).as_str(), status(w2).as_str(), status(w3).as_str()), ("queued", "queued", "waiting"), "一次检查结果写入该账号全部等待中的运行");
    }

    #[test]
    fn task_detail_queue_lists_serial_only() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let a = queued(&conn, "c1", r"D:\work\a", true, now);
        let _c = queued(&conn, "c1", r"D:\work\c", false, now + 1);
        let _b = queued(&conn, "c1", r"D:\work\b", true, now + 2);
        set_status(&conn, a, "running");
        let task_a = load_run(&conn, a).unwrap().unwrap().task_id;
        let names: Vec<(i64, String)> = task_detail(&conn, task_a).unwrap().queue.into_iter().map(|q| (q.task_id, q.status)).collect();
        assert_eq!(names, vec![(1, "running".to_string()), (3, "queued".to_string())], "同账号队列不列关闭排队的任务");
    }

    #[test]
    fn same_cwd_serial_across_accounts() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let a = queued(&conn, "c1", r"D:\Work\Shared", true, now);
        let _b = queued(&conn, "c2", "d:/work/shared/", false, now + 1);
        set_status(&conn, a, "running");
        assert!(startable(&conn).unwrap().is_empty(), "不同账号在同一工作目录（大小写、斜杠不同）也不同时运行");
    }

    #[test]
    fn parallel_cap_counts_running_serial() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let s = queued(&conn, "c1", r"D:\work\s", true, now);
        let n1 = queued(&conn, "c1", r"D:\work\n1", false, now + 1);
        let n2 = queued(&conn, "c1", r"D:\work\n2", false, now + 2);
        for run in [s, n1, n2] { set_status(&conn, run, "running"); }
        let _fourth = queued(&conn, "c1", r"D:\work\n3", false, now + 3);
        assert!(startable(&conn).unwrap().is_empty(), "1 个排队任务 + 2 个并行任务在运行时，第 4 个并行任务不放行");
    }

    #[test]
    fn stale_check_skips_runs_waiting_after_query_start() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let mut w = input("quota");
        w.serial_queue = false;
        let before = active_run(&conn, create_task(&conn, &w, now).unwrap()).unwrap().unwrap().id;
        let checked_from = now + 10;
        let after = active_run(&conn, create_task(&conn, &w, now + 20).unwrap()).unwrap().unwrap().id;
        let ok = QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows: vec![] };
        assert!(apply_account_check(&conn, "c1", None, checked_from, &ok, now + 30).unwrap());
        let status = |id| load_run(&conn, id).unwrap().unwrap().status;
        assert_eq!((status(before).as_str(), status(after).as_str()), ("queued", "waiting"), "查询开始后才进入等待的运行不使用这次的旧结果");
        assert!(!apply_account_check(&conn, "c1", Some(after), checked_from, &ok, now + 40).unwrap(), "手动检查的运行同样只认查询开始前的等待");
        assert_eq!(status(after), "waiting");
    }

    #[test]
    fn paused_waiting_run_neither_blocks_nor_wakes() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let paused_task = create_task(&conn, &input("quota"), now).unwrap();
        let paused = active_run(&conn, paused_task).unwrap().unwrap().id;
        act(&conn, paused_task, "pause", now + 1).unwrap();
        let q = queued(&conn, "c1", r"D:\work\q", true, now + 2);
        assert_eq!(startable(&conn).unwrap(), vec![q], "暂停任务的等待运行不挡账号");
        let (due, _) = due_checks(&conn, now + 10).unwrap();
        assert!(due.is_empty(), "暂停任务的等待运行不触发额度检查");
        let ok = QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows: vec![] };
        assert!(!apply_account_check(&conn, "c1", None, now + 10, &ok, now + 20).unwrap());
        assert_eq!(load_run(&conn, paused).unwrap().unwrap().status, "waiting", "合并检查不唤醒暂停任务的等待运行");
    }

    #[test]
    fn serial_queue_column_migrates_with_default_on() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE automation_tasks (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, platform TEXT NOT NULL, connection_id TEXT NOT NULL, model TEXT, cwd TEXT NOT NULL, session_id TEXT, prompt TEXT NOT NULL, skills TEXT NOT NULL DEFAULT '[]', launch TEXT NOT NULL, schedule TEXT, auto_resume INTEGER NOT NULL DEFAULT 1, wait_deadline INTEGER, state TEXT NOT NULL DEFAULT 'active', next_run_at INTEGER, missed_at INTEGER, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             INSERT INTO automation_tasks (name, platform, connection_id, cwd, prompt, launch, created_at, updated_at) VALUES ('旧任务', 'claude', 'c1', 'D:/work', '执行', 'now', 1, 1);",
        ).unwrap();
        ensure_schema(&conn).unwrap();
        ensure_schema(&conn).unwrap();
        assert!(load_task(&conn, 1).unwrap().unwrap().serial_queue, "旧任务默认保持同账号排队");
        let parsed: TaskInput = serde_json::from_value(serde_json::json!({
            "name": "a", "platform": "claude", "connection_id": "c1", "model": null, "cwd": "D:/work", "session_id": null,
            "prompt": "p", "launch": "now", "schedule": null, "auto_resume": true, "wait_deadline": null
        })).unwrap();
        assert!(parsed.serial_queue, "未传字段时默认开启");
    }

    /// 额度耗尽：Claude Code 与 Codex 各一段输出喂给 `classify`，任务进入等待并记下重置时间。
    /// 样例出处（2026-10-22 核对，本机 Claude Code 2.1.308、codex-cli 0.162.0）：
    /// - Claude：`rate_limit_event` 的字段与 `status: "rejected"`（已到限额）、重置时间为 Unix 秒，见官方文档
    ///   https://code.claude.com/docs/en/agent-sdk/python 的 RateLimitEvent / RateLimitInfo（CLI 原始字段名 `resetsAt`，
    ///   取自 claude.exe 内置的 `{status, resetsAt, rateLimitType}` 序列化代码）；提示文本 `You've hit your <限额名> · resets <时间>`
    ///   与限额名 `session limit` 取自 claude.exe 内置字符串。旧格式 `Claude AI usage limit reached|<时间戳>` 在 2.1.308 中已不存在。
    ///   整行 JSON（result 的 `is_error: true`）按上述字段构造。
    /// - Codex：提示文本 `You’ve hit your usage limit. Upgrade to Pro (…), visit … to purchase more credits or try again at <时间>.`
    ///   与跨天时间格式 `%b %-d{st|nd|rd|th}, %Y %-I:%M %p`（本机时区）取自 codex.exe 内置字符串；
    ///   JSONL 外壳（`error`、`turn.failed`）按 `Progress::feed` 现有的解析规则构造。
    #[test]
    fn classify_quota_exhausted_enters_waiting() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let running = |task: i64| {
            let run_id = active_run(&conn, task).unwrap().unwrap().id;
            conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2 WHERE id=?1", params![run_id, now]).unwrap();
            run_id
        };

        let claude = create_task(&conn, &input("now"), now).unwrap();
        let run_id = running(claude);
        let text = "You've hit your session limit · resets 3pm (Asia/Shanghai)";
        let mut p = Progress::default();
        p.feed("claude", r#"{"type":"system","subtype":"init","session_id":"s-limit"}"#);
        p.feed("claude", r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1792738800,"rateLimitType":"five_hour"},"uuid":"u-1","session_id":"s-limit"}"#);
        p.feed("claude", &serde_json::json!({"type":"result","subtype":"success","is_error":true,"result":text,"session_id":"s-limit"}).to_string());
        let outcome = classify(&p, false, "", false);
        assert_eq!(outcome, Outcome::Limit { resets_at: Some(1_792_738_800_000), message: text.into() });
        finish(&conn, run_id, &outcome, &p, now + 5_000).unwrap();
        let run = load_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(
            (run.status.as_str(), run.block_kind.as_deref(), run.resets_at, run.next_check_at, run.resume_pending, run.session_id.as_deref()),
            ("waiting", Some("quota"), Some(1_792_738_800_000), Some(1_792_738_830_000), true, Some("s-limit")),
        );

        let mut codex_input = input("now");
        codex_input.platform = "codex".into();
        codex_input.connection_id = "c2".into();
        codex_input.model = None;
        let codex = create_task(&conn, &codex_input, now).unwrap();
        let run_id = running(codex);
        let text = "You’ve hit your usage limit. Upgrade to Pro (https://chatgpt.com/explore/pro), visit https://chatgpt.com/settings/usage to purchase more credits or try again at Oct 23rd, 2026 3:05 PM.";
        let mut p = Progress::default();
        p.feed("codex", r#"{"type":"thread.started","thread_id":"t-limit"}"#);
        p.feed("codex", &serde_json::json!({"type":"error","message":text}).to_string());
        p.feed("codex", &serde_json::json!({"type":"turn.failed","error":{"message":text}}).to_string());
        let reset = Local.with_ymd_and_hms(2026, 10, 23, 15, 5, 0).earliest().unwrap().timestamp_millis();
        let outcome = classify(&p, false, "", false);
        assert_eq!(outcome, Outcome::Limit { resets_at: Some(reset), message: text.into() });
        finish(&conn, run_id, &outcome, &p, now + 5_000).unwrap();
        let run = load_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(
            (run.status.as_str(), run.resets_at, run.resume_pending, run.session_id.as_deref()),
            ("waiting", Some(reset), true, Some("t-limit")),
        );
    }

    /// `rate_limit_event` 只在文本没有给出判断时兜底；超额用量仍可用（`overageStatus` 为 allowed / allowed_warning）时会话会继续，
    /// 不算额度耗尽。字段名：官方 RateLimitInfo 的 `overage_status`，CLI 原始输出为 `overageStatus`（claude.exe 2.1.308 内置代码）。
    /// 认证与 5xx 的提示文本为构造样例。
    #[test]
    fn rate_limit_event_does_not_hide_other_failures() {
        let rejected = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1792738800,"rateLimitType":"five_hour"},"uuid":"u-1","session_id":"s-1"}"#;
        let overage = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1792738800,"rateLimitType":"five_hour","overageStatus":"allowed","isUsingOverage":true},"uuid":"u-1","session_id":"s-1"}"#;
        let result = |text: &str| serde_json::json!({"type":"result","subtype":"success","is_error":true,"result":text,"session_id":"s-1"}).to_string();

        // 主额度耗尽后又因认证过期失败：按文本判为需要重新登录
        let mut p = Progress::default();
        p.feed("claude", rejected);
        p.feed("claude", &result("OAuth token has expired. Please run /login"));
        assert!(matches!(classify(&p, false, "", false), Outcome::Reauth(_)));

        // 超额用量可用：不置限额标记；成功即完成，普通失败仍是失败
        let mut p = Progress::default();
        p.feed("claude", overage);
        assert!(!p.rate_limited);
        p.feed("claude", r#"{"type":"result","subtype":"success","is_error":false,"result":"OK","session_id":"s-1"}"#);
        assert_eq!(classify(&p, true, "", false), Outcome::Completed);
        let mut p = Progress::default();
        p.feed("claude", overage);
        p.feed("claude", &result("API Error: 500 Internal server error"));
        assert_eq!(classify(&p, false, "", false), Outcome::Failed("API Error: 500 Internal server error".into()));

        // 文本没有判断依据时，rejected 事件兜底判为额度耗尽并带上重置时间
        let mut p = Progress::default();
        p.feed("claude", rejected);
        p.feed("claude", r#"{"type":"result","subtype":"error_during_execution","is_error":true,"session_id":"s-1"}"#);
        assert_eq!(
            classify(&p, false, "", false),
            Outcome::Limit { resets_at: Some(1_792_738_800_000), message: "执行器返回 error_during_execution".into() },
        );
    }

    /// 样例为真实记录：2026-10-22 16:02 Codex 在非 Git 目录执行时 stderr 的原文（codex-cli 0.162.0）
    #[test]
    fn codex_untrusted_dir_needs_attention() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let mut i = input("now");
        i.platform = "codex".into();
        i.model = None;
        let id = create_task(&conn, &i, now).unwrap();
        let run_id = active_run(&conn, id).unwrap().unwrap().id;
        conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2 WHERE id=?1", params![run_id, now]).unwrap();
        let stderr = "Not inside a trusted directory and --skip-git-repo-check was not specified.";
        let outcome = classify(&Progress::default(), false, stderr, false);
        assert!(!matches!(outcome, Outcome::Failed(_)), "不应归为运行失败：{outcome:?}");
        finish(&conn, run_id, &outcome, &Progress::default(), now + 1_000).unwrap();
        let run = load_run(&conn, run_id).unwrap().unwrap();
        assert_eq!(
            (run.status.as_str(), run.block_kind.as_deref(), run.reason.as_deref(), run.error.as_deref()),
            ("attention", Some("cwd_missing"), Some("Codex 只能在 Git 仓库或已信任的目录中执行。请把工作目录改为 Git 仓库，或先在 Codex 中信任该目录。"), Some(stderr)),
        );
        assert_eq!(run.ended_at, None, "需要处理的运行仍可重新排队");
    }

    #[test]
    fn codex_retry_time_handles_same_day_and_dated_formats() {
        let tz = shanghai();
        let now = ms(&tz, 2026, 10, 22, 14, 0);
        let msg = |at: &str| format!("You’ve hit your usage limit. Upgrade to Plus to continue using Codex (https://chatgpt.com/explore/plus), or try again at {at}.");
        assert_eq!(codex_retry_at(&tz, &msg("3:05 PM"), now), Some(ms(&tz, 2026, 10, 22, 15, 5)));
        // 当天的时刻已经过去时视为次日
        assert_eq!(codex_retry_at(&tz, &msg("9:30 AM"), now), Some(ms(&tz, 2026, 10, 23, 9, 30)));
        // 提示只精确到分钟：恰好在重置的那一分钟内（或刚过几分钟）仍按当天算，不推迟 24 小时
        assert_eq!(codex_retry_at(&tz, &msg("2:00 PM"), now + 30_000), Some(ms(&tz, 2026, 10, 22, 14, 0)));
        assert_eq!(codex_retry_at(&tz, &msg("1:56 PM"), now), Some(ms(&tz, 2026, 10, 22, 13, 56)));
        assert_eq!(codex_retry_at(&tz, &msg("1:50 PM"), now), Some(ms(&tz, 2026, 10, 23, 13, 50)));
        assert_eq!(codex_retry_at(&tz, &msg("Nov 1st, 2026 12:00 AM"), now), Some(ms(&tz, 2026, 11, 1, 0, 0)));
        assert_eq!(codex_retry_at(&tz, "You’ve hit your usage limit. or try again later.", now), None);
        assert!(is_limit("You've hit your weekly limit · resets Oct 27, 9am"));
    }

    #[test]
    fn schedules_fire_skip_overlap_and_mark_missed() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let tz = shanghai();
        let now = ms(&tz, 2026, 10, 22, 9, 0);
        let mut i = input("schedule");
        i.schedule = Some(sched("daily", "09:00", None, None));
        let id = create_task(&conn, &i, now - 3_600_000).unwrap();
        conn.execute("UPDATE automation_tasks SET next_run_at=?2 WHERE id=?1", params![id, now]).unwrap();
        assert!(fire_schedules(&conn, &tz, now + 1_000).unwrap());
        assert!(active_run(&conn, id).unwrap().is_some());
        assert_eq!(load_task(&conn, id).unwrap().unwrap().next_run_at, Some(ms(&tz, 2026, 10, 23, 9, 0)));
        // 次日太晚才发现：标「已错过」，不自动执行
        let late = ms(&tz, 2026, 10, 23, 10, 0);
        conn.execute("UPDATE automation_runs SET status='completed'", []).unwrap();
        fire_schedules(&conn, &tz, late).unwrap();
        let task = load_task(&conn, id).unwrap().unwrap();
        assert_eq!(task.missed_at, Some(ms(&tz, 2026, 10, 23, 9, 0)));
        assert!(active_run(&conn, id).unwrap().is_none());
        recover(&conn, late).unwrap();
    }

    /* ── 任务 2.9：删除任务 ── */

    fn count(conn: &Connection, sql: &str, id: i64) -> i64 {
        conn.query_row(sql, [id], |r| r.get(0)).unwrap()
    }

    #[test]
    fn delete_task_removes_runs_keeps_usage() {
        let mut conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        conn.execute_batch("CREATE TABLE requests (id INTEGER PRIMARY KEY, platform TEXT NOT NULL, ts INTEGER NOT NULL); INSERT INTO requests (platform, ts) VALUES ('claude', 1);").unwrap();
        let now = 1_000_000;
        let id = create_task(&conn, &input("now"), now).unwrap();
        let first = active_run(&conn, id).unwrap().unwrap().id;
        set_status(&conn, first, "completed");
        act(&conn, id, "run", now + 10).unwrap();
        let second = active_run(&conn, id).unwrap().unwrap().id;
        add_event(&conn, second, now + 20, "log", "执行日志", None).unwrap();
        let other = create_task(&conn, &input("now"), now + 30).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM automation_runs WHERE task_id = ?1", id), 2);
        assert!(count(&conn, "SELECT COUNT(*) FROM automation_events WHERE run_id IN (SELECT id FROM automation_runs WHERE task_id = ?1)", id) >= 3);

        delete_task(&mut conn, id).unwrap();
        assert!(load_task(&conn, id).unwrap().is_none());
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM automation_runs WHERE task_id = ?1", id), 0);
        let left: i64 = conn.query_row("SELECT COUNT(*) FROM automation_events WHERE run_id IN (?1, ?2)", params![first, second], |r| r.get(0)).unwrap();
        assert_eq!(left, 0, "两条运行的事件都已删除");
        let usage: i64 = conn.query_row("SELECT COUNT(*) FROM requests", [], |r| r.get(0)).unwrap();
        assert_eq!(usage, 1, "用量统计保留");
        assert!(load_task(&conn, other).unwrap().is_some() && active_run(&conn, other).unwrap().is_some(), "其他任务不受影响");
        assert_eq!(delete_task(&mut conn, id).unwrap_err(), "任务不存在");
    }

    #[test]
    fn delete_running_task_rejected() {
        let mut conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let id = create_task(&conn, &input("now"), now).unwrap();
        let run = active_run(&conn, id).unwrap().unwrap().id;
        set_status(&conn, run, "running");
        let events = count(&conn, "SELECT COUNT(*) FROM automation_events WHERE run_id = ?1", run);
        assert_eq!(delete_task(&mut conn, id).unwrap_err(), "任务正在运行，请先停止");
        assert!(load_task(&conn, id).unwrap().is_some());
        assert_eq!(load_run(&conn, run).unwrap().unwrap().status, "running", "数据不变");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM automation_events WHERE run_id = ?1", run), events);
    }

    #[test]
    fn delete_waiting_task_cancels_wait() {
        let mut conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let waiting = create_task(&conn, &input("quota"), now).unwrap();
        let wait_run = active_run(&conn, waiting).unwrap().unwrap().id;
        let next = queued(&conn, "c1", r"D:\work\next", true, now + 1);
        assert!(startable(&conn).unwrap().is_empty(), "等待额度挡住同账号");
        assert_eq!(due_checks(&conn, now + 10).unwrap().0, vec!["c1".to_string()]);

        delete_task(&mut conn, waiting).unwrap();
        assert!(load_run(&conn, wait_run).unwrap().is_none());
        assert_eq!(startable(&conn).unwrap(), vec![next], "删除后不再挡住账号，也不会被启动");
        assert!(due_checks(&conn, now + 20).unwrap().0.is_empty(), "等待检查列表里不再出现");
        let ok = QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows: vec![] };
        assert!(!apply_account_check(&conn, "c1", Some(wait_run), now, &ok, now + 30).unwrap(), "拿旧 id 检查不会复活运行");

        let queued_task = load_run(&conn, next).unwrap().unwrap().task_id;
        delete_task(&mut conn, queued_task).unwrap();
        assert!(startable(&conn).unwrap().is_empty(), "排队中的任务删除后不再启动");
        assert!(!claim_run(&conn, next).unwrap(), "调度线程拿着旧 id 也认领不到");
    }

    #[test]
    fn claim_run_only_takes_queued() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let run = queued(&conn, "c1", r"D:\work\a", true, 1_000_000);
        assert!(claim_run(&conn, run).unwrap());
        assert_eq!(load_run(&conn, run).unwrap().unwrap().status, "running");
        assert!(!claim_run(&conn, run).unwrap(), "已认领的运行不重复启动");

        // 续跑：执行中额度耗尽 → 等待（resume_pending）→ 额度恢复转回排队，沿用原运行 ID，可以再次认领
        let now = 1_000_000;
        conn.execute("UPDATE automation_runs SET exec_started_at=?2 WHERE id=?1", params![run, now]).unwrap();
        let p = Progress { session_id: Some("s-1".into()), ..Default::default() };
        finish(&conn, run, &Outcome::Limit { resets_at: Some(now + 60_000), message: "usage limit".into() }, &p, now + 5_000).unwrap();
        let ok = QuotaVerdict { state: "available".into(), reason: None, resets_at: None, windows: vec![] };
        assert!(apply_check(&conn, run, &ok, now + 100_000).unwrap());
        let resumed = load_run(&conn, run).unwrap().unwrap();
        assert_eq!((resumed.status.as_str(), resumed.resume_pending), ("queued", true));
        assert!(claim_run(&conn, run).unwrap(), "续跑的运行能被认领");
    }

    #[test]
    fn claim_run_skips_paused_task() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let run = queued(&conn, "c1", r"D:\work\a", true, now);
        let task = load_run(&conn, run).unwrap().unwrap().task_id;
        act(&conn, task, "pause", now + 1).unwrap();
        assert!(!claim_run(&conn, run).unwrap(), "暂停后认领失败");
        assert_eq!(load_run(&conn, run).unwrap().unwrap().status, "queued", "仍保持排队，恢复后再启动");
        act(&conn, task, "resume", now + 2).unwrap();
        assert!(claim_run(&conn, run).unwrap(), "恢复后可以认领");
    }

    #[test]
    fn set_attention_ignores_cancelled_and_deleted_runs() {
        let mut conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let run = queued(&conn, "c1", r"D:\work\a", true, now);
        let task = load_run(&conn, run).unwrap().unwrap().task_id;
        act(&conn, task, "cancel", now + 1).unwrap();
        let events = count(&conn, "SELECT COUNT(*) FROM automation_events WHERE run_id = ?1", run);
        set_attention(&conn, run, "tool_missing", "未检测到执行器", now + 2).unwrap();
        assert_eq!(load_run(&conn, run).unwrap().unwrap().status, "cancelled", "已取消的运行不改回需要处理");
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM automation_events WHERE run_id = ?1", run), events, "也不写事件");

        let other = queued(&conn, "c1", r"D:\work\b", true, now + 3);
        let other_task = load_run(&conn, other).unwrap().unwrap().task_id;
        delete_task(&mut conn, other_task).unwrap();
        set_attention(&conn, other, "tool_missing", "未检测到执行器", now + 4).unwrap();
        assert!(load_run(&conn, other).unwrap().is_none());
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM automation_events WHERE run_id = ?1", other), 0, "已删除的运行不留孤儿事件");
    }

    #[test]
    fn stop_requested_before_register_kills_on_register() {
        let mut procs = Procs::default();
        procs.clear_stop(7);
        assert_eq!(procs.request_stop(7), None, "进程还没登记：只留下停止标记");
        assert!(procs.register(7, 4242), "登记时发现停止请求，需要立即结束");
        assert!(procs.exited(7), "退出时按已停止收尾");
        assert!(!procs.exited(7), "标记随退出清除");

        // 正常顺序：先登记再停止，返回进程 ID 由调用方结束
        assert!(!procs.register(8, 5151));
        assert_eq!(procs.request_stop(8), Some(5151));
        assert!(procs.exited(8));

        // 上一轮遗留的标记（续跑沿用运行 ID）在认领前清掉，不会误杀新一轮
        procs.request_stop(9);
        procs.clear_stop(9);
        assert!(!procs.register(9, 6161));
        assert!(!procs.exited(9));
    }

    /* ── 任务 3.3：自动化选择 Skills（D-08） ── */

    fn option(name: &str) -> crate::skills::SkillOption {
        crate::skills::SkillOption { name: name.into(), description: None, scope: "global".into(), project: None }
    }

    #[test]
    fn skill_prefix_formats() {
        let names = vec!["a".to_string(), "b".to_string()];
        assert_eq!(skill_prefix("codex", &names), "$a $b\n\n");
        assert_eq!(skill_prefix("claude", &names), "本任务请使用以下 Skills：a、b\n\n");
        assert_eq!(skill_prefix("claude", &[]), "");
        assert_eq!(skill_prefix("codex", &[]), "");
    }

    #[test]
    fn validate_rejects_too_many_skills() {
        let now = 1_000_000;
        let mut i = input("now");
        i.skills = (0..11).map(|n| format!("skill-{n}")).collect();
        assert_eq!(validate(&i, now).unwrap_err(), "最多选择 10 个 Skills");
        i.skills.truncate(10);
        assert_eq!(validate(&i, now), Ok(()));
        i.skills = vec!["Bad Name".into()];
        assert_eq!(validate(&i, now).unwrap_err(), "Skill 名称无效：Bad Name");
    }

    #[test]
    fn run_blocks_when_skill_missing() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let mut i = input("now");
        i.skills = vec!["x".into(), "y".into()];
        let task = create_task(&conn, &i, now).unwrap();
        let run = active_run(&conn, task).unwrap().unwrap().id;
        let selected = load_task(&conn, task).unwrap().unwrap().skills;
        assert_eq!(selected, vec!["x".to_string(), "y".to_string()], "选择随任务保存");
        let available = vec![option("y"), option("z")];
        assert_eq!(missing_skills(&selected, &available), vec!["x".to_string()]);
        assert!(missing_skills(&selected, &[option("x"), option("y")]).is_empty());

        assert!(!skills_ready(&conn, run, &selected, &available, now + 1).unwrap(), "缺少 Skill 时不放行");
        let r = load_run(&conn, run).unwrap().unwrap();
        assert_eq!((r.status.as_str(), r.block_kind.as_deref(), r.reason.as_deref()), ("attention", Some("skill_missing"), Some("Skill「x」已停用或不存在")));
        assert_eq!(r.started_at, None, "没有启动进程");
        assert!(!claim_run(&conn, run).unwrap(), "需要处理的运行不会被认领拉起");

        // 续跑前同样检查：重新排队后 Skill 仍缺失，再次转为需要处理
        act(&conn, task, "retry", now + 2).unwrap();
        assert!(!skills_ready(&conn, run, &selected, &[option("y")], now + 3).unwrap());
        assert_eq!(load_run(&conn, run).unwrap().unwrap().status, "attention");
        act(&conn, task, "retry", now + 4).unwrap();
        assert!(skills_ready(&conn, run, &selected, &[option("x"), option("y")], now + 5).unwrap(), "Skill 都在时放行");
        assert_eq!(load_run(&conn, run).unwrap().unwrap().status, "queued");
        assert_eq!(missing_reason(&["x".into(), "z".into()]), "Skill「x」、「z」已停用或不存在");
    }

    /// 控制方裁决：挂在已取消 / 已完成任务下的排队运行不再放行（与 `claim_run` 的 `state = 'active'` 一致）。
    /// 孤儿运行按旧库遗留数据直接写库构造（修复后 `finish` 不再产生它们）
    #[test]
    fn startable_skips_runs_of_inactive_tasks() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let a = queued(&conn, "c1", r"D:\work\a", true, now);
        let task_a = load_run(&conn, a).unwrap().unwrap().task_id;
        conn.execute("UPDATE automation_tasks SET state='cancelled' WHERE id=?1", [task_a]).unwrap();
        set_status(&conn, a, "waiting");
        let b = queued(&conn, "c1", r"D:\work\b", true, now + 3);
        assert_eq!(startable(&conn).unwrap(), vec![b], "已取消任务的等待运行不挡同账号");
        set_status(&conn, a, "queued");
        assert_eq!(startable(&conn).unwrap(), vec![b], "已取消任务的排队运行不放行");
        assert_eq!(queue_ahead(&conn, &load_run(&conn, b).unwrap().unwrap()).unwrap(), 0, "队列位置不计已取消任务");
        let task_b = load_run(&conn, b).unwrap().unwrap().task_id;
        let queue: Vec<i64> = task_detail(&conn, task_b).unwrap().queue.into_iter().map(|q| q.task_id).collect();
        assert_eq!(queue, vec![task_b], "同账号队列不列已取消任务");

        let c = queued(&conn, "c2", r"D:\work\c", true, now + 4);
        let task_c = load_run(&conn, c).unwrap().unwrap().task_id;
        conn.execute("UPDATE automation_tasks SET state='finished' WHERE id=?1", [task_c]).unwrap();
        assert_eq!(startable(&conn).unwrap(), vec![b], "已完成任务的排队运行不放行");

        // 取消后进程还没退出的运行仍占着工作目录
        let d = queued(&conn, "c3", r"D:\work\d", true, now + 5);
        let task_d = load_run(&conn, d).unwrap().unwrap().task_id;
        set_status(&conn, d, "running");
        act(&conn, task_d, "cancel", now + 6).unwrap();
        let _e = queued(&conn, "c4", r"D:\work\d", true, now + 7);
        assert_eq!(startable(&conn).unwrap(), vec![b], "同一工作目录仍串行");

        // 也仍计入同账号并行上限：3 个执行中（其中 1 个所属任务已取消）时第 4 个不放行
        let mut running = Vec::new();
        for i in 0..3 {
            let r = queued(&conn, "c5", &format!(r"D:\work\cap{i}"), false, now + 10 + i);
            set_status(&conn, r, "running");
            running.push(r);
        }
        act(&conn, load_run(&conn, running[0]).unwrap().unwrap().task_id, "cancel", now + 20).unwrap();
        let fourth = queued(&conn, "c5", r"D:\work\cap3", false, now + 21);
        assert!(!startable(&conn).unwrap().contains(&fourth), "已取消但仍在执行的运行占用并行名额");
    }

    #[test]
    fn finish_on_cancelled_task_ends_as_cancelled() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let p = Progress { session_id: Some("s-1".into()), ..Default::default() };
        let cases = [
            Outcome::Limit { resets_at: Some(now + 60_000), message: "usage limit".into() },
            Outcome::Reauth("401".into()),
            Outcome::UntrustedDir("Not inside a trusted directory".into()),
        ];
        for (i, outcome) in cases.iter().enumerate() {
            let run = queued(&conn, "c1", &format!(r"D:\work\f{i}"), false, now + i as i64);
            let task = load_run(&conn, run).unwrap().unwrap().task_id;
            conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2 WHERE id=?1", params![run, now]).unwrap();
            act(&conn, task, "cancel", now + 1).unwrap();
            assert_eq!(load_run(&conn, run).unwrap().unwrap().status, "running", "取消时执行中的运行等进程结束");
            finish(&conn, run, outcome, &p, now + 5_000).unwrap();
            let r = load_run(&conn, run).unwrap().unwrap();
            assert_eq!((r.status.as_str(), r.ended_at, r.resume_pending, r.block_kind), ("cancelled", Some(now + 5_000), false, None), "{outcome:?} 按已取消收尾");
            assert_eq!((r.exec_ms, r.session_id.as_deref()), (5_000, Some("s-1")), "执行时长与会话照常记录");
        }
        assert!(due_checks(&conn, now + 10_000).unwrap().0.is_empty(), "不会对它们做额度检查");

        // 完成是真实结果：照常记为完成，任务保持已取消
        let run = queued(&conn, "c1", r"D:\work\done", false, now + 10);
        let task = load_run(&conn, run).unwrap().unwrap().task_id;
        conn.execute("UPDATE automation_runs SET status='running', exec_started_at=?2 WHERE id=?1", params![run, now]).unwrap();
        act(&conn, task, "cancel", now + 11).unwrap();
        finish(&conn, run, &Outcome::Completed, &p, now + 12).unwrap();
        assert_eq!(load_run(&conn, run).unwrap().unwrap().status, "completed");
        assert_eq!(load_task(&conn, task).unwrap().unwrap().state, "cancelled");
    }

    #[test]
    fn due_checks_skip_cancelled_tasks() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let now = 1_000_000;
        let orphan_task = create_task(&conn, &input("quota"), now).unwrap();
        conn.execute("UPDATE automation_tasks SET state='cancelled' WHERE id=?1", [orphan_task]).unwrap();
        let mut other = input("quota");
        other.connection_id = "c2".into();
        create_task(&conn, &other, now + 1).unwrap();
        assert_eq!(due_checks(&conn, now + 10).unwrap().0, vec!["c2".to_string()], "已取消任务遗留的等待运行不再触发额度检查");
    }

    #[test]
    fn run_prompt_adds_prefix_for_first_run_and_resume() {
        let skills = vec!["a".to_string(), "b".to_string()];
        assert_eq!(run_prompt("claude", &skills, "完善表单", false), "本任务请使用以下 Skills：a、b\n\n完善表单");
        assert_eq!(run_prompt("codex", &skills, "完善表单", true), format!("$a $b\n\n{RESUME_PROMPT}"));
        assert_eq!(run_prompt("claude", &[], "完善表单", false), "完善表单");
        assert_eq!(run_prompt("codex", &[], "完善表单", true), RESUME_PROMPT);
    }

    #[test]
    fn validate_rejects_duplicate_skills() {
        let mut i = input("now");
        i.skills = vec!["a".into(), "b".into(), "a".into()];
        assert_eq!(validate(&i, 1_000_000).unwrap_err(), "Skills 不能重复选择");
    }
}
