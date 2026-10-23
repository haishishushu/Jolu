# 隔离读取修复与通知跳转

鼠鼠已在本次会话中确认交接提示词和画板对照（原第 1、2 项）通过；本次处理原第 3、4 项，没有修改设计稿或页面布局。

## 改动总结

- 托盘「立即刷新」原先绕过 `scan_local_sessions` 的隔离保护，直接调用采集器。现隔离分支只发送界面刷新事件；采集器的全量、增量入口也拒绝隔离模式下的用户日志扫描，返回明确错误。
- 会话页原先信任隔离数据库中的 `scan_state.path`，会据此读取真实文件。现隔离模式不返回源文件映射，目录、全文搜索、正文的底层读取也提前返回。隔离数据库中的统计记录仍可显示，源文件视为不可用。
- Tauri 从锁定的 2.11.5 升级为 2.12.5，通知插件从 2.4.0 升级为 2.7.0，最低 Rust 版本同步为 1.90；Cargo.lock 同步相关传递依赖。
- 在 setup 阶段、自动化调度启动前注册 Rust 通知回调。只有 `tap` 且 `extra.task_id` 为正整数时，才在主线程调用既有 `MainIntent::Automation`，显示主窗口并打开对应任务。余额通知、关闭通知、缺少或非法任务 ID 不触发跳转。

## 版本核实

先经 Context7 查询官方插件文档，再核对 crates.io 发布包源码及 CHANGELOG。旧注释“2.5 起支持桌面点击”不准确：2.5.0 的 desktop.rs 仍忽略动作，2.6.0 才引入桌面 `on_action`；本次使用稳定版 2.7.0，要求 Tauri 2.12 / Rust 1.90。本机 Rust 为 1.98.1。

参考：[通知插件源码](https://github.com/tauri-apps/plugins-workspace/tree/v2/plugins/notification)、[2.7.0 发布包文档](https://docs.rs/tauri-plugin-notification/2.7.0/tauri_plugin_notification/)。

## 验证结果

- 隔离回归测试先复现：开启 `JOLU_TEST_DIR` 后，旧采集器仍扫描了 1 个合成用户日志。修复后通过：全量和增量扫描均为 0，导入路径不能用于读取目录或正文，消息返回不可用。
- 测试使用独立子进程设置合成 USERPROFILE / HOME / CODEX_HOME，不修改测试主进程环境，避免影响并行测试。
- 通知动作测试覆盖有效任务、余额通知、零/负数/字符串/小数 ID、dismiss 和缺少 notification；先验证未实现时失败，再通过。
- `cargo test --manifest-path backend/Cargo.toml`：418 通过、0 失败、11 按原配置忽略。
- `pnpm --dir frontend typecheck`：通过；前端纯逻辑测试：186 通过。
- 隔离桌面构建：通过。5173 已被其他项目占用，因此使用 5193，独立构建目录 `output/acceptance/target-5193`，没有停止原项目服务。
- `main-window-smoke.e2e.mjs`：七页全部通过，读取时间为 55–282 ms，没有失败提示、控制台 error 或未捕获异常。
- 在隔离实例调用托盘同一路径 `menu_action refresh`：刷新前后 requests / scan_state 均为 `[0, 0]`；直接调用 `scan_local_sessions` 明确拒绝。
- 隔离目录：`output/acceptance/profile/b6dda1190a4c4ed5a3583c34ab8218ef`；页面截图：`output/acceptance/main-window/`。
- 测试结束后核对进程路径，仅停止本次应用 48804、网关 46976、前端 4040；原项目的 5173 服务保留。

## 验证边界

本次验证了通知回调的数据解析、编译接入和升级后的窗口运行，没有实际点击 Windows 通知中心的通知。因此安装版系统通知点击链路仍需实机验收，不能视为已通过。

通知跳转支持 Jolu 仍在运行（含托盘隐藏主窗口）的情况；当前插件监听不会在应用完全退出后从通知冷启动 Jolu。任务删除后点击旧通知，沿用现有详情查询失败反馈。未新增通知权限、未执行 Git 暂存、提交、推送或切换分支。
