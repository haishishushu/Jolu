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
    for (_, slug, source, platform, kind, default_model, status, models_json) in rows {
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
