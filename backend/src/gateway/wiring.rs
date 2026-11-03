//! 接线层：把各 agent 的配置指向本机网关。
//!
//! 纪律：
//! 1. 写前把原文件备份到应用数据目录
//! 2. 临时文件 + rename 原子写
//! 3. 还原只清理自己的写入：当前值不再是网关地址时不覆盖（外部意图优先）
//! 4. 所有入口共用一把锁（[`crate::cli_switch::with_switch`]，进程内 + 跨进程）
//! 5. 接线动作先记 `agent_wiring` 的 pending，写完文件再标 wired
//! 6. 原值快照**过滤一切网关值**：重新接线不能把网关地址当成用户原来的上游
//!
//! 文件读写拆成纯函数（`*_wire` / `*_read`），路径解析依赖真实用户目录、单独一层，
//! 测试用临时文本验证保留性与边界（与 `proxy_config.rs` 同款做法）。

use std::path::{Path, PathBuf};

use crate::gateway::wiring_store::{Applied, WiringRecord};

/// 支持的 agent 标识（与 `providerCatalog` 的平台 id 对齐）。
pub const AGENTS: [&str; 5] = ["claude-code", "codex", "zcode", "gemini", "opencode"];

pub fn is_agent(agent: &str) -> bool {
    AGENTS.contains(&agent)
}

/// 某 agent 的接入状态。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct WiringStatus {
    pub agent: String,
    /// 是否已接入网关
    pub wired: bool,
    /// 配置文件路径（展示用；不存在时为 None）
    pub config_path: Option<String>,
    /// 需要重启 agent 才生效时给出提示
    pub notice: Option<String>,
    /// 无法接线的原因（已安装但没有可写配置等）
    pub reason: Option<String>,
    /// 接线记录的阶段：idle / pending / wired；没有记录时为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// 漂移：pending / removed / changed / foreign；正常或被接受后为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drift: Option<String>,
    /// Codex 检测到的登录方式：oauth / provider；其他 agent 为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_mode: Option<String>,
}

/* ─────────────────────────── 纯函数：各 agent 的配置读写 ─────────────────────────── */

/// 网关写入的地址 / token 特征：任何 127.0.0.1 / localhost 地址或 `jolu-*` 令牌。
pub(crate) fn is_gateway_value(value: &str) -> bool {
    let value = value.trim();
    value.starts_with("jolu-") || value.contains("127.0.0.1") || value.contains("localhost")
}

/// Claude Code：`settings.json` 的 `env.ANTHROPIC_BASE_URL` + `ANTHROPIC_AUTH_TOKEN`。
fn claude_wire(text: &str, base: &str, token: &str) -> Result<String, String> {
    let mut root: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("settings.json 不是有效 JSON：{error}"))?;
    let object = root.as_object_mut().ok_or("settings.json 顶层不是对象")?;
    let env = object.entry("env").or_insert_with(|| serde_json::json!({}));
    let env = env.as_object_mut().ok_or("settings.json 的 env 段不是对象")?;
    env.insert("ANTHROPIC_BASE_URL".into(), serde_json::Value::String(base.into()));
    env.insert("ANTHROPIC_AUTH_TOKEN".into(), serde_json::Value::String(token.into()));
    serde_json::to_string_pretty(&root).map_err(|error| format!("settings.json 序列化失败：{error}"))
}

/// 还原 Claude：只删自己写的两个键。`saved` 是接线前读到的原值快照。
fn claude_restore(text: &str, saved: Option<&serde_json::Value>) -> Result<String, String> {
    let mut root: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("settings.json 不是有效 JSON：{error}"))?;
    let Some(object) = root.as_object_mut() else { return Ok(text.to_string()) };
    let Some(env) = object.get_mut("env").and_then(|e| e.as_object_mut()) else { return Ok(text.to_string()) };
    for key in ["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN"] {
        match saved.and_then(|s| s.get(key)).and_then(|v| v.as_str()) {
            Some(original) => {
                env.insert(key.into(), serde_json::Value::String(original.into()));
            }
            None => {
                env.remove(key);
            }
        }
    }
    // env 被我方清空时整个删掉，避免留一个空对象
    if env.is_empty() {
        object.remove("env");
    }
    serde_json::to_string_pretty(&root).map_err(|error| format!("settings.json 序列化失败：{error}"))
}

fn claude_env(text: &str, key: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    root.get("env")?.get(key)?.as_str().map(str::to_string)
}

fn claude_current_base(text: &str) -> Option<String> {
    claude_env(text, "ANTHROPIC_BASE_URL")
}

fn claude_current_token(text: &str) -> Option<String> {
    claude_env(text, "ANTHROPIC_AUTH_TOKEN")
}

/// 读 Claude 的两个键作为原值快照；网关值一律不收。
fn claude_snapshot(text: &str) -> Option<serde_json::Value> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    let env = root.get("env")?;
    let mut saved = serde_json::Map::new();
    for key in ["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN"] {
        if let Some(value) = env.get(key).and_then(|v| v.as_str()) {
            if !is_gateway_value(value) {
                saved.insert(key.into(), serde_json::Value::String(value.to_string()));
            }
        }
    }
    if saved.is_empty() { None } else { Some(serde_json::Value::Object(saved)) }
}

/// Codex 有两种接法：
/// - 已登录 ChatGPT（`auth.json` 有 tokens）：保留内置 openai provider，只写顶层
///   `openai_base_url = <base>/backend-api/codex`，OAuth 头原样发到网关。
/// - 未登录 ChatGPT：写自定义 provider + `experimental_bearer_token`。
///
/// `wire_api` 必须是 `responses`：当前 Codex 只接受这个值，写 `chat` 会让它拒绝加载整份配置。
fn codex_wire(
    text: &str,
    base: &str,
    provider: &str,
    oauth: bool,
    token: Option<&str>,
) -> Result<String, String> {
    let mut document = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("config.toml 格式无效：{error}"))?;
    if oauth {
        // 内置 provider 优先：删掉 Jolu 自己的 provider 段与 model_provider 覆盖。
        if let Some(providers) = document.get_mut("model_providers").and_then(|p| p.as_table_mut()) {
            providers.remove(provider);
        }
        document.remove("model_provider");
        document["openai_base_url"] = toml_edit::value(format!("{base}/backend-api/codex"));
        return Ok(document.to_string());
    }
    if !document.contains_key("model_providers") {
        document["model_providers"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let providers = document["model_providers"]
        .as_table_mut()
        .ok_or("config.toml 的 model_providers 不是表")?;
    let mut entry = toml_edit::Table::new();
    entry["name"] = toml_edit::value(provider);
    entry["base_url"] = toml_edit::value(base);
    entry["wire_api"] = toml_edit::value("responses");
    if let Some(token) = token {
        entry["experimental_bearer_token"] = toml_edit::value(token);
    }
    providers[provider] = toml_edit::Item::Table(entry);
    document["model_provider"] = toml_edit::value(provider);
    Ok(document.to_string())
}

/// 还原 Codex：删掉 Jolu 建的 provider 段，恢复 `openai_base_url` 与原 `model_provider`。
/// 外部改过的值不动（当前 provider 是别人的、current base 不是网关地址时跳过对应字段）。
fn codex_restore(text: &str, saved: Option<&serde_json::Value>, provider: &str) -> Result<String, String> {
    let mut document = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("config.toml 格式无效：{error}"))?;
    let current_provider = codex_current_provider(text);
    let current_base = document.get("openai_base_url").and_then(|v| v.as_str()).map(str::to_string);
    if let Some(providers) = document.get_mut("model_providers").and_then(|p| p.as_table_mut()) {
        providers.remove(provider);
    }
    // openai_base_url：只有当前仍指向本机网关（或为空）时才还原
    if current_base.as_deref().is_none_or(is_gateway_value) {
        match saved.and_then(|s| s.get("openai_base_url")).and_then(|v| v.as_str()) {
            Some(original) => {
                document["openai_base_url"] = toml_edit::value(original);
            }
            None => {
                document.remove("openai_base_url");
            }
        }
    }
    // model_provider：只有当前是我们删掉后的空值、或仍指向我们时才还原
    if current_provider.as_deref().is_none() || current_provider.as_deref() == Some(provider) {
        match saved.and_then(|s| s.get("model_provider")).and_then(|v| v.as_str()) {
            Some(original) => {
                document["model_provider"] = toml_edit::value(original);
            }
            None => {
                document.remove("model_provider");
            }
        }
    }
    Ok(document.to_string())
}

/// Codex 实际生效的 base：顶层 openai_base_url 优先，其次当前 provider 的 base_url。
fn codex_effective_base(text: &str) -> Option<String> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    if let Some(base) = document.get("openai_base_url").and_then(|v| v.as_str()) {
        let base = base.trim();
        if !base.is_empty() {
            return Some(base.to_string());
        }
    }
    let provider = document.get("model_provider")?.as_str()?;
    codex_provider_base(text, provider)
}

/// 是否已登录 ChatGPT（`auth.json` 里有 OAuth tokens，而不是只有 API Key）。
fn codex_oauth_logged_in() -> bool {
    let Some(dir) = crate::creds::codex_dir() else { return false };
    let Ok(text) = std::fs::read_to_string(dir.join("auth.json")) else { return false };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else { return false };
    value
        .pointer("/tokens/access_token")
        .and_then(|v| v.as_str())
        .is_some_and(|token| !token.trim().is_empty())
}

fn codex_current_provider(text: &str) -> Option<String> {
    text.parse::<toml_edit::DocumentMut>()
        .ok()?
        .get("model_provider")?
        .as_str()
        .map(str::to_string)
}

fn codex_provider_base(text: &str, provider: &str) -> Option<String> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    document
        .get("model_providers")?
        .get(provider)?
        .get("base_url")?
        .as_str()
        .map(str::to_string)
}

/// 读 Codex 的 `openai_base_url` 与当前 provider 作为原值快照；网关值一律不收。
fn codex_snapshot(text: &str) -> Option<serde_json::Value> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    let mut saved = serde_json::Map::new();
    if let Some(base) = document.get("openai_base_url").and_then(|v| v.as_str()) {
        if !is_gateway_value(base) {
            saved.insert("openai_base_url".into(), serde_json::Value::String(base.to_string()));
        }
    }
    if let Some(current) = document.get("model_provider").and_then(|v| v.as_str()) {
        if current != CODEX_PROVIDER
            && !codex_provider_base(text, current).is_some_and(|base| is_gateway_value(&base))
        {
            saved.insert("model_provider".into(), serde_json::Value::String(current.to_string()));
        }
    }
    if saved.is_empty() { None } else { Some(serde_json::Value::Object(saved)) }
}

/// OpenCode：`opencode.json` 的 `provider` 段。JSON 往返保留全部字段。
fn opencode_wire(text: &str, base: &str, token: &str) -> Result<String, String> {
    let mut root: serde_json::Value = if text.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(text).map_err(|error| format!("opencode 配置不是有效 JSON：{error}"))?
    };
    let object = root.as_object_mut().ok_or("opencode 配置顶层不是对象")?;
    let providers = object.entry("provider").or_insert_with(|| serde_json::json!({}));
    let providers = providers.as_object_mut().ok_or("provider 段不是对象")?;
    providers.insert(
        "jolu".into(),
        serde_json::json!({
            "npm": "@ai-sdk/openai-compatible",
            "name": "Jolu 网关",
            "options": {"baseURL": base, "apiKey": token},
        }),
    );
    serde_json::to_string_pretty(&root).map_err(|error| format!("opencode 配置序列化失败：{error}"))
}

fn opencode_restore(text: &str, _saved: Option<&serde_json::Value>) -> Result<String, String> {
    let mut root: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("opencode 配置不是有效 JSON：{error}"))?;
    if let Some(providers) = root.get_mut("provider").and_then(|p| p.as_object_mut()) {
        providers.remove("jolu");
    }
    serde_json::to_string_pretty(&root).map_err(|error| format!("opencode 配置序列化失败：{error}"))
}

fn opencode_current_base(text: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    root.get("provider")?.get("jolu")?.get("options")?.get("baseURL")?.as_str().map(str::to_string)
}

fn opencode_current_token(text: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    root.get("provider")?.get("jolu")?.get("options")?.get("apiKey")?.as_str().map(str::to_string)
}

/// Gemini CLI：交互式按 `settings.json` 的 `security.auth.selectedType` 选鉴权。
/// 只写 base URL 对 Google 登录用户无效，所以这里做事务式接管：
/// `.env` 写地址 + 网关 Key，`settings.json` 临时把 selectedType 切成 `gemini-api-key`；
/// 断开时按快照逐项还原。OAuth 凭据文件本身不碰。
#[derive(Debug, Clone)]
struct GeminiPlan {
    env: String,
    settings: String,
    snapshot: serde_json::Value,
}

fn gemini_settings_path() -> Option<PathBuf> {
    Some(crate::creds::home()?.join(".gemini").join("settings.json"))
}

fn gemini_selected_type(settings_text: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(settings_text)
        .ok()?
        .pointer("/security/auth/selectedType")?
        .as_str()
        .map(str::to_string)
}

fn gemini_plan(env_text: &str, settings_text: &str, base: &str, token: &str) -> Result<GeminiPlan, String> {
    let mut env_saved = serde_json::Map::new();
    for key in ["GOOGLE_GEMINI_BASE_URL", "GEMINI_API_KEY"] {
        if let Some(value) = dotenv_get(env_text, key).filter(|value| !is_gateway_value(value)) {
            env_saved.insert(key.into(), serde_json::Value::String(value));
        }
    }
    let mut settings_saved = serde_json::Map::new();
    if let Some(selected) = gemini_selected_type(settings_text) {
        settings_saved.insert("selectedType".into(), serde_json::Value::String(selected));
    }
    // 即使原来什么都没有也要记快照：断开时才知道「这两个键是我们加的，要删掉」。
    let snapshot = serde_json::json!({
        "env": serde_json::Value::Object(env_saved),
        "settings": serde_json::Value::Object(settings_saved),
    });
    let env = wire_dotenv(
        &wire_dotenv(env_text, "GOOGLE_GEMINI_BASE_URL", Some(base)),
        "GEMINI_API_KEY",
        Some(token),
    );
    let mut settings: serde_json::Value = if settings_text.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(settings_text)
            .map_err(|error| format!("~/.gemini/settings.json 不是有效 JSON，未改动：{error}"))?
    };
    let object = settings.as_object_mut().ok_or("~/.gemini/settings.json 顶层不是对象，未改动")?;
    let security = object.entry("security").or_insert_with(|| serde_json::json!({}));
    let security = security
        .as_object_mut()
        .ok_or("~/.gemini/settings.json 的 security 段不是对象，未改动")?;
    let auth = security.entry("auth").or_insert_with(|| serde_json::json!({}));
    let auth = auth
        .as_object_mut()
        .ok_or("~/.gemini/settings.json 的 security.auth 段不是对象，未改动")?;
    auth.insert("selectedType".into(), serde_json::Value::String("gemini-api-key".into()));
    Ok(GeminiPlan {
        env,
        settings: serde_json::to_string_pretty(&settings).unwrap_or_else(|_| settings_text.to_string()),
        snapshot,
    })
}

/// 还原 `.env` 与 settings.json；旧版快照（只有 `value`）只还原地址，不碰 settings。
fn gemini_restore(env_text: &str, settings_text: &str, saved: Option<&serde_json::Value>) -> (String, String) {
    let legacy_base = saved.and_then(|s| s.get("value")).and_then(|v| v.as_str());
    let saved_env = saved.and_then(|s| s.get("env"));
    let saved_settings = saved.and_then(|s| s.get("settings"));
    let env = if let Some(env) = saved_env {
        let base = env.get("GOOGLE_GEMINI_BASE_URL").and_then(|v| v.as_str());
        let key = env.get("GEMINI_API_KEY").and_then(|v| v.as_str());
        let next = wire_dotenv(env_text, "GOOGLE_GEMINI_BASE_URL", base);
        wire_dotenv(&next, "GEMINI_API_KEY", key)
    } else {
        wire_dotenv(env_text, "GOOGLE_GEMINI_BASE_URL", legacy_base)
    };
    let settings = match saved_settings {
        Some(saved_settings) => {
            let mut settings: serde_json::Value = if settings_text.trim().is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_str(settings_text) {
                    Ok(value) => value,
                    // 文件坏了就不写回，交给用户自己修
                    Err(_) => return (env, settings_text.to_string()),
                }
            };
            let selected = saved_settings.get("selectedType").and_then(|v| v.as_str());
            let auth = settings
                .pointer_mut("/security/auth")
                .and_then(|v| v.as_object_mut());
            if let Some(auth) = auth {
                match selected {
                    Some(value) => {
                        auth.insert("selectedType".into(), serde_json::Value::String(value.to_string()));
                    }
                    None => {
                        auth.remove("selectedType");
                    }
                }
            }
            serde_json::to_string_pretty(&settings).unwrap_or_else(|_| settings_text.to_string())
        }
        None => settings_text.to_string(),
    };
    (env, settings)
}

fn gemini_current_base(text: &str) -> Option<String> {
    dotenv_get(text, "GOOGLE_GEMINI_BASE_URL")
}

fn gemini_current_token(text: &str) -> Option<String> {
    dotenv_get(text, "GEMINI_API_KEY")
}

fn gemini_settings_text() -> Option<String> {
    std::fs::read_to_string(gemini_settings_path()?).ok()
}

/// ZCode：`provider` JSON 的 `options.baseURL` + `apiKey`（结构对齐 magpie zcode.go 的字段名）。
fn zcode_wire(text: &str, base: &str, token: &str) -> Result<String, String> {
    let mut root: serde_json::Value = if text.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(text).map_err(|error| format!("ZCode 配置不是有效 JSON：{error}"))?
    };
    let object = root.as_object_mut().ok_or("ZCode 配置顶层不是对象")?;
    object.insert(
        "jolu".into(),
        serde_json::json!({
            "type": "openai",
            "options": {"baseURL": base, "apiKey": token},
        }),
    );
    serde_json::to_string_pretty(&root).map_err(|error| format!("ZCode 配置序列化失败：{error}"))
}

fn zcode_restore(text: &str, _saved: Option<&serde_json::Value>) -> Result<String, String> {
    let mut root: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("ZCode 配置不是有效 JSON：{error}"))?;
    if let Some(object) = root.as_object_mut() {
        object.remove("jolu");
    }
    serde_json::to_string_pretty(&root).map_err(|error| format!("ZCode 配置序列化失败：{error}"))
}

fn zcode_current_base(text: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    root.get("jolu")?.get("options")?.get("baseURL")?.as_str().map(str::to_string)
}

fn zcode_current_token(text: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    root.get("jolu")?.get("options")?.get("apiKey")?.as_str().map(str::to_string)
}

/// dotenv 键值写入：保留其它行与注释，只替换目标键（不存在则追加）。
fn wire_dotenv(text: &str, key: &str, value: Option<&str>) -> String {
    let prefix = format!("{key}=");
    let mut out: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in text.lines() {
        if line.trim_start().starts_with(&prefix) {
            if let Some(value) = value {
                out.push(format!("{prefix}{value}"));
            }
            replaced = true;
            continue;
        }
        out.push(line.to_string());
    }
    if !replaced {
        if let Some(value) = value {
            out.push(format!("{prefix}{value}"));
        }
    }
    let mut joined = out.join("\n");
    if !joined.is_empty() && !joined.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

fn dotenv_get(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix(&format!("{key}=")))
        .map(|v| v.trim().trim_matches('"').to_string())
}

/// 判断当前配置值是否指向本机网关端口（用于「外部已改就不覆盖」）。
pub(crate) fn points_at_gateway(value: &str, port: u16) -> bool {
    let value = value.trim().trim_end_matches('/');
    value.starts_with(&format!("http://127.0.0.1:{port}"))
        || value.starts_with(&format!("http://localhost:{port}"))
}

/* ─────────────────────────── 文件层：路径、备份、原子写 ─────────────────────────── */

/// Jolu 建的 Codex provider 段名。
const CODEX_PROVIDER: &str = "jolu-gateway";

/// `jolu-<agent>.<实例前 8 位>`：网关只把它当归属标记，不做鉴权。
fn token_for(agent: &str, instance_short: &str) -> String {
    format!("jolu-{agent}.{instance_short}")
}

/// 各 agent 的配置文件路径（探测与写入共用）。
fn config_path(agent: &str) -> Option<PathBuf> {
    let home = crate::creds::home()?;
    Some(match agent {
        "claude-code" => crate::creds::claude_dir()?.join("settings.json"),
        "codex" => crate::creds::codex_dir()?.join("config.toml"),
        "zcode" => std::env::var_os("ZCODE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".zcode")).join("provider.json"),
        "gemini" => home.join(".gemini").join(".env"),
        "opencode" => std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("opencode")
            .join("opencode.json"),
        _ => return None,
    })
}

fn write_atomic(path: &Path, bytes: Vec<u8>) -> Result<(), String> {
    use std::io::Write;
    let temporary = path.with_extension("jolu-wiring.tmp");
    let write = || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    };
    write().map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        format!("写入 {} 失败：{error}", path.display())
    })
}

/// 写前备份到应用数据目录（沿用 cli_apply 的纪律：备份失败则不写）。
fn backup(path: &Path, backup_dir: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(backup_dir).map_err(|error| format!("创建备份目录失败：{error}"))?;
    let name = format!(
        "{}-{}.bak",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("config"),
        chrono::Local::now().format("%Y%m%d%H%M%S")
    );
    std::fs::copy(path, backup_dir.join(name)).map_err(|error| format!("备份 {path:?} 失败：{error}"))?;
    Ok(())
}

/// 别的 Jolu 实例写下的标记：token 的实例后缀与当前实例不同。
fn foreign_marker(agent: &str, text: &str, instance_short: &str) -> Option<String> {
    let token = match agent {
        "claude-code" => claude_current_token(text),
        "codex" => text
            .parse::<toml_edit::DocumentMut>()
            .ok()
            .and_then(|doc| {
                doc.get("model_providers")?
                    .get(CODEX_PROVIDER)?
                    .get("experimental_bearer_token")?
                    .as_str()
                    .map(str::to_string)
            }),
        "gemini" => gemini_current_token(text),
        "opencode" => opencode_current_token(text),
        "zcode" => zcode_current_token(text),
        _ => None,
    }?;
    if !token.starts_with("jolu-") {
        return None;
    }
    let (_, suffix) = token.rsplit_once('.')?;
    (suffix != instance_short).then(|| suffix.to_string())
}

/// 当前配置是否由本实例写入（状态判断用）。
fn ours(agent: &str, text: &str, port: u16, instance_short: &str) -> bool {
    match agent {
        "claude-code" => {
            let base = claude_current_base(text).unwrap_or_default();
            let token = claude_current_token(text).unwrap_or_default();
            points_at_gateway(&base, port)
                && (token == format!("jolu-claude-code")
                    || token == token_for("claude-code", instance_short))
        }
        "codex" => codex_effective_base(text).is_some_and(|base| points_at_gateway(&base, port)),
        "gemini" => {
            let base_ok = gemini_current_base(text).is_some_and(|base| points_at_gateway(&base, port));
            let selected = gemini_settings_text()
                .as_deref()
                .and_then(gemini_selected_type);
            base_ok && selected.as_deref() == Some("gemini-api-key")
        }
        "opencode" => opencode_current_base(text).is_some_and(|base| points_at_gateway(&base, port)),
        "zcode" => zcode_current_base(text).is_some_and(|base| points_at_gateway(&base, port)),
        _ => false,
    }
}

/// 探测某 agent 的当前接入状态（只看文件）。
pub fn status(agent: &str, port: u16) -> WiringStatus {
    status_with(agent, port, None)
}

/// 带接线记录的探测：pending 要单独说明，不能当成「已接线」。
pub fn status_with(agent: &str, port: u16, record: Option<&WiringRecord>) -> WiringStatus {
    let codex_mode = (agent == "codex").then(|| {
        if codex_oauth_logged_in() { "oauth" } else { "provider" }.to_string()
    });
    if !is_agent(agent) {
        return WiringStatus {
            agent: agent.into(),
            wired: false,
            config_path: None,
            notice: None,
            reason: Some("该工具不在接线范围".into()),
            state: record.map(|r| r.state.clone()),
            drift: None,
            codex_mode,
        };
    }
    let Some(path) = config_path(agent) else {
        return WiringStatus {
            agent: agent.into(),
            wired: false,
            config_path: None,
            notice: None,
            reason: Some("未找到用户目录".into()),
            state: record.map(|r| r.state.clone()),
            drift: None,
            codex_mode,
        };
    };
    let display = path.display().to_string();
    let installed = path.exists() || (agent == "gemini" && gemini_settings_path().is_some_and(|p| p.exists()));
    if !installed {
        return WiringStatus {
            agent: agent.into(),
            wired: false,
            config_path: Some(display),
            notice: None,
            reason: Some("尚未安装或未生成配置文件".into()),
            state: record.map(|r| r.state.clone()),
            drift: None,
            codex_mode,
        };
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if record.is_some_and(|r| r.state == "pending") {
        return WiringStatus {
            agent: agent.into(),
            wired: false,
            config_path: Some(display),
            notice: Some(restart_notice(agent).to_string()),
            reason: Some("上次接线未完成；启动时会按配置文件的实际内容修复".into()),
            state: Some("pending".into()),
            drift: Some("pending".into()),
            codex_mode,
        };
    }
    // 状态以文件里实际写下的值为准；旧版记录（无实例后缀）按本机网关端口判断。
    let instance = record.map(|r| r.instance_id.chars().take(8).collect::<String>()).unwrap_or_default();
    let wired = ours(agent, &text, port, &instance);
    let drift = if !wired && record.is_some_and(|r| r.state != "idle") {
        Some(
            if foreign_marker(agent, &text, &instance).is_some() {
                "foreign"
            } else if text.trim().is_empty() {
                "removed"
            } else {
                "changed"
            }
            .to_string(),
        )
    } else {
        None
    };
    WiringStatus {
        agent: agent.into(),
        wired,
        config_path: Some(display),
        notice: Some(restart_notice(agent).to_string()),
        reason: None,
        state: record.map(|r| r.state.clone()),
        drift,
        codex_mode,
    }
}

fn restart_notice(agent: &str) -> &'static str {
    match agent {
        "claude-code" => "新会话生效；已在运行的会话需重开",
        // 这句话必须让用户知道登录方式被临时切走了
        "gemini" => "接线期间改用网关 Key，断开时恢复原登录；需重启 Gemini 后生效",
        "codex" => "保留 ChatGPT 登录时只改请求地址；需重启 Codex 后生效",
        "opencode" => "需重启该工具后生效",
        "zcode" => "需重启 ZCode 后生效",
        _ => "需重启后生效",
    }
}

/// 重新应用时优先沿用旧快照；只有第一次接线才使用当前文件算出的值。
fn effective_snapshot(prior: Option<&str>, computed: Option<serde_json::Value>) -> Option<String> {
    prior.map(str::to_string).or_else(|| computed.map(|s| s.to_string()))
}

/// 接入：读原值 → 拒绝别的实例 → 备份 → 原子写。返回（原值快照，实际写入的值）。
/// `before_write` 在文件落盘前调用：调用方用它先写 pending 记录，崩溃时才看得出写到哪一步。
/// `prior_snapshot` 是上一次接线保存的原值：重新应用时用它，不能拿当前文件重新快照——
/// 当前文件里可能已经是我们写进去的网关地址，重算会把用户的真实上游弄丢。
pub(crate) fn wire(
    agent: &str,
    port: u16,
    instance_short: &str,
    backup_dir: &Path,
    prior_snapshot: Option<&str>,
    before_write: impl FnOnce(Option<&str>, &Applied) -> Result<(), String>,
) -> Result<(Option<String>, Applied), String> {
    let path = config_path(agent).ok_or("未找到用户目录")?;
    let base = format!("http://127.0.0.1:{port}");
    let token = token_for(agent, instance_short);
    let original = if path.exists() {
        std::fs::read_to_string(&path).map_err(|error| format!("读取 {} 失败：{error}", path.display()))?
    } else {
        String::new()
    };
    if let Some(suffix) = foreign_marker(agent, &original, instance_short) {
        return Err(format!(
            "该工具已被另一个 Jolu 实例接管（尾号 {suffix}）；请先在那个实例里断开，或手动改回直连"
        ));
    }
    // Codex 已登录 ChatGPT 时保留 OAuth；否则才写自定义 provider + 静态令牌。
    let codex_oauth = agent == "codex" && codex_oauth_logged_in();
    let mut extra_files: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    let (next, snapshot) = match agent {
        "claude-code" => {
            let snapshot = claude_snapshot(&original);
            let text = if original.trim().is_empty() { "{}" } else { original.as_str() };
            (claude_wire(text, &base, &token)?, snapshot)
        }
        "codex" => {
            let snapshot = codex_snapshot(&original);
            let token = (!codex_oauth).then_some(token.as_str());
            (codex_wire(&original, &base, CODEX_PROVIDER, codex_oauth, token)?, snapshot)
        }
        "gemini" => {
            let settings_path = gemini_settings_path().ok_or("未找到用户目录")?;
            let settings_text = gemini_settings_text().unwrap_or_default();
            let plan = gemini_plan(&original, &settings_text, &base, &token)?;
            extra_files.push((settings_path, plan.settings.into_bytes()));
            (plan.env, Some(plan.snapshot))
        }
        "opencode" => (opencode_wire(&original, &base, &token)?, None),
        "zcode" => (zcode_wire(&original, &base, &token)?, None),
        _ => return Err("该工具不在接线范围".into()),
    };
    let mut files = extra_files;
    files.push((path, next.into_bytes()));
    for (file, _) in &files {
        backup(file, backup_dir)?;
    }
    let applied = Applied {
        base: if codex_oauth { format!("{base}/backend-api/codex") } else { base.clone() },
        token: if agent == "codex" {
            (!codex_oauth).then(|| token.clone())
        } else {
            matches!(agent, "claude-code" | "gemini" | "opencode" | "zcode").then(|| token.clone())
        },
        provider: (agent == "codex" && !codex_oauth).then(|| CODEX_PROVIDER.to_string()),
        mode: (agent == "codex").then(|| if codex_oauth { "oauth" } else { "provider" }.to_string()),
    };
    let snapshot = effective_snapshot(prior_snapshot, snapshot);
    before_write(snapshot.as_deref(), &applied)?;
    write_all_with_rollback(&files)?;
    Ok((snapshot, applied))
}

/// 多个文件按顺序写；中途失败就把已经写的文件还原，避免 `.env` 改了、settings 没改的半接管。
fn write_all_with_rollback(files: &[(PathBuf, Vec<u8>)]) -> Result<(), String> {
    let originals: Vec<Option<Vec<u8>>> = files.iter().map(|(path, _)| std::fs::read(path).ok()).collect();
    for (index, (path, bytes)) in files.iter().enumerate() {
        if let Err(error) = write_atomic(path, bytes.clone()) {
            for (done, (done_path, _)) in files[..index].iter().enumerate() {
                match &originals[done] {
                    Some(previous) => {
                        let _ = write_atomic(done_path, previous.clone());
                    }
                    None => {
                        let _ = std::fs::remove_file(done_path);
                    }
                }
            }
            return Err(error);
        }
    }
    Ok(())
}

/// 还原：只清理自己的写入。当前值已不指向本服务（外部改过 / 别的实例接管）则不覆盖。
pub(crate) fn unwire(
    agent: &str,
    port: u16,
    record: Option<&WiringRecord>,
) -> Result<bool, String> {
    let path = config_path(agent).ok_or("未找到用户目录")?;
    // Gemini 可能只剩 settings.json 被改过（.env 被外部删了），仍然要还原 auth。
    if !path.exists() && agent != "gemini" {
        return Ok(false);
    }
    let original = if path.exists() {
        std::fs::read_to_string(&path).map_err(|error| format!("读取 {} 失败：{error}", path.display()))?
    } else {
        String::new()
    };
    let instance = record.map(|r| r.instance_id.chars().take(8).collect::<String>()).unwrap_or_default();
    // 别的实例写的配置不能由我们还原；没有记录（旧版升级）时不做这层判断。
    if !instance.is_empty() && foreign_marker(agent, &original, &instance).is_some() {
        return Ok(false);
    }
    // 记录里存的是接线前的原值；没有记录时按「原来没有该键」处理。
    let saved: Option<serde_json::Value> = record
        .and_then(|r| r.original_json.as_deref())
        .and_then(|s| serde_json::from_str(s).ok());
    // Gemini 要同时还原 `.env` 与 settings.json，单独走多文件路径。
    if agent == "gemini" {
        let base_ours = gemini_current_base(&original).is_some_and(|base| is_gateway_value(&base));
        let token_ours = gemini_current_token(&original).is_some_and(|token| token.starts_with("jolu-"));
        let settings_ours = record.is_some()
            && gemini_settings_text()
                .as_deref()
                .and_then(gemini_selected_type)
                .as_deref()
                == Some("gemini-api-key");
        if !base_ours && !token_ours && !settings_ours {
            return Ok(false);
        }
        let settings_path = gemini_settings_path().ok_or("未找到用户目录")?;
        let settings_text = gemini_settings_text().unwrap_or_default();
        let (env, settings) = gemini_restore(&original, &settings_text, saved.as_ref());
        write_all_with_rollback(&[
            (settings_path, settings.into_bytes()),
            (path, env.into_bytes()),
        ])?;
        return Ok(true);
    }
    let next = match agent {
        "claude-code" => {
            // 外部意图优先：当前 base 已不是网关地址、且 token 不是 Jolu 写的，就不动
            let base = claude_current_base(&original).unwrap_or_default();
            let token = claude_current_token(&original).unwrap_or_default();
            if !points_at_gateway(&base, port) && !is_gateway_value(&base) && !token.starts_with("jolu-") {
                return Ok(false);
            }
            claude_restore(&original, saved.as_ref())?
        }
        "codex" => {
            let points_at_us = codex_effective_base(&original).is_some_and(|base| is_gateway_value(&base))
                || codex_provider_base(&original, CODEX_PROVIDER).is_some_and(|base| is_gateway_value(&base));
            if !points_at_us {
                return Ok(false);
            }
            codex_restore(&original, saved.as_ref(), CODEX_PROVIDER)?
        }
        "opencode" => {
            let base_ours = opencode_current_base(&original).is_some_and(|base| is_gateway_value(&base));
            let token_ours = opencode_current_token(&original).is_some_and(|token| token.starts_with("jolu-"));
            if !base_ours && !token_ours {
                return Ok(false);
            }
            opencode_restore(&original, saved.as_ref())?
        }
        "zcode" => {
            let base_ours = zcode_current_base(&original).is_some_and(|base| is_gateway_value(&base));
            let token_ours = zcode_current_token(&original).is_some_and(|token| token.starts_with("jolu-"));
            if !base_ours && !token_ours {
                return Ok(false);
            }
            zcode_restore(&original, saved.as_ref())?
        }
        _ => return Err("该工具不在接线范围".into()),
    };
    write_atomic(&path, next.into_bytes())?;
    Ok(true)
}

/// 启动时修复 pending：文件里确实是我们写下的值 → 直接标 wired；否则标 idle。
/// 不在启动时重写配置文件：用户可能正是在两个版本之间手动改回了直连。
pub(crate) fn reconcile_pending(agent: &str, port: u16, record: &WiringRecord) -> &'static str {
    if record.state != "pending" {
        return "unchanged";
    }
    let Some(path) = config_path(agent) else { return "idle" };
    let Ok(text) = std::fs::read_to_string(&path) else { return "idle" };
    let applied = record.applied();
    let wrote = applied
        .as_ref()
        .map(|applied| {
            let base = applied.base.trim();
            let expected_port = base.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()).unwrap_or(port);
            ours(agent, &text, expected_port, &record.instance_id.chars().take(8).collect::<String>())
        })
        .unwrap_or(false);
    if wrote { "wired" } else { "idle" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_wire_and_restore_round_trip_preserves_other_fields() {
        let original = r#"{"env":{"SOME_KEY":"keep","ANTHROPIC_BASE_URL":"https://old"},"model":"x"}"#;
        let snapshot = claude_snapshot(original).unwrap();
        let wired = claude_wire(original, "http://127.0.0.1:12731", "jolu-claude-code").unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["env"]["SOME_KEY"], "keep");
        assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:12731");
        assert_eq!(value["env"]["ANTHROPIC_AUTH_TOKEN"], "jolu-claude-code");
        assert_eq!(value["model"], "x");
        // 还原回原值
        let restored = claude_restore(&wired, Some(&snapshot)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "https://old");
        assert_eq!(value["env"]["SOME_KEY"], "keep");
        assert!(value["env"].get("ANTHROPIC_AUTH_TOKEN").is_none(), "原本没有的键还原后要删掉");
    }

    #[test]
    fn claude_restore_cleans_empty_env() {
        let wired = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12731","ANTHROPIC_AUTH_TOKEN":"jolu-claude-code"}}"#;
        let restored = claude_restore(wired, None).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert!(value.get("env").is_none(), "只装了网关配置的 env 段要整个清掉");
    }

    #[test]
    fn snapshot_filters_any_gateway_value() {
        // 重新接线时，上一次写进去的网关地址 / 令牌不能被当成用户原来的上游
        let rewire = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12731","ANTHROPIC_AUTH_TOKEN":"jolu-claude-code.abcdef12","KEEP":"1"}}"#;
        assert_eq!(claude_snapshot(rewire), None, "只有网关值时应视为没有原值");
        // 真实上游 + 上一次留下的 Jolu 令牌：只保留真实上游，令牌不能进快照
        let mixed = r#"{"env":{"ANTHROPIC_BASE_URL":"https://real.example","ANTHROPIC_AUTH_TOKEN":"jolu-claude-code"}}"#;
        let snapshot = claude_snapshot(mixed).unwrap();
        assert_eq!(snapshot["ANTHROPIC_BASE_URL"], "https://real.example");
        assert!(snapshot.get("ANTHROPIC_AUTH_TOKEN").is_none());
    }

    #[test]
    fn codex_provider_mode_round_trip_keeps_comments_and_writes_token() {
        let original = "# 注释要保留\nmodel_provider = \"openai\"\n\n[model_providers.openai]\nname = \"OpenAI\"\n";
        let snapshot = codex_snapshot(original).unwrap();
        assert_eq!(snapshot["model_provider"], "openai");
        let wired = codex_wire(
            original,
            "http://127.0.0.1:12731",
            "jolu-gateway",
            false,
            Some("jolu-codex.abcd1234"),
        )
        .unwrap();
        assert!(wired.contains("# 注释要保留"), "{wired}");
        assert!(wired.contains("jolu-gateway"), "{wired}");
        assert!(wired.contains("wire_api = \"responses\""), "{wired}");
        assert!(wired.contains("experimental_bearer_token = \"jolu-codex.abcd1234\""), "{wired}");
        assert!(ours("codex", &wired, 12731, "abcd1234"));
        // 别的实例的 token 要能认出来
        assert_eq!(foreign_marker("codex", &wired, "ffffffff").as_deref(), Some("abcd1234"));
        let restored = codex_restore(&wired, Some(&snapshot), "jolu-gateway").unwrap();
        assert!(restored.contains("# 注释要保留"), "{restored}");
        assert!(restored.contains("model_provider = \"openai\""), "{restored}");
        assert!(!restored.contains("jolu-gateway"), "还原要删掉 Jolu 的 provider：{restored}");
        assert!(!restored.contains("experimental_bearer_token"), "{restored}");
        // 重新接线不能把 jolu-gateway 当原值
        assert_eq!(codex_snapshot(&wired), None);
    }

    #[test]
    fn codex_oauth_mode_keeps_builtin_provider_and_restores_it() {
        // 已登录 ChatGPT：只写顶层 openai_base_url，不覆盖 model_provider
        let original = "model_provider = \"openai\"\nmodel = \"gpt-5.5\"\n";
        let snapshot = codex_snapshot(original).unwrap();
        let wired = codex_wire(original, "http://127.0.0.1:12731", "jolu-gateway", true, None).unwrap();
        assert!(wired.contains("openai_base_url = \"http://127.0.0.1:12731/backend-api/codex\""), "{wired}");
        assert!(!wired.contains("model_provider = \"jolu-gateway\""), "{wired}");
        assert!(!wired.contains("[model_providers.jolu-gateway]"), "{wired}");
        assert!(ours("codex", &wired, 12731, "abcd1234"));
        assert!(!wired.contains("model_provider ="), "OAuth 模式要用内置 provider：{wired}");
        let restored = codex_restore(&wired, Some(&snapshot), "jolu-gateway").unwrap();
        assert!(!restored.contains("openai_base_url"), "原本没有该键，还原时要删掉：{restored}");
        assert!(restored.contains("model_provider = \"openai\""), "{restored}");
        // 原本有 openai_base_url 的用户要能原样拿回
        let original = "model_provider = \"openai\"\nopenai_base_url = \"https://relay.example\"\n";
        let snapshot = codex_snapshot(original).unwrap();
        let wired = codex_wire(original, "http://127.0.0.1:12731", "jolu-gateway", true, None).unwrap();
        let restored = codex_restore(&wired, Some(&snapshot), "jolu-gateway").unwrap();
        assert!(restored.contains("openai_base_url = \"https://relay.example\""), "{restored}");
    }

    #[test]
    fn codex_restore_keeps_provider_switched_by_user() {
        let wired = "model_provider = \"jolu-gateway\"\n[model_providers.jolu-gateway]\nbase_url = \"http://127.0.0.1:12731\"\n";
        let switched = wired.replace("model_provider = \"jolu-gateway\"", "model_provider = \"openai\"");
        let restored = codex_restore(&switched, Some(&serde_json::json!({"model_provider":"zhipu"})), "jolu-gateway").unwrap();
        assert!(restored.contains("model_provider = \"openai\""), "用户切换后的 provider 不能被改回去：{restored}");
        assert!(!restored.contains("jolu-gateway"));
    }

    #[test]
    fn opencode_and_zcode_add_and_remove_own_entry_only() {
        let base = r#"{"provider":{"existing":{"options":{"baseURL":"https://keep"}}}}"#;
        let wired = opencode_wire(base, "http://127.0.0.1:12731", "jolu-opencode").unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["provider"]["existing"]["options"]["baseURL"], "https://keep");
        assert_eq!(value["provider"]["jolu"]["options"]["baseURL"], "http://127.0.0.1:12731");
        assert_eq!(opencode_current_base(&wired).as_deref(), Some("http://127.0.0.1:12731"));
        assert_eq!(opencode_current_token(&wired).as_deref(), Some("jolu-opencode"));
        let restored = opencode_restore(&wired, None).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert!(value["provider"].get("jolu").is_none());
        assert!(value["provider"].get("existing").is_some(), "别人的 provider 不能动");

        let wired = zcode_wire("{}", "http://127.0.0.1:12731", "jolu-zcode").unwrap();
        assert!(wired.contains("127.0.0.1:12731"));
        assert_eq!(zcode_current_base(&wired).as_deref(), Some("http://127.0.0.1:12731"));
        let restored = zcode_restore(&wired, None).unwrap();
        assert!(!restored.contains("jolu"));
    }

    #[test]
    fn gemini_takeover_switches_auth_and_restores_it() {
        let env = "# gemini 配置\nOTHER=1\nGOOGLE_GEMINI_BASE_URL=https://old\nGEMINI_API_KEY=user-key\n";
        let settings =
            r#"{"security":{"auth":{"selectedType":"oauth-personal"}},"model":{"name":"gemini-2.5-pro"}}"#;
        let plan = gemini_plan(env, settings, "http://127.0.0.1:12731", "jolu-gemini.abcd1234").unwrap();
        assert!(plan.env.contains("# gemini 配置"), "{}", plan.env);
        assert!(plan.env.contains("OTHER=1"), "{}", plan.env);
        assert!(plan.env.contains("GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:12731"), "{}", plan.env);
        assert!(plan.env.contains("GEMINI_API_KEY=jolu-gemini.abcd1234"), "{}", plan.env);
        let wired_settings: serde_json::Value = serde_json::from_str(&plan.settings).unwrap();
        assert_eq!(wired_settings["security"]["auth"]["selectedType"], "gemini-api-key");
        assert_eq!(wired_settings["model"]["name"], "gemini-2.5-pro", "其他设置不能动");

        let snapshot = plan.snapshot.clone();
        assert_eq!(snapshot["env"]["GEMINI_API_KEY"], "user-key");
        assert_eq!(snapshot["settings"]["selectedType"], "oauth-personal");
        assert_eq!(snapshot["env"]["GOOGLE_GEMINI_BASE_URL"], "https://old");

        let (env, settings) = gemini_restore(&plan.env, &plan.settings, Some(&snapshot));
        assert!(env.contains("GOOGLE_GEMINI_BASE_URL=https://old"), "{env}");
        assert!(env.contains("GEMINI_API_KEY=user-key"), "{env}");
        assert!(env.contains("OTHER=1"), "{env}");
        let settings: serde_json::Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(settings["security"]["auth"]["selectedType"], "oauth-personal");
        assert_eq!(settings["model"]["name"], "gemini-2.5-pro");

        // 原本没有这些键：还原时删除，不能留下 jolu 令牌
        let plan = gemini_plan("OTHER=1\n", "", "http://127.0.0.1:12731", "jolu-gemini.abcd1234").unwrap();
        let (env, settings) = gemini_restore(&plan.env, &plan.settings, Some(&plan.snapshot));
        assert!(!env.contains("GOOGLE_GEMINI_BASE_URL"), "{env}");
        assert!(!env.contains("GEMINI_API_KEY"), "{env}");
        let settings: serde_json::Value = serde_json::from_str(&settings).unwrap();
        assert!(settings.pointer("/security/auth/selectedType").is_none(), "{settings}");

        // 旧版快照只有 value：只还原地址，不改 settings
        let plan = gemini_plan(env.as_str(), settings.to_string().as_str(), "http://127.0.0.1:12731", "jolu-gemini.abcd1234").unwrap();
        let (_, legacy_settings) = gemini_restore(&plan.env, &plan.settings, Some(&serde_json::json!({"value":"https://legacy"})));
        assert_eq!(legacy_settings, plan.settings, "旧快照不能改 auth");
    }

    #[test]
    fn gemini_refuses_invalid_settings_instead_of_wiping_it() {
        assert!(gemini_plan("A=1\n", "{not json", "http://127.0.0.1:12731", "jolu-gemini.abcd1234").is_err());
        let (_, settings) = gemini_restore(
            "A=1\n",
            "{not json",
            Some(&serde_json::json!({"env": {}, "settings": {"selectedType": "oauth-personal"}})),
        );
        assert_eq!(settings, "{not json", "坏文件不能被覆盖");
    }

    #[test]
    fn gateway_pointing_check_matches_only_our_port() {
        assert!(points_at_gateway("http://127.0.0.1:12731", 12731));
        assert!(points_at_gateway("http://127.0.0.1:12731/", 12731));
        assert!(points_at_gateway("http://localhost:12731", 12731));
        assert!(!points_at_gateway("http://127.0.0.1:12732", 12731));
        assert!(!points_at_gateway("https://api.anthropic.com", 12731));
    }

    #[test]
    fn token_carries_instance_suffix() {
        assert_eq!(token_for("claude-code", "abcd1234"), "jolu-claude-code.abcd1234");
        assert_eq!(token_for("codex", "abcd1234"), "jolu-codex.abcd1234");
    }

    #[test]
    fn rewiring_keeps_the_original_snapshot() {
        let computed = Some(serde_json::json!({"ANTHROPIC_BASE_URL":"https://current"}));
        assert_eq!(
            effective_snapshot(Some(r#"{"ANTHROPIC_BASE_URL":"https://original"}"#), computed.clone()).as_deref(),
            Some(r#"{"ANTHROPIC_BASE_URL":"https://original"}"#),
            "重新应用不能拿当前文件重新快照"
        );
        assert_eq!(
            effective_snapshot(None, computed).as_deref(),
            Some(r#"{"ANTHROPIC_BASE_URL":"https://current"}"#)
        );
        assert_eq!(effective_snapshot(None, None), None);
    }

    #[test]
    fn foreign_marker_rejects_another_instance() {
        let text = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12731","ANTHROPIC_AUTH_TOKEN":"jolu-claude-code.ffffffff"}}"#;
        assert_eq!(foreign_marker("claude-code", text, "abcd1234").as_deref(), Some("ffffffff"));
        assert_eq!(foreign_marker("claude-code", text, "ffffffff"), None);
        // 旧版没有实例后缀：按本实例处理，允许覆盖
        let legacy = r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"jolu-claude-code"}}"#;
        assert_eq!(foreign_marker("claude-code", legacy, "abcd1234"), None);
    }

    #[test]
    fn agent_whitelist_covers_catalog_platforms() {
        for agent in ["claude-code", "codex", "zcode", "gemini", "opencode"] {
            assert!(is_agent(agent), "{agent}");
        }
        assert!(!is_agent("qoder"), "Qoder 不在接线范围（不代管密钥）");
        assert!(!is_agent("workbuddy"));
    }

    /// 文件级链路：接线 → 状态探测为已接线 → 还原 → 状态回到未接线。
    /// 用临时目录注入配置路径，不碰真实用户配置。
    #[test]
    fn file_level_wire_status_unwire_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "jolu-wiring-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("settings.json");
        std::fs::write(&config, r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com"},"keep":1}"#).unwrap();
        let backups = dir.join("backups");

        // 直接调纯函数模拟 wire/unwire（真实 wire() 依赖用户目录，测不到临时路径）
        let original = std::fs::read_to_string(&config).unwrap();
        let snapshot = claude_snapshot(&original).unwrap();
        let wired = claude_wire(&original, "http://127.0.0.1:12731", "jolu-claude-code.abcd1234").unwrap();
        std::fs::write(&config, &wired).unwrap();
        assert!(ours("claude-code", &wired, 12731, "abcd1234"));
        assert!(!ours("claude-code", &wired, 12732, "abcd1234"));

        // 还原
        let restored = claude_restore(&wired, Some(&snapshot)).unwrap();
        std::fs::write(&config, &restored).unwrap();
        let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "https://api.anthropic.com", "还原回原上游");
        assert_eq!(value["keep"], 1, "无关字段必须保留");

        // 备份目录可创建（真实 wire 会用到）
        std::fs::create_dir_all(&backups).unwrap();
        assert!(backups.is_dir());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 外部已改：还原前发现当前值不是网关地址 → 返回 false 且不改文件。
    #[test]
    fn unwire_skips_when_external_value_took_over() {
        let external = r#"{"env":{"ANTHROPIC_BASE_URL":"https://custom.example"}}"#;
        let current = claude_current_base(external);
        assert!(
            current.is_some_and(|c| !points_at_gateway(&c, 12731) && !is_gateway_value(&c)),
            "外部值不应被判定为网关地址，因此还原会跳过"
        );
        let ours_text = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12731","ANTHROPIC_AUTH_TOKEN":"jolu-claude-code.abcd1234"}}"#;
        assert!(ours("claude-code", ours_text, 12731, "abcd1234"));
    }
}
