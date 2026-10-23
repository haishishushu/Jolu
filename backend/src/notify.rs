//! 系统通知（画板 94 / D-10）：任务提醒与低余额提醒。
//!
//! - 只在 Rust 端发通知，前端没有通知权限；Windows 没有运行时通知权限，画板上的「引导入口」不做。
//! - 免打扰期间一律不弹；任务通知按「额度恢复与任务继续」「任务完成与失败」两个开关分别判断。
//! - 「需要处理」和「等待超时」归入失败类通知；任务已取消的收尾不通知。
//! - 当前插件版本（tauri-plugin-notification 2.4.x，与 tauri 2.11 兼容）不支持 Windows 点击回调，
//!   按简报退一步：不注册点击回调，正文写清任务名。桌面端点击回调要到插件 2.5 起才有（要求 tauri 2.12），
//!   以后单独升级 Tauri 时再接上 `MainIntent::Automation`（前端的 automation 意图已就绪）。
//! - 开发模式下 Windows 通知显示 PowerShell 的名字和图标（插件只给安装版设置 AppUserModelID），属已知限制。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use tauri::{AppHandle, Manager};
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
/// 通知附带的任务 ID：当前插件版本不支持 Windows 点击回调，暂时只随通知带上，升级后由点击回调读取
const TASK_KEY: &str = "task_id";

// 遮蔽是临时实现：任务 3.1 的共享 `redact` 落到 main 后改用它，届时删除这一段。

/// 常见 API Key 前缀，匹配不区分大小写；前面必须是开头或非 ASCII 字母数字（避免误伤 `task-runner` 这类普通词）
const KEY_PREFIXES: [&str; 20] = [
    "sk-", "sk_", "ghp_", "gho_", "ghs_", "ghu_", "ghr_", "github_pat_", "glpat-", "xoxb-", "xoxp-", "AIza",
    "xai-", "AKIA", "hf_", "gsk_", "npm_", "rk_live_", "pplx-", "ya29.",
];
/// URL 查询参数里值需要遮掉的参数名（不区分大小写）
const QUERY_KEYS: [&str; 4] = ["api_key", "access_token", "token", "key"];
/// 认证方案：名称、是否不区分大小写、令牌至少多长。`Basic` / `Token` 也是常见英文词，
/// 所以只认首字母大写的写法，并要求令牌至少 8 个字符
const SCHEMES: [(&str, bool, usize); 3] = [("bearer", true, 1), ("Basic", false, 8), ("Token", false, 8)];
/// 前缀之后至少这么多个字符才当作 Key
const KEY_MIN_TAIL: usize = 8;
const MASK: &str = "****";

/// JWT 兜底：`eyJ` 开头、点分、去掉末尾句点后至少这么长
const JWT_MIN_LEN: usize = 16;

fn key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// Bearer 令牌可能出现的字符（base64url、JWT 的点、base64 的 `+/=`、`~`）
fn token_char(c: char) -> bool {
    key_char(c) || ".~+/=".contains(c)
}

/// URL / 代理地址里的账号密码段换成 `****`；邮箱（`@` 前没有冒号）不动。
/// 账号密码与主机的分界取「主机前的最后一个 `@`」：密码里可能还有 `@` 或 `/`
fn mask_userinfo(token: &str) -> String {
    let (start, at) = match token.find("://") {
        Some(i) => {
            let start = i + 3;
            let Some(first) = token[start..].find('@') else { return token.to_string() };
            // 主机在第一个 `@` 之后、下一个 `/` 之前；密码里的 `@` 都在这段的最后一个 `@` 之前
            let end = token[start + first..].find('/').map_or(token.len(), |j| start + first + j);
            let Some(last) = token[start..end].rfind('@') else { return token.to_string() };
            (start, start + last)
        }
        None => {
            let Some(at) = token.rfind('@') else { return token.to_string() };
            let start = token[..at].rfind(|c: char| "([{<\"'=,;/".contains(c)).map_or(0, |i| i + 1);
            (start, at)
        }
    };
    let user = &token[start..at];
    if !user.split_once(':').is_some_and(|(u, p)| !u.is_empty() && !p.is_empty()) {
        return token.to_string();
    }
    format!("{}{MASK}{}", &token[..start], &token[at..])
}

fn starts_with_ci(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len()).is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn scheme_matches(word: &str, (name, ci, _): (&str, bool, usize)) -> bool {
    if ci { word.eq_ignore_ascii_case(name) } else { word == name }
}

/// `text` 开头的令牌字符段长度（字节；令牌字符都是 ASCII）
fn token_len(text: &str) -> usize {
    text.len() - text.trim_start_matches(token_char).len()
}

/// 前缀 Key：返回整段长度。前缀之后的字符段要么不足 8 个（只看了这么几个字符），要么整段被遮掉，所以是线性的
fn prefix_key(rest: &str) -> Option<usize> {
    for prefix in KEY_PREFIXES {
        if starts_with_ci(rest, prefix) {
            // key_char 只有 ASCII，字符数即字节数
            let len = rest[prefix.len()..].chars().take_while(|c| key_char(*c)).count();
            if len >= KEY_MIN_TAIL { return Some(prefix.len() + len); }
        }
    }
    None
}

/// 同一个词里的 `Bearer:xxx` / `Basic:xxx` / `Token:xxx`：返回（保留的方案名长度，令牌长度）
fn inline_scheme(rest: &str) -> Option<(usize, usize)> {
    for scheme in SCHEMES {
        let (name, _, min) = scheme;
        let Some(head) = rest.get(..name.len()) else { continue };
        if !scheme_matches(head, scheme) || rest.as_bytes().get(name.len()) != Some(&b':') { continue; }
        let keep = name.len() + 1;
        let len = token_len(&rest[keep..]);
        if len >= min { return Some((keep, len)); }
    }
    None
}

/// 常见前缀的 Key、同一个词里的认证方案令牌与 JWT 换成 `****`。从左到右扫描，每个位置在所有前缀里找匹配，
/// 所以总是先遮最靠前的那个。JWT 判定要扫到点分段的末尾，失败后记住段尾，
/// 同一段里不再重复做 JWT 判定（前缀 Key 照常检查），避免 `eyJ-eyJ-…` 这类输入退化成平方复杂度
fn mask_keys(token: &str) -> String {
    let mut out = String::new();
    let (mut i, mut copied, mut jwt_until) = (0, 0, 0);
    while i < token.len() {
        let rest = &token[i..];
        let boundary = token[..i].chars().next_back().is_none_or(|c| !c.is_ascii_alphanumeric());
        if boundary {
            let mut hit = prefix_key(rest).map(|len| (0, len)).or_else(|| inline_scheme(rest));
            if hit.is_none() && i >= jwt_until && rest.starts_with("eyJ") {
                let raw = rest.find(|c: char| !(key_char(c) || c == '.')).unwrap_or(rest.len());
                let run = rest[..raw].trim_end_matches('.');
                if run.contains('.') && run.len() >= JWT_MIN_LEN { hit = Some((0, run.len())); } else { jwt_until = i + raw; }
            }
            if let Some((keep, len)) = hit {
                out.push_str(&token[copied..i + keep]);
                out.push_str(MASK);
                i += keep + len;
                copied = i;
                continue;
            }
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    out.push_str(&token[copied..]);
    out
}

/// URL 查询参数 `api_key` / `access_token` / `token` / `key` 的值换成 `****`
fn mask_query(token: &str) -> String {
    let mut out = String::new();
    let (mut i, mut copied) = (0, 0);
    let bytes = token.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'?' || bytes[i] == b'&' {
            let after = &token[i + 1..];
            let name = QUERY_KEYS.iter().find(|n| starts_with_ci(after, n) && after.as_bytes().get(n.len()) == Some(&b'='));
            if let Some(name) = name {
                let start = i + 1 + name.len() + 1;
                let end = token[start..].find(|c: char| "&#\"'<>()[]{},;".contains(c)).map_or(token.len(), |j| start + j);
                if end > start {
                    out.push_str(&token[copied..start]);
                    out.push_str(MASK);
                    copied = end;
                }
                i = end.max(i + 1);
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&token[copied..]);
    out
}

/// 一个词的全部遮蔽：账号密码、查询参数、前缀 Key / 认证方案 / JWT
fn mask_word(word: &str) -> String {
    mask_keys(&mask_query(&mask_userinfo(word)))
}

/// 词尾是认证方案名（可再跟一个冒号）时，返回下一个词里令牌的最短长度。
/// 前面可以紧跟引号、冒号等，如 `"Bearer`、`:'Bearer`、`Authorization:Bearer`、`Bearer:`
fn scheme_at_end(token: &str) -> Option<usize> {
    let word = token.strip_suffix(':').unwrap_or(token).rsplit(|c: char| !c.is_ascii_alphanumeric()).next()?;
    SCHEMES.into_iter().find(|s| scheme_matches(word, *s)).map(|(_, _, min)| min)
}

/// 认证方案后面那个词里的令牌换成 `****`，前导引号保留，令牌之后的剩余部分继续遮蔽；不像令牌时返回 None
fn mask_scheme_value(token: &str, min: usize) -> Option<String> {
    let lead = token.len() - token.trim_start_matches(|c: char| !token_char(c)).len();
    let body = &token[lead..];
    let len = token_len(body);
    if len == 0 || len < min { return None; }
    Some(format!("{}{MASK}{}", &token[..lead], mask_word(&body[len..])))
}

/// 遮蔽失败原因里可能带出的凭证：URL / 代理地址里的 `user:pass@`、URL 查询参数里的 Key、常见前缀的 Key、JWT，
/// 以及 `Bearer` / `Basic` / `Token` 后面的令牌。输入是已经合并成一行的文本，按空格分词处理
pub fn mask_secrets(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut pending: Option<usize> = None;
    for token in text.split(' ') {
        if let Some(min) = pending.filter(|_| !token.is_empty()) {
            pending = None;
            if let Some(masked) = mask_scheme_value(token, min) {
                out.push(masked);
                continue;
            }
        }
        pending = scheme_at_end(token);
        out.push(mask_word(token));
    }
    out.join(" ")
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

/// 插件 2.4 在后台任务里 `let _ = notification.show()`，内部丢弃系统层的发送结果：系统禁用通知
/// （或 Jolu 的通知被关掉）时不会返回错误，这里也就不会记录；这里只能记录插件准备通知时的错误
fn show(app: &AppHandle, title: String, body: String, task_id: Option<i64>) {
    let mut builder = app.notification().builder().title(title).body(body);
    if let Some(id) = task_id { builder = builder.extra(TASK_KEY, id); }
    if let Err(error) = builder.show() { eprintln!("[通知] 发送失败：{error}"); }
}

pub fn notify_task(app: &AppHandle, task_id: i64, kind: TaskNotice, platform: &str, task_name: &str) {
    let Some(cfg) = app.try_state::<crate::Cfg>() else { return };
    if !should_notify(&cfg.0.get(), &kind) { return; }
    let (title, body) = task_message(&kind, platform, task_name);
    show(app, title, body, Some(task_id));
}

static BALANCES: LazyLock<Mutex<BalanceMemory>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// 余额刷新后调用：跌破阈值时提醒一次（重启后第一次读到低于阈值会再提醒一次）
pub fn check_balance(app: &AppHandle, connection_id: &str, connection_name: &str, remaining: f64, currency: &str) {
    let Some(cfg) = app.try_state::<crate::Cfg>() else { return };
    let settings = cfg.0.get();
    let alert = {
        let mut memory = BALANCES.lock().unwrap_or_else(|p| p.into_inner());
        balance_alert(&mut memory, &settings, connection_id, remaining, currency)
    };
    if !alert { return; }
    let (title, body) = balance_message(connection_name, remaining, currency);
    show(app, title, body, None);
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(body("连接 http://alice:s3cret@proxy.local:8080 失败"), "「t」：连接 http://****@proxy.local:8080 失败");
        assert_eq!(body("代理 alice:s3cret@10.0.0.1:7890 拒绝连接"), "「t」：代理 ****@10.0.0.1:7890 拒绝连接");
        assert_eq!(body("401 Authorization: Bearer abc.DEF-123_xyz 无效"), "「t」：401 Authorization: Bearer **** 无效");
        assert_eq!(body("invalid api key: sk-proj-AbCdEf0123456789xyz."), "「t」：invalid api key: ****.");
        assert_eq!(body("key=sk-ant-api03-AAAAAAAAAAAAAAAA 已失效"), "「t」：key=**** 已失效");
        assert_eq!(body("task-runner 运行失败；联系 a@b.com"), "「t」：task-runner 运行失败；联系 a@b.com", "普通文字与邮箱不遮蔽");
        let long = format!("{} sk-AAAAAAAAAAAAAAAAAAAAAAAA", "错".repeat(55));
        assert_eq!(body(&long), format!("「t」：{} ****", "错".repeat(55)), "先遮蔽再截断，不会露出半截 Key");
    }

    #[test]
    fn mask_secrets_edge_cases() {
        // 同一段里有多个不同前缀：每轮取最靠前的匹配，不能漏掉前面的 Key
        assert_eq!(mask_secrets("token=ghp_AAAAAAAAAAAA,key=sk-BBBBBBBBBBBB"), "token=****,key=****");
        assert_eq!(mask_secrets("sk-BBBBBBBBBBBB/ghp_AAAAAAAAAAAA"), "****/****");
        // Bearer 前面紧跟引号或冒号
        assert_eq!(mask_secrets(r#"header "Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abc123" 无效"#), r#"header "Bearer ****" 无效"#);
        assert_eq!(mask_secrets("auth:'Bearer abc.def-123' 失败"), "auth:'Bearer ****' 失败");
        assert_eq!(mask_secrets("Authorization:Bearer abc123def"), "Authorization:Bearer ****");
        assert_eq!(mask_secrets(r#"Bearer "abc123def""#), r#"Bearer "****""#, "令牌前面带引号也要遮");
        // JWT 兜底：没有 Bearer 也遮
        assert_eq!(mask_secrets("token eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig-_x 过期"), "token **** 过期");
        assert_eq!(mask_secrets("invalid jwt=eyJhbGciOiJub25lIn0.eyJzdWIiOiIxIn0."), "invalid jwt=****.");
        // URL 账号密码以主机前的最后一个 @ 为界
        assert_eq!(mask_secrets("http://alice:p@ss@host/path"), "http://****@host/path");
        assert_eq!(mask_secrets("http://alice:p/w@host/x"), "http://****@host/x", "密码里带斜杠");
        assert_eq!(mask_secrets("proxy alice:p@ss@10.0.0.1:7890"), "proxy ****@10.0.0.1:7890");
        // 不误伤
        assert_eq!(mask_secrets("https://registry.example/@scope/pkg 404"), "https://registry.example/@scope/pkg 404");
        assert_eq!(mask_secrets("task-runner 失败 a@b.com"), "task-runner 失败 a@b.com");
        assert_eq!(mask_secrets("the bearer 无效"), "the bearer 无效");
    }

    #[test]
    fn mask_secrets_is_linear_on_pathological_input() {
        for unit in ["eyJ-", "eyJ_", "eyJ.", "sk-a!", "Token:"] {
            let input = unit.repeat(200_000 / unit.len());
            let started = std::time::Instant::now();
            let _ = mask_secrets(&input);
            let elapsed = started.elapsed();
            assert!(elapsed < std::time::Duration::from_secs(2), "{unit} × {}：{elapsed:?}", input.len() / unit.len());
        }
    }

    #[test]
    fn mask_secrets_round3_cases() {
        // 认证方案令牌之后，同一个词的剩余部分继续遮蔽
        assert_eq!(
            mask_secrets(r#"{"Authorization":"Bearer abc123def","key":"sk-AAAAAAAAAAAA"}"#),
            r#"{"Authorization":"Bearer ****","key":"****"}"#,
        );
        // Basic / Token / Bearer:
        assert_eq!(mask_secrets("Authorization: Basic dXNlcjpwYXNzd29yZA=="), "Authorization: Basic ****");
        assert_eq!(mask_secrets("Authorization: Token abcdef123456"), "Authorization: Token ****");
        assert_eq!(mask_secrets("Bearer: abc123def"), "Bearer: ****");
        assert_eq!(mask_secrets("Authorization:Bearer:abc123def 无效"), "Authorization:Bearer:**** 无效");
        assert_eq!(mask_secrets("Token 过期"), "Token 过期", "方案后面不像令牌时不遮");
        // 新增前缀，且不区分大小写
        for key in ["xai-AAAAAAAAAAAA", "AKIAABCDEFGHIJKLMNOP", "hf_AAAAAAAAAAAA", "gsk_AAAAAAAAAAAA", "ghs_AAAAAAAAAAAA",
            "ghu_AAAAAAAAAAAA", "ghr_AAAAAAAAAAAA", "npm_AAAAAAAAAAAA", "rk_live_AAAAAAAAAAAA", "pplx-AAAAAAAAAAAA",
            "ya29.a0AfH6SMBxyz12", "SK-AAAAAAAAAAAA", "Sk-proj-AAAAAAAAAAAA"] {
            assert_eq!(mask_secrets(&format!("key: {key} 无效")), "key: **** 无效", "{key}");
        }
        // URL 查询参数
        assert_eq!(mask_secrets("GET https://api.example.com/v1?api_key=abc123&model=x 401"), "GET https://api.example.com/v1?api_key=****&model=x 401");
        assert_eq!(mask_secrets("https://h/p?a=1&access_token=xyz#frag"), "https://h/p?a=1&access_token=****#frag");
        assert_eq!(mask_secrets("https://h/p?TOKEN=xyz&KEY=abc"), "https://h/p?TOKEN=****&KEY=****");
        assert_eq!(mask_secrets("https://h/p?keyword=abc&monkey=1"), "https://h/p?keyword=abc&monkey=1", "参数名要完全一致");
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
