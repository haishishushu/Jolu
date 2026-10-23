//! 启用连接 = 写入 CLI 配置（画布 18 后续需求，对齐 cc-switch 的切换作用）。
//!
//! 三种来源的连接（官方订阅 / API 本机读取 / API 手动填写）启用时，
//! 都把连接保存的凭证与地址写回对应 CLI 的配置文件，使新会话即刻生效：
//! - Claude API  ：`~/.claude/settings.json` 的 `env.ANTHROPIC_AUTH_TOKEN`（+ `ANTHROPIC_BASE_URL`）
//! - Claude Auth ：仅选择当前完整授权，拒绝拼接不同账号的 token
//! - Codex  API  ：`~/.codex/auth.json` 的 `OPENAI_API_KEY`（+ config.toml 当前 provider 的 base_url）
//! - Codex  Auth ：恢复该账号已保护的完整授权，清除 API 模式覆盖
//!
//! 写入路径与 `creds::read_secret` 的读取路径一一对应；每次写文件前先把原文件
//! 备份到应用数据目录，写失败不留半截内容（write_atomic 风格）。

use std::path::{Path, PathBuf};

/// 写回结果说明（给 Toast 的补充文案），不含任何凭证内容。
pub struct ApplyNote {
    pub message: String,
}

/// 同一进程的所有窗口共享一个配置切换锁；仅在 blocking 线程里持有。
pub(crate) fn serialized_switch<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().map_err(|_| "CLI 配置切换锁不可用，请重启应用")?;
    work()
}

pub fn apply_to_cli(
    platform: &str,
    kind: &str,
    secret: &str,
    base_url: Option<&str>,
    backup_dir: &Path,
    auth_snapshot: Option<&serde_json::Value>,
) -> Result<ApplyNote, String> {
    if crate::validation::isolated() { return Err("隔离验收不修改外部 CLI 登录或配置".into()); }
    if platform == "codex" { validate_codex_environment(base_url, |key| std::env::var(key).ok())?; }
    match (platform, kind) {
        ("claude", "api") => apply_claude_api(secret, base_url, backup_dir),
        ("claude", _) => apply_claude_auth(secret, backup_dir),
        ("codex", "api") => apply_codex_api(secret, base_url, backup_dir),
        ("codex", _) => apply_codex_auth(secret, backup_dir, auth_snapshot),
        _ => Err(format!("暂不支持把 {platform} 连接写入 CLI 配置")),
    }
}

/* ─────────────────────────── Claude ─────────────────────────── */

fn claude_settings_path() -> Result<PathBuf, String> {
    claude_home().map(|dir| dir.join("settings.json"))
}

fn claude_credentials_path() -> Result<PathBuf, String> {
    claude_home().map(|dir| dir.join(".credentials.json"))
}

/// settings.json 的 env 段写入：token 必写；base_url 有则写、无则删（官方 API 直连）。
fn apply_claude_api(
    secret: &str,
    base_url: Option<&str>,
    backup_dir: &Path,
) -> Result<ApplyNote, String> {
    let path = claude_settings_path()?;
    update_json(&path, backup_dir, |v| {
        let env = v
            .as_object_mut()
            .ok_or_else(|| "settings.json 顶层不是对象".to_string())?
            .entry("env")
            .or_insert_with(|| serde_json::Value::Object(Default::default()));
        let env = env
            .as_object_mut()
            .ok_or_else(|| "settings.json 的 env 段不是对象".to_string())?;
        env.remove("ANTHROPIC_API_KEY");
        env.insert(
            "ANTHROPIC_AUTH_TOKEN".into(),
            serde_json::Value::String(secret.to_string()),
        );
        match base_url {
            Some(url) => {
                env.insert(
                    "ANTHROPIC_BASE_URL".into(),
                    serde_json::Value::String(url.to_string()),
                );
            }
            None => {
                env.remove("ANTHROPIC_BASE_URL");
            }
        }
        Ok(())
    })?;
    let where_url = match base_url {
        Some(url) => format!("、Base URL {url}"),
        None => "（官方直连，已移除自定义 Base URL）".to_string(),
    };
    Ok(ApplyNote {
        message: format!("已写入 ~/.claude/settings.json：AUTH_TOKEN{where_url}，新会话生效"),
    })
}

/// OAuth 必须保留同一身份的完整凭证，不能用保存的 accessToken 覆盖另一账号。
fn apply_claude_auth(secret: &str, backup_dir: &Path) -> Result<ApplyNote, String> {
    verify_current_auth(
        &claude_credentials_path()?,
        "/claudeAiOauth/accessToken",
        secret,
    )?;
    let path = claude_settings_path()?;
    if path.exists() {
        update_json(&path, backup_dir, |v| {
            let object = v.as_object_mut().ok_or("settings.json 不是对象")?;
            object.remove("apiKeyHelper");
            if let Some(env) = object.get_mut("env").and_then(|v| v.as_object_mut()) {
                for key in [
                    "ANTHROPIC_API_KEY",
                    "ANTHROPIC_AUTH_TOKEN",
                    "ANTHROPIC_BASE_URL",
                ] {
                    env.remove(key);
                }
            }
            Ok(())
        })?;
    }
    Ok(ApplyNote {
        message: "已选择当前本机完整授权，并清除配置中的 API 覆盖；终端环境变量仍由终端管理".into(),
    })
}

/* ─────────────────────────── Codex ─────────────────────────── */

fn codex_auth_path() -> Result<PathBuf, String> {
    codex_home().map(|dir| dir.join("auth.json"))
}

fn codex_config_path() -> Result<PathBuf, String> {
    codex_home().map(|dir| dir.join("config.toml"))
}

fn apply_codex_api(
    secret: &str,
    base_url: Option<&str>,
    backup_dir: &Path,
) -> Result<ApplyNote, String> {
    apply_codex_api_at(&codex_auth_path()?, &codex_config_path()?, secret, base_url, backup_dir)
}

fn apply_codex_api_at(auth: &Path, config: &Path, secret: &str, base_url: Option<&str>, backup_dir: &Path) -> Result<ApplyNote, String> {
    let old_auth = read_optional(auth)?;
    let old_config = read_optional(config)?;
    let mut value: serde_json::Value =
        serde_json::from_slice(old_auth.as_deref().unwrap_or(b"{}"))
            .map_err(|_| "auth.json 无效")?;
    let object = value.as_object_mut().ok_or("auth.json 不是对象")?;
    object.remove("tokens");
    object.remove("last_refresh");
    object.remove("auth_mode");
    object.insert(
        "OPENAI_API_KEY".into(),
        serde_json::Value::String(secret.into()),
    );
    let document = codex_provider_config(
        std::str::from_utf8(old_config.as_deref().unwrap_or_default()).map_err(|_| "config.toml 不是 UTF-8")?,
        base_url,
    )?;
    write_pair(
        &auth,
        serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
        &config,
        document.into_bytes(),
        backup_dir,
        (old_auth, old_config),
    )?;
    Ok(ApplyNote {
        message: "Key 与服务地址已一起写入 Codex 配置，新会话生效；原配置已备份".into(),
    })
}

fn apply_codex_auth(secret: &str, backup_dir: &Path, snapshot: Option<&serde_json::Value>) -> Result<ApplyNote, String> {
    apply_codex_auth_at(&codex_auth_path()?, &codex_config_path()?, secret, backup_dir, snapshot)
}

fn apply_codex_auth_at(auth: &Path, config: &Path, secret: &str, backup_dir: &Path, snapshot: Option<&serde_json::Value>) -> Result<ApplyNote, String> {
    let old_auth = read_optional(auth)?;
    let old_config = read_optional(config)?;
    let current: serde_json::Value = serde_json::from_slice(old_auth.as_deref().unwrap_or(b"{}"))
        .map_err(|_| "auth.json 无效")?;
    let value = restore_codex_bundle(&current, snapshot, secret)?;
    let document = codex_provider_config(
        std::str::from_utf8(old_config.as_deref().unwrap_or_default()).map_err(|_| "config.toml 不是 UTF-8")?,
        None,
    )?;
    write_pair(
        &auth,
        serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
        &config,
        document.into_bytes(),
        backup_dir,
        (old_auth, old_config),
    )?;
    Ok(ApplyNote {
        message: "已恢复该账号的完整授权及官方服务；新会话生效，原配置已加密备份".into(),
    })
}

fn restore_codex_bundle(current: &serde_json::Value, saved: Option<&serde_json::Value>, secret: &str) -> Result<serde_json::Value, String> {
    crate::auth_snapshot::complete_codex(current, secret)
        .or_else(|| saved.and_then(|snapshot| crate::auth_snapshot::complete_codex(snapshot, secret)))
        .ok_or_else(|| "该账号尚无可恢复的完整授权，请在 Codex 登录此账号后重新同步；不能仅替换 access token".into())
}

fn verify_current_auth(path: &Path, pointer: &str, secret: &str) -> Result<(), String> {
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|_| "授权文件格式无效")?;
    if secret.is_empty() || value.pointer(pointer).and_then(|v| v.as_str()) != Some(secret) {
        return Err("保存的授权与当前 CLI 登录不同；请在对应 CLI 登录目标账号，再重新获取完整授权。不能仅替换 access token".into());
    }
    Ok(())
}

fn validate_codex_environment(url: Option<&str>, env: impl Fn(&str) -> Option<String>) -> Result<(), String> {
    let set = |key| env(key).is_some_and(|value| !value.trim().is_empty());
    if set("CODEX_API_KEY") { return Err("检测到 CODEX_API_KEY 环境覆盖，请清除后再切换账号".into()); }
    if url.is_none() && set("OPENAI_BASE_URL") {
        return Err("检测到 OPENAI_BASE_URL 环境覆盖，无法确认官方服务地址；请清除后再切换".into());
    }
    Ok(())
}

fn codex_provider_config(text: &str, url: Option<&str>) -> Result<String, String> {
    let mut document = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| "config.toml 格式无效")?;
    let selected_profile = document.get("profile").map(|item| item.as_str().filter(|name| !name.trim().is_empty())
        .map(str::to_owned).ok_or("profile 必须是有效名称")).transpose()?;
    let profile = if let Some(name) = selected_profile.as_deref() {
        Some(document.get("profiles").and_then(|profiles| profiles.get(name)).and_then(toml_edit::Item::as_table_like)
            .ok_or("当前 profile 不存在或格式无效，未修改授权")?)
    } else { None };
    if profile.is_some_and(|profile| profile.get("model_providers").is_some()) {
        return Err("当前 profile 内含供应商覆写，无法安全切换；请先整理该 profile 的供应商配置".into());
    }
    let store = profile.and_then(|profile| profile.get("cli_auth_credentials_store")).or_else(|| document.get("cli_auth_credentials_store"));
    if store.is_some_and(|store| store.as_str() != Some("file")) {
        return Err("当前 Codex 使用非文件授权存储，不能通过 auth.json 切换；请在 Codex 中管理该登录".into());
    }
    if let Some(url) = url {
        let mut provider = toml_edit::Table::new();
        provider["name"] = toml_edit::value("Jolu");
        provider["base_url"] = toml_edit::value(url);
        provider["wire_api"] = toml_edit::value("responses");
        provider["requires_openai_auth"] = toml_edit::value(true);
        if document.get("model_providers").is_none() {
            document["model_providers"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let providers = document
            .get_mut("model_providers")
            .and_then(|v| v.as_table_like_mut())
            .ok_or("model_providers 格式无效")?;
        providers.insert("jolu", toml_edit::Item::Table(provider));
        document["model_provider"] = toml_edit::value("jolu");
    } else {
        if let Some(providers) = document.get_mut("model_providers") {
            let providers = providers.as_table_like_mut().ok_or("model_providers 格式无效")?;
            if let Some(provider) = providers.get_mut("openai") {
                let provider = provider.as_table_like_mut().ok_or("官方供应商覆写格式无效，未修改授权")?;
                for key in ["base_url", "env_key", "env_key_instructions", "experimental_bearer_token", "auth", "gateway_oauth", "aws", "http_headers", "env_http_headers", "query_params", "model_catalog_url"] {
                    provider.remove(key);
                }
                provider.insert("requires_openai_auth", toml_edit::value(true));
                provider.insert("wire_api", toml_edit::value("responses"));
            }
        }
        document["model_provider"] = toml_edit::value("openai");
    }
    // 只调整当前默认 profile 的供应商选择，保留其模型、推理设置和其他 profiles。
    if let Some(name) = selected_profile {
        let profile = document.get_mut("profiles").and_then(|profiles| profiles.get_mut(&name)).and_then(toml_edit::Item::as_table_like_mut)
            .ok_or("当前 profile 格式无效，未修改授权")?;
        profile.insert("model_provider", toml_edit::value(if url.is_some() { "jolu" } else { "openai" }));
    }
    Ok(document.to_string())
}

/// 先验证并备份全部文件；第二个写入失败时恢复第一个，不能返回半成功。
fn write_pair(
    first: &Path,
    first_bytes: Vec<u8>,
    second: &Path,
    second_bytes: Vec<u8>,
    backup_dir: &Path,
    expected: (Option<Vec<u8>>, Option<Vec<u8>>),
) -> Result<(), String> {
    write_pair_expected(first, first_bytes, second, second_bytes, backup_dir, expected, write_atomic)
}
#[cfg(test)]
fn write_pair_using(
    first: &Path, first_bytes: Vec<u8>, second: &Path, second_bytes: Vec<u8>, backup_dir: &Path,
    write: impl FnMut(&Path, Vec<u8>) -> Result<(), String>,
) -> Result<(), String> {
    let expected = (read_optional(first)?, read_optional(second)?);
    write_pair_expected(first, first_bytes, second, second_bytes, backup_dir, expected, write)
}
fn write_pair_expected(
    first: &Path, first_bytes: Vec<u8>, second: &Path, second_bytes: Vec<u8>, backup_dir: &Path,
    expected: (Option<Vec<u8>>, Option<Vec<u8>>),
    mut write: impl FnMut(&Path, Vec<u8>) -> Result<(), String>,
) -> Result<(), String> {
    let (old_first, old_second) = expected;
    if read_optional(first)? != old_first || read_optional(second)? != old_second {
        return Err("配置在读取后发生变化，请重新读取后再启用".into());
    }
    if old_first.is_some() { backup_file(first, backup_dir)?; }
    if old_second.is_some() { backup_file(second, backup_dir)?; }
    if read_optional(first)? != old_first || read_optional(second)? != old_second
    {
        return Err("配置在准备期间发生变化，请重新读取后再启用".into());
    }
    write(first, first_bytes.clone())?;
    if let Err(error) = write(second, second_bytes) {
        if std::fs::read(first).ok().as_ref() != Some(&first_bytes) {
            return Err(format!(
                "{error}；首个文件被外部修改，无法自动回滚，请从备份恢复"
            ));
        }
        match old_first {
            Some(bytes) => write(first, bytes),
            None => std::fs::remove_file(first).map_err(|e| e.to_string()),
        }.map_err(|rollback| format!("{error}；回滚失败：{rollback}"))?;
        return Err(format!("{error}；已回滚首个文件"));
    }
    Ok(())
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(format!("无法读取配置文件 {}", path.display())),
    }
}

/* ─────────────────────────── 通用工具 ─────────────────────────── */

fn claude_home() -> Result<PathBuf, String> {
    crate::creds::claude_dir().ok_or_else(|| "未找到 Claude 配置目录".into())
}
fn codex_home() -> Result<PathBuf, String> {
    crate::creds::codex_dir().ok_or_else(|| "未找到 Codex 配置目录".into())
}

/// 读 JSON → 修改 → 备份 → 原子写。文件不存在时如实报错，不代用户创建。
fn update_json(
    path: &Path,
    backup_dir: &Path,
    edit: impl FnOnce(&mut serde_json::Value) -> Result<(), String>,
) -> Result<(), String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    let mut value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("{} 不是有效 JSON：{e}", path.display()))?;
    edit(&mut value)?;
    backup_file(path, backup_dir)?;
    if std::fs::read(path).ok().as_deref() != Some(text.as_bytes()) {
        return Err("配置在读取后发生变化，请重试".into());
    }
    let bytes = serde_json::to_vec_pretty(&value)
        .map_err(|e| format!("{} 序列化失败：{e}", path.display()))?;
    write_atomic(path, bytes)
}

/// 写前备份：DPAPI 当前用户加密后写入 cli-backups/；熵为原文件名，便于同用户恢复。
fn backup_file(path: &Path, backup_dir: &Path) -> Result<(), String> {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return Err("备份失败：文件名无法识别".into());
    };
    std::fs::create_dir_all(backup_dir).map_err(|e| format!("创建备份目录失败：{e}"))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let target = backup_dir.join(format!("{name}.{stamp}.bak.dpapi"));
    let bytes = std::fs::read(path).map_err(|_| "读取备份源文件失败")?;
    let protected = crate::auth_snapshot::protect(&bytes, name.as_bytes(), true)?;
    std::fs::write(target, protected).map_err(|_| "保存加密备份失败")?;
    Ok(())
}

fn write_atomic(path: &Path, bytes: Vec<u8>) -> Result<(), String> {
    use std::io::Write;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let temporary = path.with_extension(format!("jolu-{}-{nonce}.tmp", std::process::id()));
    let write = || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(windows)]
    fn codex_auth_api_auth_round_trip_restores_complete_identity_and_encrypted_backups() {
        let root = std::env::temp_dir().join(format!("jolu-auth-roundtrip-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        std::fs::create_dir_all(&root).unwrap();
        let auth = root.join("auth.json"); let config = root.join("config.toml"); let backups = root.join("backups");
        let saved = serde_json::json!({"tokens":{"access_token":"fixture-a","refresh_token":"refresh-a","id_token":"id-a","account_id":"account-a"},"last_refresh":"2026-10-23T00:00:00Z"});
        std::fs::write(&auth, serde_json::to_vec(&saved).unwrap()).unwrap();
        assert!(!config.exists(), "官方 CLI 可尚未创建 config.toml");
        apply_codex_api_at(&auth, &config, "fixture-api", Some("https://gateway.example/v1"), &backups).unwrap();
        let api: serde_json::Value = serde_json::from_slice(&std::fs::read(&auth).unwrap()).unwrap();
        assert!(api.get("tokens").is_none());
        assert!(apply_codex_auth_at(&auth, &config, "fixture-a", &backups, None).is_err());
        assert_eq!(api, serde_json::from_slice::<serde_json::Value>(&std::fs::read(&auth).unwrap()).unwrap());
        apply_codex_auth_at(&auth, &config, "fixture-a", &backups, Some(&saved)).unwrap();
        assert_eq!(saved, serde_json::from_slice::<serde_json::Value>(&std::fs::read(&auth).unwrap()).unwrap());
        assert!(std::fs::read_to_string(&config).unwrap().contains("openai"));
        let mut auth_backups = 0;
        for file in std::fs::read_dir(&backups).unwrap().map(Result::unwrap) {
            assert_eq!(file.path().extension().unwrap(), "dpapi");
            let bytes = std::fs::read(file.path()).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("fixture-a"));
            if file.file_name().to_string_lossy().starts_with("auth.json.") {
                let restored = crate::auth_snapshot::protect(&bytes, b"auth.json", false).unwrap();
                assert!(serde_json::from_slice::<serde_json::Value>(&restored).is_ok());
                auth_backups += 1;
            }
        }
        assert_eq!(auth_backups, 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_prepared_pair_never_overwrites_a_newer_cli_login() {
        let root = std::env::temp_dir().join(format!("jolu-pair-stale-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        std::fs::create_dir_all(&root).unwrap();
        let auth = root.join("auth.json"); let config = root.join("config.toml");
        std::fs::write(&auth, b"new-login").unwrap();
        let result = write_pair(&auth, b"stale-result".to_vec(), &config, b"new-config".to_vec(), &root.join("backups"), (Some(b"old-login".to_vec()), None));
        assert!(result.is_err());
        assert_eq!(std::fs::read(&auth).unwrap(), b"new-login");
        assert!(!config.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_second_write_removes_newly_created_first_file() {
        let root = std::env::temp_dir().join(format!("jolu-pair-missing-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        std::fs::create_dir_all(&root).unwrap();
        let first = root.join("auth.json"); let second = root.join("config.toml");
        let result = write_pair_using(&first, b"created".to_vec(), &second, b"config".to_vec(), &root.join("backups"), |path, bytes| {
            if path == second { Err("injected failure".into()) } else { write_atomic(path, bytes) }
        });
        assert!(result.is_err());
        assert!(!first.exists() && !second.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn switch_lock_serializes_all_windows() {
        use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
        let active = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..4).map(|_| {
            let active = active.clone();
            std::thread::spawn(move || serialized_switch(|| {
                assert_eq!(active.fetch_add(1, Ordering::SeqCst), 0);
                std::thread::sleep(std::time::Duration::from_millis(3));
                assert_eq!(active.fetch_sub(1, Ordering::SeqCst), 1);
                Ok(())
            }).unwrap())
        }).collect();
        for handle in handles { handle.join().unwrap(); }
    }

    #[test]
    fn codex_selected_profile_changes_provider_without_losing_preferences() {
        let config = "profile='work'\nmodel='global-model'\n[profiles.work]\nmodel_provider='old'\nmodel='profile-model'\nmodel_reasoning_effort='high'\n[profiles.other]\nmodel_provider='untouched'\n";
        let changed = codex_provider_config(config, Some("https://target.example/v1")).unwrap();
        let parsed: toml::Value = toml::from_str(&changed).unwrap();
        assert_eq!(parsed["profiles"]["work"]["model_provider"].as_str(), Some("jolu"));
        assert_eq!(parsed["profiles"]["work"]["model"].as_str(), Some("profile-model"));
        assert_eq!(parsed["profiles"]["work"]["model_reasoning_effort"].as_str(), Some("high"));
        assert_eq!(parsed["profiles"]["other"]["model_provider"].as_str(), Some("untouched"));
        let official: toml::Value = toml::from_str(&codex_provider_config(&changed, None).unwrap()).unwrap();
        assert_eq!(official["profiles"]["work"]["model_provider"].as_str(), Some("openai"));
    }

    #[test]
    fn codex_official_provider_clears_endpoint_and_auth_overrides() {
        let config = "model_provider='openai'\n[model_providers.openai]\nname='my-openai'\nbase_url='https://old.example/v1'\nenv_key='OLD_KEY'\nexperimental_bearer_token='old-bearer'\nrequires_openai_auth=false\nrequest_max_retries=3\nhttp_headers={ Authorization='old-header' }\n";
        let parsed: toml::Value = toml::from_str(&codex_provider_config(config, None).unwrap()).unwrap();
        let provider = parsed["model_providers"]["openai"].as_table().unwrap();
        for key in ["base_url", "env_key", "experimental_bearer_token", "http_headers"] { assert!(!provider.contains_key(key), "{key} must not override saved auth"); }
        assert_eq!(provider["requires_openai_auth"].as_bool(), Some(true));
        assert_eq!(provider["request_max_retries"].as_integer(), Some(3));
    }

    #[test]
    fn ambiguous_codex_profile_or_store_is_rejected_before_auth_changes() {
        let root = std::env::temp_dir().join(format!("jolu-provider-reject-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        std::fs::create_dir_all(&root).unwrap();
        let auth = root.join("auth.json"); let config = root.join("config.toml");
        let original = br#"{"OPENAI_API_KEY":"original-key"}"#;
        std::fs::write(&auth, original).unwrap();
        for source in ["profile='missing'", "cli_auth_credentials_store='keyring'", "profile='work'\n[profiles.work.model_providers.old]\nbase_url='https://old.example/v1'"] {
            std::fs::write(&config, source).unwrap();
            assert!(apply_codex_api_at(&auth, &config, "replacement-key", Some("https://target.example/v1"), &root.join("backups")).is_err());
            assert_eq!(std::fs::read(&auth).unwrap(), original);
            assert_eq!(std::fs::read_to_string(&config).unwrap(), source);
        }
        assert!(!root.join("backups").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn environment_overrides_cannot_silently_redirect_official_auth() {
        assert!(validate_codex_environment(None, |name| (name == "OPENAI_BASE_URL").then(|| "https://old.example/v1".into())).is_err());
        assert!(validate_codex_environment(Some("https://target.example/v1"), |name| (name == "CODEX_API_KEY").then(|| "other-key".into())).is_err());
        assert!(validate_codex_environment(None, |_| None).is_ok());
    }

    #[test]
    fn codex_switch_clears_selected_gateway_and_keeps_unrelated_configuration() {
        let original="model = 'test-model'\nmodel_provider = 'old'\n[model_providers.old]\nbase_url = 'https://old.example/v1'\n";
        let gateway = codex_provider_config(original, Some("https://new.example/v1")).unwrap();
        let parsed = gateway.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(parsed["model_provider"].as_str(), Some("jolu"));
        assert_eq!(
            parsed["model_providers"]["jolu"]["base_url"].as_str(),
            Some("https://new.example/v1")
        );
        assert_eq!(
            parsed["model_providers"]["jolu"]["requires_openai_auth"].as_bool(),
            Some(true)
        );
        let official = codex_provider_config(&gateway, None)
            .unwrap()
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        assert_eq!(official["model_provider"].as_str(), Some("openai"));
        assert_eq!(official["model"].as_str(), Some("test-model"));
    }
    #[test]
    fn failed_second_write_restores_first_file() {
        let root = std::env::temp_dir().join(format!(
            "jolu-pair-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let a = root.join("auth.json");
        let b = root.join("config.toml");
        std::fs::write(&a, b"old-auth").unwrap();
        std::fs::write(&b, b"old-config").unwrap();
        let error = write_pair_using(
            &a,
            b"new-auth".to_vec(),
            &b,
            b"new-config".to_vec(),
            &root.join("backups"),
            |path, bytes| {
                if path == b {
                    Err("injected second write failure".into())
                } else {
                    write_atomic(path, bytes)
                }
            },
        )
        .unwrap_err();
        assert!(error.contains("已回滚"));
        assert_eq!(std::fs::read(&a).unwrap(), b"old-auth");
        assert_eq!(std::fs::read(&b).unwrap(), b"old-config");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn mismatched_oauth_is_rejected_without_replacing_refresh_token() {
        let path = std::env::temp_dir().join(format!(
            "jolu-oauth-test-{}-{}.json",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let value = br#"{"tokens":{"access_token":"account-a","refresh_token":"refresh-a"}}"#;
        std::fs::write(&path, value).unwrap();
        assert!(verify_current_auth(&path, "/tokens/access_token", "account-b").is_err());
        assert!(verify_current_auth(&path, "/tokens/access_token", "account-a").is_ok());
        assert_eq!(std::fs::read(&path).unwrap(), value);
        std::fs::remove_file(path).unwrap();
    }
}
