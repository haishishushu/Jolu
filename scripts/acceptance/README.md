# 单智能体验收工具

仅用于本仓库开发验收。UI 自动化使用 Playwright CLI，数据库/HTTP 使用真实应用代码；网关服务返回明确的合成数据，不能替代真实账号联调。

## 隔离桌面流程

1. `cargo build --manifest-path backend/Cargo.toml`，另开终端运行 `pnpm --dir frontend dev`。
2. 确认 9337 与 18765 没有旧验收进程占用，运行 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/acceptance/start.ps1`。每次生成新的带标记的隔离目录，禁用本机日志采集/凭证发现/开机启动修改；WebView 缓存也独立。
3. `npx --yes --package @playwright/cli playwright-cli -s=acceptance open http://localhost:5173`
4. 依次通过 `run-code --filename` 执行 `http.cjs`、`windows.cjs`、`geometry.cjs`，以及灵动岛、右键菜单与设置失败相关脚本（`compact-menu.cjs`、`hover-menu.cjs`、`session-list-scroll.cjs`、`motion-window.cjs`、`motion-performance.cjs`、`hidden-panel-polling.cjs`、`connection-recovery.cjs`、`settings-failure.cjs`）。执行前核验数据路径属于 output/acceptance/profile；只在新测试目录运行增删测试。
   主窗口与灵动岛费用用直连 CDP 的脚本验收，见下方「直连 CDP 的脚本」。
5. `performance.ps1 -Minutes 10` 每 15 秒保存进程树工作集、私有内存、CPU 累计时间与响应状态，报告位于 output/acceptance。每次测试须确认 processes.json 的 PID 仍属于本次进程。
6. 完成后关闭 acceptance 浏览器，并仅停止 processes.json 中本次创建的 app/gateway PID。不要结束其他同名应用。

## 直连 CDP 的脚本

`*.e2e.mjs` 用 Playwright 的 `connectOverCDP` 直接连接 9337，不经过 playwright-cli，公共部分在 `cdp.mjs`（操作前核验数据库位于 `<仓库>/output/acceptance/profile/` 下，且所在目录有 `.acceptance-profile` 标记，否则直接退出）。仓库不依赖 Playwright：先在 output 下准备一个目录，例如 `output/acceptance/node` 里执行 `npm install playwright-core`，再在该目录里用绝对路径运行脚本。可选环境变量：`JOLU_CDP`（默认 `http://127.0.0.1:9337`）、`JOLU_DEV_URL`（默认 `http://localhost:5173`）。

- `main-window-smoke.e2e.mjs`：经托盘同一路径（`menu_action open_main`）打开主窗口，依次点击侧栏七个页面。每页要求「正在读取」10 秒内消失、不出现「读取失败」「扫描失败」「刷新失败」（包括 toast）、控制台没有 error 且没有未捕获异常；截图保存到 output/acceptance/main-window/<页面>.png。任一页失败时退出码为 1，输出里列出每页的问题。
- `four-connection-cost.e2e.mjs`：新建指向 `gateway.py` 的 Claude API 连接并设为灵动岛连接，把灵动岛切到自由位置，向隔离数据库 `requests` 表写入两条合成记录（每条 claude-sonnet-4 输入 100 万 Token，按价目表估算 $3.00）。每次写入后「恢复连接」，触发一次全局 `refresh-requested`（导入完成时发出的也是这个事件）。核对费用在同一个 DOM 节点上依次增加 $3.00；断开连接后显示「连接已断开，当前仅显示本机统计」，且仍保留所选连接。写库不经过导入代码：`import_data_file` 会弹系统文件框，无法自动化。所以「本机今日 Token」不会随之变化，脚本也不检查它。

5173 被其他程序占用时，不要结束对方进程：用 `TAURI_CONFIG` 覆盖 devUrl，编译到单独的 target 目录。如果另一个 Jolu 正在运行，单实例锁会让隔离实例启动后立即退出，这时要同时覆盖 identifier。然后用 `-Executable` 启动：

```powershell
$env:CARGO_TARGET_DIR = "output/acceptance/target-5193"
$env:TAURI_CONFIG = '{"identifier":"dev.ningz.jolu.acceptance","build":{"devUrl":"http://localhost:5193"}}'
cargo build --manifest-path backend/Cargo.toml
Remove-Item Env:CARGO_TARGET_DIR, Env:TAURI_CONFIG         # 编完立即清除，否则之后在同一终端的 cargo / tauri 命令会沿用它们
pnpm --dir frontend exec vite --port 5193 --strictPort      # 另开终端
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/acceptance/start.ps1 -Executable output/acceptance/target-5193/debug/jolu.exe
$env:JOLU_DEV_URL = "http://localhost:5193"                  # 运行 *.e2e.mjs 前设置
Remove-Item Env:JOLU_DEV_URL                                 # 验收结束后清除
```

`start.ps1` 必须保持纯 ASCII：Windows PowerShell 5.1 会把不带 BOM 的 UTF-8 文件按 ANSI 读取，中文注释可能吞掉下一行代码。

## 灵动岛分身（画布 22）

`island-clones.e2e.mjs` 直接用 Playwright 的 `connectOverCDP` 连接 9337，不经过 playwright-cli。在隔离应用启动后运行 `main` 阶段；结束应用、用同一隔离目录重启后再运行 `after-restart` 阶段。脚本会核对菜单顺序、右上角总数、销毁禁用、分身窗口与 `settings.json` 的 `island_clones` 一致，以及销毁本体时的接任。开发端口被系统排除时，用 `--config` 覆盖 `build.devUrl` 后再 `tauri dev`。

环境变量 `JOLU_TEST_DIR` 只在 debug 构建生效；路径必须为绝对路径且已有 `.acceptance-profile` 标记。正式安装包忽略该开关。CDP 端口仅由启动脚本设置，不写入产品代码。

## 真实只读接口

明确获用户授权后执行忽略的 `local_auth_read_only_smoke`（Codex）或 `local_claude_gateway_read_only_smoke`（Claude 网关）。不打印 Key、token、账号、历史记录或完整响应，不发送模型请求。其他账号类型缺失时记录缺口。

## Windows Sandbox 安装检查

运行 `prepare-sandbox.ps1` 生成 output/acceptance/sandbox/install-check.wsb。仅在已安装 Windows Sandbox 的电脑打开；脚本拒绝在普通宿主用户下执行。检查安装、同版本覆盖升级、默认卸载保留数据、升级保留开机启动及卸载删除启动项；它不等于跨版本升级或每个 WebView2 版本已验收。报告写入 sandbox-result.json。

依据：[Tauri NSIS hooks](https://v2.tauri.app/distribute/windows-installer/)、[Windows Sandbox 配置](https://learn.microsoft.com/en-us/windows/security/application-security/application-isolation/windows-sandbox/windows-sandbox-configure-using-wsb-file)。包和脚本映射为只读，仅结果目录可写。
