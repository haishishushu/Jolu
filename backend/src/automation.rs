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
            updated_at    INTEGER NOT NULL
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
    )
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
}

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

const TASK_COLS: &str = "id, name, platform, connection_id, model, cwd, session_id, prompt, skills, launch, schedule, auto_resume, wait_deadline, state, next_run_at, missed_at, created_at, updated_at";

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
        "INSERT INTO automation_tasks (name, platform, connection_id, model, cwd, session_id, prompt, skills, launch, schedule, auto_resume, wait_deadline, state, next_run_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'active', ?13, ?14, ?14)",
        params![
            input.name.trim(), input.platform, input.connection_id,
            input.model.as_deref().filter(|m| !m.is_empty()),
            input.cwd.trim(), input.session_id,
            input.prompt.trim(),
            serde_json::to_string(&input.skills).unwrap_or_else(|_| "[]".into()),
            input.launch,
            input.schedule.as_ref().and_then(|s| serde_json::to_string(s).ok()),
            input.auto_resume as i64, input.wait_deadline, next, now
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
        "UPDATE automation_tasks SET name=?2, platform=?3, connection_id=?4, model=?5, cwd=?6, session_id=?7, prompt=?8, skills=?9, launch=?10, schedule=?11, auto_resume=?12, wait_deadline=?13, next_run_at=?14, updated_at=?15 WHERE id=?1",
        params![
            id, input.name.trim(), input.platform, input.connection_id,
            input.model.as_deref().filter(|m| !m.is_empty()),
            input.cwd.trim(), input.session_id, input.prompt.trim(),
            serde_json::to_string(&input.skills).unwrap_or_else(|_| "[]".into()),
            input.launch,
            input.schedule.as_ref().and_then(|s| serde_json::to_string(s).ok()),
            input.auto_resume as i64, input.wait_deadline, next, now
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

fn queue_ahead(conn: &Connection, run: &Run) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id
         WHERE t.connection_id = ?1 AND t.state != 'paused' AND r.id != ?2
           AND r.status IN ('queued','waiting','running')
           AND (r.status = 'running' OR r.created_at < ?3 OR (r.created_at = ?3 AND r.id < ?2))",
        params![run.connection_id, run.id, run.created_at],
        |r| r.get(0),
    )
}

fn task_item(conn: &Connection, task: Task) -> rusqlite::Result<TaskItem> {
    let active = active_run(conn, task.id)?;
    let last = last_run(conn, task.id)?;
    let queue_ahead = match &active {
        Some(run) if run.status == "queued" => Some(queue_ahead(conn, run)?),
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
    /// 同账号的队列（执行中 / 等待额度 / 排队中），按先后顺序
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
         WHERE t.connection_id = ?1 AND t.state != 'paused' AND r.status IN ('queued','waiting','running')
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

fn set_attention(conn: &Connection, run_id: i64, kind: &str, reason: &str, now: i64) -> rusqlite::Result<()> {
    conn.execute("UPDATE automation_runs SET status='attention', block_kind=?2, reason=?3 WHERE id=?1", params![run_id, kind, reason])?;
    add_event(conn, run_id, now, "attention", "需要处理", Some(reason))
}

/// 每个账号只启动队首：执行中或等待额度的运行会挡住同账号后续任务；暂停的任务不占队列
pub fn startable(conn: &Connection) -> rusqlite::Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.status, t.connection_id FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id
         WHERE r.status IN ('queued','waiting','running') AND t.state != 'paused'
         ORDER BY CASE r.status WHEN 'running' THEN 0 ELSE 1 END, r.created_at, r.id",
    )?;
    let rows: Vec<(i64, String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (id, status, connection) in rows {
        if !seen.insert(connection) { continue; }
        if status == "queued" { out.push(id); }
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

/// 等待截止已过的运行转为「需要处理」，返回到期需要检查额度的运行
pub fn due_checks(conn: &Connection, now: i64) -> rusqlite::Result<(Vec<i64>, bool)> {
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
        "SELECT r.id FROM automation_runs r JOIN automation_tasks t ON t.id = r.task_id
         WHERE r.status = 'waiting' AND t.state != 'paused' AND (r.next_check_at IS NULL OR r.next_check_at <= ?1)",
    )?;
    let due = stmt.query_map([now], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok((due, !expired.is_empty()))
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

/* ─────────────────────────── 调度器 ─────────────────────────── */

struct Engine {
    wake: Mutex<bool>,
    cv: Condvar,
    /// 执行中的进程：运行 ID → 进程 ID
    procs: Mutex<HashMap<i64, u32>>,
    stopping: Mutex<HashSet<i64>>,
}

fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| Engine { wake: Mutex::new(false), cv: Condvar::new(), procs: Mutex::new(HashMap::new()), stopping: Mutex::new(HashSet::new()) })
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
    for run_id in due {
        check_run_blocking(app, run_id);
    }
    match with_conn(&db, |conn| startable(conn).map_err(|e| e.to_string())) {
        Ok(runs) => {
            for run_id in runs { start_run(app, run_id); }
        }
        Err(error) => eprintln!("[自动化] 读取队列失败：{error}"),
    }
}

fn check_run_blocking(app: &AppHandle, run_id: i64) {
    let db = shared(app);
    let Ok(Some(run)) = with_conn(&db, |conn| load_run(conn, run_id).map_err(|e| e.to_string())) else { return };
    let state = tauri::async_runtime::block_on(crate::query_connection_quota(app.clone(), run.connection_id.clone(), true));
    let v = match state {
        Ok(state) => verdict(&state),
        Err(error) => QuotaVerdict::simple("unknown", error),
    };
    if with_conn(&db, |conn| apply_check(conn, run_id, &v, now_ms()).map_err(|e| e.to_string())).is_ok() { emit(app); }
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
    let session = run.session_id.clone();
    let prompt = if run.resume_pending { RESUME_PROMPT.to_string() } else { task.prompt.clone() };
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
            "UPDATE automation_runs SET status='running', block_kind=NULL, reason=NULL, started_at=COALESCE(started_at, ?2), exec_started_at=?2 WHERE id=?1",
            params![run_id, now],
        ).map_err(|e| e.to_string())?;
        let (title, detail) = if resumed { ("恢复原会话", "继续未完成的任务") } else if session.is_some() { ("开始执行", "在所选会话中追加任务指令") } else { ("开始执行", "新建会话") };
        add_event(conn, run_id, now, "start", title, Some(detail)).map_err(|e| e.to_string())
    });
    engine().procs.lock().unwrap_or_else(|p| p.into_inner()).insert(run_id, child.id());
    emit(app);

    let app = app.clone();
    let platform = task.platform.clone();
    std::thread::spawn(move || {
        if let Some(mut stdin) = child.stdin.take() {
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
        let e = engine();
        e.procs.lock().unwrap_or_else(|p| p.into_inner()).remove(&run_id);
        let stopped = e.stopping.lock().unwrap_or_else(|p| p.into_inner()).remove(&run_id);
        let outcome = classify(&progress, exit_ok, &stderr_tail, stopped);
        if let Err(error) = with_conn(&db, |conn| finish(conn, run_id, &outcome, &progress, now_ms()).map_err(|e| e.to_string())) {
            eprintln!("[自动化] 记录运行结果失败：{error}");
        }
        emit(&app);
        kick();
    });
}

/// 结束整个进程树（执行器会再拉起子进程）
fn kill_run(run_id: i64) {
    let e = engine();
    let Some(pid) = e.procs.lock().unwrap_or_else(|p| p.into_inner()).get(&run_id).copied() else { return };
    e.stopping.lock().unwrap_or_else(|p| p.into_inner()).insert(run_id);
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

/// 任务操作：pause / resume / cancel / stop / run / retry / check
#[tauri::command]
pub async fn automation_action(app: AppHandle, id: i64, action: String) -> Result<(), String> {
    if action == "check" {
        let run = blocking(&app, move |conn| active_run(conn, id).map_err(|e| e.to_string())).await?
            .filter(|r| r.status == "waiting")
            .ok_or("当前没有等待额度的运行")?;
        let v = match crate::query_connection_quota(app.clone(), run.connection_id.clone(), true).await {
            Ok(state) => verdict(&state),
            Err(error) => QuotaVerdict::simple("unknown", error),
        };
        blocking(&app, move |conn| apply_check(conn, run.id, &v, now_ms()).map_err(|e| e.to_string())).await?;
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
}
