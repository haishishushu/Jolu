//! 手动接入密钥独立于一键接线凭据；刷新不修改任何 Agent 配置。
use aes_gcm::aead::{rand_core::RngCore, OsRng};
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};

const ENTROPY: &[u8] = b"jolu-gateway-manual-key-v1";

fn init(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS gateway_access (id INTEGER PRIMARY KEY CHECK(id=1), protected BLOB NOT NULL)")
        .map_err(|_| "无法准备网关密钥存储".into())
}

fn load(conn: &Connection) -> Result<Option<String>, String> {
    init(conn)?;
    let blob: Option<Vec<u8>> = conn
        .query_row("SELECT protected FROM gateway_access WHERE id=1", [], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|_| "无法读取网关密钥")?;
    blob.map(|bytes| {
        let plain = crate::auth_snapshot::protect(&bytes, ENTROPY, false)?;
        String::from_utf8(plain).map_err(|_| "网关密钥数据损坏".into())
    })
    .transpose()
}

fn rotate(conn: &Connection) -> Result<String, String> {
    init(conn)?;
    let mut bytes = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| "无法生成安全的网关密钥")?;
    let key = format!(
        "jolu-manual-{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let protected = crate::auth_snapshot::protect(key.as_bytes(), ENTROPY, true)?;
    // 单条原子更新；加密或写入失败时旧 Key 保持有效。
    conn.execute("INSERT INTO gateway_access(id,protected) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET protected=excluded.protected", [protected])
        .map_err(|_| "无法保存网关密钥")?;
    Ok(key)
}

fn current(conn: &Connection) -> Result<String, String> {
    match load(conn)? {
        Some(key) => Ok(key),
        None => rotate(conn),
    }
}

pub fn masked(conn: &Connection, refresh: bool) -> Result<String, String> {
    let key = if refresh {
        rotate(conn)?
    } else {
        current(conn)?
    };
    Ok(format!("••••••••{}", &key[key.len() - 4..]))
}

pub fn copy(conn: &Connection) -> Result<(), String> {
    let key = current(conn)?;
    let mut clipboard = arboard::Clipboard::new().map_err(|_| "无法打开剪贴板，请重试")?;
    clipboard
        .set_text(key)
        .map_err(|_| "复制失败，请重试".into())
}

fn equal(a: &str, b: &str) -> bool {
    let a = Sha256::digest(a.as_bytes());
    let b = Sha256::digest(b.as_bytes());
    a.iter()
        .zip(b.iter())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

/// 原生 OAuth 与桥接继续走既有接线；手动 Key 永远先校验，不能借路径绕过撤销。
pub fn authorize(conn: &Connection, token: Option<&str>, path: &str) -> Result<bool, String> {
    authorize_with(conn, token, path, || {
        crate::creds::local_codex_auth().map(|(token, _)| token)
    })
}

fn authorize_with(
    conn: &Connection,
    token: Option<&str>,
    path: &str,
    oauth: impl Fn() -> Option<String>,
) -> Result<bool, String> {
    let key = load(conn)?;
    if token.is_some_and(|t| t.starts_with("jolu-manual-")) {
        return Ok(key.as_deref().zip(token).is_some_and(|(a, b)| equal(a, b)));
    }
    // 未开通过手动接入的旧安装维持原有行为。
    let Some(key) = key else { return Ok(true) };
    if token.is_some_and(|t| equal(&key, t)) {
        return Ok(true);
    }
    if path == "/__jolu/bridge/callback" {
        return Ok(true);
    }
    Ok(super::wiring_store::all(conn).iter().any(|record| {
        record.owner == "relay"
            && matches!(record.state.as_str(), "wired" | "pending")
            && (record
                .token
                .as_deref()
                .zip(token)
                .is_some_and(|(a, b)| equal(a, b))
                || (record.agent == "codex"
                    && record
                        .applied()
                        .is_some_and(|a| a.mode.as_deref() == Some("oauth"))
                    && matches!(
                        path,
                        "/backend-api/codex/models" | "/backend-api/codex/responses"
                    )
                    && token
                        .zip(oauth())
                        .is_some_and(|(supplied, current)| equal(supplied, &current))))
    }))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn manual_key_is_protected_persistent_and_revoked_on_rotation() {
        let conn = Connection::open_in_memory().unwrap();
        let first = current(&conn).unwrap();
        assert_eq!(current(&conn).unwrap(), first);
        let blob: Vec<u8> = conn
            .query_row("SELECT protected FROM gateway_access", [], |r| r.get(0))
            .unwrap();
        assert!(!blob.windows(first.len()).any(|w| w == first.as_bytes()));
        assert!(authorize(&conn, Some(&first), "/v1/models").unwrap());
        assert!(!masked(&conn, false).unwrap().contains(&first));
        let second = rotate(&conn).unwrap();
        assert_ne!(first, second);
        for path in [
            "/v1/models",
            "/backend-api/codex/models",
            "/__jolu/bridge/callback",
        ] {
            assert!(!authorize(&conn, Some(&first), path).unwrap());
            assert!(authorize(&conn, Some(&second), path).unwrap());
        }
        assert!(!authorize(&conn, None, "/v1/models").unwrap());
        assert!(!authorize(&conn, Some("arbitrary"), "/v1/models").unwrap());
    }

    #[tokio::test]
    async fn rotation_rejects_old_http_keys_but_preserves_wired_agents() {
        let deps = crate::proxy::test_support::bare_deps();
        let first = {
            let conn = deps.db.lock().unwrap();
            super::super::wiring_store::mark_pending(
                &conn,
                "claude-code",
                "test",
                "relay",
                None,
                &super::super::wiring_store::Applied::default(),
                Some("jolu-claude-code.test"),
            )
            .unwrap();
            current(&conn).unwrap()
        };
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let url = format!("http://{}/v1/models", listener.local_addr().unwrap());
        let router = super::super::server::router(deps.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for header in ["authorization", "x-api-key", "x-goog-api-key"] {
            let value = if header == "authorization" {
                format!("Bearer {first}")
            } else {
                first.clone()
            };
            assert_eq!(
                client
                    .get(&url)
                    .header(header, value)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                200
            );
        }
        let second = rotate(&deps.db.lock().unwrap()).unwrap();
        for (token, status) in [
            (&first[..], 401),
            (&second[..], 200),
            ("jolu-claude-code.test", 200),
            ("unknown", 401),
        ] {
            assert_eq!(
                client
                    .get(&url)
                    .bearer_auth(token)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                status
            );
        }
        assert_eq!(client.get(&url).send().await.unwrap().status(), 401);
        assert_eq!(
            client
                .get(&url)
                .bearer_auth(&second)
                .header("origin", "https://example.com")
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        task.abort();
    }

    #[test]
    fn damaged_storage_does_not_silently_replace_the_key() {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        conn.execute(
            "INSERT INTO gateway_access VALUES(1,?1)",
            [b"broken".as_slice()],
        )
        .unwrap();
        assert!(current(&conn).is_err());
        assert!(authorize(&conn, Some("anything"), "/v1/models").is_err());
    }

    #[test]
    fn oauth_requires_current_native_token_and_is_unaffected_by_rotation() {
        let deps = crate::proxy::test_support::bare_deps();
        let conn = deps.db.lock().unwrap();
        super::super::wiring_store::mark_pending(
            &conn,
            "codex",
            "test",
            "relay",
            None,
            &super::super::wiring_store::Applied {
                mode: Some("oauth".into()),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let first = current(&conn).unwrap();
        for _ in 0..2 {
            let oauth = || Some("synthetic-native-token".to_string());
            assert!(authorize_with(
                &conn,
                Some("synthetic-native-token"),
                "/backend-api/codex/models",
                oauth
            )
            .unwrap());
            assert!(
                !authorize_with(&conn, Some("arbitrary"), "/backend-api/codex/models", oauth)
                    .unwrap()
            );
            assert!(
                !authorize_with(&conn, Some("synthetic-native-token"), "/v1/models", oauth)
                    .unwrap()
            );
            rotate(&conn).unwrap();
        }
        assert!(
            !authorize_with(&conn, Some(&first), "/backend-api/codex/models", || Some(
                first.clone()
            ))
            .unwrap()
        );
    }
}
