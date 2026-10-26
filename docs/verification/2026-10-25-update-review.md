# 更新链路复审

日期：2026-10-25。范围：后端检查、下载、取消、签名、安装重试，前端更新状态和发布清单校验。

## 结论与改动总结

- 检查更新已有完整的检查、下载、取消、验签和安装入口。本轮没有将默认停用开关改成启用，没有改生产公钥、端点或真实安装。
- 修复发布清单校验的确定问题：原实现只遍历平台值，遗漏 `windows-x86_64`，或者把它指向同版本且签名文件齐全的 Linux 安装包，都会通过校验。现在必须包含 Windows 平台，且其下载资源必须是同版本 `Windows-x86_64-Setup.exe`。
- 新增两项回归用例，先复现两项 `Missing expected exception`，再修改生产校验逻辑，确认通过。
- 后端和前端更新状态本轮未修改。没有为重构而重构，也没有改设计稿或界面。

## 核对依据

- 先读 `2026-10-23-update-audit.md`，再核对当前代码，避免重复实施。
- Context7 按 `library tauri updater` → `docs /tauri-apps/tauri-docs` 查询 updater 的超时、代理、下载与安装和 Windows 退出钩子。
- **当前 `backend/Cargo.lock` 锁定的是 `tauri-plugin-updater 2.12.0`**；此前审查文档写的 2.13.3 不应当作当前版本证据。本轮核对本机 2.12.0 源码，确认检查超时不自动继承到下载、下载完成后校验签名，以及 Windows 在解包后调用退出钩子、ShellExecute 成功后退出的顺序。
- 当前实现已设置 30 秒检查和 600 秒下载超时，下载事件约每 100 毫秒推送，取消通过 oneshot 丢弃下载 future，下载与安装共用互斥标记，安装前持有数据库锁排除运行任务，安装器启动失败保留安装包并恢复托盘。
- 发布校验的签名封装检查仍只是格式检查，密码学验签由 updater 执行。本轮没有重新运行真实签名下载，也没有把历史结果视为本轮结果。

## 本轮验证

- `node --test scripts/release/verify-release.test.mjs`：修改生产代码前 7 通过、2 失败，失败均为新增场景未被拒绝。
- 修复后 `node --test scripts/release/*.test.mjs frontend/src/lib/updateState.test.mjs`：23 项全部通过。
- 启动本轮独立的 `127.0.0.1:5197` Vite 服务，仅该进程启用 `VITE_JOLU_UPDATES_ENABLED=1`；运行 `node scripts/acceptance/update-flow.cjs` 通过。覆盖取消、离线重试、任务导航、任务结束、稍后安装、安装包复用、安装失败重试、完成后的监听释放，以及自动检查偏好与只检查不下载。
- 上述浏览器验证注入合成 API，没有访问真实账号、安装更新或连接真实桌面。脚本清理临时页面后，本轮启动的 Vite 服务已关闭。
- Rust 未改动；后端统一验证由主任务记录，本文不预先宣称其通过。

## 发布前仍待完成

- 正式签名私钥及配套发布资源、Release/latest.json 发布需由鼠鼠最终确定。正式发布时须同时传 `JOLU_UPDATES_ENABLED=1` 和 `VITE_JOLU_UPDATES_ENABLED=1`。
- 真正的两个不同版本 NSIS 包升级、重启和数据/设置保留，仍需要隔离环境验收；合成 API 验证不能证明这些行为。
- 既有发布校验要求 Windows、macOS 两架构和 Linux 两格式共五类安装包。是否首发只提供 Windows 属于发布范围决策，本轮没有擅自调整这个既有要求。
- 没有 Git 写操作；所有本轮修改保留在工作区。
