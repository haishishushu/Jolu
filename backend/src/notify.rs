//! 操作通知（画板 94 / D-10）：前台卡片与后台系统通知共用任务和余额提醒规则。
//!
//! - 只在 Rust 端发通知，前端没有通知权限；Windows 没有运行时通知权限，画板上的「引导入口」不做。
//! - 免打扰期间一律不弹；任务通知按「额度恢复与任务继续」「任务完成与失败」两个开关分别判断。
//! - 「需要处理」和「等待超时」归入失败类通知；任务已取消的收尾不通知。
//! - 使用 notification 2.7 的桌面点击回调（2.6 首次引入），任务正文点击通过 MainIntent 打开任务详情。
//!   回调必须在发送通知前注册；仅支持应用仍在运行时，完全退出后的系统通知不会冷启动应用。
//! - 开发模式下 Windows 通知显示 PowerShell 的名字和图标（插件只给安装版设置 AppUserModelID），属已知限制。

use std::collections::HashMap;
use std::sync::{atomic::{AtomicU64, Ordering}, LazyLock, Mutex};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::settings::Settings;

#[derive(Debug, Clone, PartialEq)]
pub enum TaskNotice {
    Resumed,
    Completed,
    Failed { reason: String },
}

/// 失败原因在通知正文里最多保留的字数
const REASON_LIMIT: usize = 60;
/// 通知附带的任务 ID；余额通知使用独立的连接 ID 定位账号。
const TASK_KEY: &str = "task_id";
const CONNECTION_KEY: &str = "connection_id";

#[derive(Clone, serde::Serialize)]
pub struct AppNotice {
    id: u64,
    #[serde(flatten)]
    body: NoticeBody,
}

#[derive(Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum NoticeBody {
    Balance { connection_id: String, connection_name: String, platform: String, remaining: f64, currency: String, threshold: f64 },
    Task { task_id: i64, state: &'static str, platform: String, task_name: String, reason: Option<String> },
}

static NEXT_NOTICE: AtomicU64 = AtomicU64::new(1);
static PENDING_NOTICES: LazyLock<Mutex<Vec<AppNotice>>> = LazyLock::new(|| Mutex::new(Vec::new()));

fn clicked_connection_id(action: &tauri_plugin_notification::ActionPerformed) -> Option<String> {
    if action.action_id() != "tap" { return None; }
    action.notification()?.extra().get(CONNECTION_KEY)?.as_str()
        .filter(|id| !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control)).map(str::to_owned)
}

fn clicked_task_id(action: &tauri_plugin_notification::ActionPerformed) -> Option<i64> {
    if action.action_id() != "tap" { return None; }
    action.notification()?.extra().get(TASK_KEY)?.as_i64().filter(|id| *id > 0)
}

/// setup 中注册一次，早于自动化调度启动；窗口操作交回主线程。
pub fn register_click_handler(app: &AppHandle) -> tauri_plugin_notification::Result<()> {
    let handle = app.clone();
    app.notification().on_action(move |action| {
        let intent = if let Some(task_id) = clicked_task_id(action) { crate::MainIntent::Automation { task_id } }
            else if let Some(connection_id) = clicked_connection_id(action) { crate::MainIntent::Providers { connection_id } }
            else { return };
        let target = handle.clone();
        if let Err(error) = handle.run_on_main_thread(move || {
            crate::show_main_with_intent(&target, intent);
        }) {
            eprintln!("[通知] 打开对应页面失败：{error}");
        }
    })
}

/// 遮蔽失败原因里可能带出的凭证：直接用会话交接的共享规则（`handoff_facts::redact`），两处口径一致
pub fn mask_secrets(text: &str) -> String {
    crate::handoff_facts::redact(text)
}

/// 失败原因合并成一行、遮蔽凭证，超过 60 字截断并以「…」结尾
fn short_reason(reason: &str) -> String {
    let line = mask_secrets(&reason.split_whitespace().collect::<Vec<_>>().join(" "));
    if line.chars().count() <= REASON_LIMIT { return line; }
    format!("{}…", line.chars().take(REASON_LIMIT).collect::<String>())
}

/// 任务通知的标题与正文；`platform` 是给用户看的工具名（如 Codex、Claude Code），与中文之间留一个空格（画板 94 样例）
pub fn task_message(kind: &TaskNotice, platform: &str, task_name: &str) -> (String, String) {
    match kind {
        TaskNotice::Resumed => ("任务已继续".into(), format!("{platform} 额度已恢复，「{task_name}」正在原会话中继续。")),
        TaskNotice::Completed => ("任务已完成".into(), format!("「{task_name}」已运行完成。")),
        TaskNotice::Failed { reason } => ("任务失败".into(), format!("「{task_name}」：{}", short_reason(reason))),
    }
}

/// 免打扰时一律不弹；恢复通知看「额度恢复与任务继续」，完成与失败看「任务完成与失败」
pub fn should_notify(settings: &Settings, kind: &TaskNotice) -> bool {
    if settings.dnd { return false; }
    match kind {
        TaskNotice::Resumed => settings.notify_task_resumed,
        TaskNotice::Completed | TaskNotice::Failed { .. } => settings.notify_task_finished,
    }
}

/// 只在从阈值以上（或第一次读到）跌到阈值以下时为 true；等于阈值不算低于
pub fn balance_crossed(prev_remaining: Option<f64>, now_remaining: f64, threshold: f64) -> bool {
    now_remaining < threshold && prev_remaining.is_none_or(|prev| prev >= threshold)
}

/// 上次读到的余额，按连接记在内存里；连同当时的阈值与币种，阈值改了就按第一次读到处理
type BalanceMemory = HashMap<String, (f64, f64, String)>;

/// 是否该弹低余额提醒，并记下这次读到的余额。
/// 未开启提醒时忘掉上次余额；币种与阈值币种不同时不比较、不记录；
/// 免打扰期间照常记录（这次跌破算已经发生过），只是不弹，结束后也不补弹
pub fn balance_alert(memory: &mut BalanceMemory, settings: &Settings, connection_id: &str, remaining: f64, currency: &str) -> bool {
    let Some(threshold) = settings.balance_alert_threshold else {
        memory.remove(connection_id);
        return false;
    };
    let currency = currency.trim().to_ascii_uppercase();
    if currency != settings.balance_alert_currency || !remaining.is_finite() { return false; }
    let prev = memory.get(connection_id)
        .filter(|(_, at, unit)| *at == threshold && *unit == currency)
        .map(|(value, ..)| *value);
    let crossed = balance_crossed(prev, remaining, threshold);
    memory.insert(connection_id.to_string(), (remaining, threshold, currency));
    crossed && !settings.dnd
}

pub fn balance_message(connection_name: &str, remaining: f64, currency: &str) -> (String, String) {
    ("余额偏低".into(), format!("{connection_name}剩余 {remaining:.2} {}，低于提醒阈值。", currency.trim().to_ascii_uppercase()))
}

/// Windows 实际发送与点击监听在插件后台线程进行；这里只能记录通知准备阶段返回的错误。
fn show(app: &AppHandle, title: String, body: String, task_id: Option<i64>) {
    let mut builder = app.notification().builder().title(title).body(body);
    if let Some(id) = task_id { builder = builder.extra(TASK_KEY, id); }
    if let Err(error) = builder.show() { eprintln!("[通知] 发送失败：{error}"); }
}

pub fn notify_task(app: &AppHandle, task_id: i64, kind: TaskNotice, platform: &str, task_name: &str) {
    let Some(cfg) = app.try_state::<crate::Cfg>() else { return };
    if !should_notify(&cfg.0.get(), &kind) { return; }
    let (state, reason) = match kind {
        TaskNotice::Resumed => ("resumed", None),
        TaskNotice::Completed => ("completed", None),
        TaskNotice::Failed { reason } => ("failed", Some(short_reason(&reason))),
    };
    dispatch(app, NoticeBody::Task { task_id, state, platform: platform.into(), task_name: mask_secrets(task_name), reason });
}

fn allowed(settings: &Settings, notice: &AppNotice) -> bool {
    if settings.dnd { return false; }
    match &notice.body {
        NoticeBody::Balance { threshold, currency, .. } => settings.balance_alert_threshold == Some(*threshold) && settings.balance_alert_currency == *currency,
        NoticeBody::Task { state, .. } => if *state == "resumed" { settings.notify_task_resumed } else { settings.notify_task_finished },
    }
}

/// 只允许主窗口读取，不广播金额；订阅建立后补读启动期间先到达的提醒。
#[tauri::command]
pub fn take_app_notices(app: AppHandle, window: tauri::WebviewWindow) -> Result<Vec<AppNotice>, String> {
    if window.label() != crate::MAIN { return Err("仅主窗口可读取操作通知".into()); }
    let settings = app.state::<crate::Cfg>().0.get();
    let mut pending = PENDING_NOTICES.lock().map_err(|_| "通知队列暂不可用")?;
    Ok(pending.drain(..).filter(|notice| allowed(&settings, notice)).collect())
}

fn native_notice(app: &AppHandle, notice: &AppNotice) {
    match &notice.body {
        NoticeBody::Balance { connection_id, connection_name, remaining, currency, .. } => {
            let (title, body) = balance_message(connection_name, *remaining, currency);
            if let Err(error) = app.notification().builder().title(title).body(body).extra(CONNECTION_KEY, connection_id.clone()).show() {
                eprintln!("[通知] 发送失败：{error}");
            }
        }
        NoticeBody::Task { task_id, state, reason, platform, task_name } => {
            let kind = match *state { "resumed" => TaskNotice::Resumed, "completed" => TaskNotice::Completed, _ => TaskNotice::Failed { reason: reason.clone().unwrap_or_default() } };
            let (title, body) = task_message(&kind, platform, task_name);
            show(app, title, body, Some(*task_id));
        }
    }
}

fn deliver(foreground: bool, in_app: impl FnOnce() -> bool, native: impl FnOnce()) {
    if !foreground || !in_app() { native(); }
}

fn dispatch(app: &AppHandle, body: NoticeBody) {
    let notice = AppNotice { id: NEXT_NOTICE.fetch_add(1, Ordering::Relaxed), body };
    let foreground = crate::MAIN_READY.load(Ordering::Acquire) && app.get_webview_window(crate::MAIN)
        .is_some_and(|window| window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false) && !window.is_minimized().unwrap_or(true));
    deliver(foreground, || {
        if let Ok(mut pending) = PENDING_NOTICES.lock() { pending.push(notice.clone()); } else { return false; }
        // 不持队列锁调用窗口接口，避免与主窗口读取互相等待。
        if app.emit_to(crate::MAIN, "app-notice", ()).is_ok() { return true; }
        if let Ok(mut pending) = PENDING_NOTICES.lock() { pending.retain(|item| item.id != notice.id); }
        false
    }, || native_notice(app, &notice));
}

static BALANCES: LazyLock<Mutex<BalanceMemory>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// 余额刷新后调用：跌破阈值时提醒一次（重启后第一次读到低于阈值会再提醒一次）
pub fn check_balance(app: &AppHandle, connection_id: &str, connection_name: &str, platform: &str, remaining: f64, currency: &str) {
    let Some(cfg) = app.try_state::<crate::Cfg>() else { return };
    let settings = cfg.0.get();
    let alert = {
        let mut memory = BALANCES.lock().unwrap_or_else(|p| p.into_inner());
        balance_alert(&mut memory, &settings, connection_id, remaining, currency)
    };
    if !alert { return; }
    let Some(threshold) = settings.balance_alert_threshold else { return };
    dispatch(app, NoticeBody::Balance { connection_id: connection_id.into(), connection_name: mask_secrets(connection_name), platform: platform.into(), remaining, currency: currency.trim().to_ascii_uppercase(), threshold });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delivery_uses_one_channel_and_falls_back_on_in_app_failure() {
        use std::cell::Cell;
        for (foreground, accepted, expected_in_app, expected_native) in [(true, true, 1, 0), (true, false, 1, 1), (false, true, 0, 1)] {
            let in_app = Cell::new(0);
            let native = Cell::new(0);
            deliver(foreground, || { in_app.set(in_app.get()+1); accepted }, || native.set(native.get()+1));
            assert_eq!((in_app.get(), native.get()), (expected_in_app, expected_native));
        }
    }
    #[test]
    fn balance_click_accepts_only_a_valid_connection_target() {
        let action = |kind: &str, extra: serde_json::Value| serde_json::from_value::<tauri_plugin_notification::ActionPerformed>(serde_json::json!({"actionId":kind,"notification":{"id":1,"extra":extra}})).unwrap();
        assert_eq!(clicked_connection_id(&action("tap", serde_json::json!({"connection_id":"synthetic-api"}))).as_deref(), Some("synthetic-api"));
        for extra in [serde_json::json!({}), serde_json::json!({"connection_id":""}), serde_json::json!({"connection_id":12}), serde_json::json!({"connection_id":"bad\nvalue"})] {
            assert!(clicked_connection_id(&action("tap", extra)).is_none());
        }
        assert!(clicked_connection_id(&action("dismiss", serde_json::json!({"connection_id":"synthetic-api"}))).is_none());
    }
    #[test]
    fn pending_notice_respects_current_settings_and_has_no_credentials() {
        let balance = AppNotice { id:1, body: NoticeBody::Balance { connection_id:"synthetic-api".into(), connection_name:"开发连接".into(), platform:"claude".into(), remaining:8.5, currency:"USD".into(), threshold:10.0 } };
        let mut s = settings(); s.balance_alert_threshold=Some(10.0); s.balance_alert_currency="USD".into();
        assert!(allowed(&s,&balance));
        s.dnd=true; assert!(!allowed(&s,&balance)); s.dnd=false;
        s.balance_alert_threshold=Some(20.0); assert!(!allowed(&s,&balance));
        let value=serde_json::to_value(balance).unwrap();
        assert_eq!(value["kind"],"balance"); assert_eq!(value["remaining"],8.5);
        assert!(value.get("secret").is_none() && value.get("token").is_none());
        let resumed=AppNotice { id:2, body:NoticeBody::Task { task_id:7,state:"resumed",platform:"Claude Code".into(),task_name:"测试任务".into(),reason:None } };
        assert!(allowed(&s,&resumed)); s.notify_task_resumed=false; assert!(!allowed(&s,&resumed));
    }

    #[test]
    fn task_click_requires_tap_and_positive_integer_id() {
        let action = |kind: &str, extra: serde_json::Value| {
            serde_json::from_value::<tauri_plugin_notification::ActionPerformed>(serde_json::json!({
                "actionId": kind, "notification": { "id": 1, "extra": extra }
            })).unwrap()
        };
        assert_eq!(clicked_task_id(&action("tap", serde_json::json!({"task_id": 42}))), Some(42));
        for extra in [serde_json::json!({}), serde_json::json!({"task_id": 0}), serde_json::json!({"task_id": -1}), serde_json::json!({"task_id": "42"}), serde_json::json!({"task_id": 1.5})] {
            assert_eq!(clicked_task_id(&action("tap", extra)), None);
        }
        assert_eq!(clicked_task_id(&action("dismiss", serde_json::json!({"task_id": 42}))), None);
        let missing = serde_json::from_value(serde_json::json!({"actionId": "tap"})).unwrap();
        assert_eq!(clicked_task_id(&missing), None);
    }

    fn settings() -> Settings {
        Settings::default()
    }

    fn failed(reason: &str) -> TaskNotice {
        TaskNotice::Failed { reason: reason.into() }
    }

    #[test]
    fn should_notify_respects_dnd_and_switches() {
        let kinds = [TaskNotice::Resumed, TaskNotice::Completed, failed("x")];
        let mut s = settings();
        for kind in &kinds { assert!(should_notify(&s, kind), "默认两个开关都开：{kind:?}"); }
        s.dnd = true;
        for kind in &kinds { assert!(!should_notify(&s, kind), "免打扰时不弹：{kind:?}"); }
        let mut s = settings();
        s.notify_task_resumed = false;
        assert!(!should_notify(&s, &TaskNotice::Resumed), "关掉 resumed 只拦恢复通知");
        assert!(should_notify(&s, &TaskNotice::Completed));
        assert!(should_notify(&s, &failed("x")));
        let mut s = settings();
        s.notify_task_finished = false;
        assert!(should_notify(&s, &TaskNotice::Resumed));
        assert!(!should_notify(&s, &TaskNotice::Completed));
        assert!(!should_notify(&s, &failed("x")));
    }

    #[test]
    fn task_message_texts() {
        assert_eq!(
            task_message(&TaskNotice::Resumed, "Codex", "完善登录流程"),
            ("任务已继续".into(), "Codex 额度已恢复，「完善登录流程」正在原会话中继续。".into()),
        );
        assert_eq!(
            task_message(&TaskNotice::Completed, "Codex", "完善登录流程"),
            ("任务已完成".into(), "「完善登录流程」已运行完成。".into()),
        );
        assert_eq!(
            task_message(&failed("项目目录不可访问，无法在原目录执行。"), "Claude Code", "整理日志"),
            ("任务失败".into(), "「整理日志」：项目目录不可访问，无法在原目录执行。".into()),
        );
        let long = "错".repeat(80);
        let (_, body) = task_message(&failed(&long), "Codex", "t");
        let reason = body.strip_prefix("「t」：").unwrap();
        assert!(reason.ends_with('…'), "超过 60 字以「…」结尾：{reason}");
        assert_eq!(reason.chars().count(), 61, "保留 60 字再加「…」");
        let (_, body) = task_message(&failed(&"错".repeat(60)), "Codex", "t");
        assert_eq!(body, format!("「t」：{}", "错".repeat(60)), "正好 60 字不截断");
        let (_, body) = task_message(&failed("  第一行\n  第二行  "), "Codex", "t");
        assert_eq!(body, "「t」：第一行 第二行", "多行原因合并成一行");
    }

    #[test]
    fn failure_reason_masks_secrets() {
        let body = |reason: &str| task_message(&failed(reason), "Codex", "t").1;
        let cases = [
            ("连接 http://alice:s3cret@proxy.local:8080 失败", "s3cret"),
            ("401 Authorization: Bearer abc.DEF-123_xyz 无效", "abc.DEF-123_xyz"),
            ("invalid api key: sk-proj-AbCdEf0123456789xyz.", "AbCdEf0123456789xyz"),
            ("key=sk-ant-api03-AAAAAAAAAAAAAAAA 已失效", "AAAAAAAAAAAAAAAA"),
            ("token=ghp_AAAAAAAAAAAAAAAAAAAA,key=sk-BBBBBBBBBBBBBBBBBBBB", "ghp_AAAAAAAAAAAAAAAAAAAA"),
            (r#"{"Authorization":"Bearer abc123def456","key":"sk-CCCCCCCCCCCCCCCCCCCC"}"#, "CCCCCCCCCCCCCCCCCCCC"),
            ("Authorization: Basic dXNlcjpwYXNzd29yZA==", "dXNlcjpwYXNzd29yZA"),
            ("token eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig-_x 过期", "eyJzdWIiOiIxIn0"),
        ];
        for (reason, secret) in cases {
            let text = body(reason);
            assert!(!text.contains(secret), "{reason} → {text}");
        }
        assert_eq!(body("task-runner 运行失败；联系 a@b.com"), "「t」：task-runner 运行失败；联系 a@b.com", "普通文字与邮箱不遮蔽");
        let long = format!("{} sk-AAAAAAAAAAAAAAAAAAAAAAAA", "错".repeat(55));
        assert!(!body(&long).contains("AAAA"), "先遮蔽再截断，不会露出半截 Key");
    }

    #[test]
    fn balance_crossed_once() {
        assert!(balance_crossed(None, 5.0, 10.0), "第一次读到就低于阈值");
        assert!(!balance_crossed(Some(5.0), 4.0, 10.0), "已经提醒过，继续下降不再提醒");
        assert!(balance_crossed(Some(12.0), 9.0, 10.0), "从阈值以上跌到以下");
        assert!(!balance_crossed(None, 12.0, 10.0));
        assert!(!balance_crossed(Some(9.0), 10.0, 10.0), "等于阈值不算低于");
        assert!(balance_crossed(Some(10.0), 9.99, 10.0), "回到阈值后重新计数");
    }

    #[test]
    fn balance_alert_compares_same_currency_once_and_respects_dnd() {
        let mut s = settings();
        s.balance_alert_threshold = Some(10.0);
        s.balance_alert_currency = "USD".into();
        let mut memory = BalanceMemory::new();
        assert!(!balance_alert(&mut memory, &s, "a", 5.0, "CNY"), "币种不同不比较");
        assert!(balance_alert(&mut memory, &s, "a", 5.0, "usd"), "币种不区分大小写；第一次读到低于阈值");
        assert!(!balance_alert(&mut memory, &s, "a", 4.0, "USD"), "只提醒一次");
        assert!(!balance_alert(&mut memory, &s, "a", 12.0, "USD"));
        assert!(balance_alert(&mut memory, &s, "a", 8.0, "USD"), "回到阈值以上后重新计数");
        assert!(balance_alert(&mut memory, &s, "b", 1.0, "USD"), "按连接分别记");

        s.dnd = true;
        assert!(!balance_alert(&mut memory, &s, "c", 1.0, "USD"), "免打扰期间不弹");
        s.dnd = false;
        assert!(!balance_alert(&mut memory, &s, "c", 0.5, "USD"), "免打扰期间那次跌破已经算过，结束后不补弹");

        s.balance_alert_threshold = Some(20.0);
        assert!(balance_alert(&mut memory, &s, "a", 15.0, "USD"), "阈值改了按第一次读到处理");
        s.balance_alert_threshold = None;
        assert!(!balance_alert(&mut memory, &s, "a", 1.0, "USD"), "关闭提醒不弹");
        assert!(!memory.contains_key("a"), "关闭提醒时忘掉上次余额");
    }

    #[test]
    fn balance_message_text() {
        assert_eq!(
            balance_message("工作网关", 4.5, "USD"),
            ("余额偏低".into(), "工作网关剩余 4.50 USD，低于提醒阈值。".into()),
        );
    }
}
