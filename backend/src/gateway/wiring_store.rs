//! agent 接线记录（`agent_wiring` 表）。
//!
//! 记录三件事：谁在接管（owner）、写到哪一步（state）、写下了什么（applied）。
//! 先记 pending 再写文件，写完改成 wired；启动时按文件里的实际值修复 pending。
//! `settings.gateway_wiring` 在过渡版本里继续双写，方便回退到旧版。

use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone, serde::Serialize)]
pub struct WiringRecord {
    pub agent: String,
    pub instance_id: String,
    /// none | observe | relay
    pub owner: String,
    /// idle | pending | wired
    pub state: String,
    pub original_json: Option<String>,
    pub applied_json: Option<String>,
    pub token: Option<String>,
    pub wired_ms: Option<i64>,
}

impl WiringRecord {
    pub fn applied(&self) -> Option<Applied> {
        serde_json::from_str(self.applied_json.as_deref()?).ok()
    }
}

/// 我们实际写进配置的值。状态与断开都以它为准，不按 settings 猜。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct Applied {
    pub base: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    /// 接线方式：oauth（保留登录，写 openai_base_url）或 provider（自定义 provider + 静态令牌）。
    #[serde(default)]
    pub mode: Option<String>,
}

pub fn load(conn: &Connection, agent: &str) -> Option<WiringRecord> {
    conn.query_row(
        "SELECT agent_id, instance_id, owner, state, original_json, applied_json, token, wired_ms
         FROM agent_wiring WHERE agent_id = ?1",
        [agent],
        |r| {
            Ok(WiringRecord {
                agent: r.get(0)?,
                instance_id: r.get(1)?,
                owner: r.get(2)?,
                state: r.get(3)?,
                original_json: r.get(4)?,
                applied_json: r.get(5)?,
                token: r.get(6)?,
                wired_ms: r.get(7)?,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
}

pub fn all(conn: &Connection) -> Vec<WiringRecord> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT agent_id, instance_id, owner, state, original_json, applied_json, token, wired_ms
         FROM agent_wiring",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok(WiringRecord {
            agent: r.get(0)?,
            instance_id: r.get(1)?,
            owner: r.get(2)?,
            state: r.get(3)?,
            original_json: r.get(4)?,
            applied_json: r.get(5)?,
            token: r.get(6)?,
            wired_ms: r.get(7)?,
        })
    }) else {
        return Vec::new();
    };
    rows.filter_map(Result::ok).collect()
}

/// 写文件之前先落 pending：崩溃后能看出「打算写、可能写了一半」。
pub fn mark_pending(
    conn: &Connection,
    agent: &str,
    instance_id: &str,
    owner: &str,
    original_json: Option<&str>,
    applied: &Applied,
    token: Option<&str>,
) -> rusqlite::Result<()> {
    let applied_json = serde_json::to_string(applied).unwrap_or_default();
    conn.execute(
        "INSERT INTO agent_wiring(agent_id, instance_id, owner, state, original_json, applied_json, token, wired_ms)
         VALUES(?1, ?2, ?3, 'pending', ?4, ?5, ?6, NULL)
         ON CONFLICT(agent_id) DO UPDATE SET
             instance_id = excluded.instance_id,
             owner = excluded.owner,
             state = 'pending',
             original_json = excluded.original_json,
             applied_json = excluded.applied_json,
             token = excluded.token,
             wired_ms = NULL",
        rusqlite::params![agent, instance_id, owner, original_json, applied_json, token],
    )?;
    Ok(())
}

pub fn mark_wired(conn: &Connection, agent: &str, wired_ms: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE agent_wiring SET state = 'wired', wired_ms = ?2 WHERE agent_id = ?1",
        rusqlite::params![agent, wired_ms],
    )?;
    Ok(())
}

/// 断开后回到 idle；保留 owner 便于界面说明「这个工具现在归直连」。
pub fn mark_idle(conn: &Connection, agent: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE agent_wiring SET state = 'idle', applied_json = NULL, original_json = NULL, wired_ms = NULL
         WHERE agent_id = ?1",
        [agent],
    )?;
    Ok(())
}

pub fn set_owner_all(conn: &Connection, owner: &str) -> rusqlite::Result<()> {
    conn.execute("UPDATE agent_wiring SET owner = ?1", [owner])?;
    Ok(())
}

/// 从旧设置导入接线记录（只导入不存在的 agent；旧版回退后仍可读 settings）。
pub fn import_legacy(conn: &Connection, snapshots: &std::collections::BTreeMap<String, String>, instance_id: &str) -> usize {
    let mut imported = 0;
    for (agent, snapshot) in snapshots {
        if load(conn, agent).is_some() {
            continue;
        }
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO agent_wiring(agent_id, instance_id, owner, state, original_json, applied_json, token, wired_ms)
             VALUES(?1, ?2, 'relay', 'wired', ?3, NULL, NULL, NULL)",
            rusqlite::params![agent, instance_id, snapshot],
        );
        if inserted.unwrap_or(0) > 0 {
            imported += 1;
        }
    }
    imported
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        crate::db::open(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn pending_then_wired_then_idle_round_trip() {
        let conn = db();
        let applied = Applied { base: "http://127.0.0.1:12731".into(), token: Some("jolu-codex.abcd1234".into()), provider: None, mode: Some("provider".into()) };
        mark_pending(&conn, "codex", "instance-1", "relay", Some(r#"{"model_provider":"openai"}"#), &applied, Some("jolu-codex.abcd1234")).unwrap();
        let record = load(&conn, "codex").unwrap();
        assert_eq!(record.state, "pending");
        assert_eq!(record.applied().unwrap().base, "http://127.0.0.1:12731");
        assert_eq!(record.original_json.as_deref(), Some(r#"{"model_provider":"openai"}"#));

        mark_wired(&conn, "codex", 42).unwrap();
        assert_eq!(load(&conn, "codex").unwrap().state, "wired");

        mark_idle(&conn, "codex").unwrap();
        let record = load(&conn, "codex").unwrap();
        assert_eq!(record.state, "idle");
        assert!(record.applied_json.is_none());
        assert!(record.original_json.is_none());
    }

    #[test]
    fn legacy_snapshots_import_once() {
        let conn = db();
        let mut map = std::collections::BTreeMap::new();
        map.insert("claude-code".to_string(), r#"{"ANTHROPIC_BASE_URL":"https://old"}"#.to_string());
        assert_eq!(import_legacy(&conn, &map, "inst"), 1);
        assert_eq!(import_legacy(&conn, &map, "inst"), 0, "重复导入不能覆盖已有记录");
        let record = load(&conn, "claude-code").unwrap();
        assert_eq!(record.state, "wired");
        assert_eq!(record.owner, "relay");
    }
}
