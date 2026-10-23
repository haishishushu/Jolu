//! 应用自更新（画布 17 · 更新功能）
//!
//! 走 tauri-plugin-updater：更新端点与签名公钥配置在 tauri.conf.json，
//! 发布时用 `tauri signer` 私钥对安装包签名，Release 附 latest.json。
//! 前端只在启动 1 秒后的静默检查与设置页手动检查时调用 `check_app_update_available`；
//! `download_app_update` 下载并校验签名，`install_downloaded_update_and_restart`
//! 在用户点击安装后使用已下载的安装包，避免再次下载。

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, Updater, UpdaterExt};
use url::Url;

/// 按应用联网方式（设置 · 网络与代理）构造 updater。
///
/// 必须显式注入代理：插件内部的 reqwest 与本项目依赖的不是同一个大版本
/// （插件 0.13 / 本项目 0.12），Cargo 的特性统一不跨大版本，我们开启的
/// `system-proxy` 传不过去。不显式设置的话，开着系统代理的用户在检查更新时
/// 仍会直连 GitHub 而超时。
/// 手动代理用设置里的地址；直连不设代理；跟随系统沿用环境变量优先、其次注册表的读取方式。
fn updater_with_proxy(app: &AppHandle) -> Result<Updater, String> {
    let mut builder = app.updater_builder().timeout(std::time::Duration::from_secs(30));
    #[cfg(target_os = "windows")]
    {
        let exit_app = app.clone();
        builder = builder.on_before_exit(move || {
            // 插件默认 cleanup_before_exit 会销毁窗口资源；ShellExecute 失败后无法恢复。
            // 改为只隐藏托盘，启动成功后进程退出由系统回收；失败时保留可操作界面。
            if let Some(tray) = exit_app.tray_by_id("main-tray") {
                let _ = tray.set_visible(false);
            }
        });
    }
    let proxy = match crate::net::current_mode() {
        crate::net::NetMode::Direct => {
            builder = builder.no_proxy();
            None
        }
        crate::net::NetMode::Manual(url) => Some(url),
        crate::net::NetMode::System => crate::net::system_proxy_url(),
    };
    if let Some(proxy) = proxy.and_then(|url| Url::parse(&url).ok()) {
        builder = builder.proxy(proxy);
    }
    builder.build().map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct UpdateCheck {
    pub available: bool,
    pub current_version: String,
    pub available_version: String,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
}

/// 下载进度事件负载；total 在拿到 content-length 前为 None
#[derive(Clone, serde::Serialize)]
pub struct UpdateDownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

struct PreparedUpdate {
    update: Update,
    bytes: Vec<u8>,
}

pub struct UpdateDownloadState {
    prepared: Mutex<Option<PreparedUpdate>>,
    busy: AtomicBool,
    cancel_download: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl Default for UpdateDownloadState {
    fn default() -> Self {
        Self {
            prepared: Mutex::new(None),
            busy: AtomicBool::new(false),
            cancel_download: Mutex::new(None),
        }
    }
}

struct OperationGuard<'a>(&'a AtomicBool);

impl<'a> OperationGuard<'a> {
    fn acquire(busy: &'a AtomicBool) -> Result<Self, String> {
        busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "更新操作正在进行中，请稍候".to_string())?;
        Ok(Self(busy))
    }
}

impl Drop for OperationGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn update_check(update: &Update) -> UpdateCheck {
    UpdateCheck {
        available: true,
        current_version: update.current_version.clone(),
        available_version: update.version.clone(),
        notes: update.body.clone(),
        pub_date: update.date.map(|date| date.to_string()),
    }
}

/// 检查更新总开关：首个 Jolu Release 发布前停用，停用时不发出任何网络请求。
/// 与前端 lib/updateState.ts 的 UPDATES_ENABLED 同步修改。
const UPDATES_ENABLED: bool = match option_env!("JOLU_UPDATES_ENABLED") {
    Some(value) => matches!(value.as_bytes(), [b'1']),
    None => false,
};
const UPDATES_DISABLED: &str = "检查更新暂时停用";

#[tauri::command]
pub fn set_auto_check_updates(app: AppHandle, on: bool) -> Result<crate::settings::Settings, String> {
    let next = app.state::<crate::Cfg>().0.try_update(|settings| settings.auto_check_updates = on)?;
    let _ = app.emit("settings-changed", &next);
    Ok(next)
}

/// 只查询是否有新版本，不下载。端点不可达或公钥校验失败时返回 Err，
/// 由前端决定静默（启动检查失败不打扰）或提示（手动检查 toast）。
#[tauri::command]
pub async fn check_app_update_available(app: AppHandle) -> Result<UpdateCheck, String> {
    if !UPDATES_ENABLED { return Err(UPDATES_DISABLED.into()); }
    let updater = updater_with_proxy(&app)?;
    Ok(match updater.check().await.map_err(|e| e.to_string())? {
        Some(update) => update_check(&update),
        None => UpdateCheck {
            available: false,
            current_version: app.package_info().version.to_string(),
            available_version: String::new(),
            notes: None,
            pub_date: None,
        },
    })
}

/// 下载并校验签名，保留安装包供用户在安装界面一键安装。
#[tauri::command]
pub async fn download_app_update(
    app: AppHandle,
    state: State<'_, UpdateDownloadState>,
) -> Result<UpdateCheck, String> {
    if !UPDATES_ENABLED { return Err(UPDATES_DISABLED.into()); }
    let _guard = OperationGuard::acquire(&state.busy)?;
    *state.prepared.lock().map_err(|e| e.to_string())? = None;
    let (cancel, cancelled) = tokio::sync::oneshot::channel();
    *state.cancel_download.lock().map_err(|e| e.to_string())? = Some(cancel);
    let result = cancellable_download(cancelled, async {
        let updater = updater_with_proxy(&app)?;
        let mut update = updater
            .check()
            .await
            .map_err(|e| e.to_string())?
            .ok_or("当前已是最新版本")?;
        // 插件把检查和下载超时分别存储；安装包允许更长的传输时间。
        update.timeout = Some(std::time::Duration::from_secs(600));
        let info = update_check(&update);
        let progress_app = app.clone();
        let mut downloaded: u64 = 0;
        let mut last_progress: Option<std::time::Instant> = None;
        let bytes = update
            .download(
                move |chunk, total| {
                    downloaded = downloaded.saturating_add(chunk as u64);
                    let now = std::time::Instant::now();
                    if last_progress.is_none_or(|last| now.duration_since(last).as_millis() >= 100)
                        || total.is_some_and(|total| downloaded >= total)
                    {
                        last_progress = Some(now);
                        let _ = progress_app.emit(
                            "update-download-progress",
                            UpdateDownloadProgress { downloaded, total },
                        );
                    }
                },
                || {},
            )
            .await
            .map_err(|e| format!("下载更新失败：{e}"))?;
        *state.prepared.lock().map_err(|e| e.to_string())? =
            Some(PreparedUpdate { update, bytes });
        Ok(info)
    }).await;
    state.cancel_download.lock().map_err(|e| e.to_string())?.take();
    result
}

const DOWNLOAD_CANCELLED: &str = "更新下载已取消";

async fn cancellable_download<T>(
    mut cancelled: tokio::sync::oneshot::Receiver<()>,
    work: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::select! {
        biased;
        _ = &mut cancelled => Err(DOWNLOAD_CANCELLED.into()),
        result = work => result,
    }
}

#[tauri::command]
pub fn cancel_app_update_download(state: State<'_, UpdateDownloadState>) -> Result<(), String> {
    if let Some(cancel) = state.cancel_download.lock().map_err(|e| e.to_string())?.take() {
        let _ = cancel.send(());
    }
    Ok(())
}

/// 在持有写连接锁期间检查并安装，避免调度器在检查后又认领新任务。
fn ensure_install_allowed(conn: &rusqlite::Connection) -> Result<(), String> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='automation_runs')",
        [], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if exists {
        let running: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM automation_runs WHERE status='running')",
            [], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if running {
            return Err("仍有自动化任务正在运行，请等待完成或先停止任务后再安装更新".into());
        }
    }
    Ok(())
}

/// 安装已校验的更新包；阻塞安装和数据库锁等待放在线程池中，避免卡住界面。
#[tauri::command]
pub async fn install_downloaded_update_and_restart(app: AppHandle) -> Result<(), String> {
    if !UPDATES_ENABLED { return Err(UPDATES_DISABLED.into()); }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<UpdateDownloadState>();
        let _guard = OperationGuard::acquire(&state.busy)?;
        let db = app.state::<crate::Db>();
        let conn = db.0.lock().map_err(|error| error.to_string())?;
        ensure_install_allowed(&conn)?;
        let prepared = state.prepared.lock().map_err(|e| e.to_string())?
            .take().ok_or("安装包尚未下载完成，请先点击更新版本按钮")?;
        if let Err(error) = prepared.update.install(&prepared.bytes) {
            #[cfg(target_os = "windows")]
            if let Some(tray) = app.tray_by_id("main-tray") {
                let _ = tray.set_visible(true);
            }
            *state.prepared.lock().map_err(|e| e.to_string())? = Some(prepared);
            return Err(format!("安装更新失败：{error}"));
        }
        drop(conn);
        #[cfg(not(target_os = "windows"))]
        app.restart();
        Ok(())
    }).await.map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_operations_are_exclusive_and_retry_after_failure() {
        let busy = AtomicBool::new(false);
        {
            let _first = OperationGuard::acquire(&busy).unwrap();
            assert!(OperationGuard::acquire(&busy).is_err());
        }
        assert!(OperationGuard::acquire(&busy).is_ok());
    }

    #[tokio::test]
    async fn cancellation_stops_pending_download_and_next_attempt_can_complete() {
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        cancel.send(()).unwrap();
        let result = cancellable_download::<()>(cancelled, std::future::pending()).await;
        assert_eq!(result.unwrap_err(), DOWNLOAD_CANCELLED);
        let (_cancel, cancelled) = tokio::sync::oneshot::channel();
        assert_eq!(cancellable_download(cancelled, async { Ok(42) }).await.unwrap(), 42);
    }

    #[test]
    fn install_blocks_running_tasks_but_allows_waiting_and_completed() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        assert!(ensure_install_allowed(&db).is_ok());
        db.execute_batch("CREATE TABLE automation_runs(status TEXT); INSERT INTO automation_runs VALUES ('waiting'), ('queued'), ('completed');").unwrap();
        assert!(ensure_install_allowed(&db).is_ok());
        db.execute("INSERT INTO automation_runs VALUES ('running')", []).unwrap();
        assert!(ensure_install_allowed(&db).unwrap_err().contains("自动化任务"));
        db.execute("UPDATE automation_runs SET status='completed' WHERE status='running'", []).unwrap();
        assert!(ensure_install_allowed(&db).is_ok());
    }
}
