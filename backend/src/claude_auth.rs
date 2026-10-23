//! Claude OAuth 身份预检。网络请求必须在取得数据库锁之前完成。
use serde_json::Value;
use std::{io::Read, path::{Path, PathBuf}};
const LIMIT: usize = 256 * 1024;
const CONFIG_LIMIT: usize = 4 * 1024 * 1024;
const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";

/// 不实现 Debug/Serialize：其中含完整授权，只在后端预检与提交之间传递。
pub(crate) struct PreparedClaude {
    pub(crate) files: Vec<(PathBuf, Option<Vec<u8>>)>,
    bundle: Option<Value>,
}

fn nonempty<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer)?.as_str().filter(|text| !text.trim().is_empty())
}

pub(crate) fn identity(bundle: &Value) -> Option<(&str, &str)> {
    Some((nonempty(bundle, "/oauthAccount/accountUuid")?, nonempty(bundle, "/oauthAccount/organizationUuid")?))
}

pub(crate) fn complete(bundle: &Value, expected: &str) -> Option<Value> {
    if bundle.get("type")?.as_str()? != "claude_oauth_v1"
        || nonempty(bundle, "/claudeAiOauth/accessToken")? != expected
        || expected.is_empty() { return None; }
    nonempty(bundle, "/claudeAiOauth/refreshToken")?;
    bundle.pointer("/claudeAiOauth/expiresAt")?.as_i64().filter(|time| *time > 0)?;
    let scopes = bundle.pointer("/claudeAiOauth/scopes")?.as_array()?;
    if !scopes.iter().all(Value::is_string) || !scopes.iter().any(|scope| scope.as_str() == Some("user:profile")) { return None; }
    let (account, org) = identity(bundle)?;
    if nonempty(bundle, "/verifiedIdentity/accountUuid")? != account
        || nonempty(bundle, "/verifiedIdentity/organizationUuid")? != org { return None; }
    Some(bundle.clone())
}

fn verified_bundle(credentials: &Value, profile: &Value, response: &Value) -> Result<Value, String> {
    let account = nonempty(response, "/account/uuid").ok_or("Claude 身份响应缺少账号标识")?;
    let org = nonempty(response, "/organization/uuid").ok_or("Claude 身份响应缺少组织标识")?;
    let email = nonempty(response, "/account/email").ok_or("Claude 身份响应缺少邮箱")?;
    if nonempty(profile, "/oauthAccount/accountUuid") != Some(account)
        || nonempty(profile, "/oauthAccount/organizationUuid") != Some(org) {
        return Err("Claude 本机账号资料与凭据身份不一致，请在原工具重新登录后同步".into());
    }
    let mut oauth_account = profile["oauthAccount"].clone();
    oauth_account["emailAddress"] = Value::String(email.into());
    let bundle = serde_json::json!({"type":"claude_oauth_v1", "claudeAiOauth":credentials["claudeAiOauth"],
        "oauthAccount":oauth_account,"verifiedIdentity":{"accountUuid":account,"organizationUuid":org}});
    let token = nonempty(credentials, "/claudeAiOauth/accessToken").ok_or("Claude 本机授权缺少 access token")?;
    complete(&bundle, token).ok_or_else(|| "Claude 本机授权不完整，请在原工具重新登录后同步".into())
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("无法读取 Claude 配置".into()),
    };
    read_limited(file, CONFIG_LIMIT).map(Some)
}

fn read_limited(reader: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader.take((limit + 1) as u64).read_to_end(&mut bytes).map_err(|_| "Claude 身份或配置读取失败")?;
    if bytes.len() > limit { return Err("Claude 身份或配置超过大小限制".into()); }
    Ok(bytes)
}

fn fetch_profile(token: &str) -> Result<Value, String> {
    if crate::validation::isolated() { return Err("隔离验收不查询真实 Claude 身份".into()); }
    let client = crate::net::blocking_builder()
        .redirect(reqwest::redirect::Policy::none()).timeout(std::time::Duration::from_secs(10))
        .build().map_err(|_| "无法创建 Claude 身份查询")?;
    let response = client.get(PROFILE_URL).bearer_auth(token)
        .header("User-Agent", "claude-code/2.1.309")
        .header("Content-Type", "application/json").header("Cache-Control", "no-cache")
        .send().map_err(|_| "Claude 身份查询失败，请检查网络后重试")?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) { return response_profile(status, std::io::empty()); }
    if response.content_length().is_some_and(|size| size > LIMIT as u64) { return Err("Claude 身份响应超过大小限制".into()); }
    response_profile(status, response)
}

fn response_profile(status: u16, body: impl Read) -> Result<Value, String> {
    if !(200..300).contains(&status) { return Err(format!("Claude 身份查询未成功（HTTP {status}）")); }
    serde_json::from_slice(&read_limited(body, LIMIT)?).map_err(|_| "Claude 身份响应格式无效".into())
}

/// 仅用户添加/同步/切换订阅时调用；调用方尚未持有数据库或 CLI 写锁。
pub(crate) fn prepare_current() -> Result<PreparedClaude, String> {
    prepare_with_saved(Vec::new())
}

/// 已保护快照中完全相同 token 的身份证据可复用；token 轮换仍须联网核实。
pub(crate) fn prepare_with_saved(saved: Vec<Value>) -> Result<PreparedClaude, String> {
    if crate::validation::isolated() { return Err("隔离验收不读取或查询真实 Claude 授权".into()); }
    let dir = crate::creds::claude_dir().ok_or("未找到 Claude 配置目录")?;
    let profile = if std::env::var_os("CLAUDE_CONFIG_DIR").is_some_and(|v| !v.is_empty()) {
        dir.join(".claude.json")
    } else { crate::creds::home().ok_or("未找到用户目录")?.join(".claude.json") };
    prepare_at([dir.join(".credentials.json"), profile, dir.join("settings.json")], |token| profile_or_fetch(&saved, token, fetch_profile))
}

fn profile_or_fetch(saved: &[Value], token: &str, fetch: impl FnOnce(&str) -> Result<Value, String>) -> Result<Value, String> {
    for value in saved {
        if complete(value, token).is_some() {
            return Ok(serde_json::json!({"account":{"uuid":value["oauthAccount"]["accountUuid"],"email":value["oauthAccount"]["emailAddress"]},
                "organization":{"uuid":value["oauthAccount"]["organizationUuid"]}}));
        }
    }
    fetch(token)
}

fn prepare_at(paths: [PathBuf; 3], query: impl FnOnce(&str) -> Result<Value, String>) -> Result<PreparedClaude, String> {
    let files: Vec<_> = paths.into_iter().map(|path| read_optional(&path).map(|bytes| (path, bytes))).collect::<Result<_, _>>()?;
    let bundle = if let Some(bytes) = &files[0].1 {
        let credentials: Value = serde_json::from_slice(bytes).map_err(|_| "Claude 凭据格式无效")?;
        if credentials.get("claudeAiOauth").is_some() {
            let token = nonempty(&credentials, "/claudeAiOauth/accessToken").ok_or("Claude 授权不完整")?;
            nonempty(&credentials, "/claudeAiOauth/refreshToken").ok_or("Claude 授权缺少完整刷新凭据，请重新登录")?;
            if !credentials.pointer("/claudeAiOauth/scopes").and_then(Value::as_array)
                .is_some_and(|scopes| scopes.iter().any(|scope| scope.as_str() == Some("user:profile"))) {
                return Err("Claude 授权缺少身份查询权限，请在原工具重新登录后同步".into());
            }
            let profile: Value = serde_json::from_slice(files[1].1.as_deref().ok_or("Claude 账号资料缺失，请重新登录")?).map_err(|_| "Claude 账号资料格式无效")?;
            Some(verified_bundle(&credentials, &profile, &query(token)?)?)
        } else { None }
    } else { None };
    let prepared = PreparedClaude { files, bundle };
    prepared.check_unchanged()?;
    Ok(prepared)
}

impl PreparedClaude {
    pub(crate) fn check_unchanged(&self) -> Result<(), String> {
        for (path, bytes) in &self.files {
            if &read_optional(path)? != bytes { return Err("Claude 配置在身份核实期间发生变化，请重试".into()); }
        }
        Ok(())
    }
    pub(crate) fn require(&self, expected: Option<&str>) -> Result<&Value, String> {
        let bundle = self.bundle.as_ref().ok_or("未找到 Claude 完整订阅授权，请在原工具登录后同步")?;
        if expected.is_some_and(|token| nonempty(bundle, "/claudeAiOauth/accessToken") != Some(token)) {
            return Err("Claude 当前授权与所选连接不一致，不能复制其他账号的凭据".into());
        }
        Ok(bundle)
    }
    /// 允许同 token 旧记录升级；不同 token 必须有已保护快照证明两项身份一致。
    pub(crate) fn sync(&self, conn: &rusqlite::Connection, id: &str) -> Result<bool, String> {
        self.check_unchanged()?;
        let bundle = self.require(None)?;
        let token = nonempty(bundle, "/claudeAiOauth/accessToken").ok_or("Claude 授权不完整")?;
        let current = crate::connections::creds_of(conn, id).map_err(|_| "无法读取连接")?.ok_or("连接不存在")?;
        if current.platform != "claude" || current.kind != "auth" { return Err("连接不是 Claude 订阅账号".into()); }
        let changed = current.secret.as_deref() != Some(token);
        if changed {
            let saved = crate::auth_snapshot::load(conn, id, current.secret.as_deref().unwrap_or_default())?
                .ok_or("旧连接缺少完整身份快照，请先切回原账号同步，或将当前账号新增为连接")?;
            if identity(&saved).is_none() || identity(&saved) != identity(bundle) {
                return Err("Claude 当前登录属于其他账号或组织，已保留原连接；请新增连接".into());
            }
        }
        conn.execute_batch("SAVEPOINT jolu_claude_sync").map_err(|_| "无法开始 Claude 授权保存")?;
        let result = (|| {
            crate::auth_snapshot::save(conn, id, bundle)?;
            conn.execute("UPDATE connections SET secret=?2,masked=?3,account_email=?4,last_sync_ms=?5 WHERE id=?1",
                rusqlite::params![id, token, crate::connections::mask(token), bundle["oauthAccount"]["emailAddress"].as_str(), chrono::Utc::now().timestamp_millis()])
                .map_err(|_| "无法更新 Claude 授权")?;
            self.check_unchanged()?;
            Ok(())
        })();
        if let Err(error) = result {
            conn.execute_batch("ROLLBACK TO jolu_claude_sync; RELEASE jolu_claude_sync").map_err(|_| "Claude 授权保存失败且无法回滚")?;
            return Err(error);
        }
        conn.execute_batch("RELEASE jolu_claude_sync").map_err(|_| "无法完成 Claude 授权保存")?;
        Ok(changed)
    }
    /// 仅在要覆盖订阅凭据前保全当前账号；API 切换不覆盖订阅文件，无需联网。
    pub(crate) fn preserve(&self, conn: &rusqlite::Connection) -> Result<(), String> {
        self.check_unchanged()?;
        let Some(bundle) = &self.bundle else { return Ok(()) };
        let token = nonempty(bundle, "/claudeAiOauth/accessToken").ok_or("Claude 授权不完整")?;
        if let Some(id) = crate::connections::find_by_secret(conn, "claude", "auth", token)? { self.sync(conn, &id)?; return Ok(()); }
        let mut statement = conn.prepare("SELECT id,secret FROM connections WHERE platform='claude' AND kind='auth'").map_err(|_| "无法检查 Claude 账号")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))
            .map_err(|_| "无法检查 Claude 账号")?.collect::<rusqlite::Result<Vec<_>>>().map_err(|_| "无法检查 Claude 账号")?;
        let mut unknown = false;
        for (id, secret) in rows {
            let saved = secret.as_deref().and_then(|secret| crate::auth_snapshot::load(conn, &id, secret).ok()).flatten();
            let Some(saved) = saved else { unknown = true; continue; };
            if identity(&saved) == identity(bundle) { self.sync(conn, &id)?; return Ok(()); }
        }
        if unknown { return Err("存在未保存完整身份的 Claude 账号，请先同步当前账号或新增连接，再切换".into()); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn credentials(token: &str) -> Value {
        serde_json::json!({"claudeAiOauth":{"accessToken":token,"refreshToken":"fixture-refresh","expiresAt":4102444800000i64,"scopes":["user:profile"]},"unrelated":"keep"})
    }
    fn profile() -> Value {
        serde_json::json!({"oauthAccount":{"accountUuid":"account-a","organizationUuid":"org-a","emailAddress":"old@example.test","displayName":"name"},"theme":"dark"})
    }
    fn response() -> Value {
        serde_json::json!({"account":{"uuid":"account-a","email":"current@example.test"},"organization":{"uuid":"org-a"}})
    }
    struct Fixture { root: PathBuf, paths: [PathBuf; 3] }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("jolu-claude-auth-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap()));
            std::fs::create_dir_all(&root).unwrap();
            let paths = [root.join(".credentials.json"), root.join(".claude.json"), root.join("settings.json")];
            std::fs::write(&paths[0], serde_json::to_vec(&credentials("access-a")).unwrap()).unwrap();
            std::fs::write(&paths[1], serde_json::to_vec(&profile()).unwrap()).unwrap();
            Self { root, paths }
        }
        fn prepare(&self) -> PreparedClaude { prepare_at(self.paths.clone(), |_| Ok(response())).unwrap() }
    }
    impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); } }
    fn insert(db: &rusqlite::Connection, id: &str, token: &str) {
        db.execute("INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms) VALUES(?1,'claude','auth','fixture',?2,'***',0)", rusqlite::params![id,token]).unwrap();
    }
    #[test]
    fn verified_identity_binds_complete_tokens_and_only_profile_fragment() {
        let bundle = verified_bundle(&credentials("access-a"), &profile(), &response()).unwrap();
        assert_eq!(bundle["claudeAiOauth"]["refreshToken"], "fixture-refresh");
        assert_eq!(bundle["oauthAccount"]["emailAddress"], "current@example.test");
        assert!(bundle.get("theme").is_none() && bundle.get("unrelated").is_none());
        assert_eq!(bundle["oauthAccount"]["displayName"], "name");
    }
    #[test]
    fn response_cannot_bind_another_account_or_organization_even_with_same_email() {
        for (group, key) in [("account", "uuid"), ("organization", "uuid")] {
            let mut reply = response(); reply[group][key] = serde_json::json!("different");
            assert!(verified_bundle(&credentials("access-a"), &profile(), &reply).is_err());
        }
        let mut incomplete = credentials("access-a");
        incomplete["claudeAiOauth"].as_object_mut().unwrap().remove("refreshToken");
        assert!(verified_bundle(&incomplete, &profile(), &response()).is_err());
        assert!(verified_bundle(&credentials("access-a"), &profile(), &serde_json::json!({"error":"private upstream body"})).is_err());
    }
    #[test]
    fn response_errors_and_limits_never_echo_private_body() {
        for status in [302, 401, 403, 429, 500] {
            let error = response_profile(status, b"private-body-token-email".as_slice()).unwrap_err();
            assert!(error.contains(&status.to_string()) && !error.contains("private"));
        }
        assert!(response_profile(200, vec![b' '; LIMIT + 1].as_slice()).unwrap_err().contains("大小限制"));
        assert!(response_profile(200, b"private-invalid-json".as_slice()).unwrap_err().contains("格式无效"));
        assert!(read_limited(vec![b' '; LIMIT + 1].as_slice(), CONFIG_LIMIT).is_ok());
    }
    #[test]
    fn preflight_failure_or_concurrent_change_leaves_files_untouched() {
        let fixture = Fixture::new();
        let before = std::fs::read(&fixture.paths[0]).unwrap();
        assert!(prepare_at(fixture.paths.clone(), |_| Err("synthetic network failure".into())).is_err());
        assert_eq!(std::fs::read(&fixture.paths[0]).unwrap(), before);
        let changed = fixture.paths[0].clone();
        let result = prepare_at(fixture.paths.clone(), |_| {
            std::fs::write(&changed, serde_json::to_vec(&credentials("rotated-during-query")).unwrap()).unwrap();
            Ok(response())
        });
        assert!(result.is_err());
        assert!(std::fs::read_to_string(changed).unwrap().contains("rotated-during-query"));
    }
    #[test]
    fn missing_credentials_do_not_query_or_borrow_profile() {
        let fixture = Fixture::new();
        std::fs::remove_file(&fixture.paths[0]).unwrap();
        let prepared = prepare_at(fixture.paths.clone(), |_| panic!("must not query without credentials")).unwrap();
        assert!(prepared.require(None).is_err());
    }
    #[test]
    fn exact_verified_token_can_preserve_expired_current_login_without_network() {
        let mut saved = verified_bundle(&credentials("access-a"), &profile(), &response()).unwrap();
        saved["claudeAiOauth"]["expiresAt"] = serde_json::json!(1);
        let cached = profile_or_fetch(&[saved.clone()], "access-a", |_| panic!("same-token identity needs no request")).unwrap();
        assert_eq!(cached["account"]["uuid"], "account-a");
        assert!(profile_or_fetch(&[saved], "rotated", |_| Err("needs live identity".into())).is_err());
    }
    #[test]
    #[cfg(windows)]
    fn legacy_upgrade_rotation_and_identity_rejection_are_atomic() {
        let fixture = Fixture::new();
        let db = crate::db::open(Path::new(":memory:")).unwrap();
        insert(&db, "legacy-other", "different-legacy-token");
        insert(&db, "a", "access-a");
        let prepared = fixture.prepare();
        assert!(!prepared.sync(&db, "a").unwrap());
        assert!(prepared.sync(&db, "legacy-other").is_err());
        assert!(crate::auth_snapshot::load(&db, "a", "access-a").unwrap().is_some());
        std::fs::write(&fixture.paths[0], serde_json::to_vec(&credentials("rotated-a")).unwrap()).unwrap();
        fixture.prepare().preserve(&db).unwrap();
        assert!(crate::auth_snapshot::load(&db, "a", "rotated-a").unwrap().is_some());
        // 同邮箱不同账号/组织仍拒绝覆盖，原快照及凭据保留。
        let mut other_profile = profile(); other_profile["oauthAccount"]["organizationUuid"] = serde_json::json!("org-b");
        let mut other_response = response(); other_response["organization"]["uuid"] = serde_json::json!("org-b");
        std::fs::write(&fixture.paths[0], serde_json::to_vec(&credentials("access-b")).unwrap()).unwrap();
        std::fs::write(&fixture.paths[1], serde_json::to_vec(&other_profile).unwrap()).unwrap();
        let other = prepare_at(fixture.paths.clone(), |_| Ok(other_response)).unwrap();
        assert!(other.sync(&db, "a").is_err());
        assert_eq!(crate::connections::creds_of(&db, "a").unwrap().unwrap().secret.as_deref(), Some("rotated-a"));
        assert!(crate::auth_snapshot::load(&db, "a", "rotated-a").unwrap().is_some());
        assert!(other.sync(&db, "legacy-other").is_err());
        let export = crate::db::export_backup(&db, None, None, None).unwrap();
        assert!(!export.contains("refreshToken") && !export.contains("verifiedIdentity"));
    }
    #[test]
    #[cfg(debug_assertions)]
    fn isolation_rejects_before_real_files_or_network() {
        if std::env::var_os("JOLU_CLAUDE_ISOLATION_CHILD").is_none() {
            let fixture = Fixture::new();
            std::fs::write(fixture.root.join(".acceptance-profile"), "fixture").unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "claude_auth::tests::isolation_rejects_before_real_files_or_network"])
                .env("JOLU_CLAUDE_ISOLATION_CHILD", "1").env("JOLU_TEST_DIR", &fixture.root)
                .env("CLAUDE_CONFIG_DIR", fixture.root.join("must-not-read"))
                .output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
            return;
        }
        assert!(prepare_current().err().unwrap().contains("隔离验收"));
        assert!(fetch_profile("synthetic-never-sent").unwrap_err().contains("隔离验收"));
    }
}
