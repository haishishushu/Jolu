use std::{fs, io::{Read, Write}, path::Path};
use tauri::{Manager, State};
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

#[tauri::command]
pub async fn import_data_file(app: tauri::AppHandle, db: State<'_, super::Db>) -> Result<Option<super::db::ImportSummary>, String> {
    let window = app.get_webview_window(super::MAIN).ok_or("请从主面板导入数据")?;
    let dialog = app.dialog().file().set_parent(&window).set_title("导入统计备份").add_filter("JSON 备份", &["json"]);
    let selected = tauri::async_runtime::spawn_blocking(move || -> Result<Option<String>, String> {
        let Some(file) = dialog.blocking_pick_file() else { return Ok(None); };
        read_backup(&file.into_path().map_err(|e| e.to_string())?).map(Some)
    }).await.map_err(|e| e.to_string())??;
    match selected {
        None => Ok(None),
        Some(json) => super::import_data(app, db, json).await.map(Some),
    }
}

/// 导出范围校验：不传表示全部；传入时必须是平台目录里的已知平台
pub(crate) fn valid_export_scope(platform: Option<&str>) -> Result<(), String> {
    if platform.is_some_and(|p| !super::platforms::known(p)) { return Err("导出范围无效".into()); }
    Ok(())
}

#[tauri::command]
pub async fn export_data_file(app: tauri::AppHandle, db: State<'_, super::DbRead>, platform: Option<String>) -> Result<Option<String>, String> {
    valid_export_scope(platform.as_deref())?;
    let window = app.get_webview_window(super::MAIN).ok_or("请从主面板导出数据")?;
    let name = format!("jolu-{}-{}.json", platform.as_deref().unwrap_or("all"), chrono::Local::now().format("%Y-%m-%d"));
    let dialog = app.dialog().file().set_parent(&window).set_title("导出统计备份").add_filter("JSON 备份", &["json"]).set_file_name(name);
    let selected = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file()).await.map_err(|e| e.to_string())?;
    let Some(file) = selected else { return Ok(None); };
    let path = file.into_path().map_err(|e| e.to_string())?;
    let json = super::export_data(db, platform).await?;
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
            assert_eq!(valid_export_scope(Some(id)), Ok(()), "{id} 应允许导出");
        }
        assert_eq!(valid_export_scope(None), Ok(()));
        assert_eq!(valid_export_scope(Some("unknown")), Err("导出范围无效".to_string()));
    }
}
