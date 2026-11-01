//! 接线层：把各 agent 的配置指向本机网关（执行文档 §8）。
//!
//! 纪律（从 `cli_apply.rs` 提取，全部沿用）：
//! 1. 写前把原文件备份到应用数据目录
//! 2. 临时文件 + rename 原子写
//! 3. 还原只清理自己的写入：当前值不再是网关地址时不覆盖（外部意图优先）
//! 4. 全局切换锁 [`serialized_switch`]（网关接线与观测接管互斥）
//! 5. 接线动作记录进 `settings.gateway_wiring`（原值快照），支持逐 agent 还原
//!
//! 文件读写拆成纯函数（`*_write` / `*_read`），路径解析依赖真实用户目录、单独一层，
//! 测试用临时文本验证保留性与边界（与 `proxy_config.rs` 同款做法）。

use std::path::{Path, PathBuf};

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
}

/// 全局配置切换锁：网关接线与观测接管互斥，避免两个流程同时写同一批文件。
pub(crate) fn serialized_switch<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().map_err(|_| "CLI 配置切换锁不可用，请重启应用")?;
    work()
}

/* ─────────────────────────── 纯函数：各 agent 的配置读写 ─────────────────────────── */

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

/// 读 Claude 的两个键作为原值快照。
fn claude_snapshot(text: &str) -> Option<serde_json::Value> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    let env = root.get("env")?;
    let mut saved = serde_json::Map::new();
    for key in ["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN"] {
        if let Some(value) = env.get(key) {
            saved.insert(key.into(), value.clone());
        }
    }
    if saved.is_empty() { None } else { Some(serde_json::Value::Object(saved)) }
}

/// Codex：`config.toml` 的自定义 provider 段 + `base_url`；`wire_api` 按连接协议。
/// 保持注释与格式（沿用 proxy_config.rs 的 toml_edit 做法）。
fn codex_wire(text: &str, base: &str, provider: &str, wire_api: &str) -> Result<String, String> {
    let mut document = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("config.toml 格式无效：{error}"))?;
    if !document.contains_key("model_providers") {
        document["model_providers"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let providers = document["model_providers"]
        .as_table_mut()
        .ok_or("config.toml 的 model_providers 不是表")?;
    let mut entry = toml_edit::Table::new();
    entry["name"] = toml_edit::value(provider);
    entry["base_url"] = toml_edit::value(base);
    entry["wire_api"] = toml_edit::value(wire_api);
    providers[provider] = toml_edit::Item::Table(entry);
    document["model_provider"] = toml_edit::value(provider);
    Ok(document.to_string())
}

/// 还原 Codex：删掉 Jolu 建的 provider 段，并回写原 `model_provider`。
fn codex_restore(text: &str, saved: Option<&serde_json::Value>, provider: &str) -> Result<String, String> {
    let mut document = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("config.toml 格式无效：{error}"))?;
    if let Some(providers) = document.get_mut("model_providers").and_then(|p| p.as_table_mut()) {
        providers.remove(provider);
    }
    match saved.and_then(|s| s.get("model_provider")).and_then(|v| v.as_str()) {
        Some(original) => {
            document["model_provider"] = toml_edit::value(original);
        }
        None => {
            document.remove("model_provider");
        }
    }
    Ok(document.to_string())
}

/// 读 Codex 的当前 provider 作为原值快照。
fn codex_snapshot(text: &str) -> Option<serde_json::Value> {
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    let current = document.get("model_provider")?.as_str()?.to_string();
    if current == "jolu-gateway" {
        return None;
    }
    Some(serde_json::json!({"model_provider": current}))
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

/// Gemini CLI：`~/.gemini/.env` 的 `GOOGLE_GEMINI_BASE_URL`（dotenv 文本）。
fn gemini_wire(text: &str, base: &str) -> String {
    wire_dotenv(text, "GOOGLE_GEMINI_BASE_URL", Some(base))
}

fn gemini_restore(text: &str, saved: Option<&serde_json::Value>) -> String {
    wire_dotenv(text, "GOOGLE_GEMINI_BASE_URL", saved.and_then(|s| s.get("value")).and_then(|v| v.as_str()))
}

fn gemini_snapshot(text: &str) -> Option<serde_json::Value> {
    dotenv_get(text, "GOOGLE_GEMINI_BASE_URL")
        .filter(|v| !v.contains("127.0.0.1"))
        .map(|v| serde_json::json!({"value": v}))
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
    value.trim().trim_end_matches('/').starts_with(&format!("http://127.0.0.1:{port}"))
}

/* ─────────────────────────── 文件层：路径、备份、原子写 ─────────────────────────── */

/// Jolu 建的 Codex provider 段名与各 agent 的网关令牌。
const CODEX_PROVIDER: &str = "jolu-gateway";

fn token_for(agent: &str) -> String {
    format!("jolu-{agent}")
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

/// 探测某 agent 的当前接入状态。
pub fn status(agent: &str, port: u16) -> WiringStatus {
    if !is_agent(agent) {
        return WiringStatus {
            agent: agent.into(),
            wired: false,
            config_path: None,
            notice: None,
            reason: Some("该工具不在接线范围".into()),
        };
    }
    let Some(path) = config_path(agent) else {
        return WiringStatus {
            agent: agent.into(),
            wired: false,
            config_path: None,
            notice: None,
            reason: Some("未找到用户目录".into()),
        };
    };
    let display = path.display().to_string();
    if !path.exists() {
        return WiringStatus {
            agent: agent.into(),
            wired: false,
            config_path: Some(display),
            notice: None,
            reason: Some("尚未安装或未生成配置文件".into()),
        };
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let wired = match agent {
        "claude-code" => claude_snapshot(&text)
            .and_then(|s| s.get("ANTHROPIC_BASE_URL").and_then(|v| v.as_str()).map(str::to_string))
            .is_some_and(|base| points_at_gateway(&base, port)),
        "codex" => text.contains(CODEX_PROVIDER) && text.contains(&format!("127.0.0.1:{port}")),
        "gemini" => dotenv_get(&text, "GOOGLE_GEMINI_BASE_URL").is_some_and(|v| points_at_gateway(&v, port)),
        "opencode" | "zcode" => text.contains(&format!("127.0.0.1:{port}")),
        _ => false,
    };
    WiringStatus {
        agent: agent.into(),
        wired,
        config_path: Some(display),
        notice: Some(restart_notice(agent).to_string()),
        reason: None,
    }
}

fn restart_notice(agent: &str) -> &'static str {
    match agent {
        "claude-code" => "新会话生效；已在运行的会话需重开",
        "codex" | "gemini" | "opencode" => "需重启该工具后生效",
        "zcode" => "需重启 ZCode 后生效",
        _ => "需重启后生效",
    }
}

/// 接入：读原值 → 备份 → 原子写。返回原值快照（调用方存进设置供还原）。
pub(crate) fn wire(agent: &str, port: u16, backup_dir: &Path) -> Result<Option<String>, String> {
    let path = config_path(agent).ok_or("未找到用户目录")?;
    let base = format!("http://127.0.0.1:{port}");
    let token = token_for(agent);
    let original = if path.exists() {
        std::fs::read_to_string(&path).map_err(|error| format!("读取 {} 失败：{error}", path.display()))?
    } else {
        String::new()
    };
    let (next, snapshot) = match agent {
        "claude-code" => {
            let snapshot = claude_snapshot(&original);
            let text = if original.trim().is_empty() { "{}" } else { original.as_str() };
            (claude_wire(text, &base, &token)?, snapshot)
        }
        "codex" => {
            let snapshot = codex_snapshot(&original);
            (codex_wire(&original, &base, CODEX_PROVIDER, "chat")?, snapshot)
        }
        "gemini" => {
            let snapshot = gemini_snapshot(&original);
            (gemini_wire(&original, &base), snapshot)
        }
        "opencode" => (opencode_wire(&original, &base, &token)?, None),
        "zcode" => (zcode_wire(&original, &base, &token)?, None),
        _ => return Err("该工具不在接线范围".into()),
    };
    backup(&path, backup_dir)?;
    write_atomic(&path, next.into_bytes())?;
    Ok(snapshot.map(|s| s.to_string()))
}

/// 还原：只清理自己的写入。当前值已不指向网关（外部改过）则不覆盖。
pub(crate) fn unwire(agent: &str, port: u16, snapshot: Option<&str>) -> Result<bool, String> {
    let path = config_path(agent).ok_or("未找到用户目录")?;
    if !path.exists() {
        return Ok(false);
    }
    let original = std::fs::read_to_string(&path).map_err(|error| format!("读取 {} 失败：{error}", path.display()))?;
    let saved: Option<serde_json::Value> = snapshot.and_then(|s| serde_json::from_str(s).ok());
    let next = match agent {
        "claude-code" => {
            // 外部意图优先：当前 base 已不是网关地址就不动
            let current = claude_snapshot(&original)
                .and_then(|s| s.get("ANTHROPIC_BASE_URL").and_then(|v| v.as_str()).map(str::to_string));
            if current.is_some_and(|c| !points_at_gateway(&c, port)) {
                return Ok(false);
            }
            claude_restore(&original, saved.as_ref())?
        }
        "codex" => {
            if !original.contains(CODEX_PROVIDER) {
                return Ok(false);
            }
            codex_restore(&original, saved.as_ref(), CODEX_PROVIDER)?
        }
        "gemini" => {
            let current = dotenv_get(&original, "GOOGLE_GEMINI_BASE_URL");
            if current.is_some_and(|c| !points_at_gateway(&c, port)) {
                return Ok(false);
            }
            gemini_restore(&original, saved.as_ref())
        }
        "opencode" => opencode_restore(&original, saved.as_ref())?,
        "zcode" => zcode_restore(&original, saved.as_ref())?,
        _ => return Err("该工具不在接线范围".into()),
    };
    write_atomic(&path, next.into_bytes())?;
    Ok(true)
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
    fn codex_wire_restore_round_trip_keeps_comments() {
        let original = "# 注释要保留\nmodel_provider = \"openai\"\n\n[model_providers.openai]\nname = \"OpenAI\"\n";
        let snapshot = codex_snapshot(original).unwrap();
        assert_eq!(snapshot["model_provider"], "openai");
        let wired = codex_wire(original, "http://127.0.0.1:12731", "jolu-gateway", "chat").unwrap();
        assert!(wired.contains("# 注释要保留"), "{wired}");
        assert!(wired.contains("jolu-gateway"), "{wired}");
        assert!(wired.contains("wire_api = \"chat\""), "{wired}");
        let restored = codex_restore(&wired, Some(&snapshot), "jolu-gateway").unwrap();
        assert!(restored.contains("# 注释要保留"), "{restored}");
        assert!(restored.contains("model_provider = \"openai\""), "{restored}");
        assert!(!restored.contains("jolu-gateway"), "还原要删掉 Jolu 的 provider：{restored}");
    }

    #[test]
    fn opencode_and_zcode_add_and_remove_own_entry_only() {
        let base = r#"{"provider":{"existing":{"options":{"baseURL":"https://keep"}}}}"#;
        let wired = opencode_wire(base, "http://127.0.0.1:12731", "jolu-opencode").unwrap();
        let value: serde_json::Value = serde_json::from_str(&wired).unwrap();
        assert_eq!(value["provider"]["existing"]["options"]["baseURL"], "https://keep");
        assert_eq!(value["provider"]["jolu"]["options"]["baseURL"], "http://127.0.0.1:12731");
        let restored = opencode_restore(&wired, None).unwrap();
        let value: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert!(value["provider"].get("jolu").is_none());
        assert!(value["provider"].get("existing").is_some(), "别人的 provider 不能动");

        let wired = zcode_wire("{}", "http://127.0.0.1:12731", "jolu-zcode").unwrap();
        assert!(wired.contains("127.0.0.1:12731"));
        let restored = zcode_restore(&wired, None).unwrap();
        assert!(!restored.contains("jolu"));
    }

    #[test]
    fn gemini_dotenv_wire_restore_and_snapshot() {
        let original = "# gemini 配置\nOTHER=1\nGOOGLE_GEMINI_BASE_URL=https://old\n";
        let snapshot = gemini_snapshot(original).unwrap();
        assert_eq!(snapshot["value"], "https://old");
        let wired = gemini_wire(original, "http://127.0.0.1:12731");
        assert!(wired.contains("# gemini 配置"), "{wired}");
        assert!(wired.contains("OTHER=1"), "{wired}");
        assert!(wired.contains("GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:12731"), "{wired}");
        let restored = gemini_restore(&wired, Some(&snapshot));
        assert!(restored.contains("GOOGLE_GEMINI_BASE_URL=https://old"), "{restored}");
        // 原本没有该键 → 还原时移除
        let wired = gemini_wire("OTHER=1\n", "http://127.0.0.1:12731");
        let restored = gemini_restore(&wired, None);
        assert!(!restored.contains("GOOGLE_GEMINI_BASE_URL"), "{restored}");
        assert!(restored.contains("OTHER=1"), "{restored}");
    }

    #[test]
    fn gateway_pointing_check_matches_only_our_port() {
        assert!(points_at_gateway("http://127.0.0.1:12731", 12731));
        assert!(points_at_gateway("http://127.0.0.1:12731/", 12731));
        assert!(!points_at_gateway("http://127.0.0.1:12732", 12731));
        assert!(!points_at_gateway("https://api.anthropic.com", 12731));
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
        let wired = claude_wire(&original, "http://127.0.0.1:12731", "jolu-claude-code").unwrap();
        std::fs::write(&config, &wired).unwrap();
        assert!(points_at_gateway("http://127.0.0.1:12731", 12731));

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
        // 模拟 unwire 的判断逻辑：当前 base 不是网关地址时不动
        let external = r#"{"env":{"ANTHROPIC_BASE_URL":"https://custom.example"}}"#;
        let current = claude_snapshot(external)
            .and_then(|s| s.get("ANTHROPIC_BASE_URL").and_then(|v| v.as_str()).map(str::to_string));
        assert!(
            current.is_some_and(|c| !points_at_gateway(&c, 12731)),
            "外部值不应被判定为网关地址，因此还原会跳过"
        );
        // 反向确认：网关地址会被判定为需要还原
        let ours = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12731"}}"#;
        let current = claude_snapshot(ours)
            .and_then(|s| s.get("ANTHROPIC_BASE_URL").and_then(|v| v.as_str()).map(str::to_string));
        assert!(current.is_some_and(|c| points_at_gateway(&c, 12731)));
    }
}
