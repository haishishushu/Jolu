//! 本机实际生效连接快照。先确定 CLI 使用的认证来源，再匹配已保存账号。
//! 不因当前来源尚未登记而回退到另一个磁盘凭证；原值只在后端短暂使用。

pub(crate) struct LocalConnection {
    pub platform: String,
    pub kind: String,
    pub secret: String,
    pub base_url: Option<String>,
}

/// 隔离入口先返回，不能为了判断当前账号读取真实配置或环境凭证。
pub(crate) fn read_local() -> Vec<LocalConnection> {
    if crate::validation::isolated() { return vec![]; }
    let mut local = Vec::new();
    if let Some(kind) = crate::creds::detect("claude").kind {
        if let Some((secret, base_url)) = crate::creds::read_local_pair("claude", &kind) {
            local.push(LocalConnection { platform: "claude".into(), kind, secret, base_url });
        }
    }
    if let Some(dir) = crate::creds::codex_dir() {
        let config = match std::fs::read_to_string(dir.join("config.toml")) {
            Ok(config) => Some(config),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(String::new()),
            Err(_) => None,
        };
        if let Some(config) = config {
            let auth = std::fs::read(dir.join("auth.json")).ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
            if let Some(current) = codex_current(&config, auth.as_ref(), |key| std::env::var(key).ok()) {
                local.push(current);
            }
        }
    }
    local
}

fn codex_current(config: &str, auth: Option<&serde_json::Value>, env: impl Fn(&str) -> Option<String>) -> Option<LocalConnection> {
    let config: toml::Value = toml::from_str(config).ok()?;
    let profile = match config.get("profile") {
        Some(name) => Some(config.get("profiles")?.get(name.as_str()?)?),
        None => None,
    };
    let selected = profile.and_then(|profile| profile.get("model_provider")).or_else(|| config.get("model_provider"));
    let name = match selected { Some(name) => name.as_str()?, None => "openai" };
    let provider = config.get("model_providers").and_then(|providers| providers.get(name));
    let text = |key: &str| provider.and_then(|value| value.get(key)).and_then(toml::Value::as_str)
        .map(str::trim).filter(|text| !text.is_empty());
    let clean = |value: Option<String>| value.map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    let base_url = text("base_url").map(str::to_string)
        .or_else(|| (name == "openai").then(|| clean(env("OPENAI_BASE_URL"))).flatten());
    if name != "openai" && base_url.is_none() { return None; }
    let make = |kind: &str, secret: String| LocalConnection {
        platform: "codex".into(), kind: kind.into(), secret,
        base_url: if kind == "api" { base_url.clone() } else { None },
    };
    // Provider bearer auth 优先；缺失显式 env_key 必须停止，不能借用另一个来源。
    if let Some(key) = text("env_key") { return clean(env(key)).map(|secret| make("api", secret)); }
    if let Some(secret) = text("experimental_bearer_token") { return Some(make("api", secret.to_string())); }
    // 命令型 provider auth 尚无法确认实际用户，不能冒认磁盘 OAuth。
    if provider.is_some_and(|provider| provider.get("auth").is_some()) { return None; }
    let requires_auth = provider.and_then(|provider| provider.get("requires_openai_auth"))
        .and_then(toml::Value::as_bool).unwrap_or(name == "openai");
    if !requires_auth { return None; }
    // keyring/auto/ephemeral 的实际授权不一定在 auth.json，不能冒认遗留磁盘凭证。
    let store = profile.and_then(|profile| profile.get("cli_auth_credentials_store"))
        .or_else(|| config.get("cli_auth_credentials_store"));
    if store.is_some_and(|store| store.as_str() != Some("file")) { return None; }
    let stored = |pointer: &str| auth.and_then(|auth| auth.pointer(pointer)).and_then(serde_json::Value::as_str)
        .map(str::trim).filter(|value| !value.is_empty()).map(str::to_string);
    // OPENAI_API_KEY 是 login --with-api-key 的输入，不是默认运行时覆盖项。
    // CODEX_API_KEY 仅部分执行入口启用，不能把 Jolu 进程环境当作交互 CLI 身份。
    if clean(env("CODEX_API_KEY")).is_some() { return None; }
    if let Some(key) = stored("/OPENAI_API_KEY") {
        return Some(make("api", key));
    }
    stored("/tokens/access_token").map(|secret| make("auth", secret))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(debug_assertions)]
    fn isolated_mode_never_reports_real_environment_accounts() {
        const CHILD: &str = "JOLU_CURRENT_CONNECTION_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() { assert!(read_local().is_empty()); return; }
        let root = std::env::temp_dir().join(format!("jolu-current-isolation-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".acceptance-profile"), "test").unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "current_connection::tests::isolated_mode_never_reports_real_environment_accounts"])
            .env(CHILD, "1").env("JOLU_TEST_DIR", &root)
            .env("ANTHROPIC_API_KEY", "synthetic-current-key")
            .env("OPENAI_API_KEY", "synthetic-current-key")
            .output().unwrap();
        std::fs::remove_file(root.join(".acceptance-profile")).unwrap();
        std::fs::remove_dir(&root).unwrap();
        assert!(result.status.success(), "隔离当前账号识别失败");
    }
    fn auth() -> serde_json::Value { serde_json::json!({"OPENAI_API_KEY":"stored-key","tokens":{"access_token":"oauth"}}) }
    #[test]
    fn selected_missing_env_key_does_not_fall_back_to_stored_key_or_oauth() {
        let config = "model_provider='gateway'\n[model_providers.gateway]\nbase_url='https://gateway.test/v1'\nenv_key='GATEWAY_KEY'";
        assert!(codex_current(config, Some(&auth()), |_| None).is_none());
        let current = codex_current(config, Some(&auth()), |name| (name == "GATEWAY_KEY").then(|| "selected-key".into())).unwrap();
        assert_eq!(current.secret, "selected-key");
        assert_eq!(current.base_url.as_deref(), Some("https://gateway.test/v1"));
    }
    #[test]
    fn anonymous_custom_provider_never_claims_saved_oauth() {
        let config = "model_provider='local'\n[model_providers.local]\nbase_url='http://localhost:1234/v1'\nrequires_openai_auth=false";
        assert!(codex_current(config, Some(&auth()), |_| None).is_none());
    }
    #[test]
    fn default_profile_selects_its_provider_before_global_provider() {
        let config = "profile='work'\n[profiles.work]\nmodel_provider='gateway'\n[model_providers.gateway]\nbase_url='https://gateway.test/v1'\nenv_key='WORK_KEY'";
        assert!(codex_current(config, Some(&auth()), |_| None).is_none());
    }
    #[test]
    fn official_source_prefers_api_and_uses_auth_only_when_no_api_is_active() {
        assert_eq!(codex_current("", Some(&auth()), |_| None).unwrap().kind, "api");
        let auth_only = serde_json::json!({"tokens":{"access_token":"oauth"}});
        assert_eq!(codex_current("", Some(&auth_only), |_| None).unwrap().kind, "auth");
    }
    #[test]
    fn non_file_auth_storage_never_claims_stale_auth_json() {
        for config in [
            "cli_auth_credentials_store='keyring'",
            "cli_auth_credentials_store='auto'",
            "cli_auth_credentials_store='ephemeral'",
            "cli_auth_credentials_store='file'\nprofile='work'\n[profiles.work]\ncli_auth_credentials_store='keyring'",
        ] {
            assert!(codex_current(config, Some(&auth()), |_| None).is_none(), "{config}");
        }
        let file_profile = "cli_auth_credentials_store='keyring'\nprofile='work'\n[profiles.work]\ncli_auth_credentials_store='file'";
        assert_eq!(codex_current(file_profile, Some(&auth()), |_| None).unwrap().secret, "stored-key");
    }

    #[test]
    fn login_input_environment_does_not_override_saved_oauth() {
        let auth_only = serde_json::json!({"tokens":{"access_token":"oauth"}});
        let current = codex_current("", Some(&auth_only), |name| (name == "OPENAI_API_KEY").then(|| "login-input".into())).unwrap();
        assert_eq!(current.kind, "auth");
        assert_eq!(current.secret, "oauth");
        assert!(codex_current("", Some(&auth_only), |name| (name == "CODEX_API_KEY").then(|| "exec-only".into())).is_none());
    }
}
