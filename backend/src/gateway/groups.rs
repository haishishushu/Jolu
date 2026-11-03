//! 来源分组与 Agent 分组（只影响排序，不参与路由、接线、token 或菜单成员资格）。
//!
//! 单归属：成员表以 connection_id / agent_id 为主键，移动到另一个组是 UPSERT。
//! 没有成员行就是「未分组」。

use rusqlite::{params, Connection};

#[derive(Debug, Clone, serde::Serialize)]
pub struct GroupDto {
    pub id: String,
    pub name: String,
    pub sort: i64,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GroupsDto {
    pub sources: Vec<GroupDto>,
    pub agents: Vec<GroupDto>,
}

struct Tables {
    groups: &'static str,
    members: &'static str,
    member_column: &'static str,
}

fn tables(kind: &str) -> Result<Tables, String> {
    match kind {
        "source" => Ok(Tables {
            groups: "source_groups",
            members: "source_group_members",
            member_column: "connection_id",
        }),
        "agent" => Ok(Tables {
            groups: "agent_groups",
            members: "agent_group_members",
            member_column: "agent_id",
        }),
        _ => Err("分组类型只能是 source 或 agent".into()),
    }
}

fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 40 {
        return Err("分组名称应为 1 到 40 个字符".into());
    }
    Ok(name)
}

fn list_kind(conn: &Connection, kind: &str) -> rusqlite::Result<Vec<GroupDto>> {
    let tables = tables(kind).map_err(|error| {
        rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(error)))
    })?;
    let mut stmt = conn.prepare(&format!(
        "SELECT id, name, sort FROM {} ORDER BY sort, created_ms, id",
        tables.groups
    ))?;
    let mut groups = stmt
        .query_map([], |r| {
            Ok(GroupDto {
                id: r.get(0)?,
                name: r.get(1)?,
                sort: r.get(2)?,
                members: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut member_stmt = conn.prepare(&format!(
        "SELECT group_id, {} FROM {} ORDER BY sort, {}",
        tables.member_column, tables.members, tables.member_column
    ))?;
    let members = member_stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (group_id, member) in members {
        if let Some(group) = groups.iter_mut().find(|group| group.id == group_id) {
            group.members.push(member);
        }
    }
    Ok(groups)
}

/// 来源在菜单里的顺序：分组 sort、组内 sort。未分组的来源排在最后。
pub fn source_order(conn: &Connection) -> std::collections::HashMap<String, (i64, i64)> {
    let mut out = std::collections::HashMap::new();
    let Ok(mut stmt) = conn.prepare(
        "SELECT m.connection_id, g.sort, m.sort
         FROM source_group_members m JOIN source_groups g ON g.id = m.group_id",
    ) else {
        return out;
    };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))) else {
        return out;
    };
    for row in rows.flatten() {
        out.insert(row.0, (row.1, row.2));
    }
    out
}

pub fn list(conn: &Connection) -> rusqlite::Result<GroupsDto> {
    Ok(GroupsDto {
        sources: list_kind(conn, "source")?,
        agents: list_kind(conn, "agent")?,
    })
}

pub fn create(conn: &Connection, kind: &str, name: &str) -> Result<GroupDto, String> {
    let tables = tables(kind)?;
    let name = validate_name(name)?;
    let duplicate: bool = conn
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {} WHERE name = ?1)", tables.groups),
            [&name],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if duplicate {
        return Err(format!("已经有名为「{name}」的分组"));
    }
    let id: String = conn
        .query_row("SELECT lower(hex(randomblob(8)))", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let sort: i64 = conn
        .query_row(
            &format!("SELECT COALESCE(MAX(sort), -1) + 1 FROM {}", tables.groups),
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    conn.execute(
        &format!("INSERT INTO {}(id, name, sort, created_ms) VALUES(?1, ?2, ?3, ?4)", tables.groups),
        params![id, name, sort, chrono::Local::now().timestamp_millis()],
    )
    .map_err(|e| e.to_string())?;
    Ok(GroupDto { id, name, sort, members: Vec::new() })
}

pub fn rename(conn: &Connection, kind: &str, id: &str, name: &str) -> Result<(), String> {
    let tables = tables(kind)?;
    let name = validate_name(name)?;
    let duplicate: bool = conn
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {} WHERE name = ?1 AND id != ?2)", tables.groups),
            params![name, id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if duplicate {
        return Err(format!("已经有名为「{name}」的分组"));
    }
    let changed = conn
        .execute(&format!("UPDATE {} SET name = ?2 WHERE id = ?1", tables.groups), params![id, name])
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("分组不存在".into());
    }
    Ok(())
}

/// 删除分组：成员回到未分组（外键级联），来源、连接、Agent 都不删。
pub fn delete(conn: &Connection, kind: &str, id: &str) -> Result<(), String> {
    let tables = tables(kind)?;
    let changed = conn
        .execute(&format!("DELETE FROM {} WHERE id = ?1", tables.groups), [id])
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("分组不存在".into());
    }
    Ok(())
}

/// 调整分组顺序：`ids` 必须正好是当前分组的完整列表，否则视为视图过期。
pub fn reorder(conn: &Connection, kind: &str, ids: &[String]) -> Result<(), String> {
    let tables = tables(kind)?;
    let existing: Vec<String> = {
        let mut stmt = conn
            .prepare(&format!("SELECT id FROM {}", tables.groups))
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<String>>>()
            .map_err(|e| e.to_string())?;
        rows
    };
    if ids.len() != existing.len() || !ids.iter().all(|id| existing.contains(id)) {
        return Err("分组列表已变化，请刷新后再试".into());
    }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    for (index, id) in ids.iter().enumerate() {
        tx.execute(
            &format!("UPDATE {} SET sort = ?2 WHERE id = ?1", tables.groups),
            params![id, index as i64],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// 移动成员：`group_id = None` 表示移回未分组。
pub fn move_member(conn: &Connection, kind: &str, member: &str, group_id: Option<&str>) -> Result<(), String> {
    let tables = tables(kind)?;
    if kind == "agent" && !crate::gateway::wiring::is_agent(member) {
        return Err("不支持的工具".into());
    }
    if kind == "source" {
        let exists: bool = conn
            .query_row("SELECT EXISTS(SELECT 1 FROM connections WHERE id = ?1)", [member], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if !exists {
            return Err("来源不存在".into());
        }
    }
    match group_id {
        Some(group_id) => {
            let exists: bool = conn
                .query_row(
                    &format!("SELECT EXISTS(SELECT 1 FROM {} WHERE id = ?1)", tables.groups),
                    [group_id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !exists {
                return Err("分组不存在".into());
            }
            conn.execute(
                &format!(
                    "INSERT INTO {}({}, group_id, sort) VALUES(?1, ?2, 0)
                     ON CONFLICT({}) DO UPDATE SET group_id = excluded.group_id",
                    tables.members, tables.member_column, tables.member_column
                ),
                params![member, group_id],
            )
            .map_err(|e| e.to_string())?;
        }
        None => {
            conn.execute(
                &format!("DELETE FROM {} WHERE {} = ?1", tables.members, tables.member_column),
                [member],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        crate::db::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn source(conn: &Connection, id: &str, slug: &str) {
        conn.execute(
            "INSERT INTO connections(id,platform,kind,name,serve_enabled,created_ms,slug)
             VALUES(?1,'custom','api',?1,1,1,?2)",
            params![id, slug],
        )
        .unwrap();
    }

    #[test]
    fn groups_are_single_membership_and_delete_keeps_members() {
        let conn = db();
        source(&conn, "c1", "kimi");
        source(&conn, "c2", "glm");
        let a = create(&conn, "source", "我的常用").unwrap();
        let b = create(&conn, "source", "备用").unwrap();
        move_member(&conn, "source", "c1", Some(&a.id)).unwrap();
        move_member(&conn, "source", "c2", Some(&a.id)).unwrap();
        move_member(&conn, "source", "c2", Some(&b.id)).unwrap();
        let groups = list(&conn).unwrap();
        assert_eq!(groups.sources.len(), 2);
        assert_eq!(groups.sources[0].members, vec!["c1"]);
        assert_eq!(groups.sources[1].members, vec!["c2"]);
        delete(&conn, "source", &a.id).unwrap();
        let groups = list(&conn).unwrap();
        assert_eq!(groups.sources.len(), 1);
        let alive: i64 = conn.query_row("SELECT COUNT(*) FROM connections", [], |r| r.get(0)).unwrap();
        assert_eq!(alive, 2, "删分组不能删来源");
        move_member(&conn, "source", "c1", None).unwrap();
        assert!(conn.query_row("SELECT COUNT(*) FROM source_group_members", [], |r| r.get::<_, i64>(0)).unwrap() <= 1);
    }

    #[test]
    fn reorder_sets_sort_and_rejects_stale_lists() {
        let conn = db();
        let a = create(&conn, "source", "A").unwrap();
        let b = create(&conn, "source", "B").unwrap();
        reorder(&conn, "source", &[b.id.clone(), a.id.clone()]).unwrap();
        let groups = list(&conn).unwrap();
        assert_eq!(groups.sources[0].id, b.id);
        assert_eq!(groups.sources[1].id, a.id);
        assert!(reorder(&conn, "source", &[a.id.clone()]).is_err(), "缺项视为视图过期");
        assert!(reorder(&conn, "source", &[a.id, "nope".into()]).is_err());
    }

    #[test]
    fn duplicate_and_invalid_names_are_rejected() {
        let conn = db();
        create(&conn, "agent", "编码 Agent").unwrap();
        assert!(create(&conn, "agent", "编码 Agent").is_err());
        assert!(create(&conn, "agent", "  ").is_err());
        assert!(create(&conn, "nope", "x").is_err());
    }
}
