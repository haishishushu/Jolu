use std::{fs, io::{Read, Write}, path::Path, sync::Mutex, time::{Duration, Instant}};
use super::db::{ImportPreview, ImportSummary, ParsedBackup};
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

fn read_backup(path: &Path) -> Result<String, String> {
    const LIMIT: u64 = 100 * 1024 * 1024;
    let file = fs::File::open(path).map_err(|e| format!("无法读取备份：{e}"))?;
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT { return Err("备份超过 100 MB，请拆分后导入".into()); }
    let text = String::from_utf8(bytes).map_err(|_| "备份必须使用 UTF-8 编码")?;
    Ok(text.trim_start_matches('\u{feff}').to_string())
}

pub(crate) fn write_backup(path: &Path, json: &str) -> Result<(), String> {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
    let tmp = path.with_file_name(format!(".jolu-export-{}-{nonce}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new().create_new(true).write(true).open(&tmp).map_err(|e| e.to_string())?;
    let result = (|| {
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() { let _ = fs::remove_file(&tmp); }
    result.map_err(|e: std::io::Error| format!("保存失败：{e}"))
}

/// 预览后暂存的备份：最多保留 1 份，10 分钟后过期
pub(crate) const PENDING_TTL: Duration = Duration::from_secs(10 * 60);
const PENDING_EXPIRED: &str = "预览已过期，请重新选择文件";

pub(crate) struct PendingImport { token: String, created: Instant, parsed: ParsedBackup }

static PENDING: Mutex<Option<PendingImport>> = Mutex::new(None);

/// 新的预览替换旧的，只保留最新一份
pub(crate) fn stash_pending(slot: &Mutex<Option<PendingImport>>, token: String, parsed: ParsedBackup, now: Instant) {
    if let Ok(mut pending) = slot.lock() { *pending = Some(PendingImport { token, created: now, parsed }); }
}

/// 取出暂存的备份（一次性）：token 不符或已超过 10 分钟都按过期处理
pub(crate) fn take_pending(slot: &Mutex<Option<PendingImport>>, token: &str, now: Instant) -> Result<ParsedBackup, String> {
    let mut pending = slot.lock().map_err(|e| e.to_string())?;
    match pending.take() {
        Some(item) if item.token == token && now.saturating_duration_since(item.created) <= PENDING_TTL => Ok(item.parsed),
        other => {
            // token 不符时保留现有预览，过期的直接丢弃
            *pending = other.filter(|item| now.saturating_duration_since(item.created) <= PENDING_TTL);
            Err(PENDING_EXPIRED.into())
        }
    }
}

/// 丢弃暂存的备份：只有 token 仍是当前这份时才清空，返回是否清空。
/// 取消预览与 10 分钟到期清理共用；已被新预览替换时什么都不做。
pub(crate) fn discard_pending(slot: &Mutex<Option<PendingImport>>, token: &str) -> bool {
    let removed = match slot.lock() {
        Ok(mut pending) if pending.as_ref().is_some_and(|item| item.token == token) => pending.take(),
        _ => None,
    };
    // 解析后的备份可能很大，在锁外释放
    removed.is_some()
}

/// 选择备份文件并预览（画板 100 · 05）：只读校验，不写库；用户取消选择时返回 None
#[tauri::command]
pub async fn preview_import_file(app: tauri::AppHandle, db: State<'_, super::DbRead>) -> Result<Option<ImportPreview>, String> {
    let window = app.get_webview_window(super::MAIN).ok_or("请从主面板导入数据")?;
    let dialog = app.dialog().file().set_parent(&window).set_title("导入统计备份").add_filter("JSON 备份", &["json"]);
    let selected = tauri::async_runtime::spawn_blocking(move || -> Result<Option<(String, String)>, String> {
        let Some(file) = dialog.blocking_pick_file() else { return Ok(None); };
        let path = file.into_path().map_err(|e| e.to_string())?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        read_backup(&path).map(|json| Some((name, json)))
    }).await.map_err(|e| e.to_string())??;
    let Some((file_name, json)) = selected else { return Ok(None); };
    let (mut preview, parsed) = super::with_reader(db.0.clone(), move |conn| super::db::preview_backup(conn, &json)).await?;
    preview.file_name = file_name;
    stash_pending(&PENDING, preview.token.clone(), parsed, Instant::now());
    // 到期后主动释放内存（常驻托盘）：到点时还是这份预览才清空
    let token = preview.token.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(PENDING_TTL).await;
        discard_pending(&PENDING, &token);
    });
    Ok(Some(preview))
}

/// 取消或关闭导入预览：释放暂存的备份
#[tauri::command]
pub fn discard_import(token: String) -> Result<(), String> {
    discard_pending(&PENDING, &token);
    Ok(())
}

/// 确认导入：只插入有效且不重复的记录；预览过期时报「预览已过期，请重新选择文件」
#[tauri::command]
pub async fn confirm_import(app: tauri::AppHandle, db: State<'_, super::Db>, token: String) -> Result<ImportSummary, String> {
    let parsed = take_pending(&PENDING, &token, Instant::now())?;
    let result = super::with_database(db.0.clone(), move |conn| super::db::import_parsed(conn, &parsed)).await?;
    let _ = app.emit("refresh-requested", ());
    Ok(result)
}

/// 导出范围校验：平台不传表示全部，传入时必须是平台目录里的已知平台；
/// 日期不传表示全部，传入时起止都要有且开始早于结束（左闭右开）
pub(crate) fn valid_export_scope(platform: Option<&str>, start_ms: Option<i64>, end_ms: Option<i64>) -> Result<(), String> {
    if platform.is_some_and(|p| !super::platforms::known(p)) { return Err("导出范围无效".into()); }
    match (start_ms, end_ms) {
        (None, None) => Ok(()),
        (Some(start), Some(end)) if start < end => Ok(()),
        _ => Err("导出日期范围无效".into()),
    }
}

#[tauri::command]
pub async fn export_data_file(app: tauri::AppHandle, db: State<'_, super::DbRead>, platform: Option<String>, start_ms: Option<i64>, end_ms: Option<i64>) -> Result<Option<String>, String> {
    valid_export_scope(platform.as_deref(), start_ms, end_ms)?;
    let window = app.get_webview_window(super::MAIN).ok_or("请从主面板导出数据")?;
    let day = |ms: i64| chrono::TimeZone::timestamp_millis_opt(&chrono::Local, ms).single().map(|d| d.format("%Y%m%d").to_string()).unwrap_or_default();
    let dates = match (start_ms, end_ms) {
        (Some(start), Some(end)) => format!("{}-{}", day(start), day(end - 1)),
        _ => chrono::Local::now().format("%Y-%m-%d").to_string(),
    };
    let name = format!("jolu-{}-{dates}.json", platform.as_deref().unwrap_or("all"));
    let dialog = app.dialog().file().set_parent(&window).set_title("导出统计备份").add_filter("JSON 备份", &["json"]).set_file_name(name);
    let selected = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file()).await.map_err(|e| e.to_string())?;
    let Some(file) = selected else { return Ok(None); };
    let path = file.into_path().map_err(|e| e.to_string())?;
    let json = super::export_data(db, platform, start_ms, end_ms).await?;
    tauri::async_runtime::spawn_blocking(move || {
        write_backup(&path, &json)?;
        Ok(Some(path.display().to_string()))
    }).await.map_err(|e| e.to_string())?
}

/// 调用明细导出（画板 34）：按当前筛选写 CSV，返回保存路径与条数；取消保存时返回 None
#[derive(serde::Serialize)]
pub struct CsvExport { pub path: String, pub count: i64 }

#[tauri::command]
pub async fn export_calls_csv(app: tauri::AppHandle, db: State<'_, super::DbRead>, filter: super::usage_stats::StatsFilter) -> Result<Option<CsvExport>, String> {
    filter.validate()?;
    let window = app.get_webview_window(super::MAIN).ok_or("请从主面板导出")?;
    let day = |ms: i64| chrono::TimeZone::timestamp_millis_opt(&chrono::Local, ms).single().map(|d| d.format("%Y%m%d").to_string()).unwrap_or_default();
    let name = format!("jolu-calls-{}-{}.csv", day(filter.start), day(filter.end - 1));
    let dialog = app.dialog().file().set_parent(&window).set_title("导出调用明细").add_filter("CSV 表格", &["csv"]).set_file_name(name);
    let selected = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file()).await.map_err(|e| e.to_string())?;
    let Some(file) = selected else { return Ok(None); };
    let path = file.into_path().map_err(|e| e.to_string())?;
    let (csv, count) = super::with_reader(db.0.clone(), move |conn| super::usage_stats::calls_csv(conn, &filter).map_err(|e| e.to_string())).await?;
    tauri::async_runtime::spawn_blocking(move || {
        write_backup(&path, &csv)?;
        Ok(Some(CsvExport { path: path.display().to_string(), count }))
    }).await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_roundtrip_replaces_existing_backup_and_accepts_utf8_bom() {
        let dir = std::env::temp_dir().join(format!("island-backup-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("backup.json");
        fs::write(&path, "old").unwrap();
        write_backup(&path, "\u{feff}{\"name\":\"统计\"}").unwrap();
        assert_eq!(read_backup(&path).unwrap(), "{\"name\":\"统计\"}");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn export_scope_accepts_every_known_platform() {
        // 直接读随程序打包的平台目录，新增平台时自动覆盖
        let catalog: Vec<serde_json::Value> = serde_json::from_str(include_str!("../../frontend/src/lib/platformCatalog.json")).unwrap();
        assert!(!catalog.is_empty());
        for platform in &catalog {
            let id = platform["id"].as_str().unwrap();
            assert_eq!(valid_export_scope(Some(id), None, None), Ok(()), "{id} 应允许导出");
        }
        assert_eq!(valid_export_scope(None, None, None), Ok(()));
        assert_eq!(valid_export_scope(Some("unknown"), None, None), Err("导出范围无效".to_string()));
    }

    #[test]
    fn export_scope_requires_complete_half_open_range() {
        assert_eq!(valid_export_scope(Some("claude"), Some(1_000), Some(2_000)), Ok(()));
        let invalid = Err("导出日期范围无效".to_string());
        assert_eq!(valid_export_scope(None, Some(2_000), Some(2_000)), invalid);
        assert_eq!(valid_export_scope(None, Some(3_000), Some(2_000)), invalid);
        assert_eq!(valid_export_scope(None, Some(1_000), None), invalid);
        assert_eq!(valid_export_scope(None, None, Some(1_000)), invalid);
    }

    #[test]
    fn pending_import_expires_and_is_single_use() {
        let slot = std::sync::Mutex::new(None);
        let now = std::time::Instant::now();
        let expired = Some("预览已过期，请重新选择文件".to_string());
        stash_pending(&slot, "a".into(), ParsedBackup::default(), now);
        // 只保留最新一份：新的预览替换旧的
        stash_pending(&slot, "b".into(), ParsedBackup::default(), now);
        assert_eq!(take_pending(&slot, "a", now).err(), expired.clone());
        assert!(take_pending(&slot, "b", now + std::time::Duration::from_secs(60)).is_ok());
        assert_eq!(take_pending(&slot, "b", now).err(), expired.clone());
        stash_pending(&slot, "c".into(), ParsedBackup::default(), now);
        assert_eq!(take_pending(&slot, "c", now + PENDING_TTL + std::time::Duration::from_secs(1)).err(), expired);
    }

    #[test]
    fn discard_pending_only_clears_matching_token() {
        // 取消预览与 10 分钟到期清理共用：token 仍相同才清空，被新预览替换后什么都不做
        let slot = std::sync::Mutex::new(None);
        let now = std::time::Instant::now();
        stash_pending(&slot, "old".into(), ParsedBackup::default(), now);
        stash_pending(&slot, "new".into(), ParsedBackup::default(), now);
        assert!(!discard_pending(&slot, "old"));
        assert!(slot.lock().unwrap().is_some());
        assert!(discard_pending(&slot, "new"));
        assert!(slot.lock().unwrap().is_none());
        assert!(!discard_pending(&slot, "new"));
    }
}
