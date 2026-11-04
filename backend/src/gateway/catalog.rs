//! 模型清单（只读）：把模型池里每个来源展开成 `<slug>/<model>`。
//! 只读数据库缓存，不打上游；拉取新清单仍由 `/v1/models` 负责。

use rusqlite::Connection;

#[derive(Debug, Clone, serde::Serialize)]
pub struct CatalogModel {
    pub id: String,
    pub source: String,
    pub slug: String,
    pub model: String,
    pub kind: String,
    pub status: String,
}

pub fn list(conn: &Connection) -> rusqlite::Result<Vec<CatalogModel>> {
    let order = crate::gateway::groups::source_order(conn);
    let mut stmt = conn.prepare(
        "SELECT id, slug, name, platform, kind, model, status, models_json
         FROM connections WHERE serve_enabled = 1 AND slug IS NOT NULL AND slug != ''
         ORDER BY created_ms ASC",
    )?;
    let mut rows: Vec<(String, String, String, String, String, Option<String>, String, Option<String>)> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.sort_by_key(|row| order.get(&row.0).copied().unwrap_or((i64::MAX, i64::MAX)));
    let mut out = Vec::new();
    for (id, slug, source, platform, kind, default_model, status, models_json) in rows {
        let models: Vec<String> = if kind == "api" {
            let cached = models_json
                .as_deref()
                .and_then(|text| serde_json::from_str::<Vec<String>>(text).ok())
                .unwrap_or_default();
            if cached.is_empty() {
                default_model.into_iter().collect()
            } else {
                cached
            }
        } else if platform == "workbuddy" {
            crate::gateway::adapter::workbuddy::cached_models(conn, &id).unwrap_or_default()
        } else if platform == "qoder" {
            crate::gateway::adapter::qoder::cached_models(conn, &id).unwrap_or_default()
        } else if platform == "trae" {
            crate::gateway::adapter::trae_models::cached_models(conn, &id).unwrap_or_default()
        } else {
            crate::gateway::server::builtin_subscription_models(&platform)
                .iter()
                .map(|model| model.to_string())
                .collect()
        };
        for model in models {
            out.push(CatalogModel {
                id: format!("{slug}/{model}"),
                source: source.clone(),
                slug: slug.clone(),
                model,
                kind: kind.clone(),
                status: status.clone(),
            });
        }
    }
    Ok(out)
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn trae_catalog_reads_fresh_bound_models_without_using_legacy_cache() {
        use sha2::{Digest, Sha256};
        let deps = crate::proxy::test_support::bare_deps();
        let secret = crate::provider_browser_auth::seal("trae-cache", &crate::provider_browser_auth::Credential::Trae {
            edition: "cn".into(), session: crate::trae_browser_auth::Session {
                token: "fixture".into(), host: "https://api.trae.cn".into(), expires_at: "2099-01-01T00:00:00Z".into(), account: None, user_id: "cache-user".into(),
            },
        }).unwrap();
        let cache = serde_json::json!({"credential_hash":format!("{:x}",Sha256::digest(secret.as_bytes())),"edition":"cn","fetched_ms":chrono::Utc::now().timestamp_millis(),"models":["account-model"]}).to_string();
        let conn = deps.db.lock().unwrap();
        conn.execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug,serve_enabled,models_json) VALUES('trae-cache','trae','auth','Trae',?1,1,'trae-cache',1,?2)", rusqlite::params![secret,cache]).unwrap();
        assert_eq!(super::list(&conn).unwrap().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["trae-cache/account-model"]);
        conn.execute("UPDATE connections SET models_json='[\"unverified\"]' WHERE id='trae-cache'", []).unwrap();
        assert!(super::list(&conn).unwrap().is_empty());
    }
    #[test]
    fn subscription_catalog_uses_qoder_builtin_but_never_unverified_trae_model() {
        let deps = crate::proxy::test_support::bare_deps();
        let conn = deps.db.lock().unwrap();
        let secret = crate::provider_browser_auth::seal("qoder-source", &crate::provider_browser_auth::Credential::Qoder {
            edition: "cn".into(),
            session: crate::qoder_desktop::Session {
                token: "fixture-token".into(), refresh_token: None,
                expires_at: Some("2099-01-01T00:00:00Z".into()),
                account: None, user_id: Some("fixture-user".into()), machine_id: None,
            },
        }).unwrap();
        conn.execute("INSERT INTO connections(id,platform,kind,name,secret,created_ms,slug,serve_enabled) VALUES('qoder-source','qoder','auth','Qoder',?1,1,'qoder-source',1)", [secret]).unwrap();
        conn.execute("INSERT INTO connections(id,platform,kind,name,created_ms,slug,serve_enabled,models_json) VALUES('trae-source','trae','auth','Trae',2,'trae-source',1,'[\"unverified-cache\"]')", []).unwrap();
        let list = super::list(&conn).unwrap();
        // 从未读到账号列表：Qoder 国内版内置清单（magpie CN_MODELS）
        assert_eq!(list.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), vec!["qoder-source/ultimate", "qoder-source/performance", "qoder-source/efficient", "qoder-source/lite"]);
        conn.execute("UPDATE connections SET model='explicit-model' WHERE id='trae-source'", []).unwrap();
        let list = super::list(&conn).unwrap();
        assert!(!list.iter().any(|v| v.id == "trae-source/explicit-model"), "无授权不能退回手填模型冒充可用来源");
        let trae_secret = crate::provider_browser_auth::seal("trae-source", &crate::provider_browser_auth::Credential::Trae {
            edition: "cn".into(), session: crate::trae_browser_auth::Session {
                token: "fixture".into(), host: "https://api.trae.cn".into(), expires_at: "2099-01-01T00:00:00Z".into(), account: None, user_id: "fixture-user".into(),
            },
        }).unwrap();
        conn.execute("UPDATE connections SET secret=?1 WHERE id='trae-source'", [trae_secret]).unwrap();
        let list = super::list(&conn).unwrap();
        assert!(!list.iter().any(|v| v.id == "trae-source/explicit-model"), "仅有授权和手填 ID 不能替代账号目录路由");
        assert!(!list.iter().any(|v| v.model == "unverified-cache"));
    }
}
