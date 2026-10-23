//! 需要持久化的运行时设置
//!
//! 灵动岛、常规、外观与数据保留偏好在各窗口共享同一个值，因此集中放在这里，
//! 由 Rust 持有并落到 JSON，避免前端各存一份导致两处不一致。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 仅 Windows 自动启动时不显示主面板；灵动岛仍按显示偏好运行。
    #[serde(default = "default_silent_startup")]
    pub silent_startup: bool,
    /// 灵动岛当前显示的平台。与设置里「灵动岛显示配置」是同一个值
    pub island_platform: String,
    #[serde(default = "default_kind")]
    pub island_kind: String,
    /// 灵动岛绑定的具体连接。None 表示尚未选择，不能静默取同平台第一条连接。
    #[serde(default)]
    pub island_connection_id: Option<String>,
    /// 最近一次选择时的连接名。连接被移除后仍用于说明是哪一个失效连接。
    #[serde(default)]
    pub island_connection_name: Option<String>,
    /// 本地统计目前按平台聚合；保留稳定来源 ID，后续支持多来源时无需迁移配置。
    #[serde(default)]
    pub island_source_id: Option<String>,
    #[serde(default = "default_topmost")]
    pub always_on_top: bool,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_opacity")]
    pub island_opacity: u8,
    #[serde(default = "default_scale")]
    pub island_scale: u16,
    /// 停靠条缩放百分比。仅作用于停靠条；探出/展开仍用 island_scale。
    #[serde(default = "default_shrink_scale")]
    pub island_shrink_scale: u16,
    #[serde(default = "default_refresh")]
    pub refresh_minutes: u32,
    #[serde(default)]
    pub balance_alert_threshold: Option<f64>,
    #[serde(default = "default_currency")]
    pub balance_alert_currency: String,
    /// 是否允许拖到屏幕边缘后自动吸附。
    #[serde(default = "default_dock_enabled")]
    pub dock_enabled: bool,
    /// None 表示不自动清理；其余值由设置页显式选择。
    #[serde(default)]
    pub retention_days: Option<u32>,
    /// 免打扰：只暂停提示与动效，**不停止采集与统计**（§2.6）
    pub dnd: bool,
    /// 灵动岛是否可见。隐藏后托盘是唯一入口
    pub island_visible: bool,
    /// 停靠状态：边缘与沿边偏移，重启后恢复（§2.1.3）
    #[serde(default)]
    pub dock: crate::dock::DockState,
    /// 灵动岛分身列表（画布 22）。本体仍用上面的 island_* 与 dock 字段，分身各自一条。
    #[serde(default)]
    pub island_clones: Vec<crate::island_clones::IslandClone>,
    /// 主面板上次的位置与尺寸；旧版设置缺少此字段时首次打开仍居中。
    #[serde(default)]
    pub main_window: Option<crate::main_window::Placement>,
    /// 本地代理（阶段二）：默认关闭，不开启时应用保持纯只读采集行为。
    #[serde(default)]
    pub proxy_enabled: bool,
    /// 代理监听端口（仅 127.0.0.1）。
    #[serde(default = "default_proxy_port")]
    pub proxy_port: u16,
    /// 上游连续不可达时自动还原 CLI 直连配置并停止代理。
    #[serde(default = "default_proxy_fallback")]
    pub proxy_fallback_direct: bool,
    /// 接管时保存的 Claude 真实上游（settings.json 原 ANTHROPIC_BASE_URL）。URL 非凭证。
    #[serde(default)]
    pub proxy_claude_upstream: Option<String>,
    /// 接管时保存的 Codex 真实上游（config.toml 原 provider base_url）。
    #[serde(default)]
    pub proxy_codex_upstream: Option<String>,
    /// 默认打开页面（画板 86）：`last` 表示上次访问的页面，其余为七个主窗口页面之一。
    #[serde(default = "default_default_page")]
    pub default_page: String,
    /// 上次访问的主窗口页面；只由 `remember_page` 写入，从未记录时为 None。
    #[serde(default)]
    pub last_page: Option<String>,
    /// 主窗口界面缩放百分比（画板 88）；只作用于 `main` 窗口，灵动岛不受影响。
    #[serde(default = "default_main_zoom")]
    pub main_zoom: u16,
    /// 列表密度：`comfortable` 或 `compact`，只作用于主窗口。
    #[serde(default = "default_list_density")]
    pub list_density: String,
    /// 减少动效开关；系统开启「减少动画」时主窗口同样减少，与本值无关。
    #[serde(default)]
    pub reduce_motion: bool,
    /// 应用联网方式（画板 90 / D-24）：`system` 跟随系统、`direct` 直连、`manual` 手动代理。
    /// 只作用于 Jolu 自己的出站请求，CLI 本地代理的上游不跟随。
    #[serde(default = "default_network_mode")]
    pub network_mode: String,
    /// 手动代理地址（仅 http:// / https://，带端口）；切到其他方式时保留，便于切回。
    #[serde(default)]
    pub network_proxy_url: Option<String>,
    /// 任务提醒（画板 94 / D-10）：额度恢复后在原会话继续时通知。
    #[serde(default = "yes")]
    pub notify_task_resumed: bool,
    /// 任务提醒：任务完成与失败（含「需要处理」「等待超时」）时通知。
    #[serde(default = "yes")]
    pub notify_task_finished: bool,
}

fn default_kind() -> String { "api".into() }
fn default_silent_startup() -> bool { true }
fn default_topmost() -> bool { true }
fn default_theme() -> String { "light".into() }
fn default_opacity() -> u8 { 100 }
fn default_scale() -> u16 { 100 }
fn default_shrink_scale() -> u16 { 100 }
fn default_refresh() -> u32 { 5 }
fn default_currency() -> String { "USD".into() }
fn default_dock_enabled() -> bool { true }
fn default_proxy_port() -> u16 { 12731 }
fn default_proxy_fallback() -> bool { true }
fn default_default_page() -> String { "last".into() }
fn default_main_zoom() -> u16 { 100 }
fn default_list_density() -> String { "comfortable".into() }

/// 主窗口的七个页面，与前端 `MainPage` 一致。
const MAIN_PAGES: [&str; 7] = ["island", "providers", "usage", "sessions", "automation", "skills", "settings"];

pub fn validate_default_page(page: &str) -> Result<(), String> {
    if page == "last" || MAIN_PAGES.contains(&page) { Ok(()) } else { Err(format!("未知的默认打开页面：{page}")) }
}

pub fn validate_last_page(page: &str) -> Result<(), String> {
    if MAIN_PAGES.contains(&page) { Ok(()) } else { Err(format!("未知的主窗口页面：{page}")) }
}

pub fn validate_display_preferences(opacity: u8, scale: u16, shrink_scale: u16, refresh_minutes: u32) -> Result<(), String> {
    if !(60..=100).contains(&opacity) || !matches!(scale, 85 | 100 | 115)
        || !matches!(shrink_scale, 75 | 100 | 125)
        || !matches!(refresh_minutes, 1 | 5 | 15 | 30) {
        return Err("透明度须为 60–100%，大小须为 85/100/115%，缩小后大小须为 75/100/125%，刷新间隔须为 1/5/15/30 分钟".into());
    }
    Ok(())
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            silent_startup: true,
            island_platform: "claude".into(),
            island_kind: default_kind(),
            island_connection_id: None,
            island_connection_name: None,
            island_source_id: None,
            always_on_top: true,
            theme: default_theme(),
            island_opacity: default_opacity(),
            island_scale: default_scale(),
            island_shrink_scale: default_shrink_scale(),
            refresh_minutes: default_refresh(),
            balance_alert_threshold: None,
            balance_alert_currency: default_currency(),
            dock_enabled: true,
            retention_days: None,
            dnd: false,
            island_visible: true,
            dock: Default::default(),
            island_clones: Vec::new(),
            main_window: None,
            proxy_enabled: false,
            proxy_port: default_proxy_port(),
            proxy_fallback_direct: default_proxy_fallback(),
            proxy_claude_upstream: None,
            proxy_codex_upstream: None,
            default_page: default_default_page(),
            last_page: None,
            main_zoom: default_main_zoom(),
            list_density: default_list_density(),
            reduce_motion: false,
            network_mode: default_network_mode(),
            network_proxy_url: None,
            notify_task_resumed: true,
            notify_task_finished: true,
        }
    }
}

pub struct Store {
    path: PathBuf,
    inner: Mutex<Settings>,
}

impl Store {
    pub fn load(dir: &std::path::Path) -> Self {
        let path = dir.join("settings.json");
        // 缺失字段逐项补默认值，保留旧版本已经保存的连接；仅无法解析时回落。
        let inner = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
            .unwrap_or_default();
        Self {
            path,
            inner: Mutex::new(inner),
        }
    }

    pub fn get(&self) -> Settings {
        self.inner.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// 系统窗口回调使用：失败保留原快照并记录日志。
    pub fn update(&self, f: impl FnOnce(&mut Settings)) -> Settings {
        match self.try_update(f) {
            Ok(next) => next,
            Err(error) => {
                eprintln!("[设置] {error}");
                self.get()
            }
        }
    }

    /// 写入成功才发布内存快照；持锁覆盖整个保存过程，防止并发写入顺序颠倒。
    pub fn try_update(&self, f: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
        use std::io::Write;
        let mut guard = self.inner.lock().map_err(|e| format!("设置锁获取失败：{e}"))?;
        let mut next = guard.clone();
        f(&mut next);
        let json = serde_json::to_vec_pretty(&next).map_err(|e| format!("设置序列化失败：{e}"))?;
        let temporary = self.path.with_extension("json.tmp");
        let write = || -> std::io::Result<()> {
            if let Some(parent) = self.path.parent() { std::fs::create_dir_all(parent)?; }
            let mut file = std::fs::File::create(&temporary)?;
            file.write_all(&json)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, &self.path)
        };
        if let Err(error) = write() {
            let _ = std::fs::remove_file(&temporary);
            return Err(format!("设置保存失败：{error}"));
        }
        *guard = next.clone();
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn failed_save_keeps_previous_settings() {
        let dir = std::env::temp_dir().join(format!("island-settings-failure-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("settings.json")).unwrap();
        let store = super::Store::load(&dir);
        assert!(store.try_update(|s| s.theme = "dark".into()).is_err());
        assert_eq!(store.get().theme, "light", "失败写入不能伪装成已保存");
        std::fs::remove_dir(dir.join("settings.json")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn settings_roundtrip_preserves_all_preferences() {
        let dir = std::env::temp_dir().join(format!("island-settings-roundtrip-{}", std::process::id()));
        let store = super::Store::load(&dir);
        store.try_update(|s| {
            s.theme = "dark".into(); s.dnd = true; s.dock_enabled = false;
            s.always_on_top = false; s.silent_startup = false;
            s.island_opacity = 70; s.island_scale = 115; s.island_shrink_scale = 125; s.refresh_minutes = 15;
            s.retention_days = Some(90); s.balance_alert_threshold = Some(12.5);
            s.balance_alert_currency = "CNY".into();
            s.island_connection_id = Some("test-connection".into());
            s.island_source_id = Some("local:codex".into()); s.island_platform = "codex".into();
        }).unwrap();
        // 再次替换已有文件，覆盖 Windows 上的保存与重启恢复路径。
        store.try_update(|s| s.theme = "system".into()).unwrap();
        assert_eq!(serde_json::to_value(store.get()).unwrap(), serde_json::to_value(super::Store::load(&dir).get()).unwrap());
        std::fs::remove_file(dir.join("settings.json")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn upgrade_keeps_saved_island_connection_when_other_fields_are_missing() {
        let dir = std::env::temp_dir().join(format!("island-upgrade-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("settings.json"), r#"{"island_platform":"codex","island_kind":"auth","island_connection_id":"saved-account","island_connection_name":"上次使用的连接"}"#).unwrap();
        let store = super::Store::load(&dir);
        assert_eq!(store.get().island_connection_id.as_deref(), Some("saved-account"));
        assert!(store.get().island_visible);
        store.try_update(|s| s.theme = "dark".into()).unwrap();
        let restored = super::Store::load(&dir).get();
        assert_eq!(restored.island_connection_id.as_deref(), Some("saved-account"));
        assert_eq!(restored.island_connection_name.as_deref(), Some("上次使用的连接"));
        assert_eq!(restored.island_platform, "codex");
        assert_eq!(restored.island_kind, "auth");
        std::fs::remove_file(dir.join("settings.json")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn concurrent_updates_keep_disk_and_memory_in_order() {
        let dir = std::env::temp_dir().join(format!("island-settings-concurrent-{}", std::process::id()));
        let store = std::sync::Arc::new(super::Store::load(&dir));
        let workers: Vec<_> = (0..4).map(|_| {
            let store = store.clone();
            std::thread::spawn(move || {
                for _ in 0..8 { store.try_update(|s| s.dock.offset += 1.0).unwrap(); }
            })
        }).collect();
        for worker in workers { worker.join().unwrap(); }
        assert_eq!(store.get().dock.offset, 32.0);
        assert_eq!(super::Store::load(&dir).get().dock.offset, 32.0);
        std::fs::remove_file(dir.join("settings.json")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn display_preferences_validate_and_old_settings_keep_defaults() {
        assert!(super::validate_display_preferences(60, 85, 100, 1).is_ok());
        assert!(super::validate_display_preferences(100, 115, 125, 30).is_ok());
        for (opacity, scale, shrink, refresh) in [(0, 100, 100, 5), (101, 100, 100, 5), (100, 0, 100, 5), (100, 100, 90, 5), (100, 100, 100, 0)] {
            assert!(super::validate_display_preferences(opacity, scale, shrink, refresh).is_err());
        }
        let old = r#"{"island_platform":"claude","dnd":false,"island_visible":true}"#;
        let settings: Settings = serde_json::from_str(old).unwrap();
        assert!(settings.silent_startup);
        assert_eq!((settings.island_opacity, settings.island_scale, settings.island_shrink_scale, settings.refresh_minutes), (100, 100, 100, 5));
    }

    #[test]
    fn older_settings_keep_docking_enabled_and_optional_connection_name_empty() {
        let settings: Settings = serde_json::from_str(
            r#"{"island_platform":"codex","island_kind":"auth","island_connection_id":null,"island_source_id":null,"always_on_top":true,"dnd":false,"island_visible":true,"dock":{"edge":null,"offset":0.0,"monitor":null}}"#,
        )
        .expect("旧设置应可迁移");
        assert!(settings.dock_enabled);
        assert_eq!(settings.theme, "light");
        assert_eq!(settings.island_connection_name, None);
    }

    #[test]
    fn silent_startup_choice_survives_reload_without_changing_island_visibility() {
        let dir = std::env::temp_dir().join(format!("island-startup-settings-{}", std::process::id()));
        let store = super::Store::load(&dir);
        for on in [false, true] {
            store.update(|settings| settings.silent_startup = on);
            let restored = super::Store::load(&dir).get();
            assert_eq!(restored.silent_startup, on);
            assert!(restored.island_visible);
        }
        std::fs::remove_file(dir.join("settings.json")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn default_page_rejects_unknown() {
        assert!(super::validate_default_page("bogus").is_err());
        assert!(super::validate_default_page("").is_err());
        for page in ["last", "island", "providers", "usage", "sessions", "automation", "skills", "settings"] {
            assert!(super::validate_default_page(page).is_ok(), "{page} 应当可选");
        }
        assert!(super::validate_last_page("last").is_err(), "上次访问的页面只能记具体页面");
        assert!(super::validate_last_page("skills").is_ok());
    }

    #[test]
    fn old_settings_default_to_last_visited_page() {
        let settings: Settings = serde_json::from_str(r#"{"island_platform":"claude","dnd":false,"island_visible":true}"#).unwrap();
        assert_eq!(settings.default_page, "last");
        assert_eq!(settings.last_page, None);
        assert_eq!(Settings::default().default_page, "last");
    }
}

// 任务 2.2 开始
/// 主窗口界面缩放档位（画板 88 / D-21）。
pub const MAIN_ZOOM_STEPS: [u16; 6] = [80, 90, 100, 110, 125, 150];

/// 盘上的缩放值可能被手工改坏：不在档位里就按 100% 处理，与前端 `normalizeZoom` 一致。
pub fn normalize_zoom(zoom: u16) -> u16 {
    if MAIN_ZOOM_STEPS.contains(&zoom) { zoom } else { 100 }
}

pub fn validate_appearance(zoom: u16, density: &str) -> Result<(), String> {
    if !MAIN_ZOOM_STEPS.contains(&zoom) {
        return Err("界面缩放须为 80/90/100/110/125/150%".into());
    }
    if !matches!(density, "comfortable" | "compact") {
        return Err("列表密度须为 comfortable 或 compact".into());
    }
    Ok(())
}

#[cfg(test)]
mod appearance_tests {
    use super::Settings;

    #[test]
    fn appearance_validation() {
        for zoom in [80, 90, 100, 110, 125, 150] {
            assert!(super::validate_appearance(zoom, "comfortable").is_ok(), "{zoom}% 应当可选");
        }
        for zoom in [0, 75, 100 - 1, 115, 175, 200] {
            assert!(super::validate_appearance(zoom, "comfortable").is_err(), "{zoom}% 不在档位里");
        }
        assert!(super::validate_appearance(100, "compact").is_ok());
        for density in ["", "Compact", "dense"] {
            assert!(super::validate_appearance(100, density).is_err(), "密度只接受 comfortable / compact：{density}");
        }
    }

    #[test]
    fn saved_zoom_outside_steps_falls_back_to_standard() {
        for zoom in [80, 90, 100, 110, 125, 150] {
            assert_eq!(super::normalize_zoom(zoom), zoom);
        }
        for zoom in [0, 75, 115, 300, u16::MAX] {
            assert_eq!(super::normalize_zoom(zoom), 100, "{zoom}% 不在档位里，按 100% 处理");
        }
    }

    #[test]
    fn old_settings_default_to_standard_appearance() {
        let settings: Settings = serde_json::from_str(r#"{"island_platform":"claude","dnd":false,"island_visible":true}"#).unwrap();
        assert_eq!((settings.main_zoom, settings.list_density.as_str(), settings.reduce_motion), (100, "comfortable", false));
        let defaults = Settings::default();
        assert_eq!((defaults.main_zoom, defaults.list_density.as_str(), defaults.reduce_motion), (100, "comfortable", false));
    }
}
// 任务 2.2 结束
// 任务 2.3 开始
fn default_network_mode() -> String { "system".into() }
// 任务 2.3 结束
// 任务 2.5 开始
fn yes() -> bool { true }

#[cfg(test)]
mod notify_settings_tests {
    use super::Settings;

    #[test]
    fn old_settings_default_task_notices_on() {
        let settings: Settings = serde_json::from_str(r#"{"island_platform":"claude","dnd":false,"island_visible":true}"#).unwrap();
        assert!(settings.notify_task_resumed && settings.notify_task_finished);
        let defaults = Settings::default();
        assert!(defaults.notify_task_resumed && defaults.notify_task_finished);
    }
}
// 任务 2.5 结束
