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
use tauri::{AppHandle, Emitter, State};
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
    let mut builder = app.updater_builder();
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
    downloading: AtomicBool,
}

impl Default for UpdateDownloadState {
    fn default() -> Self {
        Self {
            prepared: Mutex::new(None),
            downloading: AtomicBool::new(false),
        }
    }
}

struct DownloadGuard<'a>(&'a AtomicBool);

impl Drop for DownloadGuard<'_> {
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
const UPDATES_ENABLED: bool = false;
const UPDATES_DISABLED: &str = "检查更新暂时停用";

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
    if state.downloading.swap(true, Ordering::AcqRel) {
        return Err("更新包正在下载中".into());
    }
    let _guard = DownloadGuard(&state.downloading);
    *state.prepared.lock().map_err(|e| e.to_string())? = None;
    async {
        let updater = updater_with_proxy(&app)?;
        let update = updater
            .check()
            .await
            .map_err(|e| e.to_string())?
            .ok_or("当前已是最新版本")?;
        let info = update_check(&update);
        let progress_app = app.clone();
        let mut downloaded: u64 = 0;
        let bytes = update
            .download(
                move |chunk, total| {
                    downloaded = downloaded.saturating_add(chunk as u64);
                    let _ = progress_app.emit(
                        "update-download-progress",
                        UpdateDownloadProgress { downloaded, total },
                    );
                },
                || {},
            )
            .await
            .map_err(|e| format!("下载更新失败：{e}"))?;
        *state.prepared.lock().map_err(|e| e.to_string())? =
            Some(PreparedUpdate { update, bytes });
        Ok(info)
    }
    .await
}

/// 安装已校验的更新包并重启。
///
/// Windows：NSIS 安装器拉起后当前进程直接退出，新版本由安装器自动重启；
/// 插件 install 内部会硬退出（不走 Drop），因此托盘移除必须前置，
/// 否则安装器接管期间会残留无法自行消失的死图标。
/// macOS / Linux：install 原地替换后正常返回，手动 restart 进新版本。
#[tauri::command]
pub async fn install_downloaded_update_and_restart(
    app: AppHandle,
    state: State<'_, UpdateDownloadState>,
) -> Result<(), String> {
    let prepared = state
        .prepared
        .lock()
        .map_err(|e| e.to_string())?
        .take()
        .ok_or("安装包尚未下载完成，请先点击更新版本按钮")?;

    #[cfg(target_os = "windows")]
    let _ = app.remove_tray_by_id("main-tray");

    if let Err(error) = prepared.update.install(&prepared.bytes) {
        *state.prepared.lock().map_err(|e| e.to_string())? = Some(prepared);
        return Err(format!("安装更新失败：{error}"));
    }

    #[cfg(not(target_os = "windows"))]
    app.restart();

    Ok(())
}
