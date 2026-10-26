//! Official CLI browser sign-in in a disposable, process-scoped profile. No model requests.
use crate::quota::QuotaState;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs, io::Read, path::{Path, PathBuf}, process::{Child, Command, Stdio}, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

const LIMIT: u64 = 1024 * 1024;
const LIFETIME: Duration = Duration::from_secs(600);
const GEMINI_HELPER: &str = include_str!("cli-browser-auth.cjs");

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub platform: String,
    pub credentials: Value,
    pub account: Option<String>,
}

struct Profile {
    parent: PathBuf,
    temp: PathBuf,
    path: PathBuf,
}
impl Profile {
    fn new() -> Result<Self, String> {
        let temp = fs::canonicalize(std::env::temp_dir()).map_err(|_| "无法定位授权临时目录")?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| "无法创建授权临时目录")?.as_nanos();
        let parent = temp.join(format!("jolu-cli-auth-{}-{stamp}", std::process::id()));
        fs::create_dir(&parent).map_err(|_| "无法创建授权临时目录")?;
        let parent = fs::canonicalize(parent).map_err(|_| "无法确认授权临时目录")?;
        let path = parent.join("profile");
        let profile = Self { parent, temp, path };
        fs::create_dir(&profile.path).map_err(|_| "无法创建授权隔离配置")?;
        Ok(profile)
    }
    fn owns(&self, path: &Path) -> bool {
        self.parent.parent() == Some(self.temp.as_path())
            && fs::canonicalize(&self.parent).is_ok_and(|parent| parent == self.parent)
            && fs::canonicalize(path).is_ok_and(|resolved| resolved.starts_with(&self.parent) && resolved != self.parent)
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        if self.owns(&self.path) { let _ = fs::remove_dir_all(&self.path); }
        // Never recursively delete a computed parent; only the exact, now empty directory we created.
        if self.parent.parent() == Some(self.temp.as_path()) && fs::canonicalize(&self.parent).is_ok_and(|parent| parent == self.parent) {
            let _ = fs::remove_dir(&self.parent);
        }
    }
}

pub(crate) struct Pending {
    child: Option<Child>,
    profile: Profile,
    platform: String,
    started: Instant,
}
impl Pending {
    pub(crate) fn poll(&mut self) -> Result<Option<Session>, String> {
        if self.started.elapsed() >= LIFETIME { self.stop(); return Err("浏览器授权已超时，请重新发起".into()); }
        let child = self.child.as_mut().ok_or("授权已经结束，请重新发起")?;
        let Some(status) = child.try_wait().map_err(|_| "无法读取官方授权进程状态")? else { return Ok(None); };
        self.child = None;
        if !status.success() { return Err("官方浏览器授权未完成，请检查客户端、网络或重新发起".into()); }
        session_from_profile(&self.profile, &self.platform).map(Some)
    }
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if !matches!(child.try_wait(), Ok(Some(_))) {
                // Each login runs directly as its official executable (or our single Gemini
                // helper). A browser opened by it is user-owned and must survive cancellation.
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}
impl Drop for Pending { fn drop(&mut self) { self.stop(); } }

pub(crate) fn npm_prefix(tool: &str) -> Option<PathBuf> {
    crate::session_threads::find_executable(tool).and_then(|path| path.parent().map(Path::to_path_buf))
        .or_else(|| std::env::var_os("APPDATA").map(|path| PathBuf::from(path).join("npm")))
}
fn node() -> Result<PathBuf, String> {
    crate::session_threads::find_executable("node").filter(|path| !cfg!(windows) || path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe")))
        .ok_or_else(|| "未检测到官方 CLI 所需的 Node.js，请先安装后重试".into())
}
fn codex_from_npm(prefix: &Path) -> Option<PathBuf> {
    let (package, target) = if cfg!(target_arch = "aarch64") {
        ("codex-win32-arm64", "aarch64-pc-windows-msvc")
    } else {
        ("codex-win32-x64", "x86_64-pc-windows-msvc")
    };
    let codex = prefix.join("node_modules/@openai/codex");
    [codex.join("node_modules/@openai").join(package).join("vendor"),
        prefix.join("node_modules/@openai").join(package).join("vendor"), codex.join("vendor")]
        .into_iter().map(|vendor| vendor.join(target).join("bin/codex.exe")).find(|path| path.is_file())
}
fn command(platform: &str, profile: &Profile) -> Result<Command, String> {
    match platform {
        "claude" => {
            let executable = crate::session_threads::find_executable("claude").filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe")))
                .or_else(|| npm_prefix("claude").map(|prefix| prefix.join("node_modules/@anthropic-ai/claude-code/bin/claude.exe")).filter(|path| path.is_file()))
                .ok_or("未找到支持浏览器授权的官方 Claude Code，请先安装")?;
            let mut command = Command::new(executable);
            command.args(["auth", "login", "--claudeai"]);
            Ok(command)
        }
        "codex" | "opencode" => {
            let executable = crate::session_threads::find_executable("codex").filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe")))
                .or_else(|| npm_prefix("codex").and_then(|prefix| codex_from_npm(&prefix)))
                .ok_or("未找到官方 Codex CLI，OpenAI 订阅授权需要先安装 Codex CLI")?;
            // Avoid the npm Node wrapper: cancellation can then kill exactly the login process.
            let mut command = Command::new(executable);
            command.args(["login", "-c", "cli_auth_credentials_store=\"file\""]);
            Ok(command)
        }
        "gemini" => {
            let bundle = npm_prefix("gemini").map(|prefix| prefix.join("node_modules/@google/gemini-cli/bundle"))
                .filter(|path| path.join("gemini.js").is_file()).ok_or("未找到支持授权的官方 Gemini CLI npm 安装")?;
            let helper = profile.path.join("jolu-gemini-auth.cjs");
            fs::write(&helper, GEMINI_HELPER).map_err(|_| "无法准备 Gemini 官方授权")?;
            let mut command = Command::new(node()?);
            command.arg(helper).arg(bundle).arg(&profile.path);
            Ok(command)
        }
        _ => Err("该平台不支持 CLI 浏览器授权".into()),
    }
}

fn configure_environment(command: &mut Command, profile: &Profile) {
    // Only OS/runtime/proxy configuration is inherited. API keys, OAuth overrides and BROWSER
    // cannot silently select another account, endpoint or browser in the isolated login process.
    command.env_clear();
    for key in ["PATH", "PATHEXT", "SYSTEMROOT", "WINDIR", "SYSTEMDRIVE", "COMSPEC", "TEMP", "TMP", "HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "HOMEDRIVE", "HOMEPATH", "USERDOMAIN", "USERNAME", "PROGRAMDATA", "PROGRAMFILES", "PROGRAMFILES(X86)", "PROGRAMW6432", "PUBLIC", "LANG", "LC_ALL", "HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY", "NO_PROXY", "https_proxy", "http_proxy", "all_proxy", "no_proxy"] {
        if let Some(value) = std::env::var_os(key) { command.env(key, value); }
    }
    command.current_dir(&profile.path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        // Preserve OS profile locations for the system browser; isolate only the official CLI stores.
        .env("CLAUDE_CONFIG_DIR", &profile.path).env("CODEX_HOME", &profile.path).env("GEMINI_CLI_HOME", &profile.path)
        .env("GEMINI_FORCE_ENCRYPTED_FILE_STORAGE", "false").env("DISABLE_TELEMETRY", "1")
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1");
    configure_network(command,&crate::net::current_mode(),crate::net::system_proxy_url().as_deref());
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
}

fn configure_network(command:&mut Command,mode:&crate::net::NetMode,system_proxy:Option<&str>) {
    let proxy=match mode {crate::net::NetMode::Manual(url)=>Some(url.as_str()),crate::net::NetMode::System=>system_proxy,crate::net::NetMode::Direct=>None};
    if matches!(mode,crate::net::NetMode::Direct) {
        for key in ["HTTP_PROXY","HTTPS_PROXY","ALL_PROXY","http_proxy","https_proxy","all_proxy","JOLU_AUTH_PROXY_URL"] {command.env_remove(key);}
        command.env("NO_PROXY","*").env("no_proxy","*");
    } else if let Some(proxy)=proxy {
        for key in ["HTTP_PROXY","HTTPS_PROXY","http_proxy","https_proxy","JOLU_AUTH_PROXY_URL"] {command.env(key,proxy);}
        command.env("NO_PROXY","localhost,127.0.0.1,::1").env("no_proxy","localhost,127.0.0.1,::1");
    }
}

pub(crate) fn begin(platform: &str) -> Result<Pending, String> {
    if crate::validation::isolated() { return Err("隔离验收环境不创建真实授权".into()); }
    let profile = Profile::new()?;
    let mut command = command(platform, &profile)?;
    configure_environment(&mut command, &profile);
    let child = command.spawn().map_err(|_| "无法启动官方 CLI 浏览器授权，请检查安装")?;
    Ok(Pending { child: Some(child), profile, platform: platform.into(), started: Instant::now() })
}

fn bounded_json(profile: &Profile, path: &Path) -> Result<Value, String> {
    if !profile.owns(path) { return Err("授权文件未保存在本次隔离目录内".into()); }
    let file = fs::File::open(path).map_err(|_| "未找到本次浏览器授权文件")?;
    if file.metadata().map_err(|_| "无法读取授权文件")?.len() > LIMIT { return Err("授权文件大小超出限制".into()); }
    let mut data = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut data).map_err(|_| "无法读取授权文件")?;
    if data.len() as u64 > LIMIT { return Err("授权文件大小超出限制".into()); }
    serde_json::from_slice(&data).map_err(|_| "官方授权文件格式无效".into())
}
fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::trim).filter(|text| !text.is_empty() && text.len() <= 256 * 1024 && !text.chars().any(char::is_control)).map(str::to_string)
}
fn email(value: Option<String>) -> Option<String> { value.filter(|text| text.len() <= 320) }
fn session_from_profile(profile: &Profile, platform: &str) -> Result<Session, String> {
    let (credentials, account) = match platform {
        "claude" => {
            let credentials = bounded_json(profile, &profile.path.join(".credentials.json"))?;
            if text(&credentials["claudeAiOauth"], "accessToken").is_none() { return Err("Claude 授权未返回订阅访问凭证".into()); }
            let account = bounded_json(profile, &profile.path.join(".claude.json")).ok().and_then(|value| email(text(&value["oauthAccount"], "emailAddress")));
            (credentials, account)
        }
        "codex" | "opencode" => {
            let credentials = bounded_json(profile, &profile.path.join("auth.json"))?;
            if text(&credentials["tokens"], "access_token").is_none() { return Err("OpenAI 授权未返回订阅访问凭证".into()); }
            let account = text(&credentials["tokens"], "id_token").and_then(|token| token.split('.').nth(1).map(str::to_string))
                .and_then(|payload| URL_SAFE_NO_PAD.decode(payload).ok()).and_then(|data| serde_json::from_slice::<Value>(&data).ok())
                .and_then(|claims| email(text(&claims, "email")));
            (credentials, account)
        }
        "gemini" => {
            let result = bounded_json(profile, &profile.path.join("jolu-session.json"))?;
            let credentials = result["credentials"].clone();
            if text(&credentials, "access_token").is_none() && text(&credentials, "refresh_token").is_none() { return Err("Gemini 授权未返回访问凭证".into()); }
            (credentials, email(text(&result, "account")))
        }
        _ => return Err("该平台不支持 CLI 浏览器授权".into()),
    };
    Ok(Session { platform: platform.into(), credentials, account })
}

pub(crate) fn query(session: &Session) -> QuotaState {
    if crate::validation::isolated() { return QuotaState::Unauthorized { reason: "隔离验收环境不查询真实授权".into() }; }
    let missing = || QuotaState::Unauthorized { reason: "浏览器授权缺少有效凭证，请重新授权".into() };
    let mut state = match session.platform.as_str() {
        "claude" => match text(&session.credentials["claudeAiOauth"], "accessToken") {
            Some(token) => crate::quota::claude_oauth_quota(&token), None => return missing(),
        },
        "codex" | "opencode" => match text(&session.credentials["tokens"], "access_token") {
            Some(token) => crate::quota::codex_oauth_quota(&token, text(&session.credentials["tokens"], "account_id").as_deref()), None => return missing(),
        },
        "gemini" => crate::gemini_quota::authorized_quota(&session.credentials, session.account.as_deref()),
        _ => return QuotaState::Unsupported { reason: "未知的官方 CLI 授权供应商".into() },
    };
    if let QuotaState::Ok { account: Some(account), .. } = &mut state { account.email = session.account.clone(); }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn official_login_network_matches_app_mode_and_keeps_loopback_direct() {
        let mut command=Command::new("synthetic");
        configure_network(&mut command,&crate::net::NetMode::Manual("http://127.0.0.1:8080".into()),None);
        let value=|command:&Command,key:&str|command.get_envs().find(|(name,_)|*name==std::ffi::OsStr::new(key)).and_then(|(_,value)|value).map(|value|value.to_string_lossy().into_owned());
        assert_eq!(value(&command,"HTTPS_PROXY").as_deref(),Some("http://127.0.0.1:8080"));
        assert_eq!(value(&command,"JOLU_AUTH_PROXY_URL"),value(&command,"HTTPS_PROXY"));
        assert!(value(&command,"NO_PROXY").unwrap().contains("127.0.0.1"));
        configure_network(&mut command,&crate::net::NetMode::Direct,Some("http://127.0.0.1:8081"));
        assert_eq!(value(&command,"HTTPS_PROXY"),None);
        assert_eq!(value(&command,"NO_PROXY").as_deref(),Some("*"));
        configure_network(&mut command,&crate::net::NetMode::System,Some("http://127.0.0.1:8082"));
        assert_eq!(value(&command,"HTTPS_PROXY").as_deref(),Some("http://127.0.0.1:8082"));
    }

    #[test]
    fn browser_keeps_os_profile_and_only_cli_stores_are_isolated() {
        let profile = Profile::new().unwrap();
        let mut process = Command::new("unused-test-command");
        configure_environment(&mut process, &profile);
        let vars: std::collections::HashMap<_, _> = process.get_envs().collect();
        for key in ["HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA"] {
            if let Some(original) = std::env::var_os(key) {
                assert_eq!(vars.get(std::ffi::OsStr::new(key)), Some(&Some(original.as_os_str())));
            }
        }
        for key in ["CLAUDE_CONFIG_DIR", "CODEX_HOME", "GEMINI_CLI_HOME"] {
            assert_eq!(vars.get(std::ffi::OsStr::new(key)), Some(&Some(profile.path.as_os_str())));
        }
        assert!(!vars.contains_key(std::ffi::OsStr::new("BROWSER")));
        assert!(!vars.contains_key(std::ffi::OsStr::new("OPENAI_API_KEY")));
    }

    #[test]
    fn npm_codex_is_resolved_to_native_binary_without_a_node_wrapper() {
        let profile = Profile::new().unwrap();
        let (package, target) = if cfg!(target_arch = "aarch64") {
            ("codex-win32-arm64", "aarch64-pc-windows-msvc")
        } else { ("codex-win32-x64", "x86_64-pc-windows-msvc") };
        let binary = profile.path.join("node_modules/@openai/codex/node_modules/@openai")
            .join(package).join("vendor").join(target).join("bin/codex.exe");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"synthetic-executable").unwrap();
        assert_eq!(codex_from_npm(&profile.path), Some(binary));
    }

    #[test]
    fn cancelling_auth_does_not_kill_the_browser_descendant() {
        let profile = Profile::new().unwrap();
        let heartbeat = profile.path.join("synthetic-browser-heartbeat");
        // A harmless Node descendant stands in for a newly opened browser. It exits after three
        // seconds or when this test's profile is removed, including during panic cleanup.
        let script = r#"const {spawn}=require('node:child_process');const code="const fs=require('node:fs');const end=Date.now()+3000;setInterval(()=>{try{fs.writeFileSync(process.argv[1],String(Date.now()))}catch{process.exit(0)}if(Date.now()>end)process.exit(0)},25)";spawn(process.execPath,['-e',code,process.argv[1]],{stdio:'ignore',detached:true,windowsHide:true}).unref();setInterval(()=>{},1000)"#;
        let mut process = Command::new(node().unwrap());
        process.args(["-e", script]).arg(&heartbeat);
        configure_environment(&mut process, &profile);
        let child = process.spawn().unwrap();
        let mut pending = Pending { child: Some(child), profile, platform: "synthetic".into(), started: Instant::now() };
        let started = Instant::now();
        while !heartbeat.exists() && started.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(heartbeat.exists());
        pending.stop();
        let before = fs::read_to_string(&heartbeat).unwrap_or_default();
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_millis(30));
            if fs::read_to_string(&heartbeat).is_ok_and(|value| !value.is_empty() && value != before) { return; }
        }
        panic!("取消授权错误终止了独立的浏览器进程");
    }

    #[test]
    fn isolated_files_are_bounded_and_profiles_are_cleaned_without_touching_other_paths() {
        let profile = Profile::new().unwrap();
        let parent = profile.parent.clone();
        let fixture = profile.path.join("auth.json");
        fs::write(&fixture, serde_json::to_vec(&json!({"tokens":{"access_token":"synthetic"}})).unwrap()).unwrap();
        let session = session_from_profile(&profile, "opencode").unwrap();
        assert_eq!(session.platform, "opencode");
        assert!(session.account.is_none());
        assert!(!profile.owns(&profile.temp));
        fs::write(&fixture, vec![b'x'; LIMIT as usize + 1]).unwrap();
        assert!(bounded_json(&profile, &fixture).is_err());
        drop(profile);
        assert!(!parent.exists());
    }

    #[test]
    fn claude_and_gemini_sessions_read_only_the_created_profile() {
        let profile = Profile::new().unwrap();
        fs::write(profile.path.join(".credentials.json"), r#"{"claudeAiOauth":{"accessToken":"synthetic"}}"#).unwrap();
        fs::write(profile.path.join(".claude.json"), r#"{"oauthAccount":{"emailAddress":"synthetic@example.invalid"}}"#).unwrap();
        assert_eq!(session_from_profile(&profile, "claude").unwrap().account.as_deref(), Some("synthetic@example.invalid"));
        fs::write(profile.path.join("jolu-session.json"), r#"{"credentials":{"refresh_token":"synthetic"},"account":"synthetic@example.invalid"}"#).unwrap();
        assert_eq!(session_from_profile(&profile, "gemini").unwrap().account.as_deref(), Some("synthetic@example.invalid"));
        assert!(session_from_profile(&profile, "unsupported").is_err());
    }
}
