//! Qoder CLI SDK control 协议的只读账号额度查询。
//! 对齐官方 SDK 1.0.56 / CLI 1.1.78：仅 initialize + get_usage_info，绝不发送 user 帧。
//! SDK 模式必须同时设置 ENTRYPOINT、VERSION 和一次性 qodercli auth payload；
//! 单独设置 stream-json 会进入普通 CLI 模式，不能用于此查询。
use crate::quota::{AccountMetadata, QuotaState, QuotaWindow};
use serde_json::{json, Value};
use std::{io::{BufRead, BufReader, Read, Write}, path::{Path, PathBuf}, process::{Child, Command, Stdio}, sync::mpsc, time::{Duration, Instant}};

fn failed(reason: &str) -> QuotaState { QuotaState::Failed { reason: reason.into() } }
fn unsupported(reason: &str) -> QuotaState { QuotaState::Unsupported { reason: reason.into() } }

fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key)?.as_f64().filter(|n| n.is_finite() && *n >= 0.0)
}

fn label(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::trim).filter(|s| !s.is_empty() && s.len() <= 320 && !s.chars().any(char::is_control)).map(str::to_string)
}

fn parse_usage(value: &Value) -> QuotaState {
    let Some(usage) = value.get("usage").filter(|value| value.is_object()) else {
        return failed("Qoder 未返回账号额度；当前会话用量不能代替账号额度");
    };
    let plan = label(usage, "userType");
    let mut windows = Vec::new();
    if let Some(percent) = number(usage, "totalUsagePercentage").filter(|n| *n <= 100.0) {
        windows.push(QuotaWindow { key: "qoder:overall".into(), window_name: "总额度".into(), used_percent: Some(percent), amount_text: None, remaining_text: None, resets_at: None });
    }
    for (key, name, total_key) in [("userQuota", "套餐额度", "total"), ("addOnQuota", "加购额度", "total"), ("orgResourcePackage", "组织额度", "cap")] {
        let Some(bucket) = usage.get(key).filter(|v| v.is_object()) else { continue };
        let unit = label(bucket, "unit").map(|unit| format!(" {unit}")).unwrap_or_default();
        windows.push(QuotaWindow {
            key: format!("qoder:{key}"), window_name: name.into(),
            used_percent: number(bucket, "percentage").filter(|n| *n <= 100.0),
            amount_text: match (number(bucket, "used"), number(bucket, total_key)) {
                (Some(used), Some(total)) => Some(format!("{used} / {total}{unit}")),
                (None, Some(total)) => Some(format!("总额度 {total}{unit}；已用未知")),
                _ => None,
            },
            remaining_text: number(bucket, "remaining").map(|remaining| format!("剩余 {remaining}{unit}")),
            // expiresAt 是订阅期限，不是额度重置时刻；不进行猜测转换。
            resets_at: None,
        });
    }
    if windows.is_empty() && plan.is_none() { return failed("Qoder 未返回可识别的账号套餐或额度"); }
    QuotaState::Ok { windows, plan, account: None }
}

/// 不回显 CLI 的原始错误，避免原始响应中夹带账号或认证信息。
fn protocol_error(text: &str) -> QuotaState {
    let text = text.to_ascii_lowercase();
    if text.contains("login") || text.contains("sign in") || text.contains("unauthorized") || text.contains("authentication required") {
        QuotaState::Unauthorized { reason: "Qoder CLI 登录不可用，请先在原工具登录后刷新".into() }
    } else if text.contains("unsupported") || text.contains("unknown request") || text.contains("unknown control") {
        unsupported("当前 Qoder CLI 不支持只读额度控制接口，请升级 Qoder CLI")
    } else if text.contains("429") || text.contains("rate limit") {
        QuotaState::RateLimited { reason: "Qoder 额度查询过于频繁，请稍后刷新".into() }
    } else if text.contains("403") || text.contains("forbidden") {
        QuotaState::Forbidden { reason: "Qoder 当前账号无权读取额度".into() }
    } else { failed("Qoder 额度控制请求失败，请检查原工具登录与网络状态") }
}

fn control_response(frame: &Value, id: &str) -> Result<Option<Value>, QuotaState> {
    match frame.get("type").and_then(Value::as_str) {
        Some("control_response") => {
            let response = &frame["response"];
            if response["request_id"].as_str() != Some(id) { return Ok(None); }
            match response["subtype"].as_str() {
                Some("success") => response.get("response").filter(|v| v.is_object()).cloned().map(Some).ok_or_else(|| failed("Qoder 控制响应格式无效")),
                Some("error") => Err(protocol_error(response["error"].as_str().unwrap_or_default())),
                _ => Err(failed("Qoder 控制响应格式无效")),
            }
        }
        Some("result") => Err(protocol_error(&frame.get("errors").unwrap_or(&Value::Null).to_string())),
        Some("assistant" | "user" | "control_request") => Err(unsupported("Qoder CLI 未保持只读控制模式，已终止额度查询")),
        Some("system" | "keepalive" | "keep_alive" | "status_event") => Ok(None),
        _ => Err(failed("Qoder 控制响应格式无效")),
    }
}

struct Runtime { executable: PathBuf, script: Option<PathBuf> }
impl Runtime {
    fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        if let Some(script) = &self.script { command.arg(script); }
        command.env_remove("NODE_OPTIONS");
        #[cfg(windows)] {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        command
    }
}

fn locate_runtime() -> Result<Runtime, QuotaState> {
    let path = crate::session_threads::find_executable("qodercli")
        .or_else(|| crate::creds::home().map(|home| home.join(".qoder/local").join(if cfg!(windows) { "qodercli.exe" } else { "qodercli" })).filter(|p| p.is_file()))
        .ok_or_else(|| unsupported("未检测到 Qoder CLI，请先安装支持 SDK 控制接口的 Qoder CLI"))?;
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or_default().to_ascii_lowercase();
    if extension == "cmd" || extension == "bat" {
        // 只接受官方 npm 布局；不通过 cmd /c 执行未知包装脚本。
        let parent = path.parent().ok_or_else(|| unsupported("无法定位 Qoder CLI 安装目录"))?;
        let script = parent.join("node_modules/@qoder-ai/qodercli/bundle/qodercli.js");
        let node = parent.join("node.exe");
        let executable = if node.is_file() { Some(node) } else { crate::session_threads::find_executable("node") }
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")))
            .ok_or_else(|| unsupported("Qoder npm 安装缺少可用的 Node.js 运行时"))?;
        if !script.is_file() { return Err(unsupported("当前 Qoder CLI 包装器不属于受支持的官方 npm 安装布局")); }
        return Ok(Runtime { executable, script: Some(script) });
    }
    Ok(Runtime { executable: path, script: None })
}

/// 创建的是本次专属临时目录；只删除该目录，不修改原工具配置与登录。
struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Result<Self, QuotaState> {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|_| failed("创建 Qoder 查询目录失败"))?.as_nanos();
        let path = std::env::temp_dir().join(format!("jolu-qoder-quota-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&path).map_err(|_| failed("创建 Qoder 查询目录失败"))?;
        Ok(Self(path))
    }
}
impl Drop for Workspace { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
struct Process(Child);
impl Drop for Process { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }

fn verify_version(runtime: &Runtime, cwd: &Path) -> Result<(), QuotaState> {
    let mut child = Process(runtime.command().arg("--version").current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
        .env_remove("QODER_AGENT_SDK_ENTRYPOINT").env_remove("QODER_SDK_AUTH_PAYLOAD_FILE").spawn().map_err(|_| failed("启动 Qoder CLI 失败"))?);
    let output = child.0.stdout.take().ok_or_else(|| failed("读取 Qoder CLI 版本失败"))?;
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || { let mut value = String::new(); let result = output.take(256).read_to_string(&mut value).map(|_| value); let _ = tx.send(result); });
    let text = rx.recv_timeout(Duration::from_secs(10)).map_err(|_| failed("读取 Qoder CLI 版本超时"))?.map_err(|_| failed("读取 Qoder CLI 版本失败"))?;
    let version: Vec<u64> = text.trim().trim_start_matches('v').split('.').map(str::parse).collect::<Result<_, _>>()
        .map_err(|_| unsupported("无法识别 Qoder CLI 版本，未启动额度查询"))?;
    if version.len() != 3 || version.as_slice() < [1, 1, 78].as_slice() {
        return Err(unsupported("Qoder 额度查询需要 Qoder CLI 1.1.78 或更新版本"));
    }
    Ok(())
}

fn request(child: &mut Child, id: &str, kind: &str, extra: Value) -> Result<(), QuotaState> {
    let mut request = extra;
    request["type"] = json!(kind);
    request["subtype"] = json!(kind); // 与 SDK 的 mo(request) 序列化一致。
    let frame = json!({"type":"control_request", "request_id":id, "request":request});
    writeln!(child.stdin.as_mut().ok_or_else(|| failed("Qoder 控制输入已关闭"))?, "{frame}")
        .map_err(|_| failed("发送 Qoder 只读控制请求失败"))
}

fn query(runtime: &Runtime, cwd: &Path) -> Result<QuotaState, QuotaState> {
    let auth_path = cwd.join("auth.json");
    std::fs::write(&auth_path, br#"{"type":"qodercli"}"#).map_err(|_| failed("创建 Qoder 本地登录选择文件失败"))?;
    let mut child = Process(runtime.command().args([
        "--print", "--input-format", "stream-json", "--output-format", "stream-json", "--no-session-persistence",
        "--setting-sources", "", "--strict-mcp-config", "--mcp-config", r#"{"mcpServers":{}}"#,
        "--disable-builtin-skills", "--tools", "", "--settings", r#"{"disableAllHooks":true,"enabledPlugins":{}}"#,
    ]).env("QODER_AGENT_SDK_ENTRYPOINT", "sdk-ts").env("QODER_AGENT_SDK_VERSION", "1.0.56")
        .env("QODER_SDK_AUTH_PAYLOAD_FILE", &auth_path).env_remove("QODER_PERSONAL_ACCESS_TOKEN").env_remove("QODER_SERVICE_ACCOUNT_KEY")
        .current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|_| failed("启动 Qoder CLI 失败"))?);
    let output = child.0.stdout.take().ok_or_else(|| failed("Qoder 控制输出不可用"))?;
    let (tx, rx) = mpsc::sync_channel(4);
    std::thread::spawn(move || {
        // 整次输出最多 2 MiB，避免异常 CLI 无界占用内存；不保存原始输出。
        let reader = BufReader::new(output.take(2 * 1024 * 1024));
        for line in reader.lines() {
            let frame = line.ok().and_then(|line| serde_json::from_str::<Value>(&line).ok());
            if tx.send(frame).is_err() { return; }
        }
    });
    request(&mut child.0, "jolu-init", "initialize", json!({"skills":[],"plugins":[],"mcpServers":{},"hooks":{},"allowedTools":[],
        "memory":{"requester":"sdk","mode":"custom","projectScope":false,"userScope":false,"generation":{"enabled":false},"consumption":{"enabled":false}},
        "evolution":{"skill":{"requester":"sdk","mode":"custom","generation":{"enabled":false}}}}))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut id = "jolu-init";
    let mut email = None;
    loop {
        let wait = deadline.checked_duration_since(Instant::now()).ok_or_else(|| failed("Qoder 额度查询超时"))?;
        let frame = rx.recv_timeout(wait).map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => failed("Qoder 额度查询超时"),
            _ => failed("Qoder CLI 在返回额度前退出"),
        })?.ok_or_else(|| failed("Qoder 控制响应格式无效"))?;
        let Some(response) = control_response(&frame, id)? else { continue };
        if id == "jolu-init" {
            email = label(&response["account"], "email");
            request(&mut child.0, "jolu-quota", "get_usage_info", json!({}))?;
            id = "jolu-quota";
        } else {
            let mut state = parse_usage(&response);
            if let QuotaState::Ok { account, .. } = &mut state {
                *account = Some(AccountMetadata { email, ..AccountMetadata::default() });
            }
            return Ok(state);
        }
    }
}

pub(crate) fn local_qoder_quota() -> QuotaState {
    if crate::validation::isolated() { return QuotaState::Unauthorized { reason: "隔离环境不读取本机 Qoder 登录".into() }; }
    (|| {
        let runtime = locate_runtime()?;
        let workspace = Workspace::new()?;
        verify_version(&runtime, &workspace.0)?;
        query(&runtime, &workspace.0)
    })().unwrap_or_else(|state| state)
}

/// 优先使用对应版本桌面登录；仅国际版未登录桌面端时使用国际 CLI。
pub(crate) fn source_quota(source_id: &str) -> QuotaState {
    match source_id {
        "native:qoder:global" | "native:qoder:cn" => crate::qoder_desktop::source_quota(source_id),
        _ => unsupported("未知 Qoder 登录来源"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_sources_do_not_start_global_cli() {
        for source in ["unknown", "monitor:native:qoder:global"] {
            assert!(matches!(source_quota(source), QuotaState::Unsupported { .. }));
        }
    }

    #[test]
    #[ignore = "需要本机 Node.js；只运行临时合成控制端，不联网、不读取真实登录"]
    fn protocol_fixture_round_trip_uses_sdk_mode_without_prompts() {
        let workspace = Workspace::new().unwrap();
        let script = workspace.0.join("fixture.cjs");
        std::fs::write(&script, r#"
const assert=require('node:assert/strict'),fs=require('node:fs'),readline=require('node:readline');
const args=process.argv.slice(2), has=(key,value)=>args[args.indexOf(key)+1]===value;
assert.equal(process.env.QODER_AGENT_SDK_ENTRYPOINT,'sdk-ts');
assert.equal(process.env.QODER_AGENT_SDK_VERSION,'1.0.56');
assert.deepEqual(JSON.parse(fs.readFileSync(process.env.QODER_SDK_AUTH_PAYLOAD_FILE,'utf8')),{type:'qodercli'});
assert.ok(args.includes('--no-session-persistence')&&args.includes('--strict-mcp-config'));
assert.ok(has('--setting-sources','')&&has('--tools',''));
assert.equal(JSON.parse(args[args.indexOf('--settings')+1]).disableAllHooks,true);
assert.deepEqual(JSON.parse(args[args.indexOf('--mcp-config')+1]),{mcpServers:{}});
let count=0;readline.createInterface({input:process.stdin}).on('line',line=>{
 const frame=JSON.parse(line);assert.equal(frame.type,'control_request');count++;
 assert.equal(frame.request.type,count===1?'initialize':'get_usage_info');assert.ok(count<=2);
 if(count===1){assert.deepEqual(frame.request.skills,[]);assert.deepEqual(frame.request.plugins,[]);assert.equal(frame.request.memory.generation.enabled,false)}
 const response=count===1?{account:{email:'fixture@example.test'}}:{usage:{userType:'free',userQuota:{remaining:0,unit:'credits'}}};
 console.log(JSON.stringify({type:'control_response',response:{request_id:frame.request_id,subtype:'success',response}}));
});
"#).unwrap();
        let runtime = Runtime { executable: crate::session_threads::find_executable("node").expect("Node.js"), script: Some(script) };
        let QuotaState::Ok { windows, account, .. } = query(&runtime, &workspace.0).unwrap() else { panic!("expected fixture quota") };
        assert_eq!(windows[0].remaining_text.as_deref(), Some("剩余 0 credits"));
        assert_eq!(account.unwrap().email.as_deref(), Some("fixture@example.test"));
    }

    #[test]
    #[ignore = "需要鼠鼠授权：通过 Qoder CLI 只读查询本机登录的额度，只输出状态类别"]
    fn local_read_only_smoke() {
        let state = local_qoder_quota();
        let value = serde_json::to_value(&state).unwrap();
        println!("Qoder: state={} windows={}", value["state"].as_str().unwrap_or("?"), value["windows"].as_array().map_or(0, Vec::len));
    }

    #[test]
    fn account_quota_keeps_real_zero_and_distinct_buckets() {
        let state = parse_usage(&json!({"usage": {"userType":"pro", "expiresAt":1800000000000u64,
            "userQuota":{"used":0,"total":100,"remaining":100,"percentage":0,"unit":"credits"},
            "addOnQuota":{"remaining":0,"unit":"credits"},
            "orgResourcePackage":{"used":20,"cap":80,"remaining":60,"percentage":25,"unit":"credits"}},
            "session":{"total_credits":0}}));
        let QuotaState::Ok { windows, plan, .. } = state else { panic!("expected account quota") };
        assert_eq!(plan.as_deref(), Some("pro"));
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].used_percent, Some(0.0));
        assert_eq!(windows[1].used_percent, None);
        assert_eq!(windows[1].remaining_text.as_deref(), Some("剩余 0 credits"));
        assert_eq!(windows[2].amount_text.as_deref(), Some("20 / 80 credits"));
        assert!(windows.iter().all(|window| window.resets_at.is_none()));
    }

    #[test]
    fn partial_total_keeps_explicit_total_without_inventing_used() {
        let state = parse_usage(&json!({"usage":{"userType":"pro",
            "userQuota":{"total":100,"remaining":40,"unit":"credits"},
            "orgResourcePackage":{"cap":0,"remaining":0,"unit":"credits"}}}));
        let QuotaState::Ok { windows, .. } = state else { panic!("expected account quota") };
        assert_eq!(windows[0].amount_text.as_deref(), Some("总额度 100 credits；已用未知"));
        assert_eq!(windows[0].remaining_text.as_deref(), Some("剩余 40 credits"));
        assert_eq!(windows[1].amount_text.as_deref(), Some("总额度 0 credits；已用未知"));
        assert_eq!(windows[1].remaining_text.as_deref(), Some("剩余 0 credits"));
        assert!(windows.iter().all(|window| window.used_percent.is_none()));
    }

    #[test]
    fn session_usage_cannot_masquerade_as_account_quota() {
        for value in [json!({"usage":null,"session":{"total_credits":0}}),json!({"usage":[]}),json!({"usage":{}})] {
            assert!(matches!(parse_usage(&value), QuotaState::Failed { .. }));
        }
        assert!(matches!(parse_usage(&json!({"usage":{"userType":"free"}})), QuotaState::Ok { .. }));
    }

    #[test]
    fn invalid_numbers_stay_unknown_and_overall_percentage_is_supported() {
        let QuotaState::Ok { windows, .. } = parse_usage(&json!({"usage":{"totalUsagePercentage":40,
            "userQuota":{"used":-1,"total":"100","remaining":-2,"percentage":101}}})) else { panic!() };
        assert_eq!(windows[0].used_percent, Some(40.0));
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[1].used_percent, None);
        assert_eq!(windows[1].remaining_text, None);
        assert_eq!(windows[1].amount_text, None);
    }

    #[test]
    fn control_frames_are_matched_and_agent_output_is_rejected() {
        assert!(control_response(&json!({"type":"system","subtype":"init"}), "q").unwrap().is_none());
        assert!(control_response(&json!({"type":"control_response","response":{"request_id":"other","subtype":"success","response":{}}}), "q").unwrap().is_none());
        assert_eq!(control_response(&json!({"type":"control_response","response":{"request_id":"q","subtype":"success","response":{"usage":{}}}}), "q").unwrap(), Some(json!({"usage":{}})));
        assert!(matches!(control_response(&json!({"type":"assistant"}), "q"), Err(QuotaState::Unsupported { .. })));
        assert!(matches!(control_response(&json!({"type":"control_response","response":{"request_id":"q","subtype":"error","error":"unsupported request"}}), "q"), Err(QuotaState::Unsupported { .. })));
        assert!(matches!(control_response(&json!({"type":"result","subtype":"error_during_execution","errors":["Please login first"]}), "q"), Err(QuotaState::Unauthorized { .. })));
    }
}
