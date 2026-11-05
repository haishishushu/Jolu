//! 胶囊灵动岛的全局指针推送（设计说明 §6.2）：bot 的眼睛在鼠标离开灵动岛窗口后仍能跟随。
//!
//! 灵动岛是小的透明窗口，DOM 指针事件只覆盖窗口内部。这里约 30Hz 读取桌面物理坐标，
//! 位置变化才发 `island-pointer`。
//!
//! 成本控制：
//! - Windows 直接调 `GetCursorPos`（线程安全，不经事件循环）；`AppHandle::cursor_position()`
//!   每次都要和主线程往返，只作其他平台的兜底。
//! - 设置每秒只读一次：只在「胶囊样式 + 灵动岛可见 + 非免打扰」时采样；拖动窗口期间暂停
//!   （窗口原点在变，前端此时只用窗口内 pointermove）。
//! - 事件只有灵动岛窗口订阅；Tauri 对默认目标（Any）的监听不做窗口过滤，靠的是其他窗口不订阅。
//!
//! 只发坐标，不记录、不落盘。

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

pub(crate) const EVENT: &str = "island-pointer";
const ACTIVE_INTERVAL: Duration = Duration::from_millis(33);
const IDLE_INTERVAL: Duration = Duration::from_secs(1);
const SETTINGS_RECHECK: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, serde::Serialize)]
struct Pointer { x: f64, y: f64 }

/// 两次采样的位置差小于半个物理像素时不发事件。
fn moved(previous: Option<(f64, f64)>, next: (f64, f64)) -> bool {
    previous.map_or(true, |(x, y)| (x - next.0).abs() >= 0.5 || (y - next.1).abs() >= 0.5)
}

fn should_sample(settings: &crate::settings::Settings) -> bool {
    settings.island_style == "capsule" && settings.island_visible && !settings.dnd
}

#[cfg(target_os = "windows")]
fn cursor(_app: &tauri::AppHandle) -> Option<(f64, f64)> {
    #[repr(C)]
    struct Point { x: i32, y: i32 }
    #[link(name = "user32")]
    extern "system" { fn GetCursorPos(point: *mut Point) -> i32; }
    let mut point = Point { x: 0, y: 0 };
    // 锁屏、安全桌面等情况下会失败，此时不发事件
    (unsafe { GetCursorPos(&mut point) } != 0).then(|| (f64::from(point.x), f64::from(point.y)))
}

#[cfg(not(target_os = "windows"))]
fn cursor(app: &tauri::AppHandle) -> Option<(f64, f64)> {
    app.cursor_position().ok().map(|p| (p.x, p.y))
}

pub(crate) fn start(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("island-pointer".into())
        .spawn(move || {
            let mut last: Option<(f64, f64)> = None;
            let mut active = false;
            let mut checked: Option<Instant> = None;
            loop {
                if checked.map_or(true, |at| at.elapsed() >= SETTINGS_RECHECK) {
                    active = app.try_state::<crate::Cfg>().is_some_and(|cfg| should_sample(&cfg.0.get()));
                    checked = Some(Instant::now());
                }
                if !active {
                    last = None;
                    std::thread::sleep(IDLE_INTERVAL);
                    continue;
                }
                if !crate::dock::DRAGGING.load(Ordering::Relaxed) {
                    if let Some(next) = cursor(&app) {
                        if moved(last, next) {
                            last = Some(next);
                            let _ = app.emit(EVENT, Pointer { x: next.0, y: next.1 });
                        }
                    }
                }
                std::thread::sleep(ACTIVE_INTERVAL);
            }
        })
        .map(|_| ())
        .unwrap_or_else(|error| eprintln!("[灵动岛] 指针采样线程启动失败：{error}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_emits_when_position_changes() {
        assert!(moved(None, (10.0, 10.0)));
        assert!(!moved(Some((10.0, 10.0)), (10.2, 10.3)));
        assert!(moved(Some((10.0, 10.0)), (10.6, 10.0)));
        assert!(moved(Some((10.0, 10.0)), (10.0, 9.0)));
    }

    #[test]
    fn samples_only_for_visible_capsule_outside_dnd() {
        let mut settings = crate::settings::Settings::default();
        assert!(!should_sample(&settings), "默认经典样式不采样");
        settings.island_style = "capsule".into();
        assert!(should_sample(&settings));
        settings.dnd = true;
        assert!(!should_sample(&settings), "免打扰暂停动效，也不采样");
        settings.dnd = false;
        settings.island_visible = false;
        assert!(!should_sample(&settings), "隐藏灵动岛时不采样");
    }
}
