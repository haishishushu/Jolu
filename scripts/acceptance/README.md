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

隔离模式下，「立即刷新」只刷新界面与测试连接，不采集本机日志；会话列表仍可展示隔离数据库里的统计记录，但不读取源文件正文、项目目录或全文搜索内容。即使隔离库中已有真实 `scan_state.path`，也不会打开这些文件。需要测试消息解析时使用 Rust 合成文件测试，不在隔离桌面中接入真实会话。此模式只在 debug 构建生效。

## 直连 CDP 的脚本

`*.e2e.mjs` 用 Playwright 的 `connectOverCDP` 直接连接 9337，不经过 playwright-cli，公共部分在 `cdp.mjs`（操作前核验数据库位于 `<仓库>/output/acceptance/profile/` 下，且所在目录有 `.acceptance-profile` 标记，否则直接退出）。仓库不依赖 Playwright：先在 output 下准备一个目录，例如 `output/acceptance/node` 里执行 `npm install playwright-core`，再在该目录里用绝对路径运行脚本。可选环境变量：`JOLU_CDP`（默认 `http://127.0.0.1:9337`）、`JOLU_DEV_URL`（默认 `http://localhost:5173`）。

- `main-window-smoke.e2e.mjs`：经托盘同一路径（`menu_action open_main`）打开主窗口，依次点击侧栏七个页面。每页要求「正在读取」10 秒内消失、不出现「读取失败」「扫描失败」「刷新失败」（包括 toast）、控制台没有 error 且没有未捕获异常；截图保存到 output/acceptance/main-window/<页面>.png。任一页失败时退出码为 1，输出里列出每页的问题。
- `app-notices-desktop.e2e.mjs`：需要隔离桌面及 `gateway.py`，验证真实余额查询→金额卡片→账号定位、持续低余额不重复、仅主窗口读取通知、免打扰抑制及结束后不补弹。只创建并清理本机合成网关连接、只修改隔离阈值；截图在 `output/acceptance/app-notices/`。不运行真实自动化或发送模型请求。
- `reset-window-layout.e2e.mjs`：点击主面板「重置位置」，再经菜单与主面板命令重复重置，检查本体与分身解除停靠、隐藏主面板后仍可重置、默认尺寸恢复，以及窗口与 IPC 持续响应。仅在带标记的隔离实例运行；原生操作限时 8 秒，死锁时明确失败。
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

## 账号与更新组件回归

- `node scripts/acceptance/notification-threshold.cjs`：默认开发预览 5173（`JOLU_COMPONENT_URL` 可覆盖，仅限本机），挂载真实通知设置组件，以内存桥接验证整数显示、灰色固定币种、数字宽度伸缩、最大合法金额完整可见、小数与草稿保留、阈值保存和无效值。只使用合成设置，不修改真实通知偏好；浅深色截图位于 `output/acceptance/notification-threshold/`。
- `node scripts/acceptance/toast-cards.cjs`：使用独立无头 Edge 和真实通知组件，验证右上角三张卡片、排队后才计时、3/6 秒与持续错误、重复合并、关闭、鼠标和焦点独立暂停、原因展开、重试动作与再次失败保留错误、表单内键盘访问、Esc 和鼠标关闭不误关表单、浅深色、窄窗口与减少动效。默认开发预览 5314（`JOLU_COMPONENT_URL` 可覆盖，仅限本机）；不访问真实后端。截图位于 `output/acceptance/toast-cards/`。
- `node scripts/acceptance/account-refresh.cjs`：使用 5193 Vite 开发服务器（`JOLU_PREVIEW_URL` 可覆盖，仅限本机）、Edge 和上述 playwright-core。挂载真实账号面板、额度 hook 与 API 桥接，只替换原生调用为合成 Promise；验证刷新一个账号不改变另一张卡片，成功更新重置卡次数，失败保留旧值并标注。不会连接真实桌面、读取凭据或查询上游服务。
- `node scripts/acceptance/account-metadata.cjs`：需已启动 5193 前端服务及 `output/acceptance/node` 中的 playwright-core，使用 Edge 无头浏览器注入合成额度；检查四列、5 小时/周窗口、订阅快照、重置卡真实零和隐私显示。不会查询真实账号，也不执行账号切换。
- `node scripts/acceptance/account-quota-sources.cjs`：同样使用 5193 前端服务与合成桥接；覆盖真实重置卡对象才出现查询入口、服务未返回字段、单卡查询归属，以及套餐 Key 额度与余额并存、真实零、自定义供应商额度和余额、旧值标记与不支持状态。截图保存在 `output/playwright/account-sources-*.png`。
- `node scripts/acceptance/account-cards.cjs`：默认使用 5314 开发服务器（`JOLU_COMPONENT_URL` 可覆盖，仅限本机），挂载真实 MainPanelWindow、懒加载外层、供应商页与合成桥接。验证 1280×900 和 1920×1080 下四列、首屏两行八张、仅列表滚轮查看后续账号、筛选归零滚动位置，以及完整六模型详情、Auth 订阅到期、ApiKey 独立摘要和真实套餐到期、类型/供应商/搜索组合筛选及清除、隐私隐藏、真实零、按来源显示重置卡和单账号查询。浅深色截图位于 `output/acceptance/account-cards/`；不访问真实桌面或账号。
- `node scripts/acceptance/usage-overview.cjs`：同样默认使用 5314，挂载真实概览组件。验证混合已知/未知记录时显示已知 Token、缓存可计算覆盖次数、模型占比、不做部分总量环比、未知不变零，以及四条趋势线跨缺失点连续且悬浮说明保留。仅使用合成数据；截图位于 `output/acceptance/usage-overview/`。
- `node --test backend/src/workbuddy_auth.test.mjs`：通过合成官方 sym-v1 信封和原生存储替身，验证 WorkBuddy 凭证内存解码、篡改拒绝、不同客户端密钥拒绝及错误输出脱敏；不读取真实账号或联网。真实只读查询仍需单独运行 Rust 的忽略测试。
- `node scripts/acceptance/update-flow.cjs`：使用单独启用 `VITE_JOLU_UPDATES_ENABLED=1` 的 5197 前端服务（可用 `JOLU_PREVIEW_URL` 覆盖），合成更新 API 验证取消、重试、稍后安装和任务导航。不会安装更新；真实签名与安装链路须分别验证。

## 组件异步状态回归

`async-components.e2e.mjs` 在独立无头 Edge 中挂载真实 `SkillDetail`、`SkillEditor` 和 `TaskDetail`，由 `async-components.fixture.mjs` 将相关 API 替换为可控 Promise。fixture 仅由脚本提供给浏览器，不进入产品构建；未声明的原生调用一律拒绝。不会读取或写入真实 Skills、账号、额度，也不连接真实桌面。

在 `output/acceptance/node` 安装 `playwright-core`，单独启动开发服务器：`pnpm --dir frontend dev --host localhost --port 5311 --strictPort`，然后执行：

```powershell
node scripts/acceptance/async-components.e2e.mjs
```

用 `JOLU_COMPONENT_URL` 覆盖默认 `http://localhost:5311`，仅接受本机地址。此脚本需要 Vite 开发服务器加载真实 TSX 组件，与上面的生产拆包回归不同。

- 快速切文件：新成功后旧成功、旧失败均不能覆盖；当前失败仍显示；切换 Skill 本身时的旧失败也被忽略。
- 保存：双击仅发出一次保存，输入框、供应商/范围、模板、启用选项与取消入口禁用；失败保留草稿并允许重试，成功只回调一次。
- 任务额度：切账号立即移除旧值；旧成功/失败不能覆盖新状态；同账号新检查失败清除旧额度，再成功清除错误。
- 只输出通过摘要，不打印正文、草稿或输入值；任何断言失败返回非零退出码，finally 关闭自建浏览器。不代替真实桌面/文件系统/API 联调。

`node scripts/acceptance/automation-snapshot.e2e.mjs` 默认使用 5193（可用 `JOLU_COMPONENT_URL` 覆盖），挂载真实任务详情并注入合成运行快照。验证任务编辑后，详情、额度、目录、指令与会话跳转仍使用原运行配置，编辑入口使用当前任务，未知快照不猜账号或开放会话跳转。截图位于 `output/acceptance/automation-snapshot/`，不执行真实自动化。

## ZCode 首次接入模型验证

使用上面的 5311 Vite 开发服务器和 `output/acceptance/node` 内的 playwright-core，执行 `node scripts/acceptance/zcode-model-verify.e2e.mjs`。`JOLU_COMPONENT_URL` 可覆盖本机地址。脚本挂载真实 `ConnectDialog`，仅用合成 Key、模型和可控 Promise，所有未替换的原生调用一律拒绝；不查询或保存真实账号，不向供应商发送请求。

覆盖模型枚举不可用时改手填、明确用量确认后才验证、取消确认不请求、连点不重复、验证期间禁止保存、换 Key/区域/模型后旧结果失效、失败保留输入、重新获取模型重置状态，以及四组国内/国际 OpenAI/Anthropic 传参和保存。枚举选择和自定义供应商的既有保存规则也做回归。截图保存在 `output/acceptance/zcode-model/`；只输出通过摘要，失败亦关闭自建浏览器。它不能替代用户填入真实 ZCode Key 后的接口联调。

## 真实只读接口的执行条件

明确获用户授权后执行忽略的 `local_auth_read_only_smoke`（Codex）或 `local_claude_gateway_read_only_smoke`（Claude 网关）。不打印 Key、token、账号、历史记录或完整响应，不发送模型请求。其他账号类型缺失时记录缺口。

## 生产预览的页面加载回归

`lazy-pages-preview.e2e.mjs` 与 `lazy-jumps-preview.e2e.mjs` 使用独立的无头 Edge 浏览器，只访问本机前端预览的合成数据，不连接 CDP、不调用真实桌面接口，也不创建或执行自动化任务。它们不能替代真实账号或桌面功能验收。脚本按仓库位置解析依赖与产物目录，可以从任意工作目录运行。

准备：在 `output/acceptance/node` 中安装 `playwright-core`（不加入项目依赖），本机需有 Microsoft Edge。先执行 `pnpm --dir frontend build`，再在单独终端运行 `pnpm --dir frontend preview --host localhost --port 5307 --strictPort`。必须使用生产预览；开发服务器没有待验证的生产页面 chunk。

```powershell
node scripts/acceptance/lazy-pages-preview.e2e.mjs
node scripts/acceptance/lazy-jumps-preview.e2e.mjs
```

可用 `JOLU_PREVIEW_URL` 覆盖默认 `http://localhost:5307`，只接受本机地址；端口被其他程序占用时换端口，不结束其他进程。

- 页面加载脚本核对首开不请求其他六页、七页真实内容、慢请求局部骨架、等待期间切页，以及资源加载失败后的显式“重新加载应用”恢复。截图与结果保存到 `output/acceptance/lazy-pages/`。
- 跨页脚本核对会话 ID 带入用量搜索，以及 Codex、原会话、项目目录和任务名称带入新任务配置。仅输出断言结果，不输出整页正文或输入框值。
- 成功或失败均关闭脚本自己创建的浏览器；失败时退出码非零。前端预览服务器由启动它的终端停止。

## Windows Sandbox 安装检查

运行 `prepare-sandbox.ps1` 生成 output/acceptance/sandbox/install-check.wsb。仅在已安装 Windows Sandbox 的电脑打开；脚本拒绝在普通宿主用户下执行。检查安装、同版本覆盖升级、默认卸载保留数据、升级保留开机启动及卸载删除启动项；它不等于跨版本升级或每个 WebView2 版本已验收。报告写入 sandbox-result.json。

依据：[Tauri NSIS hooks](https://v2.tauri.app/distribute/windows-installer/)、[Windows Sandbox 配置](https://learn.microsoft.com/en-us/windows/security/application-security/application-isolation/windows-sandbox/windows-sandbox-configure-using-wsb-file)。包和脚本映射为只读，仅结果目录可写。

## 自定义余额桌面链路

`custom-balance-desktop.e2e.mjs` 在 start.ps1 创建的带标记隔离目录中运行。验证真实 Tauri 命令的余额零值、无权限、未授权、格式错误与不支持；列表不返回凭证；隔离模式拒绝读取 Trae / ZCode / WorkBuddy / Qoder 真实登录。只使用本机合成网关，结束时移除脚本创建的连接。

## 本地登录与浏览器授权入口

`provider-authorization.cjs` 使用真实 React 连接弹窗与合成 Tauri 桥接，在独立无头 Edge 中验证全部 12 个订阅入口、六版本的 family/edition 传递、查询通过且填写名称后保存、待授权禁止保存、关闭后迟到授权取消；另验证真实供应商页的 13 张卡片及浅深色截图。不读取真实凭证、不打开真实授权页面、不调用模型。默认连接本机开发预览 `http://localhost:5314`，可用 `JOLU_COMPONENT_URL` 指定其他本机端口；截图位于 `output/acceptance/provider-auth/`。这仅证明组件交互，不能替代系统默认浏览器和真实供应商授权验证。

## 一键签到

- `node scripts/acceptance/account-connect-dialogs.cjs`：默认本机 Vite 5173（`JOLU_COMPONENT_URL` 可覆盖），独立无头 Edge、真实账号管理与签到组件、合成 IPC。验证顶部按钮顺序、卡片无加号、添加账号先选供应商、自动查询不弹窗、手动刷新成功/失败在右侧通知且无居中弹窗，以及去连接原地弹窗、版本预选、保存刷新、Trae 两账号、保存防连点与冻结。含品牌图标的浅深色及窄屏截图，位于 `output/acceptance/account-connect-dialogs/`。不读取真实凭证、不打开真实授权页、不发送领取请求。
- `node scripts/acceptance/daily-checkin.cjs`：默认本机 Vite 5318（`JOLU_COMPONENT_URL` 可覆盖），独立 Edge、真实主窗口与签到页、合成 IPC。验证六版能力、身份去重与参数、零奖励、失败继续、防连点、切页续跑、隐私、空/错误恢复、窄屏和浅深色；也复现主窗口就绪前隐藏状态的启动回归。不读取真实账号、不发领取请求。
- `daily-checkin-desktop.e2e.mjs`：按隔离流程构建并启动应用；在 `output/acceptance/node` 运行绝对脚本路径，前端非 5173 时设置 `JOLU_DEV_URL`。只在带标记数据库插入六个合成本机连接；验证主窗口命令限制、脱敏、隔离拒绝读取真实登录/领取、默认页与实际 WebView。每次应使用新隔离目录。
- 真实 `native_claim_once` 为默认忽略测试，且必须额外提供仅该测试进程生效的 `JOLU_RUN_REAL_CHECKIN=1`。只有获明确授权后运行，实际状态 Ready 才领取一次，随后只读复核；不得当作普通回归反复执行。协议、已执行结果及尚缺样本见 `docs/verification/2026-10-27-daily-checkin.md`。

账号统计 A：`daily-checkin.cjs` 同时核对周/月/累计、零奖励与未知金额、无历史记录、逐包原始额度优先于旧汇总、Qoder 多范围不相加、仅按领取事件的连接 ID 刷新额度。`account-connect-dialogs.cjs` 核对手动刷新强制更新额度，以及状态查询中途切页后不新发强制额度请求。使用 `JOLU_COMPONENT_URL=http://localhost:5173` 可复用本机独立预览；A 卡片与整页的浅深色合成截图分别为 `output/acceptance/daily-checkin/statistics-a-card-*.png` 和 `statistics-a-*.png`。这些截图为合成 IPC 数据，不能替代真实账号额度或领奖验收。

2026-10-27 补充：上述脚本已覆盖订阅只读账号/免填名称、宽屏四列、官方日期与活动累计、明确计算的可用量及不限量、官方日期缺失时本机项的来源，以及每账号删除的取消、失败、执行冻结、防连点和历史保留。非手动签到版本的已连接账号也有删除入口。`provider-authorization.cjs` 的 12 个订阅入口均无需手填名称。验收边界见 `docs/verification/2026-10-27-checkin-account-identity-data.md`。

统一摘要调整：`daily-checkin.cjs` 另核对主卡固定字段、底部「详情｜签到状态｜删除」同行、右侧详情不增加请求、刷新保持、窄屏/Esc/隐私及单删确认。截图为 `account-details-*.png`、`account-actions-dark.png` 和 `statistics-a-*.png`。原摘要阶段记录见 `docs/verification/2026-10-27-checkin-card-summary-details.md`，最新批量流程见下文。

两页批量删除：`node scripts/acceptance/bulk-account-management.cjs` 默认本机 Vite 5173（`JOLU_COMPONENT_URL` 可覆盖），独立无头 Edge 挂载真实账号管理组件、共享选择与确认，注入合成 IPC。覆盖筛选裁剪与全选、空选择、取消、固定目标 ID、暂停项、当前账号不切换、失败继续、只重试失败、关闭后保留失败勾选、隐私、连点冻结、未选账号及历史保留、通知定位及删空恢复；截图位于 `output/acceptance/bulk-account-management/`。`daily-checkin.cjs` 同时覆盖签到页全部保存连接的选择（含暂停、重复身份、未知版本）、精确删除、部分失败重试、防连点/关闭、历史保留及空卡片连接冻结，截图增加 `bulk-*.png`。所有删除只作用于脚本内存数据，绝不调用真实桌面删除或修改原工具登录。
