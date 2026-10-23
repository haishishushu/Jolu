//! 完整 Codex OAuth 授权快照，使用当前 Windows 用户的 DPAPI 保护。
//! 快照与解密结果不进入前端 DTO 或统计导出。
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;

pub fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS auth_snapshots (
        connection_id TEXT PRIMARY KEY REFERENCES connections(id) ON DELETE CASCADE,
        protected BLOB NOT NULL, updated_ms INTEGER NOT NULL);
        CREATE TRIGGER IF NOT EXISTS delete_auth_snapshot AFTER DELETE ON connections
        BEGIN DELETE FROM auth_snapshots WHERE connection_id=OLD.id; END;")
}

fn codex_path() -> Option<std::path::PathBuf> {
    crate::creds::codex_dir().map(|dir| dir.join("auth.json"))
}

/// 完整授权不可拆分；不从其他登录借用 refresh token 或 ID token。
pub(crate) fn complete_codex(value: &Value, expected: &str) -> Option<Value> {
    let tokens = value.get("tokens")?.as_object()?;
    let nonempty = |key: &str| tokens.get(key)?.as_str().filter(|v| !v.trim().is_empty());
    if expected.is_empty() || nonempty("access_token")? != expected { return None; }
    nonempty("refresh_token")?;
    nonempty("id_token")?;
    nonempty("account_id")?;
    let mut clean = serde_json::json!({"tokens": tokens});
    if let Some(refresh) = value.get("last_refresh").filter(|value| value.is_string()) {
        clean["last_refresh"] = refresh.clone();
    }
    Some(clean)
}

/// 只有已保存完整身份与本机新 token 的 account_id 相同，才允许轮换凭证。
pub fn can_refresh(conn: &Connection, id: &str, platform: &str, old: Option<&str>, new: &str) -> bool {
    if old == Some(new) { return true; }
    if platform != "codex" || crate::validation::isolated() { return false; }
    let Some(old) = old else { return false };
    let Ok(Some(saved)) = load(conn, id, old) else { return false };
    let Some(current) = codex_path().and_then(|path| std::fs::read(path).ok()).and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()) else { return false };
    same_codex_identity(&saved, &current, new)
}

fn codex_identity(value: &Value) -> Option<(String, String)> {
    let workspace = value.pointer("/tokens/account_id")?.as_str()?.to_owned();
    let claims = crate::login_detect::jwt_payload(value.pointer("/tokens/id_token")?.as_str()?)?;
    let auth = claims.get("https://api.openai.com/auth")?;
    if auth.get("chatgpt_account_id")?.as_str()? != workspace { return None; }
    let user = auth.get("chatgpt_user_id").or_else(|| auth.get("user_id")).or_else(|| claims.get("sub"))?.as_str()?;
    if user.is_empty() || workspace.is_empty() { return None; }
    Some((workspace, user.to_owned()))
}

fn same_codex_identity(saved: &Value, current: &Value, new: &str) -> bool {
    let Some(current) = complete_codex(current, new) else { return false };
    let Some(saved_identity) = codex_identity(saved) else { return false };
    codex_identity(&current).as_ref() == Some(&saved_identity)
}

pub fn capture(conn: &Connection, id: &str, platform: &str, secret: &str) -> Result<bool, String> {
    if platform != "codex" || crate::validation::isolated() { return Ok(false); }
    let Some(path) = codex_path() else { return Ok(false) };
    let Some(value) = std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()) else { return Ok(false) };
    let Some(bundle) = complete_codex(&value, secret) else { return Ok(false) };
    save(conn, id, &bundle)?;
    Ok(true)
}

/// 切 API 删除本机 tokens 前，保全已登记的订阅账号完整授权。
pub fn preserve_current(conn: &Connection, platform: &str) -> Result<(), String> {
    if platform != "codex" || crate::validation::isolated() { return Ok(()); }
    let Some(path) = codex_path() else { return Ok(()) };
    let Some(value) = std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()) else { return Ok(()) };
    preserve_current_value(conn, &value)
}

fn preserve_current_value(conn: &Connection, value: &Value) -> Result<(), String> {
    let Some(secret) = value.pointer("/tokens/access_token").and_then(Value::as_str) else { return Ok(()) };
    let current = complete_codex(value, secret).ok_or("当前订阅授权不完整，无法安全保存；请先重新登录并同步")?;
    if let Some(id) = crate::connections::find_by_secret(conn, "codex", "auth", secret)? { return save(conn, &id, &current); }
    let mut statement = conn.prepare("SELECT id,secret FROM connections WHERE platform='codex' AND kind='auth'").map_err(|_| "无法检查已保存授权")?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))
        .map_err(|_| "无法检查已保存授权")?.collect::<rusqlite::Result<Vec<_>>>().map_err(|_| "无法检查已保存授权")?;
    let mut has_unverifiable = false;
    for (id, old) in rows {
        let saved = old.as_deref().and_then(|old| load(conn, &id, old).ok()).flatten();
        let Some(saved) = saved else { has_unverifiable = true; continue };
        if codex_identity(&saved).is_none() || codex_identity(&current).is_none() {
            has_unverifiable = true;
            continue;
        }
        if !same_codex_identity(&saved, &current, secret) { continue; }
        conn.execute_batch("SAVEPOINT jolu_preserve_auth").map_err(|_| "无法开始授权保存")?;
        let result = (|| {
            save(conn, &id, &current)?;
            conn.execute("UPDATE connections SET secret=?2,masked=?3 WHERE id=?1", rusqlite::params![id, secret, crate::connections::mask(secret)]).map_err(|_| "无法保存轮换后的授权")?;
            Ok(())
        })();
        if let Err(error) = result {
            conn.execute_batch("ROLLBACK TO jolu_preserve_auth; RELEASE jolu_preserve_auth").map_err(|_| "保存授权失败且事务无法回滚")?;
            return Err(error);
        }
        conn.execute_batch("RELEASE jolu_preserve_auth").map_err(|_| "无法完成授权保存")?;
        return Ok(());
    }
    if has_unverifiable { return Err("无法核实当前订阅身份；请重新同步或重新添加完整授权后再切换".into()); }
    Ok(())
}

fn save(conn: &Connection, id: &str, bundle: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(bundle).map_err(|_| "完整授权无法编码")?;
    let protected = protect(&bytes, id.as_bytes(), true)?;
    conn.execute("INSERT INTO auth_snapshots(connection_id,protected,updated_ms) VALUES(?1,?2,?3)
        ON CONFLICT(connection_id) DO UPDATE SET protected=excluded.protected, updated_ms=excluded.updated_ms",
        rusqlite::params![id, protected, chrono::Utc::now().timestamp_millis()])
        .map_err(|_| "完整授权无法安全保存")?;
    Ok(())
}

pub fn load(conn: &Connection, id: &str, expected: &str) -> Result<Option<Value>, String> {
    let protected: Option<Vec<u8>> = conn.query_row("SELECT protected FROM auth_snapshots WHERE connection_id=?1", [id], |row| row.get(0))
        .optional().map_err(|_| "读取完整授权失败")?;
    let Some(protected) = protected else { return Ok(None) };
    let bytes = protect(&protected, id.as_bytes(), false)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "完整授权格式无效，请重新同步")?;
    complete_codex(&value, expected).map(Some).ok_or_else(|| "保存的完整授权与连接身份不一致，请重新同步".into())
}

/// DPAPI 默认限定当前用户，不使用允许同机其他用户解密的 LOCAL_MACHINE 标记。
#[cfg(windows)]
pub(crate) fn protect(bytes: &[u8], entropy: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB}};
    let blob = |bytes: &[u8]| CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr() as *mut u8 };
    if bytes.len() > 4 * 1024 * 1024 { return Err("完整授权超过大小限制".into()); }
    let input = blob(bytes);
    let entropy = blob(entropy);
    let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // Windows 用 LocalAlloc 分配输出，必须使用 LocalFree 释放。
    let ok = unsafe {
        if encrypt {
            CryptProtectData(&input, std::ptr::null(), &entropy, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        } else {
            CryptUnprotectData(&input, std::ptr::null_mut(), &entropy, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        }
    };
    if ok == 0 { return Err("Windows 无法保护或解锁授权；请使用原 Windows 用户重新同步".into()); }
    let result = if output.cbData == 0 { Vec::new() }
        else if output.pbData.is_null() { return Err("Windows 返回无效的授权数据".into()); }
        else { unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() } };
    if !output.pbData.is_null() { unsafe { LocalFree(output.pbData as *mut core::ffi::c_void); } }
    Ok(result)
}

#[cfg(not(windows))]
pub(crate) fn protect(_: &[u8], _: &[u8], _: bool) -> Result<Vec<u8>, String> {
    Err("完整授权存储仅支持 Windows 用户保护".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bundle() -> Value { serde_json::json!({"tokens":{"access_token":"a","refresh_token":"r","id_token":"h.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOiB7ImNoYXRncHRfdXNlcl9pZCI6ICJ1c2VyLWEiLCAiY2hhdGdwdF9hY2NvdW50X2lkIjogImFjY291bnQtYSJ9fQ.s","account_id":"account-a"},"last_refresh":"2026-10-23T00:00:00Z","OPENAI_API_KEY":"unrelated-api"}) }
    #[test]
    fn bundle_requires_complete_same_identity_and_drops_api_overrides() {
        let value = complete_codex(&bundle(), "a").unwrap();
        assert!(value.get("OPENAI_API_KEY").is_none());
        assert!(complete_codex(&bundle(), "b").is_none());
        for field in ["refresh_token", "id_token", "account_id"] {
            let mut incomplete = bundle(); incomplete["tokens"].as_object_mut().unwrap().remove(field);
            assert!(complete_codex(&incomplete, "a").is_none());
        }
    }
    #[test]
    fn refresh_accepts_token_rotation_but_rejects_another_account() {
        let saved = complete_codex(&bundle(), "a").unwrap();
        let mut rotated = bundle();
        rotated["tokens"]["access_token"] = serde_json::json!("new-access");
        rotated["tokens"]["refresh_token"] = serde_json::json!("new-refresh");
        assert!(same_codex_identity(&saved, &rotated, "new-access"));
        rotated["tokens"]["id_token"] = serde_json::json!("h.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOiB7ImNoYXRncHRfdXNlcl9pZCI6ICJ1c2VyLWIiLCAiY2hhdGdwdF9hY2NvdW50X2lkIjogImFjY291bnQtYSJ9fQ.s");
        assert!(!same_codex_identity(&saved, &rotated, "new-access"), "同工作区不同用户不能互相覆盖");
        rotated["tokens"]["account_id"] = serde_json::json!("account-b");
        assert!(!same_codex_identity(&saved, &rotated, "new-access"));
    }

    #[test]
    #[cfg(windows)]
    fn switching_preserves_rotated_refresh_token_before_overwriting_cli() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms) VALUES('0-legacy','codex','auth','legacy','legacy-token','***',0)", []).unwrap();
        db.execute("INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms) VALUES('a','codex','auth','test','a','***',1)", []).unwrap();
        save(&db, "a", &complete_codex(&bundle(), "a").unwrap()).unwrap();
        let mut rotated = bundle();
        rotated["tokens"]["access_token"] = serde_json::json!("rotated");
        rotated["tokens"]["refresh_token"] = serde_json::json!("rotated-refresh");
        preserve_current_value(&db, &rotated).unwrap();
        assert_eq!(load(&db, "a", "rotated").unwrap().unwrap()["tokens"]["refresh_token"], "rotated-refresh");
        assert_eq!(crate::connections::creds_of(&db, "a").unwrap().unwrap().secret.as_deref(), Some("rotated"));
    }

    #[test]
    #[cfg(windows)]
    fn empty_configuration_backup_round_trips() {
        let encrypted = protect(&[], b"config.toml", true).unwrap();
        assert_eq!(protect(&encrypted, b"config.toml", false).unwrap(), Vec::<u8>::new());
    }

    #[test]
    #[cfg(windows)]
    fn encrypted_bundle_round_trip_is_identity_bound_and_excluded_from_export() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms) VALUES('a','codex','auth','test','a','***',1)", []).unwrap();
        save(&db, "a", &complete_codex(&bundle(), "a").unwrap()).unwrap();
        let bytes: Vec<u8> = db.query_row("SELECT protected FROM auth_snapshots", [], |row| row.get(0)).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("refresh_token"));
        assert!(protect(&bytes, b"wrong-connection", false).is_err());
        assert!(load(&db, "a", "b").is_err());
        assert_eq!(load(&db, "a", "a").unwrap().unwrap()["tokens"]["refresh_token"], "r");
        let export = crate::db::export_backup(&db, None, None, None).unwrap();
        assert!(!export.contains("refresh_token") && !export.contains("auth_snapshots") && !export.contains("protected"));
        crate::connections::remove(&db, "a").unwrap();
        assert!(load(&db, "a", "a").unwrap().is_none());
    }
}
