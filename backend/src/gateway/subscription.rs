//! 订阅授权只能来自选中的连接。明文仅在后端短暂持有。
use rusqlite::Connection;
use serde_json::Value;

pub(crate) struct BoundAuth {
    pub secret: String,
    pub bundle: Value,
}

/// 只保存上游真实提供的数字；不保存 prompt、回答正文或凭据。
pub(crate) fn record_usage(deps: &std::sync::Arc<crate::proxy::ProxyDeps>, entry: &super::route::PoolEntry, response: &Value, elapsed_ms: i64) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let now = chrono::Utc::now();
    let id = response.get("id").and_then(Value::as_str).map(str::to_owned)
        .unwrap_or_else(|| format!("{}-{}", now.timestamp_micros(), SEQUENCE.fetch_add(1, Ordering::Relaxed)));
    let usage = &response["usage"];
    let n = |key: &str| usage.get(key).and_then(Value::as_i64).filter(|v| *v >= 0);
    let cached = if matches!(entry.platform.as_str(), "codex" | "opencode") {
        usage.pointer("/input_tokens_details/cached_tokens").and_then(Value::as_i64).filter(|v| *v >= 0)
    } else { n("cache_read_input_tokens") };
    let total = n("total_tokens").or_else(|| {
        let base = n("input_tokens")?.checked_add(n("output_tokens")?)?;
        if matches!(entry.platform.as_str(), "codex" | "opencode") { Some(base) }
        else { base.checked_add(cached?)?.checked_add(n("cache_creation_input_tokens")?) }
    });
    let record = crate::db::RequestRecord {
        platform: entry.platform.clone(), source: "gateway-subscription".into(),
        dedup_key: format!("{}:{}:{id}",entry.connection_id.len(), entry.connection_id), session_id: None,
        ts: now.timestamp_millis(), model: response.get("model").and_then(Value::as_str).map(str::to_owned),
        input_tokens: n("input_tokens"), output_tokens: n("output_tokens"), cache_read_tokens: cached,
        cache_write_tokens: n("cache_creation_input_tokens"), total_tokens: total, effort: None,
    };
    let saved = deps.db.lock().map_err(|_| ()).and_then(|mut conn| crate::db::insert_gateway_record(&mut conn, &record, &entry.connection_id, elapsed_ms).map_err(|_| ()));
    if saved.is_err() { eprintln!("[网关] 订阅调用已完成，但本次用量记录保存失败"); }
}

#[derive(serde::Serialize)]
pub(crate) struct Capability {
    pub connection_id: String,
    pub can_serve: bool,
    pub state: &'static str,
    pub reason: String,
}

pub(crate) fn capability(conn: &Connection, id: &str) -> Capability {
    let mut result = Capability {
        connection_id: id.into(),
        can_serve: false,
        state: "unsupported",
        reason: "暂未支持模型转发".into(),
    };
    let row: Result<(String, String), _> = conn.query_row(
        "SELECT platform,kind FROM connections WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    );
    let Ok((platform, kind)) = row else {
        result.reason = "账号不存在，请刷新".into();
        return result;
    };
    if kind == "api" {
        result.can_serve = true;
        result.state = "ready";
        result.reason = "可通过 API 接入".into();
        return result;
    }
    if matches!(platform.as_str(), "grok" | "gemini" | "workbuddy" | "zcode" | "qoder" | "trae") {
        let bound = match platform.as_str() {
            "grok" => super::adapter::grok::bound(conn,id).map(|_|()),
            "workbuddy" => super::adapter::workbuddy::bound(conn,id).map(|_|()),
            "zcode" => super::adapter::zcode::bound(conn,id).map(|_|()),
            "qoder" => super::adapter::qoder::bound(conn,id).map(|_|()),
            "trae" => super::adapter::trae::bound(conn,id).map(|_|()),
            _ => super::adapter::gemini_subscription::bound(conn,id).map(|_|()),
        };
        match bound {
            Ok(_) => { result.can_serve = true; result.state = "ready"; result.reason = match platform.as_str() {
                "grok" => "支持 API 调用 · Chat / Responses，真实账号待验收",
                "workbuddy" => "支持 API 调用 · Chat，真实账号待验收",
                "zcode" => "支持 API 调用 · 个人 Coding Plan / Messages，真实账号待验收",
                "qoder" => "已接入隔离 CLI · Chat 非流式文本 / auto，真实账号待验收",
                "trae" => "已接入账号目录 · Chat / Messages 流式与工具转换；真实账号待验收",
                _ => "支持 API 调用 · Gemini 非流式 generateContent，真实账号待验收",
            }.into(); }
            Err(reason) => { result.state = "needs_auth"; result.reason = reason; }
        }
        return result;
    }
    if !matches!(platform.as_str(), "claude" | "codex" | "opencode") {
        result.reason = "暂未支持订阅转发；已保存登录不代表可以调用模型".into();
        return result;
    }
    match load(conn, id, &platform) {
        Ok(_) => {
            result.can_serve = true;
            result.state = "ready";
            result.reason = if platform == "claude" {
                "支持 API 调用 · Messages 非流式文本"
            } else {
                "支持 API 调用 · Chat / Responses"
            }
            .into();
        }
        Err(reason) => {
            result.state = "needs_auth";
            result.reason = reason;
        }
    }
    result
}

pub(crate) fn capabilities(conn: &Connection) -> Result<Vec<Capability>, String> {
    let mut statement = conn
        .prepare("SELECT id FROM connections ORDER BY created_ms")
        .map_err(|_| "无法读取来源能力")?;
    let ids = statement
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|_| "无法读取来源能力")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "无法读取来源能力")?;
    Ok(ids.iter().map(|id| capability(conn, id)).collect())
}

pub(crate) fn load(conn: &Connection, id: &str, platform: &str) -> Result<BoundAuth, String> {
    let secret: Option<String> = conn
        .query_row(
            "SELECT secret FROM connections WHERE id=?1 AND platform=?2 AND kind='auth'",
            [id, platform],
            |r| r.get(0),
        )
        .map_err(|_| "所选订阅账号不存在，请刷新来源列表")?;
    let secret = secret
        .filter(|v| !v.is_empty())
        .ok_or("所选账号缺少授权，请重新登录并保存")?;
    let (bundle, token) = if crate::provider_browser_auth::is_browser(Some(&secret)) {
        match crate::provider_browser_auth::load(id, platform, &secret)? {
            crate::provider_browser_auth::Credential::Cli { session } => {
                let pointer = if platform == "claude" { "/claudeAiOauth/accessToken" } else { "/tokens/access_token" };
                let token = session.credentials.pointer(pointer).and_then(Value::as_str).ok_or("所选账号缺少模型调用凭据")?.to_owned();
                (session.credentials, token)
            }
            _ => return Err("该订阅需要对应平台的模型适配器".into()),
        }
    } else {
        (crate::auth_snapshot::load(conn, id, &secret)?
            .ok_or("所选账号缺少完整授权，请在账号管理中重新登录并保存；不会使用其他账号代替")?, secret.clone())
    };
    let valid = match platform {
        "codex" | "opencode" => crate::auth_snapshot::same_codex_identity(&bundle, &bundle, &token),
        "claude" => crate::claude_auth::complete(&bundle, &token).is_some(),
        _ => false,
    };
    if !valid {
        return Err("所选账号的授权身份无法确认，请重新登录并保存".into());
    }
    Ok(BoundAuth { secret, bundle })
}

/// 同一账号刷新串行化；固定数量的锁避免请求驱动的无界注册表。
pub(crate) async fn lock(id: &str) -> tokio::sync::MutexGuard<'static, ()> {
    use std::hash::{Hash, Hasher};
    static LOCKS: std::sync::LazyLock<Vec<tokio::sync::Mutex<()>>> =
        std::sync::LazyLock::new(|| (0..64).map(|_| tokio::sync::Mutex::new(())).collect());
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hash);
    LOCKS[hash.finish() as usize % LOCKS.len()].lock().await
}

/// 校验身份 + 比较旧凭据 + 原子更新；并发重新登录不会被迟到的刷新覆盖。
pub(crate) fn save_refresh(
    conn: &Connection,
    id: &str,
    platform: &str,
    old: &BoundAuth,
    next: &Value,
) -> Result<(), String> {
    let token = match platform {
        "codex" | "opencode" => next.pointer("/tokens/access_token"),
        "claude" => next.pointer("/claudeAiOauth/accessToken"),
        _ => None,
    }
    .and_then(Value::as_str)
    .ok_or("刷新后的授权不完整")?;
    let same = match platform {
        "codex" | "opencode" => crate::auth_snapshot::same_codex_identity(&old.bundle, next, token),
        "claude" => {
            crate::claude_auth::complete(next, token).is_some()
                && crate::claude_auth::identity(next) == crate::claude_auth::identity(&old.bundle)
        }
        _ => false,
    };
    if !same {
        return Err("刷新后的账号身份发生变化，已停止转发，请重新登录".into());
    }
    conn.execute_batch("SAVEPOINT jolu_gateway_auth")
        .map_err(|_| "无法保存订阅授权")?;
    let result = (|| {
        if crate::provider_browser_auth::is_browser(Some(&old.secret)) {
            let mut credential = crate::provider_browser_auth::load(id, platform, &old.secret)?;
            let crate::provider_browser_auth::Credential::Cli { session } = &mut credential else { return Err("授权类型不匹配".into()); };
            session.credentials = next.clone();
            crate::provider_browser_auth::refresh_bound(conn, id, platform, &old.secret, &credential)?;
            crate::connections::refresh_account_fp(conn, id).map_err(|_| "无法更新账号标识")?;
            return Ok(());
        }
        let changed = conn.execute("UPDATE connections SET secret=?3,masked=?4 WHERE id=?1 AND platform=?2 AND kind='auth' AND secret=?5", rusqlite::params![id,platform,token,crate::connections::mask(token),old.secret])
            .map_err(|_| "无法更新订阅授权")?;
        if changed != 1 {
            return Err("账号授权已被更新，请重试；旧请求不会覆盖新登录".to_string());
        }
        crate::auth_snapshot::save(conn, id, next)?;
        crate::connections::refresh_account_fp(conn, id).map_err(|_| "无法更新账号标识")?;
        Ok(())
    })();
    if result.is_err() {
        conn.execute_batch("ROLLBACK TO jolu_gateway_auth; RELEASE jolu_gateway_auth")
            .map_err(|_| "订阅授权保存失败且无法回滚")?;
    } else {
        conn.execute_batch("RELEASE jolu_gateway_auth")
            .map_err(|_| "无法完成订阅授权保存")?;
    }
    result
}

#[cfg(all(test, windows))]
pub(crate) mod tests {
    use super::*;
    use base64::Engine;

    pub(crate) fn codex_bundle(token: &str, account: &str) -> Value {
        let claims = serde_json::json!({"https://api.openai.com/auth":{"chatgpt_account_id":account,"chatgpt_user_id":format!("user-{account}")}});
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
        serde_json::json!({"tokens":{"access_token":token,"refresh_token":format!("refresh-{account}"),"account_id":account,"id_token":format!("h.{encoded}.s")},"last_refresh":chrono::Utc::now().to_rfc3339()})
    }

    pub(crate) fn claude_bundle(token: &str, account: &str) -> Value {
        serde_json::json!({"type":"claude_oauth_v1","claudeAiOauth":{"accessToken":token,"refreshToken":"fixture-refresh","expiresAt":4102444800000i64,"scopes":["user:profile"]},"oauthAccount":{"accountUuid":account,"organizationUuid":"org"},"verifiedIdentity":{"accountUuid":account,"organizationUuid":"org"}})
    }

    pub(crate) fn insert(conn: &Connection, id: &str, platform: &str, token: &str, bundle: &Value) {
        conn.execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug) VALUES(?1,?2,'auth',?1,?3,1,?1)", rusqlite::params![id,platform,token]).unwrap();
        crate::auth_snapshot::save(conn, id, bundle).unwrap();
    }

    #[test]
    fn sealed_cli_accounts_refresh_without_changing_storage_or_neighbours() {
        let deps = crate::proxy::test_support::bare_deps();
        let conn = deps.db.lock().unwrap();
        for platform in ["codex", "opencode", "claude"] {
            let make = if platform == "claude" { claude_bundle } else { codex_bundle };
            let mut originals = Vec::new();
            for suffix in ["a", "b"] {
                let id = format!("{platform}-{suffix}");
                let credential = crate::provider_browser_auth::Credential::Cli { session: crate::cli_browser_auth::Session {
                    platform: platform.into(), credentials: make(suffix, suffix), account: Some(format!("{suffix}@example.invalid")),
                }};
                let sealed = crate::provider_browser_auth::seal(&id, &credential).unwrap();
                conn.execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug) VALUES(?1,?2,'auth',?1,?3,1,?1)", rusqlite::params![id,platform,sealed]).unwrap();
                originals.push((id,sealed));
            }
            let (a, _) = &originals[0];
            let (b, b_secret) = &originals[1];
            let old = load(&conn, a, platform).unwrap();
            assert!(save_refresh(&conn, a, platform, &old, &make("wrong", "b")).is_err());
            let next = make("new", "a");
            save_refresh(&conn, a, platform, &old, &next).unwrap();
            let updated = load(&conn, a, platform).unwrap();
            assert!(crate::provider_browser_auth::is_browser(Some(&updated.secret)));
            assert_eq!(updated.bundle, next);
            assert_eq!(load(&conn, b, platform).unwrap().secret, *b_secret);
            assert!(save_refresh(&conn, a, platform, &old, &make("late", "a")).is_err());
            assert!(crate::provider_browser_auth::load(b, platform, &updated.secret).is_err());
        }
    }

    #[test]
    fn selected_account_has_its_own_complete_auth_and_never_falls_back() {
        let deps = crate::proxy::test_support::bare_deps();
        let conn = deps.db.lock().unwrap();
        insert(
            &conn,
            "a",
            "codex",
            "token-a",
            &codex_bundle("token-a", "account-a"),
        );
        insert(
            &conn,
            "b",
            "codex",
            "token-b",
            &codex_bundle("token-b", "account-b"),
        );
        let auth = load(&conn, "a", "codex").unwrap();
        assert_eq!(auth.secret, "token-a");
        assert_eq!(auth.bundle["tokens"]["account_id"], "account-a");
        assert!(load(&conn, "a", "claude").is_err());
        conn.execute("DELETE FROM auth_snapshots WHERE connection_id='a'", [])
            .unwrap();
        assert!(load(&conn, "a", "codex").is_err());
        assert_eq!(load(&conn, "b", "codex").unwrap().secret, "token-b");
        assert!(load(&conn, "missing", "codex").is_err());
    }

    #[test]
    fn refresh_is_scoped_atomic_and_rejects_identity_or_concurrent_changes() {
        let deps = crate::proxy::test_support::bare_deps();
        let conn = deps.db.lock().unwrap();
        for platform in ["claude", "codex"] {
            let make = if platform == "claude" {
                claude_bundle
            } else {
                codex_bundle
            };
            let a = format!("{platform}-a");
            let b = format!("{platform}-b");
            insert(&conn, &a, platform, "a-old", &make("a-old", "account-a"));
            insert(&conn, &b, platform, "b-old", &make("b-old", "account-b"));
            let old = load(&conn, &a, platform).unwrap();
            assert!(save_refresh(&conn, &a, platform, &old, &make("wrong", "account-b")).is_err());
            assert_eq!(load(&conn, &a, platform).unwrap().secret, "a-old");
            save_refresh(&conn, &a, platform, &old, &make("a-new", "account-a")).unwrap();
            assert_eq!(load(&conn, &b, platform).unwrap().secret, "b-old");
            assert!(save_refresh(&conn, &a, platform, &old, &make("late", "account-a")).is_err());
            assert_eq!(load(&conn, &a, platform).unwrap().secret, "a-new");
            assert!(capability(&conn, &a).can_serve);
        }
        assert!(!capability(&conn, "missing").can_serve);
    }

    #[test]
    fn qoder_and_trae_require_bound_authorization_instead_of_native_fallback() {
        let deps = crate::proxy::test_support::bare_deps();
        let conn = deps.db.lock().unwrap();
        for platform in ["qoder", "trae"] {
            insert(
                &conn,
                platform,
                platform,
                "synthetic-secret",
                &claude_bundle("synthetic-secret", "account"),
            );
            let state = capability(&conn, platform);
            assert!(!state.can_serve);
            assert_eq!(state.state, "needs_auth", "{platform} 应走自己的授权检查，不再是未接入适配器");
            assert!(!serde_json::to_string(&state)
                .unwrap()
                .contains("synthetic-secret"));
        }
    }
}
