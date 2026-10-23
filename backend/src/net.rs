//! 应用联网方式（画板 90 / 102，D-24）与连接诊断（D-25）
//!
//! 作用范围：Jolu 自己发出的额度、余额、模型列表、Key 检测请求（`quota::client()`、
//! `connections::fetch_models`、`provider_key::validate`）与检查更新（`updater.rs`）。
//! CLI 本地代理的上游（`proxy.rs`）不跟随这里的设置。
//!
//! - 跟随系统：reqwest 自带的系统代理（环境变量优先，其次注册表 `Internet Settings`）。
//!   reqwest 把 `ProxyServer` 整串当作代理地址，按协议分开写（`http=..;https=..`）时会失效，
//!   这种写法由本模块解析，取 `https=` 那一项，没有就取 `http=` 那一项。PAC 不支持。
//! - 直连：`no_proxy()`。
//! - 手动代理：`proxy(Proxy::all(url))`，只支持 `http://` / `https://`，本机回环地址不走代理。

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// 代理地址格式错误时的提示（D-24）。
pub const PROXY_URL_HINT: &str = "请输入 http:// 或 https:// 开头、带端口的代理地址";

/// 手动代理下不走代理的本机地址：本机自建网关仍按原地址直连。
const LOOPBACK_NO_PROXY: &str = "localhost,127.0.0.1,::1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetMode {
    System,
    Direct,
    Manual(String),
}

impl NetMode {
    /// 设置值 → 联网方式；手动代理的地址必须先通过校验。
    pub fn parse(mode: &str, proxy_url: Option<&str>) -> Result<Self, String> {
        match mode {
            "system" => Ok(Self::System),
            "direct" => Ok(Self::Direct),
            "manual" => validate_proxy_url(proxy_url.unwrap_or_default()).map(Self::Manual),
            other => Err(format!("未知的联网方式：{other}")),
        }
    }
}

static MODE: RwLock<NetMode> = RwLock::new(NetMode::System);
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// 当前联网方式；启动时由 `init` 从设置读入，之后只由 `set_mode` 更新。
pub fn current_mode() -> NetMode {
    MODE.read().map(|mode| mode.clone()).unwrap_or(NetMode::System)
}

/// 每次修改联网方式加一，共享客户端据此判断是否要立即重建。
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

pub fn set_mode(mode: NetMode) {
    if let Ok(mut current) = MODE.write() {
        *current = mode;
    }
    GENERATION.fetch_add(1, Ordering::AcqRel);
}

/// 启动时按已保存的设置初始化。手工改坏的手动代理地址无法使用，按跟随系统处理并记录原因。
pub fn init(settings: &crate::settings::Settings) {
    let mode = NetMode::parse(&settings.network_mode, settings.network_proxy_url.as_deref()).unwrap_or_else(|error| {
        eprintln!("[网络] 已保存的联网方式无效，按跟随系统处理：{error}");
        NetMode::System
    });
    set_mode(mode);
}

/// 按当前联网方式配置好代理的 blocking 客户端构造器；超时、重定向等由调用方继续设置。
pub fn blocking_builder() -> reqwest::blocking::ClientBuilder {
    builder_for(&current_mode())
}

fn builder_for(mode: &NetMode) -> reqwest::blocking::ClientBuilder {
    let builder = reqwest::blocking::Client::builder();
    match mode {
        NetMode::System => match system_per_protocol_proxy() {
            Some(proxy) => builder.proxy(proxy),
            None => builder,
        },
        NetMode::Direct => builder.no_proxy(),
        NetMode::Manual(url) => match reqwest::Proxy::all(url.as_str()) {
            Ok(proxy) => builder.proxy(proxy.no_proxy(reqwest::NoProxy::from_string(LOOPBACK_NO_PROXY))),
            Err(error) => {
                // 地址在保存前已校验，这里只防御异常值；不能悄悄改成直连，指向一个必然失败的本机端口让请求报错。
                eprintln!("[网络] 手动代理地址无法使用：{error}");
                match reqwest::Proxy::all("http://127.0.0.1:9") {
                    Ok(proxy) => builder.proxy(proxy),
                    Err(_) => builder,
                }
            }
        },
    }
}

/// 跟随系统时，只在 reqwest 自己处理不了的情况下接手：环境变量没有设置 HTTP(S) 代理、
/// 注册表开启了系统代理、且 `ProxyServer` 按协议分开写。其余情况交给 reqwest 原样处理。
fn system_per_protocol_proxy() -> Option<reqwest::Proxy> {
    if first_env(&["HTTP_PROXY", "http_proxy"]).is_some() || first_env(&["HTTPS_PROXY", "https_proxy"]).is_some() {
        return None;
    }
    let registry = windows_internet_settings()?;
    if !registry.enabled {
        return None;
    }
    let server = registry.server.filter(|server| server.contains('='))?;
    let url = parse_proxy_server(&server)?;
    let no_proxy = match first_env(&["NO_PROXY", "no_proxy"]) {
        Some(value) => reqwest::NoProxy::from_string(&value),
        None => registry.bypass.as_deref().map(registry_no_proxy).and_then(|list| reqwest::NoProxy::from_string(&list)),
    };
    reqwest::Proxy::all(url).ok().map(|proxy| proxy.no_proxy(no_proxy))
}

fn first_env(keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
}

/// 注册表 `ProxyOverride`（分号分隔、`*.` 通配）→ reqwest 的 NoProxy 列表，与 reqwest 自身的转换一致。
fn registry_no_proxy(raw: &str) -> String {
    raw.split(';').map(str::trim).filter(|part| !part.is_empty()).collect::<Vec<_>>().join(",").replace("*.", "")
}

/// 系统代理地址（检查更新用）：优先环境变量，其次 Windows 系统代理；都没有返回 None。
/// 更新插件内部的 reqwest 与本项目不是同一个大版本，不会读系统代理，须显式注入。
pub fn system_proxy_url() -> Option<String> {
    for key in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy", "HTTP_PROXY", "http_proxy"] {
        if let Some(url) = std::env::var(key).ok().and_then(|value| parse_proxy_server(&value)) {
            return Some(url);
        }
    }
    let registry = windows_internet_settings()?;
    if !registry.enabled {
        return None;
    }
    parse_proxy_server(registry.server.as_deref()?)
}

/// 把系统代理写法归一为带协议的地址。
///
/// 兼容整串 `host:port` 与按协议分开的 `http=host:port;https=host:port`（取 https，其次 http）。
pub fn parse_proxy_server(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let address = if raw.contains('=') {
        let mut https = None;
        let mut http = None;
        for part in raw.split(';') {
            let Some((scheme, value)) = part.split_once('=') else { continue };
            match scheme.trim().to_ascii_lowercase().as_str() {
                "https" => https = Some(value.trim()),
                "http" => http = Some(value.trim()),
                _ => {}
            }
        }
        https.or(http).filter(|value| !value.is_empty())?.to_string()
    } else {
        raw.to_string()
    };
    let address = if address.contains("://") { address } else { format!("http://{address}") };
    url::Url::parse(&address).ok()?;
    Some(address)
}

/// 手动代理地址：只接受 `http://` / `https://`、带端口、不带账号密码与路径；返回规整后的地址。
pub fn validate_proxy_url(raw: &str) -> Result<String, String> {
    let hint = || PROXY_URL_HINT.to_string();
    let raw = raw.trim();
    let (scheme, rest) = raw.split_once("://").ok_or_else(hint)?;
    let scheme = scheme.to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https") {
        return Err(hint());
    }
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    if authority.is_empty() || authority.contains(['/', '?', '#', '@']) {
        return Err(hint());
    }
    // 端口必须显式写出：Url 会省略与协议默认值相同的端口，因此按原文判断。
    let (host, port) = authority.rsplit_once(':').ok_or_else(hint)?;
    if host.is_empty() || host.ends_with(':') || port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return Err(hint());
    }
    let port: u16 = port.parse().map_err(|_| hint())?;
    if port == 0 {
        return Err(hint());
    }
    let normalized = format!("{scheme}://{authority}");
    let parsed = url::Url::parse(&normalized).map_err(|_| hint())?;
    if parsed.host_str().map_or(true, str::is_empty) {
        return Err(hint());
    }
    Ok(normalized)
}

/* ---------------------------- Windows 系统代理 ---------------------------- */

struct InternetSettings {
    enabled: bool,
    server: Option<String>,
    bypass: Option<String>,
}

/// 读取 Windows「Internet 选项」里的系统代理（Clash / v2ray 等客户端写在这里）。
///
/// 用 `reg query` 而不是引入注册表 crate：只在这一处用到；子进程隐藏窗口，避免 GUI 下闪黑框。
#[cfg(target_os = "windows")]
fn windows_internet_settings() -> Option<InternetSettings> {
    use std::os::windows::process::CommandExt;
    /// 不给子进程分配控制台窗口
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const INTERNET_SETTINGS: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings";

    let output = std::process::Command::new("reg")
        .args(["query", INTERNET_SETTINGS])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let enabled = parse_reg_value(&stdout, "ProxyEnable")
        .is_some_and(|value| value.trim_start_matches("0x").trim_start_matches('0') != "");
    Some(InternetSettings {
        enabled,
        server: parse_reg_value(&stdout, "ProxyServer"),
        bypass: parse_reg_value(&stdout, "ProxyOverride"),
    })
}

#[cfg(not(target_os = "windows"))]
fn windows_internet_settings() -> Option<InternetSettings> {
    None
}

/// 从 `reg query` 输出里取出指定值的数据部分。
///
/// 输出形如：`    ProxyServer    REG_SZ    127.0.0.1:7890`；值本身可能含空格，所以按类型列切分。
/// 名称之后必须是空白，避免 `ProxyServer` 误配到 `ProxyServerXxx`。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_reg_value(stdout: &str, name: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(name)?;
        if !rest.starts_with(char::is_whitespace) {
            return None;
        }
        let (_, data) = rest.trim_start().split_once(char::is_whitespace)?;
        let data = data.trim();
        (!data.is_empty()).then(|| data.to_string())
    })
}

/* -------------------------------- 设置命令 -------------------------------- */

/// 保存联网方式。手动代理必须带合法地址；其他方式不传地址时保留上次的手动代理地址。
#[tauri::command]
pub fn set_network(app: AppHandle, mode: String, proxy_url: Option<String>) -> Result<crate::settings::Settings, String> {
    let next_mode = NetMode::parse(&mode, proxy_url.as_deref())?;
    let proxy_url = match (&next_mode, proxy_url) {
        (NetMode::Manual(url), _) => Some(url.clone()),
        (_, Some(raw)) if !raw.trim().is_empty() => Some(validate_proxy_url(&raw)?),
        _ => None,
    };
    let next = app.state::<crate::Cfg>().0.try_update(|settings| {
        settings.network_mode = mode;
        if proxy_url.is_some() {
            settings.network_proxy_url = proxy_url;
        }
    })?;
    // 先落盘再切换，保存失败时运行中的联网方式保持不变。
    set_mode(next_mode);
    let _ = app.emit("settings-changed", next.clone());
    Ok(next)
}

/* -------------------------------- 连接诊断 -------------------------------- */

const PROXY_TIMEOUT: Duration = Duration::from_secs(3);
const ENDPOINT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize)]
pub struct Probe {
    pub ok: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
    /// 未实际检测（该连接没有可检测的地址，或代理不可达而跳过）；原因在 `error`。
    pub skipped: bool,
}

impl Probe {
    fn reachable(latency: Duration) -> Self {
        Self { ok: true, latency_ms: Some(latency.as_millis() as u64), error: None, skipped: false }
    }
    fn failed(reason: impl Into<String>) -> Self {
        Self { ok: false, latency_ms: None, error: Some(reason.into()), skipped: false }
    }
    fn skipped(reason: impl Into<String>) -> Self {
        Self { ok: false, latency_ms: None, error: Some(reason.into()), skipped: true }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EndpointProbe {
    pub connection_name: String,
    pub platform: String,
    pub probe: Probe,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkDiagnosis {
    /// 只有手动代理才检测代理本身；其余方式为 None。
    pub proxy: Option<Probe>,
    pub endpoints: Vec<EndpointProbe>,
}

/// 用草稿参数检测，不保存设置：手动代理先测代理的 TCP 连通，再对每个已保存连接的
/// 额度或模型地址发一个不带凭证的 GET，只记是否收到响应与延迟，不读取响应内容。
#[tauri::command]
pub async fn diagnose_network(app: AppHandle, mode: String, proxy_url: Option<String>) -> Result<NetworkDiagnosis, String> {
    let mode = NetMode::parse(&mode, proxy_url.as_deref())?;
    let pool = app.try_state::<crate::DbRead>().ok_or("数据尚未就绪，请稍后再试")?.0.clone();
    let connections = crate::with_reader(pool, |conn| crate::connections::list(conn).map_err(|e| e.to_string())).await?;
    let targets: Vec<_> = connections
        .into_iter()
        .map(|c| {
            let url = probe_url(&c.platform, &c.kind, c.base_url.as_deref(), c.protocol.as_deref());
            (c.name, c.platform, url)
        })
        .collect();
    tauri::async_runtime::spawn_blocking(move || diagnose(&mode, targets)).await.map_err(|e| e.to_string())
}

fn diagnose(mode: &NetMode, targets: Vec<(String, String, Option<String>)>) -> NetworkDiagnosis {
    let proxy = match mode {
        NetMode::Manual(url) => Some(probe_proxy(url)),
        _ => None,
    };
    let proxy_down = proxy.as_ref().is_some_and(|probe| !probe.ok);
    let client = builder_for(mode).timeout(ENDPOINT_TIMEOUT).redirect(reqwest::redirect::Policy::none()).build();
    let endpoints = std::thread::scope(|scope| {
        let handles: Vec<_> = targets
            .iter()
            .map(|(_, _, url)| {
                let client = client.as_ref();
                scope.spawn(move || match (url, client) {
                    (None, _) => Probe::skipped("该连接没有可检测的额度或模型地址"),
                    (Some(_), _) if proxy_down => Probe::skipped("代理不可达，未检测"),
                    (Some(_), Err(_)) => Probe::failed("无法创建 HTTP 客户端"),
                    (Some(url), Ok(client)) => probe_endpoint(client, url),
                })
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().unwrap_or_else(|_| Probe::failed("检测线程异常退出"))).collect::<Vec<_>>()
    });
    NetworkDiagnosis {
        proxy,
        endpoints: targets
            .into_iter()
            .zip(endpoints)
            .map(|((connection_name, platform, _), probe)| EndpointProbe { connection_name, platform, probe })
            .collect(),
    }
}

/// 与代理建立 TCP 连接（3 秒超时），不发送任何内容。
fn probe_proxy(proxy_url: &str) -> Probe {
    use std::net::ToSocketAddrs;
    let Ok(url) = url::Url::parse(proxy_url) else { return Probe::failed(PROXY_URL_HINT) };
    let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default()) else {
        return Probe::failed(PROXY_URL_HINT);
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let started = Instant::now();
    let addrs = match (host, port).to_socket_addrs() {
        Ok(addrs) => addrs.collect::<Vec<_>>(),
        Err(_) => return Probe::failed("无法解析代理主机名"),
    };
    let mut last = "无法解析代理主机名".to_string();
    for addr in addrs {
        let remaining = PROXY_TIMEOUT.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            last = "连接超时（3 秒）".into();
            break;
        }
        match std::net::TcpStream::connect_timeout(&addr, remaining) {
            Ok(_) => return Probe::reachable(started.elapsed()),
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => last = "连接超时（3 秒）".into(),
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => last = "端口没有服务监听".into(),
            Err(_) => last = "无法建立连接".into(),
        }
    }
    Probe::failed(last)
}

/// 只发 GET、不带凭证、不读响应体；收到任何 HTTP 响应即视为可达。
fn probe_endpoint(client: &reqwest::blocking::Client, url: &str) -> Probe {
    let started = Instant::now();
    match client.get(url).send() {
        Ok(response) => {
            let latency = started.elapsed();
            drop(response);
            Probe::reachable(latency)
        }
        // 不回显 reqwest 的原始错误：其中带请求地址，统一换成固定文案。
        Err(error) if error.is_timeout() => Probe::failed("连接超时（5 秒）"),
        Err(error) if error.is_connect() => Probe::failed("无法建立连接"),
        Err(_) => Probe::failed("请求失败"),
    }
}

/// 每个连接用来检测连通性的地址：自定义网关用其模型列表地址（按连接协议拼接），官方连接用额度或模型接口。
/// 地址里的账号密码先清掉：诊断不带任何凭证。
fn probe_url(platform: &str, kind: &str, base_url: Option<&str>, protocol: Option<&str>) -> Option<String> {
    if let Some(base) = base_url.map(str::trim).filter(|base| kind == "api" && !base.is_empty()) {
        let endpoint = crate::connections::models_endpoint(base, crate::connections::auth_protocol(platform, protocol), None).ok()?;
        let mut url = url::Url::parse(&endpoint).ok()?;
        url.set_username("").ok()?;
        url.set_password(None).ok()?;
        return Some(url.to_string());
    }
    let url = match (platform, kind) {
        ("claude", "auth") => "https://api.anthropic.com/api/oauth/usage",
        ("codex", "auth") => "https://chatgpt.com/backend-api/wham/usage",
        ("claude", "api") => "https://api.anthropic.com/v1/models",
        ("codex", "api") => "https://api.openai.com/v1/models",
        ("grok", "api") => "https://api.x.ai/v1/api-key",
        ("gemini", "api") => "https://generativelanguage.googleapis.com/v1beta/openai/models",
        ("grok", "auth") => "https://grok.com/grok_api_v2.GrokBuildBilling/GetGrokCreditsConfig",
        _ => return None,
    };
    Some(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_proxy_server_handles_per_protocol() {
        assert_eq!(parse_proxy_server("http=a:1;https=b:2").as_deref(), Some("http://b:2"));
        assert_eq!(parse_proxy_server("c:3").as_deref(), Some("http://c:3"));
        assert_eq!(parse_proxy_server(""), None);
    }

    #[test]
    fn parse_proxy_server_keeps_scheme_and_falls_back_to_http_entry() {
        assert_eq!(parse_proxy_server("http://127.0.0.1:7890").as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(parse_proxy_server("ftp=10.0.0.3:1082;http=10.0.0.1:1080").as_deref(), Some("http://10.0.0.1:1080"));
        assert_eq!(parse_proxy_server("   "), None);
        // 只有无法识别的协议段时不猜测
        assert_eq!(parse_proxy_server("ftp=10.0.0.3:1082"), None);
    }

    #[test]
    fn validate_proxy_url_rules() {
        assert_eq!(validate_proxy_url("http://127.0.0.1:7890").as_deref(), Ok("http://127.0.0.1:7890"));
        for raw in ["socks5://x:1", "127.0.0.1", "http://host"] {
            assert_eq!(validate_proxy_url(raw), Err(PROXY_URL_HINT.to_string()), "{raw} 应被拒绝");
        }
    }

    #[test]
    fn validate_proxy_url_normalizes_and_rejects_extras() {
        assert_eq!(validate_proxy_url("  HTTPS://proxy.test:443/ ").as_deref(), Ok("https://proxy.test:443"));
        assert_eq!(validate_proxy_url("http://[::1]:7890").as_deref(), Ok("http://[::1]:7890"));
        for raw in ["", "http://", "http://:7890", "http://host:", "http://host:0", "http://host:70000", "http://host:80/path",
            "http://user:pass@host:8080", "http://host:8080?x=1", "http://[::1]"] {
            assert!(validate_proxy_url(raw).is_err(), "{raw} 应被拒绝");
        }
    }

    #[test]
    fn client_rebuilds_after_network_change() {
        let before = generation();
        set_mode(NetMode::System);
        assert_eq!(generation(), before + 1);
        let now = std::time::Instant::now();
        assert!(crate::quota::cache_valid(now, before, now, before));
        assert!(!crate::quota::cache_valid(now, before, now, generation()), "代数变化后必须重建客户端");
    }

    #[test]
    fn mode_parse_requires_valid_manual_url() {
        assert_eq!(NetMode::parse("system", None), Ok(NetMode::System));
        assert_eq!(NetMode::parse("direct", Some("garbage")), Ok(NetMode::Direct));
        assert_eq!(NetMode::parse("manual", Some("http://127.0.0.1:7890")), Ok(NetMode::Manual("http://127.0.0.1:7890".into())));
        assert_eq!(NetMode::parse("manual", None), Err(PROXY_URL_HINT.to_string()));
        assert!(NetMode::parse("pac", None).is_err());
    }

    #[test]
    fn old_settings_default_to_follow_system() {
        let settings: crate::settings::Settings = serde_json::from_str(r#"{"island_platform":"claude","dnd":false,"island_visible":true}"#).unwrap();
        assert_eq!((settings.network_mode.as_str(), settings.network_proxy_url.as_deref()), ("system", None));
        assert_eq!(crate::settings::Settings::default().network_mode, "system");
    }

    #[test]
    fn registry_bypass_list_matches_reqwest_conversion() {
        assert_eq!(registry_no_proxy("<local>;*.cn; 10.0.0.1;"), "<local>,cn,10.0.0.1");
    }

    #[test]
    fn reg_value_reads_data_column() {
        let stdout = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings\r\n    ProxyServer    REG_SZ    127.0.0.1:7890\r\n\r\n";
        assert_eq!(parse_reg_value(stdout, "ProxyServer").as_deref(), Some("127.0.0.1:7890"));
    }

    #[test]
    fn reg_value_reads_dword_and_missing_name() {
        let stdout = "    ProxySettingsPerUser    REG_DWORD    0x1\r\n    ProxyEnable    REG_DWORD    0x1\r\n";
        assert_eq!(parse_reg_value(stdout, "ProxyEnable").as_deref(), Some("0x1"));
        assert!(parse_reg_value(stdout, "ProxyServer").is_none());
        assert!(parse_reg_value("    ProxyServerBackup    REG_SZ    a:1\r\n", "ProxyServer").is_none(), "名称必须完整匹配");
    }

    #[test]
    fn reg_value_keeps_spaces_inside_data() {
        let stdout = "    ProxyOverride    REG_SZ    <local>;*.cn; 10.0.0.1\r\n";
        assert_eq!(parse_reg_value(stdout, "ProxyOverride").as_deref(), Some("<local>;*.cn; 10.0.0.1"));
    }

    #[test]
    fn probe_url_uses_connection_own_address() {
        assert_eq!(probe_url("claude", "api", Some("https://gw.test"), None).as_deref(), Some("https://gw.test/v1/models"));
        assert_eq!(probe_url("codex", "api", Some("https://gw.test/v1/"), None).as_deref(), Some("https://gw.test/v1/models"));
        assert_eq!(probe_url("claude", "auth", None, None).as_deref(), Some("https://api.anthropic.com/api/oauth/usage"));
        assert_eq!(probe_url("codex", "api", Some("  "), None).as_deref(), Some("https://api.openai.com/v1/models"));
        assert_eq!(probe_url("qoder", "auth", None, None), None);
        assert_eq!(probe_url("claude", "api", Some("ftp://gw.test"), None), None);
    }

    #[test]
    fn probe_url_follows_custom_protocol_and_drops_credentials() {
        // Anthropic 兼容只认 /v1 结尾；OpenAI 兼容把 /v<数字> 结尾视为已带版本
        assert_eq!(probe_url("custom", "api", Some("https://gw.test/api/v4"), Some("anthropic")).as_deref(), Some("https://gw.test/api/v4/v1/models"));
        assert_eq!(probe_url("custom", "api", Some("https://gw.test/api/v4"), Some("openai")).as_deref(), Some("https://gw.test/api/v4/models"));
        assert_eq!(probe_url("custom", "api", Some("https://user:pass@gw.test/v1"), Some("anthropic")).as_deref(), Some("https://gw.test/v1/models"));
    }

    /// 本机起一个只回状态行的 HTTP 服务：可达、延迟有值、请求里不带凭证。
    #[test]
    fn diagnosis_reports_reachable_and_unreachable_without_credentials() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 1024];
            let size = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..size]).to_string();
            stream.write_all(b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 5\r\nconnection: close\r\n\r\nnope!").unwrap();
            request
        });
        let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead_port = dead.local_addr().unwrap().port();
        drop(dead);
        let result = diagnose(&NetMode::Direct, vec![
            ("网关".into(), "claude".into(), Some(format!("http://127.0.0.1:{port}/v1/models"))),
            ("失效网关".into(), "codex".into(), Some(format!("http://127.0.0.1:{dead_port}/v1/models"))),
            ("本机账号".into(), "qoder".into(), None),
        ]);
        let request = server.join().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /v1/models "), "只发 GET：{request}");
        assert!(!request.contains("authorization") && !request.contains("x-api-key"), "诊断不带凭证");
        assert!(result.proxy.is_none());
        assert!(result.endpoints[0].probe.ok && result.endpoints[0].probe.latency_ms.is_some());
        assert!(!result.endpoints[1].probe.ok && !result.endpoints[1].probe.skipped);
        assert_eq!(result.endpoints[1].probe.error.as_deref(), Some("无法建立连接"));
        assert!(result.endpoints[2].probe.skipped && result.endpoints[2].probe.latency_ms.is_none());
    }

    #[test]
    fn unreachable_manual_proxy_skips_endpoints() {
        let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = dead.local_addr().unwrap().port();
        drop(dead);
        let result = diagnose(&NetMode::Manual(format!("http://127.0.0.1:{port}")), vec![
            ("网关".into(), "claude".into(), Some("https://gw.test/v1/models".into())),
        ]);
        let proxy = result.proxy.expect("手动代理要检测代理本身");
        assert!(!proxy.ok);
        assert_eq!(proxy.error.as_deref(), Some("端口没有服务监听"));
        assert!(result.endpoints[0].probe.skipped);
        assert_eq!(result.endpoints[0].probe.error.as_deref(), Some("代理不可达，未检测"));
    }

    /// 手动代理：外部地址经代理转发（代理收到绝对地址形式的请求行），本机回环地址不走代理。
    #[test]
    fn manual_proxy_routes_external_requests_through_proxy() {
        use std::io::{Read, Write};
        let proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_port = proxy.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut lines = Vec::new();
            // 第一次连接是代理可达检测（不发内容），第二次是转发的 GET
            for _ in 0..2 {
                let (mut stream, _) = proxy.accept().unwrap();
                let mut buffer = [0u8; 1024];
                let size = stream.read(&mut buffer).unwrap_or(0);
                if size == 0 { continue; }
                lines.push(String::from_utf8_lossy(&buffer[..size]).lines().next().unwrap_or_default().to_string());
                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
            }
            lines
        });
        let result = diagnose(&NetMode::Manual(format!("http://127.0.0.1:{proxy_port}")), vec![
            ("外部网关".into(), "claude".into(), Some("http://jolu-proxy-check.invalid/v1/models".into())),
        ]);
        let lines = server.join().unwrap();
        assert!(result.proxy.as_ref().is_some_and(|probe| probe.ok && probe.latency_ms.is_some()));
        assert!(result.endpoints[0].probe.ok, "经代理收到响应即可达：{:?}", result.endpoints[0].probe);
        assert_eq!(lines, vec!["GET http://jolu-proxy-check.invalid/v1/models HTTP/1.1".to_string()]);
    }
}
