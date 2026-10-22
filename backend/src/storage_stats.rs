//! 设置 · 数据与存储（Jolu.pen 画板 92）的本地数据概览：请求记录数与会话索引数。
//! 容量与路径沿用 `data_info`；这里只做计数，不读取任何会话正文或凭证。

use rusqlite::Connection;
use serde::Serialize;
use tauri::State;

#[derive(Debug, Serialize, PartialEq)]
pub struct StorageCounts {
    /// requests 表的全部行（含被聚合口径排除的退化记录，反映实际占用）
    pub requests: i64,
    /// sessions 表的会话索引条数
    pub sessions: i64,
}

pub fn counts(conn: &Connection) -> rusqlite::Result<StorageCounts> {
    let requests = conn.query_row("SELECT COUNT(*) FROM requests", [], |row| row.get(0))?;
    let sessions = conn.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
    Ok(StorageCounts { requests, sessions })
}

#[tauri::command]
pub async fn storage_counts(db: State<'_, crate::DbRead>) -> Result<StorageCounts, String> {
    crate::with_reader(db.0.clone(), |conn| counts(conn).map_err(|e| e.to_string())).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_requests_and_sessions_separately() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE requests (id INTEGER PRIMARY KEY);
             CREATE TABLE sessions (platform TEXT, session_id TEXT);
             INSERT INTO requests DEFAULT VALUES;
             INSERT INTO requests DEFAULT VALUES;
             INSERT INTO sessions VALUES ('claude', 'a');",
        )
        .unwrap();
        assert_eq!(counts(&conn).unwrap(), StorageCounts { requests: 2, sessions: 1 });
    }

    #[test]
    fn empty_tables_report_real_zero() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE requests (id INTEGER); CREATE TABLE sessions (id INTEGER);").unwrap();
        assert_eq!(counts(&conn).unwrap(), StorageCounts { requests: 0, sessions: 0 });
    }
}
