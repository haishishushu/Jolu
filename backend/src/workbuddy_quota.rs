//! WorkBuddy 个人积分查询。账单字段与端点对齐官方客户端 account.js / tar.js，
//! 浏览器登录使用官方 state 轮询协议；账单接口未公开文档。
//!
//! - 凭证：`%LOCALAPPDATA%\CodeBuddyExtension\Data\Public\auth\workbuddy-desktop.info`
//!   （国际版 `workbuddy-desktop-ai.info`），JSON：`auth.{accessToken,domain,expiresAt}` +
//!   `account.{uid,nickname,email,enterpriseId}`
//! - 积分：`POST {base}/billing/meter/get-user-resource-summary`，`data.Packages[]` 的
//!   `CycleTotalCapacity / CycleRemainCapacity / CycleEndTime`
//! - 浏览器授权：系统默认浏览器完成官方登录，保存独立登录令牌，不读取浏览器数据库。
//!
//! 纪律：只读查询语义的 POST（请求体为空对象）；地址固定为官方域名，不按文件里的域名拼接；
//! 新版 sym-v1 凭证通过对应官方客户端运行时在内存中解码；不刷新、不写回令牌。

use crate::quota::{self, AccountMetadata, HttpOutcome, QuotaState, QuotaWindow};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::{io::{Read, Write}, process::{Child, Command, Stdio}, sync::mpsc, time::Duration};

struct RuntimeProcess(Child);
impl Drop for RuntimeProcess {
    fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
}

/// 固定脚本通过参数传入，密文仅通过 stdin；限时、限长且不回显进程错误。
fn runtime_output(mut command: Command, input: Vec<u8>) -> Result<Vec<u8>, QuotaState> {
    let failed = || QuotaState::Failed { reason: "WorkBuddy 本机凭证读取失败，请检查对应客户端版本".into() };
    if input.len() > 65536 { return Err(failed()); }
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = RuntimeProcess(command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|_| failed())?);
    let mut stdin = child.0.stdin.take().ok_or_else(failed)?;
    let stdout = child.0.stdout.take().ok_or_else(failed)?;
    let (tx, rx) = mpsc::sync_channel(2);
    let reader_tx = tx.clone();
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let result = stdout.take(65537).read_to_end(&mut output).map(|_| output).map_err(|_| ());
        let _ = reader_tx.send(result);
    });
    std::thread::spawn(move || { let _ = tx.send(stdin.write_all(&input).map(|_| Vec::new()).map_err(|_| ())); });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut output = Vec::new();
    for _ in 0..2 {
        let wait = deadline.saturating_duration_since(std::time::Instant::now());
        let value = rx.recv_timeout(wait).map_err(|_| failed())?.map_err(|_| failed())?;
        if value.len() > 65536 { return Err(failed()); }
        if !value.is_empty() { output = value; }
    }
    Ok(output)
}

fn runtime(edition: Edition) -> Option<PathBuf> {
    let (name, folder, variable) = match edition {
        Edition::Cn => ("WorkBuddy.exe", "WorkBuddy", "JOLU_WORKBUDDY_EXE"),
        Edition::Ai => ("WorkBuddyAI.exe", "WorkBuddyAI", "JOLU_WORKBUDDY_AI_EXE"),
    };
    if let Some(path) = std::env::var_os(variable).map(PathBuf::from) {
        return (path.is_absolute() && path.is_file() && path.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name))).then_some(path);
    }
    for variable in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(variable).map(PathBuf::from) {
            for path in [root.join("Programs").join(folder).join(name), root.join(folder).join(name)] {
                if path.is_file() { return Some(path); }
            }
        }
    }
    #[cfg(windows)] {
        // 只读取安装注册表；UTF-8 输出支持中文安装路径，不解释卸载命令。
        let script = r#"[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*' -ErrorAction SilentlyContinue | Where-Object {$_.DisplayName -match '^WorkBuddy(?: AI)?(?:\s|$)'} | ForEach-Object { if ($_.DisplayIcon) { ($_.DisplayIcon -replace ',\s*-?\d+$','').Trim('"') } }"#;
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        if let Ok(output) = runtime_output(command, Vec::new()) {
            if let Ok(output) = String::from_utf8(output) {
                return output.lines().map(str::trim).map(PathBuf::from)
                    .find(|p| p.is_absolute() && p.is_file() && p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name)));
            }
        }
    }
    None
}

fn resolve_session(edition: Edition, mut value: Value, now: chrono::DateTime<chrono::Utc>) -> Result<Session, QuotaState> {
    // 域名/到期时间先检查，未知来源不启动运行时。
    if edition.base(text(&value["auth"], "domain").unwrap_or_default()).is_none() {
        return Err(QuotaState::Failed { reason: "WorkBuddy 登录区域无法识别".into() });
    }
    if timestamp(value["auth"].get("expiresAt")).is_some_and(|at| at <= now) {
        return Err(QuotaState::Unauthorized { reason: "所选 WorkBuddy 版本登录已过期，请在对应原工具刷新登录".into() });
    }
    if matches!(parse_session(edition, &value, now), Session::Encrypted) {
        let executable = runtime(edition).ok_or_else(|| QuotaState::Unsupported { reason: "未找到对应 WorkBuddy 官方客户端，无法读取加密登录；请检查安装路径".into() })?;
        let mut command = Command::new(executable);
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy().to_ascii_uppercase();
            if name.starts_with("NODE_") || name.starts_with("ELECTRON_") { command.env_remove(key); }
        }
        command.env("ELECTRON_RUN_AS_NODE", "1").args(["-e", include_str!("workbuddy_auth.cjs")]);
        let output = runtime_output(command, value["auth"]["accessToken"].to_string().into_bytes())?;
        let result: Value = serde_json::from_slice(&output).map_err(|_| QuotaState::Failed { reason: "WorkBuddy 凭证读取结果无效".into() })?;
        let token = result.get("token").and_then(Value::as_str).filter(|t| !t.is_empty() && t.len() <= 32768 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b"._~+/-=".contains(&b)))
            .ok_or_else(|| QuotaState::Unsupported { reason: "无法读取此版本 WorkBuddy 的加密登录；客户端密钥或加密格式不匹配".into() })?;
        value["auth"]["accessToken"] = json!(token);
    }
    match parse_session(edition, &value, now) {
        session @ Session::Ready { .. } => Ok(session),
        _ => Err(QuotaState::Failed { reason: "WorkBuddy 登录文件格式无法识别".into() }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Edition {
    Cn,
    Ai,
}

impl Edition {
    fn from_browser(edition: &str) -> Result<Self, String> {
        match edition {
            "cn" => Ok(Self::Cn),
            "ai" => Ok(Self::Ai),
            _ => Err("未知 WorkBuddy 浏览器授权版本".into()),
        }
    }


    fn file(self) -> &'static str {
        match self { Self::Cn => "workbuddy-desktop.info", Self::Ai => "workbuddy-desktop-ai.info" }
    }
    /// 只接受官方域名；空域名按版本默认
    fn base(self, domain: &str) -> Option<&'static str> {
        match (self, domain.to_ascii_lowercase().as_str()) {
            (Self::Cn, "" | "codebuddy.cn" | "www.codebuddy.cn") => Some("https://www.codebuddy.cn"),
            (Self::Cn, "workbuddy.cn" | "www.workbuddy.cn") => Some("https://www.workbuddy.cn"),
            (Self::Ai, "" | "workbuddy.ai" | "www.workbuddy.ai") => Some("https://www.workbuddy.ai"),
            _ => None,
        }
    }
}

fn auth_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
        .map(|dir| dir.join("CodeBuddyExtension").join("Data").join("Public").join("auth"))
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn number(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.trim().parse().ok()).filter(|n: &f64| n.is_finite())
}

/// 秒或毫秒时间戳，或无时区的「YYYY-MM-DD HH:MM:SS」（按东八区，与 WorkBuddy 网页一致）
fn timestamp(value: Option<&Value>) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    let value = value?;
    let east8 = chrono::FixedOffset::east_opt(8 * 3600)?;
    if let Some(n) = number(value) {
        let seconds = if n.abs() > 10_000_000_000.0 { (n / 1000.0) as i64 } else { n as i64 };
        return chrono::DateTime::from_timestamp(seconds, 0).map(|t| t.with_timezone(&east8));
    }
    let raw = value.as_str()?.trim();
    chrono::DateTime::parse_from_rfc3339(raw).ok().or_else(|| {
        chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S").ok()?.and_local_timezone(east8).single()
    })
}

#[derive(Debug, PartialEq)]
enum Session {
    Ready { edition: Edition, token: String, base: &'static str, headers: Vec<(&'static str, String)>, email: Option<String> },
    Encrypted,
    Expired,
    Invalid,
}

fn parse_session(edition: Edition, value: &Value, now: chrono::DateTime<chrono::Utc>) -> Session {
    let auth = &value["auth"];
    let account = &value["account"];
    let token = match &auth["accessToken"] {
        Value::String(token) if !token.trim().is_empty() => token.trim().to_string(),
        Value::Object(map) if map.contains_key("$wbEncrypted") => return Session::Encrypted,
        _ => return Session::Invalid,
    };
    let Some(base) = edition.base(text(auth, "domain").unwrap_or_default()) else { return Session::Invalid };
    if timestamp(auth.get("expiresAt")).is_some_and(|at| at <= now) { return Session::Expired; }
    let mut headers = Vec::new();
    if let Some(domain) = text(auth, "domain") { headers.push(("X-Domain", domain.to_string())); }
    if let Some(uid) = text(account, "uid") { headers.push(("X-User-Id", uid.to_string())); }
    if let Some(enterprise) = text(account, "enterpriseId") {
        headers.push(("X-Enterprise-Id", enterprise.to_string()));
        headers.push(("X-Tenant-Id", enterprise.to_string()));
    }
    let email = text(account, "email").or_else(|| text(account, "nickname"))
        .filter(|s| s.len() <= 320 && !s.chars().any(char::is_control)).map(str::to_string);
    Session::Ready { edition, token, base, headers, email }
}

/// `data.Packages` 可能套在 `data.data` 或 `data.Response.Data` 里
fn packages(body: &Value) -> Option<&Vec<Value>> {
    ["/data", "/data/data", "/data/Response/Data", "/data/data/Response/Data"].iter()
        .find_map(|prefix| ["Packages", "packages"].iter().find_map(|key| body.pointer(&format!("{prefix}/{key}"))?.as_array()))
}

fn capacity(item: &Value, suffix: &str) -> Result<Option<f64>, ()> {
    let alias = match suffix { "Size" => "CycleTotalCapacity", "Remain" => "CycleRemainCapacity", _ => "CycleUsedCapacity" };
    ["CycleCapacity", "Capacity"].iter()
        .flat_map(|prefix| [format!("{prefix}{suffix}Precise"), format!("{prefix}{suffix}")])
        .chain(std::iter::once(alias.to_string()))
        .find_map(|key| item.get(&key).filter(|v| !v.is_null()))
        .map(|value| number(value).filter(|n| *n >= 0.0).ok_or(()))
        .transpose()
}

fn parse_summary(body: &Value, now: chrono::DateTime<chrono::Utc>) -> QuotaState {
    match body.get("code").and_then(Value::as_i64) {
        Some(0 | 200) => {}
        Some(401 | 10085) => return QuotaState::Unauthorized { reason: "WorkBuddy 登录已失效，请打开 WorkBuddy 重新登录".into() },
        _ => return QuotaState::Failed { reason: "WorkBuddy 积分接口返回失败".into() },
    }
    let Some(items) = packages(body) else {
        return QuotaState::Failed { reason: "WorkBuddy 积分响应缺少资源包列表".into() };
    };
    let (mut total, mut remaining, mut seen) = (0.0, 0.0, false);
    let mut reset: Option<chrono::DateTime<chrono::FixedOffset>> = None;
    let mut plan = ["/data", "/data/data", "/data/Response/Data", "/data/data/Response/Data"].iter()
        .find_map(|prefix| text(body.pointer(prefix)?, "SubscriptionPackageName")).map(str::to_string);
    for item in items {
        // 官网还可能返回其他单位的资源，不能把次数、容量等累加成积分。
        if let Some(unit) = item.get("CapacityUnit").filter(|unit| !unit.is_null()) {
            match unit.as_str() {
                Some(unit) if unit.eq_ignore_ascii_case("credits") => {}
                Some(_) => continue,
                None => return QuotaState::Failed { reason: "WorkBuddy 资源包单位格式无效".into() },
            }
        }
        let end = timestamp(item.get("CycleEndTime"));
        let invalid = || QuotaState::Failed { reason: "WorkBuddy 资源包额度缺失或无效，无法确认完整积分".into() };
        if item.get("CycleEndTime").is_some_and(|value| !value.is_null()) && end.is_none() {
            return invalid();
        }
        if end.is_some_and(|end| end <= now) { continue; }
        let (Ok(size), Ok(left), Ok(used)) = (capacity(item, "Size"), capacity(item, "Remain"), capacity(item, "Used")) else {
            return invalid();
        };
        let Some(size) = size.or_else(|| left.zip(used).map(|(l, u)| l + u)) else { return invalid() };
        let Some(left) = left.or_else(|| used.map(|used| size - used)) else { return invalid() };
        if !size.is_finite() || left < 0.0 || left > size || used.is_some_and(|used| {
            used > size || (size - left - used).abs() > 1e-9 * size.max(1.0)
        }) {
            return invalid();
        }
        seen = true;
        total += size;
        remaining += left;
        if !total.is_finite() || !remaining.is_finite() { return invalid(); }
        if left > 0.0 { if let Some(end) = end { reset = Some(reset.map_or(end, |r| r.min(end))); } }
        if plan.is_none() { plan = text(item, "SubscriptionPackageName").map(str::to_string); }
    }
    if !seen {
        return QuotaState::Ok { windows: Vec::new(), plan, account: None };
    }
    let used = total - remaining;
    let window = QuotaWindow {
        key: "credits".into(),
        window_name: "积分".into(),
        used_percent: (total > 0.0).then(|| (used / total * 1000.0).round() / 10.0),
        amount_text: Some(format!("{used:.2} / {total:.2} credits")),
        remaining_text: Some(format!("剩余 {remaining:.2} credits")),
        // CycleEndTime 是本周期最后一秒，下一秒才重置
        resets_at: reset.and_then(|end| end.checked_add_signed(chrono::Duration::seconds(1))).map(|end| end.to_rfc3339()),
    };
    QuotaState::Ok { windows: vec![window], plan, account: None }
}

// WorkBuddy 官方客户端 account.js 的付费/免费资源包编码；仅用于查询周期，不据此推断套餐。
const PAID_PACKAGE_CODES: &[&str] = &[
    "TCACA_code_002_AkiJS3ZHF5", "TCACA_code_005_maRGyrHhw1", "TCACA_code_003_FAnt7lcmRT",
    "TCACA_code_023_4xbGhMrE6q", "TCACA_code_026_BaESVICNoi", "TCACA_code_027_0FCGVA6vSa",
    "TCACA_code_009_0XmEQc2xOf", "TCACA_code_038_OhvqZtiPKr", "TCACA_code_036_lupO5WgNdG",
];
const FREE_PACKAGE_CODES: &[&str] = &[
    "TCACA_code_001_PqouKr6QWV", "TCACA_code_008_cfWoLwvjU4", "TCACA_code_035_ArVxJcGDsm",
    "TCACA_code_006_DbXS0lrypC", "TCACA_code_039_KRcQj7wUat", "TCACA_code_040_mi9rCYg46x",
    "TCACA_code_007_nzdH5h4Nl0", "TCACA_code_028_NtpWi0jzXs", "TCACA_code_037_WxOD3MpI2o",
    "TCACA_code_029_6wCGEWquYy", "TCACA_code_030_BjSt89qTvr",
];

fn listing_reset(listing: &Value, summary: &Value, now: chrono::DateTime<chrono::Utc>) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    if listing.get("code").and_then(Value::as_i64) != Some(0) { return None; }
    let data = listing.get("data")?;
    let accounts = data.get("Accounts")?.as_array()?;
    // 截断的列表不能证明最早周期；宁可保留未知。
    if number(&data["TotalCount"]).is_some_and(|count| count > accounts.len() as f64) { return None; }
    let summary = packages(summary)?;
    accounts.iter().filter(|item| {
        let Some(code) = text(item, "PackageCode") else { return false };
        summary.iter().any(|package| text(package, "PackageCode") == Some(code)
            && text(package, "CapacityUnit").is_none_or(|unit| unit.eq_ignore_ascii_case("credits")))
            && text(item, "CapacityUnit").is_none_or(|unit| unit.eq_ignore_ascii_case("credits"))
            && capacity(item, "Remain").ok().flatten().is_some_and(|left| left > 0.0)
    }).filter_map(|item| timestamp(item.get("CycleEndTime")))
        .filter(|end| *end > now).min()
}

fn cycle_reset(client: &reqwest::blocking::Client, base:&str, token:&str, headers:&[(&str,String)], summary: &Value) -> Option<String> {
    let packages = packages(summary)?;
    if !packages.iter().any(|item| text(item, "PackageCode").is_some()) { return None; }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let now = chrono::Utc::now();
    let day = now.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600)?).format("%Y-%m-%d").to_string();
    let mut reset = None;
    for (kind, codes) in [("paid", PAID_PACKAGE_CODES), ("free", FREE_PACKAGE_CODES)] {
        let wait = deadline.saturating_duration_since(std::time::Instant::now());
        if wait.is_zero() { break; }
        let mut body = json!({"PageNumber":1,"PageSize":100,"PackageCodes":codes,"Status":[0,3]});
        if kind == "paid" { body["NeedRenewInfo"] = json!(true); }
        else {
            body["SlicePeriodStartTime"] = json!(format!("{day} 00:00:00"));
            body["SlicePeriodEndTime"] = json!(format!("{day} 23:59:59"));
        }
        let mut request = client.post(format!("{base}/billing/meter/get-user-resource-{kind}-packages"))
            .bearer_auth(token).header("Accept", "application/json")
            .header("Origin", base).header("Referer", format!("{base}/profile/plans-usage"))
            .timeout(wait).json(&body);
        for (name,value) in headers {request=request.header(*name,value);}
        let Ok(response)=request.send() else {continue};
        if response.status() != reqwest::StatusCode::OK { continue; }
        let Ok(body) = response.json::<Value>() else { continue };
        if let Some(end) = listing_reset(&body, summary, now) {
            reset = Some(reset.map_or(end, |current: chrono::DateTime<chrono::FixedOffset>| current.min(end)));
        }
    }
    reset?.checked_add_signed(chrono::Duration::seconds(1)).map(|end| end.to_rfc3339())
}

/// 独立浏览器授权只用自身的令牌和已验证账号查询，不借用原工具的本地登录。
pub(crate) fn authorized_quota(edition: &str, session:&crate::workbuddy_browser_auth::Session) -> QuotaState {
    let edition = match Edition::from_browser(edition) {
        Ok(edition) => edition,
        Err(reason) => return QuotaState::Unsupported { reason },
    };
    if session.expires_at.is_some_and(|expires|expires<=chrono::Utc::now().timestamp_millis()) {
        return QuotaState::Unauthorized{reason:"WorkBuddy 浏览器授权已过期，请重新授权".into()};
    }
    if session.token.is_empty()||session.user_id.is_empty() {return QuotaState::Unauthorized{reason:"WorkBuddy 浏览器授权信息无效，请重新授权".into()};}
    let mut headers=vec![("X-Domain",session.domain.clone()),("X-User-Id",session.user_id.clone())];
    if let Some(enterprise)=&session.enterprise_id {headers.push(("X-Enterprise-Id",enterprise.clone()));headers.push(("X-Tenant-Id",enterprise.clone()));}
    // domain 是官方验证返回的认证头，不用于决定请求主机。
    let base=match edition {Edition::Cn=>"https://www.workbuddy.cn",Edition::Ai=>"https://www.workbuddy.ai"};
    query(Session::Ready{edition,token:session.token.clone(),base,headers,email:session.email.clone()})
}

fn query(session: Session) -> QuotaState {
    let Session::Ready { edition, token, base, headers, email } = session else { unreachable!() };
    let client = match quota::client() { Ok(c) => c, Err(reason) => return QuotaState::Failed { reason } };
    // 国际版接口有过 `/v2` 前缀的迁移，404 时再试一次
    let paths: &[&str] = if edition == Edition::Ai {
        &["/billing/meter/get-user-resource-summary", "/v2/billing/meter/get-user-resource-summary"]
    } else {
        &["/billing/meter/get-user-resource-summary"]
    };
    for (index, path) in paths.iter().enumerate() {
        let mut request = client.post(format!("{base}{path}"))
            .bearer_auth(&token)
            .header("Accept", "application/json")
            .header("Origin", base)
            .header("Referer", format!("{base}/profile/plans-usage"))
            .json(&json!({}));
        for (name, value) in &headers { request = request.header(*name, value); }
        let response = match request.send() {
            Ok(response) => response,
            Err(_) => return QuotaState::Failed { reason: "WorkBuddy 积分查询失败，请检查网络后刷新".into() },
        };
        if response.status().as_u16() == 404 && index + 1 < paths.len() { continue; }
        let body = match quota::read(response) {
            HttpOutcome::Body(body) => body,
            HttpOutcome::Unauthorized(_) => return QuotaState::Unauthorized { reason: "WorkBuddy 登录已失效，请打开 WorkBuddy 重新登录".into() },
            HttpOutcome::Forbidden(reason) => return QuotaState::Forbidden { reason },
            HttpOutcome::RateLimited(reason) => return QuotaState::RateLimited { reason },
            HttpOutcome::Failed(reason) => return QuotaState::Failed { reason: format!("WorkBuddy 积分查询失败：{reason}") },
        };
        let Ok(value) = serde_json::from_str::<Value>(&body) else {
            return QuotaState::Failed { reason: "WorkBuddy 积分响应格式无效".into() };
        };
        let mut state = parse_summary(&value, chrono::Utc::now());
        if let QuotaState::Ok { windows,account, .. } = &mut state {
            *account = Some(AccountMetadata { email, ..AccountMetadata::default() });
            if let Some(window)=windows.iter_mut().find(|window|window.key=="credits"&&window.resets_at.is_none()) {
                window.resets_at=cycle_reset(&client,base,&token,&headers,&value);
            }
        }
        return state;
    }
    QuotaState::Failed { reason: "WorkBuddy 积分接口不可用".into() }
}

#[cfg(test)]
pub(crate) fn local_workbuddy_quota() -> QuotaState {
    if crate::validation::isolated() {
        return QuotaState::Unauthorized { reason: "隔离环境不读取本机 WorkBuddy 登录".into() };
    }
    let Some(dir) = auth_dir() else {
        return QuotaState::Unauthorized { reason: "无法定位 WorkBuddy 登录目录".into() };
    };
    let source = if dir.join(Edition::Cn.file()).is_file() { "native:workbuddy:main" } else { "native:workbuddy:ai" };
    source_quota(source)
}

fn source_session(dir: &Path, source_id: &str, now: chrono::DateTime<chrono::Utc>) -> Result<Session, QuotaState> {
    let edition = match source_id {
        "native:workbuddy:main" => Edition::Cn,
        "native:workbuddy:ai" => Edition::Ai,
        _ => return Err(QuotaState::Unsupported { reason: "未知 WorkBuddy 登录来源".into() }),
    };
    if dir.join(format!("{}.logged-out", edition.file())).exists() {
        return Err(QuotaState::Unauthorized { reason: "所选 WorkBuddy 版本已退出登录，请先在对应原工具登录".into() });
    }
    // 精确读取对应版本，缺失、过期、加密或损坏均不切换到另一版本。
    let raw = std::fs::read_to_string(dir.join(edition.file())).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            QuotaState::Unauthorized { reason: "未找到所选 WorkBuddy 版本的登录，请先在对应原工具登录".into() }
        } else { QuotaState::Failed { reason: "无法读取所选 WorkBuddy 版本的登录文件".into() } }
    })?;
    let value = serde_json::from_str::<Value>(&raw)
        .map_err(|_| QuotaState::Failed { reason: "WorkBuddy 登录文件格式无法识别".into() })?;
    resolve_session(edition, value, now)
}

pub(crate) fn source_quota(source_id: &str) -> QuotaState {
    if !matches!(source_id, "native:workbuddy:main" | "native:workbuddy:ai") {
        return QuotaState::Unsupported { reason: "未知 WorkBuddy 登录来源".into() };
    }
    if crate::validation::isolated() {
        return QuotaState::Unauthorized { reason: "隔离环境不读取本机 WorkBuddy 登录".into() };
    }
    let Some(dir) = auth_dir() else {
        return QuotaState::Unauthorized { reason: "无法定位 WorkBuddy 登录目录".into() };
    };
    match source_session(&dir, source_id, chrono::Utc::now()) {
        Ok(session) => query(session),
        Err(state) => state,
    }
}

fn session_identity(session: &Session) -> String {
    let Session::Ready { base, token, headers, .. } = session else { unreachable!("identity requires validated session") };
    let mut hash = Sha256::new();
    for value in std::iter::once(*base).chain(std::iter::once(token.as_str()))
        .chain(headers.iter().flat_map(|(name, value)| [*name, value.as_str()])) {
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

/// 后端缓存专用摘要，不输出到日志或前端；邮箱等资料字段不参与身份判断。
pub(crate) fn source_identity(source_id: &str) -> Result<String, QuotaState> {
    if !matches!(source_id, "native:workbuddy:main" | "native:workbuddy:ai") {
        return Err(QuotaState::Unsupported { reason: "未知 WorkBuddy 登录来源".into() });
    }
    if crate::validation::isolated() {
        return Err(QuotaState::Unauthorized { reason: "隔离环境不读取本机 WorkBuddy 登录".into() });
    }
    let dir = auth_dir().ok_or_else(|| QuotaState::Unauthorized { reason: "无法定位 WorkBuddy 登录目录".into() })?;
    source_session(&dir, source_id, chrono::Utc::now()).map(|session| session_identity(&session))
}

/// 登录检测仅返回展示账号，认证原值始终留在本模块。
pub(crate) fn source_account(source_id: &str) -> Result<Option<String>, QuotaState> {
    if crate::validation::isolated() { return Err(QuotaState::Unauthorized {reason:"隔离环境不读取本机 WorkBuddy 登录".into()}); }
    let dir=auth_dir().ok_or_else(|| QuotaState::Unauthorized {reason:"无法定位 WorkBuddy 登录目录".into()})?;
    match source_session(&dir,source_id,chrono::Utc::now())? {
        Session::Ready {email,..} => Ok(email),
        _ => Err(QuotaState::Unauthorized {reason:"未读取到有效 WorkBuddy 登录".into()}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_authorization_requires_correct_region_and_unexpired_session() {
        let session=crate::workbuddy_browser_auth::Session{token:"synthetic-session".into(),domain:"www.workbuddy.cn".into(),user_id:"user".into(),enterprise_id:None,email:None,expires_at:Some(1)};
        assert!(matches!(authorized_quota("unknown",&session),QuotaState::Unsupported{..}));
        assert!(matches!(authorized_quota("cn",&session),QuotaState::Unauthorized{..}));
        assert!(matches!(authorized_quota("ai",&session),QuotaState::Unauthorized{..}));
    }

    #[test]
    fn summary_does_not_add_non_credit_resources() {
        let body = json!({"code":0,"data":{"Packages":[
            {"CapacityUnit":"credits","CycleTotalCapacity":100,"CycleRemainCapacity":40},
            {"CapacityUnit":"requests","CycleTotalCapacity":1000,"CycleRemainCapacity":900},
            {"CapacityUnit":"bytes"}
        ]}});
        let QuotaState::Ok { windows, .. } = parse_summary(&body, now()) else { panic!() };
        assert_eq!(windows[0].amount_text.as_deref(), Some("60.00 / 100.00 credits"));
        assert_eq!(windows[0].remaining_text.as_deref(), Some("剩余 40.00 credits"));
    }

    #[test]
    fn listing_reset_uses_only_matching_live_credits_and_complete_lists() {
        let summary = json!({"code":0,"data":{"Packages":[{"PackageCode":"credits-package","CapacityUnit":"credits"}]}});
        let mut listing = json!({"code":0,"data":{"TotalCount":4,"Accounts":[
            {"PackageCode":"credits-package","CycleCapacityRemainPrecise":"40","CycleEndTime":"2026-11-01 23:59:59"},
            {"PackageCode":"credits-package","CycleCapacityRemainPrecise":"0","CycleEndTime":"2026-10-25 23:59:59"},
            {"PackageCode":"other-package","CycleCapacityRemainPrecise":"40","CycleEndTime":"2026-10-26 23:59:59"},
            {"PackageCode":"credits-package","CapacityUnit":"requests","CycleCapacityRemainPrecise":"40","CycleEndTime":"2026-10-27 23:59:59"}
        ]}});
        assert_eq!(listing_reset(&listing, &summary, now()).unwrap().to_rfc3339(), "2026-11-01T23:59:59+08:00");
        listing["data"]["TotalCount"] = json!(1000);
        assert!(listing_reset(&listing, &summary, now()).is_none());
        listing["code"] = json!(401);
        assert!(listing_reset(&listing, &summary, now()).is_none());
    }

    #[test]
    fn source_identity_ignores_profile_but_binds_token_base_and_headers() {
        let base = json!({"auth":{"accessToken":"synthetic-a","domain":"codebuddy.cn"},"account":{"uid":"u1","email":"a@example.invalid"}});
        let initial = session_identity(&parse_session(Edition::Cn, &base, now()));
        let mut other = base.clone();
        other["account"]["email"] = json!("b@example.invalid");
        assert_eq!(initial, session_identity(&parse_session(Edition::Cn, &other, now())));
        other["auth"]["accessToken"] = json!("synthetic-b");
        assert_ne!(initial, session_identity(&parse_session(Edition::Cn, &other, now())));
        other = base.clone();
        other["account"]["uid"] = json!("u2");
        assert_ne!(initial, session_identity(&parse_session(Edition::Cn, &other, now())));
        other = base.clone();
        other["auth"]["domain"] = json!("workbuddy.cn");
        assert_ne!(initial, session_identity(&parse_session(Edition::Cn, &other, now())));
    }

    #[test]
    #[ignore = "需要鼠鼠授权：读取本机 WorkBuddy 登录并向官方接口发起只读查询，只输出状态类别"]
    fn local_read_only_smoke() {
        let state = local_workbuddy_quota();
        let kind = serde_json::to_value(&state).unwrap()["state"].as_str().unwrap_or("?").to_string();
        let windows = match &state { QuotaState::Ok { windows, .. } => windows.len(), _ => 0 };
        println!("WorkBuddy: state={kind} windows={windows}");
        if let QuotaState::Failed {reason} = &state { println!("WorkBuddy failure: {reason}"); }
        assert_eq!(kind, "ok", "本机授权未通过真实积分查询");
    }

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-10-24T00:00:00Z").unwrap().with_timezone(&chrono::Utc)
    }

    #[test]
    fn source_binding_never_falls_back_between_editions() {
        let dir = std::env::temp_dir().join(format!("jolu-workbuddy-source-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(Edition::Cn.file()), json!({"auth":{"accessToken":"cn-synthetic","domain":"codebuddy.cn","expiresAt":1}}).to_string()).unwrap();
        std::fs::write(dir.join(Edition::Ai.file()), json!({"auth":{"accessToken":"ai-synthetic","domain":"workbuddy.ai","expiresAt":1893456000000_i64}}).to_string()).unwrap();
        let cn = source_session(&dir, "native:workbuddy:main", now());
        let ai = source_session(&dir, "native:workbuddy:ai", now());
        let missing = source_session(&dir, "unknown", now());
        std::fs::write(dir.join(format!("{}.logged-out", Edition::Ai.file())), "").unwrap();
        let logged_out_ai = source_session(&dir, "native:workbuddy:ai", now());
        // 删除精确的合成文件，不涉及任何真实客户端数据。
        std::fs::remove_file(dir.join(Edition::Cn.file())).unwrap();
        let absent_cn = source_session(&dir, "native:workbuddy:main", now());
        std::fs::remove_file(dir.join(Edition::Ai.file())).unwrap();
        std::fs::remove_file(dir.join(format!("{}.logged-out", Edition::Ai.file()))).unwrap();
        std::fs::remove_dir(&dir).unwrap();
        assert!(matches!(cn, Err(QuotaState::Unauthorized { .. })));
        assert!(matches!(ai, Ok(Session::Ready { edition: Edition::Ai, base: "https://www.workbuddy.ai", .. })));
        assert!(matches!(absent_cn, Err(QuotaState::Unauthorized { .. })));
        assert!(matches!(logged_out_ai, Err(QuotaState::Unauthorized { .. })));
        assert!(matches!(missing, Err(QuotaState::Unsupported { .. })));
    }

    #[test]
    fn session_requires_plain_token_and_official_domain() {
        let good = json!({"auth":{"accessToken":"t","domain":"www.codebuddy.cn","expiresAt":1893456000000_i64},"account":{"uid":"u1","nickname":"Nick","enterpriseId":"e1"}});
        match parse_session(Edition::Cn, &good, now()) {
            Session::Ready { base, headers, email, .. } => {
                assert_eq!(base, "https://www.codebuddy.cn");
                assert_eq!(email.as_deref(), Some("Nick"));
                assert!(headers.contains(&("X-User-Id", "u1".into())) && headers.contains(&("X-Tenant-Id", "e1".into())));
            }
            other => panic!("{other:?}"),
        }
        let mut encrypted = good.clone();
        encrypted["auth"]["accessToken"] = json!({"$wbEncrypted":true,"envelope":"x"});
        assert_eq!(parse_session(Edition::Cn, &encrypted, now()), Session::Encrypted);
        let mut evil = good.clone();
        evil["auth"]["domain"] = json!("www.codebuddy.cn.evil.test");
        assert_eq!(parse_session(Edition::Cn, &evil, now()), Session::Invalid);
        assert_eq!(parse_session(Edition::Ai, &good, now()), Session::Invalid);
        let mut expired = good.clone();
        expired["auth"]["expiresAt"] = json!(1_700_000_000);
        assert_eq!(parse_session(Edition::Cn, &expired, now()), Session::Expired);
        let mut empty = good;
        empty["auth"]["accessToken"] = json!("");
        assert_eq!(parse_session(Edition::Cn, &empty, now()), Session::Invalid);
    }

    #[test]
    fn summary_sums_live_packages_and_keeps_real_zero() {
        let body = json!({"code":0,"data":{"Packages":[
            {"SubscriptionPackageName":"体验版","CycleTotalCapacity":500,"CycleRemainCapacity":0,"CycleEndTime":"2026-11-01 23:59:59"},
            {"CycleCapacitySize":100,"CycleCapacityRemain":40,"CycleEndTime":1793462399},
            {"CycleTotalCapacity":50,"CycleRemainCapacity":50,"CycleEndTime":"2026-10-01 00:00:00"}
        ]}});
        let QuotaState::Ok { windows, plan, .. } = parse_summary(&body, now()) else { panic!() };
        assert_eq!(plan.as_deref(), Some("体验版"));
        let credits = &windows[0];
        assert_eq!(credits.used_percent, Some(93.3));
        assert_eq!(credits.remaining_text.as_deref(), Some("剩余 40.00 credits"));
        // 只有剩余不为 0 的包决定最近的重置时间
        assert_eq!(credits.resets_at.as_deref(), Some("2026-11-01T00:00:00+08:00"));
    }

    #[test]
    fn summary_reads_top_level_subscription_name_without_guessing_expiry() {
        let body = json!({"code":0,"data":{"SubscriptionPackageName":"高级版","IsPaidUser":true,"Packages":[{"CycleTotalCapacity":500,"CycleRemainCapacity":0}]}});
        let QuotaState::Ok { plan, account, windows } = parse_summary(&body, now()) else { panic!() };
        assert_eq!(plan.as_deref(), Some("高级版"));
        assert_eq!(windows[0].remaining_text.as_deref(), Some("剩余 0.00 credits"));
        assert!(account.and_then(|account| account.expires_at).is_none());
    }

    #[test]
    fn summary_errors_are_not_reported_as_zero() {
        assert!(matches!(parse_summary(&json!({"code":10085}), now()), QuotaState::Unauthorized { .. }));
        assert!(matches!(parse_summary(&json!({"code":500}), now()), QuotaState::Failed { .. }));
        assert!(matches!(parse_summary(&json!({"code":0,"data":{}}), now()), QuotaState::Failed { .. }));
        let QuotaState::Ok { windows, .. } = parse_summary(&json!({"code":0,"data":{"Packages":[]}}), now()) else { panic!() };
        assert!(windows.is_empty());
    }

    #[test]
    fn summary_rejects_missing_invalid_and_partial_capacities() {
        for item in [
            json!({"CycleTotalCapacity":100}),
            json!({"CycleTotalCapacity":100,"CycleRemainCapacity":-1}),
            json!({"CycleTotalCapacity":100,"CycleRemainCapacity":101}),
            json!({"CycleTotalCapacity":100,"CycleUsedCapacity":101}),
            json!({"CycleTotalCapacity":100,"CycleRemainCapacity":40,"CycleUsedCapacity":30}),
            json!({"CycleTotalCapacity":100,"CycleCapacityRemainPrecise":"bad","CycleRemainCapacity":40}),
            json!({"CycleTotalCapacity":100,"CycleRemainCapacity":40,"CycleEndTime":"bad"}),
            json!({}),
        ] {
            let body = json!({"code":0,"data":{"Packages":[
                {"CycleTotalCapacity":50,"CycleRemainCapacity":20}, item
            ]}});
            assert!(matches!(parse_summary(&body, now()), QuotaState::Failed { .. }), "{body}");
        }
    }

    #[test]
    fn summary_derives_only_from_two_known_capacities() {
        for item in [
            json!({"CycleTotalCapacity":100,"CycleUsedCapacity":60}),
            json!({"CycleRemainCapacity":40,"CycleUsedCapacity":60}),
        ] {
            let body = json!({"code":0,"data":{"Packages":[item]}});
            let QuotaState::Ok { windows, .. } = parse_summary(&body, now()) else { panic!() };
            assert_eq!(windows[0].used_percent, Some(60.0));
            assert_eq!(windows[0].remaining_text.as_deref(), Some("剩余 40.00 credits"));
        }
    }
}
