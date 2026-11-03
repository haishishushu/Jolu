//! 连接身份：持久化 slug 与账号指纹。
//!
//! slug 是模型菜单里的来源前缀（`<slug>/<model>`），一旦分配就不再随改名变化；
//! account_fp 用来识别「同一账号」，冷却、去重与部分唯一索引都以它为准。
//! 两者都只在后端使用，凭证原值从不进入 slug 或指纹的可见部分。

use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};

/// 留给路由组与内部命名空间的 slug，连接不能占用。
pub const RESERVED_SLUGS: [&str; 5] = ["route", "auto", "jolu", "group", "default"];

pub fn hash_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 连接名 → slug 基名：小写、非字母数字转连字符、合并连续连字符。
pub fn slugify(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    collapsed_dashes(&cleaned)
}

fn collapsed_dashes(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in s.chars() {
        if c == '-' {
            if !prev_dash && !out.is_empty() {
                out.push('-');
            }
            prev_dash = true;
        } else {
            out.push(c);
            prev_dash = false;
        }
    }
    out.trim_matches('-').to_string()
}

/// 持久化 slug 的完整校验：字符集、首尾、长度、保留字。
pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 64
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !RESERVED_SLUGS.contains(&slug)
}

/// 没有可用字符（纯中文名等）时的兜底：平台 + id 哈希前 6 位。
pub fn fallback_slug(platform: &str, id: &str) -> String {
    let platform = slugify(platform);
    let platform = if platform.is_empty() { "conn".to_string() } else { platform };
    format!("{platform}-{}", &hash_hex(id)[..6])
}

/// 账号指纹：同一平台、同一接入方式、同一凭据视为同一账号。
/// 不含 base_url —— 同一把 Key 的两种协议端点可以各服务各的。
pub fn account_fingerprint(platform: &str, kind: &str, secret: Option<&str>) -> Option<String> {
    let secret = secret?.trim();
    if secret.is_empty() {
        return None;
    }
    Some(hash_hex(&format!("{kind}\n{platform}\n{secret}")))
}

/// 把基名整理成合法 slug；保留字与空结果都按规则兜底。
fn normalize_base(base: &str, platform: &str, id: &str) -> String {
    let mut base = base.trim_matches('-').chars().take(48).collect::<String>();
    if base.is_empty() {
        base = fallback_slug(platform, id);
    }
    if RESERVED_SLUGS.contains(&base.as_str()) {
        base.push_str("-src");
    }
    base
}

fn slug_taken(conn: &Connection, slug: &str, except_id: &str) -> rusqlite::Result<bool> {
    let in_connections: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM connections WHERE slug = ?1 AND id != ?2)",
        [slug, except_id],
        |r| r.get(0),
    )?;
    if in_connections {
        return Ok(true);
    }
    conn.query_row("SELECT EXISTS(SELECT 1 FROM slug_history WHERE slug = ?1)", [slug], |r| r.get(0))
}

/// 同一来源重建（同一 connection_id，或同一账号指纹）时恢复旧 slug，旧 id 继续可用。
fn historical_slug(
    conn: &Connection,
    connection_id: &str,
    account_fp: Option<&str>,
) -> rusqlite::Result<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT h.slug FROM slug_history h
         WHERE (h.connection_id = ?1 OR (?2 IS NOT NULL AND h.account_fp = ?2))
           AND NOT EXISTS (SELECT 1 FROM connections c WHERE c.slug = h.slug)
         ORDER BY h.retired_ms DESC LIMIT 1",
    )?;
    let mut rows = stmt.query(rusqlite::params![connection_id, account_fp])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

/// 为一条连接分配 slug。调用方必须已经知道连接 id 与（可选的）account_fp。
/// 已存在的行直接沿用原 slug；删除后重建则优先恢复历史 slug。
pub fn allocate_slug(
    conn: &Connection,
    id: &str,
    platform: &str,
    name: &str,
    account_fp: Option<&str>,
) -> rusqlite::Result<String> {
    let existing: Option<String> = conn
        .query_row("SELECT slug FROM connections WHERE id = ?1", [id], |r| r.get(0))
        .optional()?
        .flatten();
    if let Some(slug) = existing.filter(|s| valid_slug(s)) {
        return Ok(slug);
    }
    if let Some(slug) = historical_slug(conn, id, account_fp)? {
        if valid_slug(&slug) {
            return Ok(slug);
        }
    }
    let base = normalize_base(&slugify(name), platform, id);
    let mut candidate = base.clone();
    let mut n = 2;
    while slug_taken(conn, &candidate, id)? {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    Ok(candidate)
}

/// 启动迁移：回填 slug / account_fp、清理存量重复服务、建索引与触发器。
///
/// 规则：
/// - 只补空值，已有 slug 永不改写；
/// - 重复服务每组保留最早创建的一条，其余只关闭服务并记 `migration_log`，不删数据；
/// - 索引建不出来只记诊断，不阻止启动（下次启动会再试）。
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS slug_history (
             slug          TEXT PRIMARY KEY,
             connection_id TEXT NOT NULL,
             account_fp    TEXT,
             retired_ms    INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS migration_log (
             id      INTEGER PRIMARY KEY,
             at_ms   INTEGER NOT NULL,
             kind    TEXT NOT NULL,
             detail  TEXT NOT NULL
         );",
    )?;

    let missing: i64 = conn.query_row(
        "SELECT COUNT(*) FROM connections WHERE slug IS NULL OR slug = ''",
        [],
        |r| r.get(0),
    )?;
    if missing > 0 {
        let tx = conn.unchecked_transaction()?;
        let rows: Vec<(String, String, String, Option<String>)> = {
            let mut stmt = tx.prepare(
                "SELECT id, platform, name, secret FROM connections
                 WHERE slug IS NULL OR slug = '' ORDER BY created_ms, id",
            )?;
            let collected = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            collected
        };
        for (id, platform, name, secret) in rows {
            let fp = account_fingerprint(&platform, "api", secret.as_deref());
            let slug = allocate_slug(&tx, &id, &platform, &name, fp.as_deref())?;
            tx.execute("UPDATE connections SET slug = ?2 WHERE id = ?1", rusqlite::params![id, slug])?;
        }
        tx.commit()?;
    }

    // account_fp 只补空值：改过 Key 的连接由写入凭证的路径在同一事务里重算。
    let without_fp: Vec<(String, String, String, Option<String>)> = {
        let mut stmt = conn.prepare(
            "SELECT id, platform, kind, secret FROM connections
             WHERE account_fp IS NULL AND secret IS NOT NULL AND secret != ''",
        )?;
        let collected = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        collected
    };
    if !without_fp.is_empty() {
        let tx = conn.unchecked_transaction()?;
        for (id, platform, kind, secret) in without_fp {
            let fp = account_fingerprint(&platform, &kind, secret.as_deref());
            tx.execute(
                "UPDATE connections SET account_fp = ?2 WHERE id = ?1",
                rusqlite::params![id, fp],
            )?;
        }
        tx.commit()?;
    }

    dedupe_serving(conn)?;

    // 部分唯一索引：同一账号同一协议只服务一次；同平台订阅只服务一次。
    let indexes = [
        ("idx_connections_serve_account",
         "CREATE UNIQUE INDEX IF NOT EXISTS idx_connections_serve_account
          ON connections(account_fp, IFNULL(protocol, ''))
          WHERE serve_enabled = 1 AND account_fp IS NOT NULL"),
        ("idx_connections_serve_auth_platform",
         "CREATE UNIQUE INDEX IF NOT EXISTS idx_connections_serve_auth_platform
          ON connections(platform) WHERE serve_enabled = 1 AND kind = 'auth'"),
        // 菜单 id 的前缀必须唯一：重复的 slug 会让两个来源抢同一个 `<slug>/<model>`
        ("idx_connections_slug",
         "CREATE UNIQUE INDEX IF NOT EXISTS idx_connections_slug
          ON connections(slug) WHERE slug IS NOT NULL"),
    ];
    for (name, sql) in indexes {
        if let Err(error) = conn.execute_batch(sql) {
            let _ = conn.execute(
                "INSERT INTO migration_log(at_ms, kind, detail) VALUES(?1, 'index_failed', ?2)",
                rusqlite::params![now_ms(), format!("{name}: {error}")],
            );
        }
    }

    // 触发器是生产环境的兜底：任何入口都不能插入没有 slug 或非法 slug 的连接。
    // 单元测试的夹具直接建旧式连接行（不关心菜单 id），由专门的一条测试验证触发器本身。
    #[cfg(not(test))]
    install_slug_triggers(conn)?;
    Ok(())
}

/// 安装 slug 兜底触发器。`migrate` 在生产构建里调用；测试单独验证。
///
/// 插入：
/// - 调试构建直接拒绝，开发期漏掉 slug 的入口会立刻暴露；
/// - 发布构建自动补 `legacy-<hex>` 并记 `migration_log`，这样回退到旧版后仍能添加连接。
///
/// 更新：任何入口都不允许把 slug 改成空值或保留字。
fn install_slug_triggers(conn: &Connection) -> rusqlite::Result<()> {
    #[cfg(debug_assertions)]
    let insert_action = "BEGIN SELECT RAISE(ABORT, 'Jolu: connection slug is required'); END;";
    #[cfg(not(debug_assertions))]
    let insert_action = "BEGIN
             UPDATE connections SET slug = 'legacy-' || lower(hex(randomblob(8)))
               WHERE id = NEW.id AND (slug IS NULL OR slug = '');
             INSERT INTO migration_log(at_ms, kind, detail)
               VALUES(CAST(strftime('%s','now') AS INTEGER) * 1000, 'slug_missing', 'connection ' || NEW.id);
         END;";
    conn.execute_batch(&format!(
        "DROP TRIGGER IF EXISTS connections_slug_insert;
         CREATE TRIGGER connections_slug_insert
         AFTER INSERT ON connections
         WHEN NEW.slug IS NULL OR NEW.slug = ''
         {insert_action}
         DROP TRIGGER IF EXISTS connections_slug_invalid;
         CREATE TRIGGER connections_slug_invalid
         BEFORE INSERT ON connections
         WHEN NEW.slug IS NOT NULL AND NEW.slug != ''
              AND (NEW.slug GLOB '*[^a-z0-9-]*' OR NEW.slug GLOB '-*'
                   OR NEW.slug IN ('route','auto','jolu','group','default'))
         BEGIN SELECT RAISE(ABORT, 'Jolu: connection slug is invalid'); END;
         DROP TRIGGER IF EXISTS connections_slug_update;
         CREATE TRIGGER connections_slug_update
         BEFORE UPDATE OF slug ON connections
         WHEN NEW.slug IS NULL OR NEW.slug = ''
              OR NEW.slug GLOB '*[^a-z0-9-]*' OR NEW.slug GLOB '-*'
              OR NEW.slug IN ('route','auto','jolu','group','default')
         BEGIN SELECT RAISE(ABORT, 'Jolu: connection slug is required'); END;"
    ))
}

/// 存量重复服务清理：每组保留最早创建的一条，其余关闭服务并记日志。
fn dedupe_serving(conn: &Connection) -> rusqlite::Result<()> {
    let mut groups: Vec<String> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT account_fp, IFNULL(protocol, '')
             FROM connections
             WHERE serve_enabled = 1 AND kind = 'api' AND account_fp IS NOT NULL
             GROUP BY account_fp, IFNULL(protocol, '') HAVING COUNT(*) > 1",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (fp, protocol) = row?;
            groups.push(format!("api|{fp}|{protocol}"));
        }
    }
    {
        let mut stmt = conn.prepare(
            "SELECT platform FROM connections
             WHERE serve_enabled = 1 AND kind = 'auth'
             GROUP BY platform HAVING COUNT(*) > 1",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for platform in rows {
            groups.push(format!("auth|{}", platform?));
        }
    }
    if groups.is_empty() {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    for key in groups {
        let ids: Vec<String> = if let Some(rest) = key.strip_prefix("api|") {
            let (fp, protocol) = rest.split_once('|').unwrap_or((rest, ""));
            let mut stmt = tx.prepare(
                "SELECT id FROM connections
                 WHERE serve_enabled = 1 AND kind = 'api' AND account_fp = ?1
                   AND IFNULL(protocol, '') = ?2
                 ORDER BY created_ms, id",
            )?;
            let collected = stmt.query_map(rusqlite::params![fp, protocol], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            collected
        } else {
            let platform = key.strip_prefix("auth|").unwrap_or(&key);
            let mut stmt = tx.prepare(
                "SELECT id FROM connections
                 WHERE serve_enabled = 1 AND kind = 'auth' AND platform = ?1
                 ORDER BY created_ms, id",
            )?;
            let collected = stmt.query_map([platform], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            collected
        };
        if ids.len() < 2 {
            continue;
        }
        for loser in &ids[1..] {
            tx.execute("UPDATE connections SET serve_enabled = 0 WHERE id = ?1", [loser])?;
            tx.execute(
                "INSERT INTO migration_log(at_ms, kind, detail) VALUES(?1, 'duplicate_serve', ?2)",
                rusqlite::params![
                    now_ms(),
                    format!("kept {}, disabled {loser}（同一账号或同一平台只服务一次，可手动恢复）", ids[0])
                ],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> Connection {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,slug,created_ms)
             VALUES('c1','custom','api','Kimi','kimi',1)",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn slugify_keeps_existing_behaviour() {
        assert_eq!(slugify("智谱 GLM"), "glm");
        assert_eq!(slugify("Kimi -- K2!!"), "kimi-k2");
        assert_eq!(slugify("  --  "), "");
    }

    #[test]
    fn reserved_and_duplicate_slugs_are_avoided() {
        assert_eq!(normalize_base("route", "custom", "id-1"), "route-src");
        assert!(valid_slug("route-src"));
        assert!(!valid_slug("route"));
        assert!(!valid_slug("-bad"));
        assert!(!valid_slug("Bad"));
    }

    #[test]
    fn allocate_appends_suffix_for_duplicate_names() {
        let conn = open();
        let slug = allocate_slug(&conn, "c2", "custom", "Kimi", None).unwrap();
        assert_eq!(slug, "kimi-2");
        let slug = allocate_slug(&conn, "c3", "custom", "智谱 GLM", None).unwrap();
        assert_eq!(slug, "glm");
        // 纯中文名兜底为 platform-hash6
        let slug = allocate_slug(&conn, "c4", "custom", "中文名称", None).unwrap();
        assert!(slug.starts_with("custom-"), "{slug}");
        assert_eq!(slug.len(), "custom-".len() + 6);
    }

    #[test]
    fn existing_row_keeps_its_slug_after_rename() {
        let conn = open();
        assert_eq!(allocate_slug(&conn, "c1", "custom", "完全改名", None).unwrap(), "kimi");
    }

    #[test]
    fn recreated_account_restores_historical_slug() {
        let conn = open();
        let fp = account_fingerprint("custom", "api", Some("sk-abc")).unwrap();
        conn.execute(
            "INSERT INTO slug_history(slug,connection_id,account_fp,retired_ms) VALUES('old','gone',?1,1)",
            [&fp],
        )
        .unwrap();
        let slug = allocate_slug(&conn, "c-new", "custom", "新名字", Some(&fp)).unwrap();
        assert_eq!(slug, "old", "同一账号重建应恢复旧 slug");
    }

    #[test]
    fn migration_backfills_slug_and_is_idempotent() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,created_ms) VALUES('c1','custom','api','Kimi 网关',1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,created_ms) VALUES('c2','custom','api','中文名称',2)",
            [],
        )
        .unwrap();
        migrate(&conn).unwrap();
        let slug = |id: &str| -> String {
            conn.query_row("SELECT slug FROM connections WHERE id = ?1", [id], |r| r.get(0)).unwrap()
        };
        assert_eq!(slug("c1"), "kimi");
        assert!(slug("c2").starts_with("custom-"), "{}", slug("c2"));
        // 第二次迁移不改写任何已有 slug
        migrate(&conn).unwrap();
        assert_eq!(slug("c1"), "kimi");
        assert!(slug("c2").starts_with("custom-"));
    }

    #[test]
    fn migration_disables_duplicate_serving_rows_and_logs() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE connections (
                id TEXT PRIMARY KEY, platform TEXT NOT NULL, kind TEXT NOT NULL,
                name TEXT NOT NULL, secret TEXT, protocol TEXT,
                serve_enabled INTEGER NOT NULL DEFAULT 0, created_ms INTEGER NOT NULL,
                slug TEXT, account_fp TEXT
            );",
        )
        .unwrap();
        let fp = account_fingerprint("custom", "api", Some("sk-same")).unwrap();
        for (id, created) in [("c1", 1), ("c2", 2)] {
            conn.execute(
                "INSERT INTO connections(id,platform,kind,name,secret,protocol,serve_enabled,created_ms,account_fp)
                 VALUES(?1,'custom','api',?1,'sk-same','anthropic',1,?2,?3)",
                rusqlite::params![id, created, fp],
            )
            .unwrap();
        }
        migrate(&conn).unwrap();
        let served = |id: &str| -> i64 {
            conn.query_row("SELECT serve_enabled FROM connections WHERE id = ?1", [id], |r| r.get(0)).unwrap()
        };
        assert_eq!(served("c1"), 1, "保留最早创建的一条");
        assert_eq!(served("c2"), 0, "重复的一条只关闭服务，不删数据");
        let logged: i64 = conn
            .query_row("SELECT COUNT(*) FROM migration_log WHERE kind='duplicate_serve'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(logged, 1);
        // 部分唯一索引兜底：再开启会失败，除非先关掉另一条
        assert!(conn.execute("UPDATE connections SET serve_enabled = 1 WHERE id = 'c2'", []).is_err());
    }

    #[test]
    fn slug_trigger_rejects_missing_or_reserved_slug() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        install_slug_triggers(&conn).unwrap();
        let insert = |id: &str, slug: Option<&str>| {
            conn.execute(
                "INSERT INTO connections(id,platform,kind,name,created_ms,slug) VALUES(?1,'custom','api','x',1,?2)",
                rusqlite::params![id, slug],
            )
        };
        assert!(insert("c1", None).is_err(), "没有 slug 的插入必须被拒绝");
        assert!(insert("c1", Some("route")).is_err(), "保留字必须被拒绝");
        assert!(insert("c1", Some("Bad Slug")).is_err(), "非法字符必须被拒绝");
        assert!(insert("c1", Some("kimi")).is_ok());
        assert!(conn
            .execute("UPDATE connections SET slug = '' WHERE id = 'c1'", [])
            .is_err());
    }

    #[test]
    fn fingerprint_ignores_base_url_and_trims_secret() {
        let a = account_fingerprint("custom", "api", Some(" sk-abc ")).unwrap();
        let b = account_fingerprint("custom", "api", Some("sk-abc")).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, account_fingerprint("custom", "api", Some("sk-other")).unwrap());
        assert_eq!(account_fingerprint("custom", "api", Some("  ")), None);
        assert_eq!(account_fingerprint("custom", "api", None), None);
    }
}
