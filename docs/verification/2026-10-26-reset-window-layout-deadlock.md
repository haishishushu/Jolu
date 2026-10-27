# 重置窗口位置死锁修复

## 原因与改动

`reset_window_layout` 持有 `MAIN_PLACEMENT` 互斥锁时调用 `main_window::capture`，原生窗口查询需要等待 UI 线程；UI 线程的移动或缩放事件回调又等待该锁，形成互相等待。

改为先查询原生窗口位置、尺寸，再短暂持锁写入结果。主窗口初始化 `main_panel_ready` 的同类调用也统一调整。未修改窗口布局、账号、历史数据或产品窗口权限。

## 验证

- 修复前使用独立 `.acceptance-profile` 数据目录和 WebView2 缓存，点击真实主面板「重置位置」，复现超过 8 秒未返回。
- `cargo test --manifest-path backend/Cargo.toml main_window::tests --lib`：2 项通过。
- `cargo build --manifest-path backend/Cargo.toml --bin jolu`：通过；链接器输出创建库的既有提示。
- `scripts/acceptance/reset-window-layout.e2e.mjs`：修复后通过。覆盖主面板按钮、菜单和命令重复重置、本体与分身解除停靠、隐藏后重置及重新打开，核对主窗口恢复 1280 × 900 逻辑尺寸且 IPC 继续响应。
- 桌面测试仅操作隔离实例；结束测试后重新启动正常数据目录的新版应用。

回归脚本通过 `cdp.mjs` 核验隔离数据库路径和标记，窗口操作设置超时，未扩大应用权限。

参考：通过 Context7 核对 [Tauri 命令执行线程说明](https://v2.tauri.app/develop/calling-rust/#async-commands)，并结合本项目窗口事件回调与锁持有顺序定位。
