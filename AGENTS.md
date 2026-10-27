# Jolu 开发规范

Windows 桌面 AI 工具管理器。前端 `frontend/`：React 19 + TypeScript + Vite + Tailwind CSS 4；后端 `backend/`：Tauri 2 + Rust + rusqlite（SQLite）。

## 1. UI 设计依据（最高优先级）

- **唯一设计依据是 `docs/ui/Jolu.pen`。**
- **主窗口全部界面和对应的前后端功能都严格按 `Jolu.pen` 重做，旧界面不是参考。** 唯一例外是浮动灵动岛窗口本身的展示（`components/island/`、`views/IslandWindow.tsx`），保持现状。旧组件只能复用其中的数据逻辑；页面按新稿重做完成后，删除被取代的旧组件。
- `.pen` 是加密文件，只能通过 Pencil MCP 读取（`get_app_state`，`execute` 里的 `Get` / `GetVariables` / `TakeScreenshot`）；禁止用 Read、Grep 或文本编辑器打开。未经鼠鼠明确要求，不修改 `Jolu.pen`。
- 按模块找画板。画板按序号成对出现，奇数是浅色、偶数是深色；先看画板本身，再看旁边的「画板序号与标题」和行标题里的说明：

  | 顶层画板 | 画板 | 内容 | 现有实现（截至 2026-10-22） |
  |---|---|---|---|
  | 00 · Jolu 模块目录 | — | 索引与阅读方式 | — |
  | 模块 A · 灵动岛 | 01–04 | 主窗口「灵动岛」页、侧栏收起形态 | 已按新稿实现：`views/MainPanelWindow.tsx`、`views/IslandView.tsx`、`components/panel/SideNav.tsx` / `PageHeader.tsx` |
  | 模块 B · 供应商 | 05–29 | 直接连接、账号管理、订阅连接、API Key 与模型获取、OpenCode、账号与连接的异常状态；29 是平台能力规则 | 已按新稿实现：`views/ProvidersView.tsx`、`components/providers/`、`lib/providerCatalog.ts`；自定义供应商、ZCode API Key、OpenCode 模型服务已接入；Trae 双版账号、ZCode BigModel 个人订阅、自定义余额已接入，真实与合成验证范围见未验证清单；Qoder / WorkBuddy 不代管 API Key |
  | 模块 I · 一键签到 | 133–160 | 供应商下方同级入口、六版本品牌卡片、批量与单账号签到、记录、供应商选择、连接表单、右侧通知、四列统一摘要、右侧详情、统一操作区与两页批量删除 | `views/CheckinView.tsx`、`components/checkin/`、`lib/dailyCheckin.ts` / `checkinApi.ts` / `checkinStats.ts`，后端 `daily_checkin.rs` / `checkin_*.rs`；WorkBuddy 国内、Qoder 双版、Trae 国内已接入官方每日奖励；WorkBuddy AI 为自动活跃奖励、Trae 国际未开放；顶部刷新显示右上角通知，添加账号先选供应商，选择项与卡片有品牌图标且无加号；共用 `AccountConnectDialog.tsx` 原地保存，支持浏览器多账号；订阅免填名称并显示官方账号，宽屏每行四卡；149–150 为最新统一摘要，151–152 右侧完整详情，底部详情/状态/删除并排；153–160 为签到及账号管理批量选择、共用确认和部分失败重试，`AccountBatchControls.tsx` / `useAccountSelection.ts` / `accountActions.ts` 按选中连接 ID 删除、筛选裁剪选择并保留历史；A 统计按来源显示官方与 Jolu 记录，WorkBuddy 接返回日期及本期累计，Trae 接汇总/逐包与标明计算的可用量，不限量及未知分别保留；Trae 国内真实网页授权至保存及只读签到状态已通过，第二个真实账号尚缺登录样本，见同日验收记录 |
  | 模块 C · 用量统计 | 30–39 | 概览、用量分析、调用明细、日期范围弹窗、交互状态 | 已按新稿实现：`views/UsageView.tsx`、`components/usage/`、`lib/usageStats.ts`，后端 `usage_stats.rs`；会话项目名、额度提醒与按账号同步失败状态待确认 |
  | 模块 D · 会话线程 | 40–57 | 会话列表与消息、搜索、在原工具继续、压缩会话与 Markdown 交接、从会话创建自动化 | 已按新稿实现：`views/SessionsView.tsx`、`components/sessions/`、`lib/sessionThreads.ts`，后端 `session_threads.rs`；压缩会话（48/50/52/54/57A）已接入配置、对话式修订、MD 预览与导出；创建自动化携带会话与项目；短会话真实探索评测完成，长会话正式评测缺少样本 |
  | 模块 E · 自动化 | 58–73 | 任务列表、新建任务、等待额度与续跑、运行记录与详情、执行会话选择 | 已按新稿实现：`views/AutomationView.tsx`、`components/automation/`、`lib/automation.ts` / `automationApi.ts`，后端 `automation.rs`（Claude Code / Codex 非交互执行与本机调度）；Skills 选择（D-08，`SkillPicker.tsx`，下拉面板待确认）只列目标工具已启用的全局 + 工作目录项目 Skills，执行前与续跑前复查，执行账号须为原工具当前登录的账号 |
  | 模块 F · Skills 管理 | 74–85 | Skills 列表、供应商选择、详情与文件、新建 / 编辑、导入与冲突检查 | 已按新稿实现：`views/SkillsView.tsx`、`components/skills/`、`lib/skillsManager.ts` / `skillsApi.ts`，后端 `skills.rs`；已接入 Claude Code、Codex、Gemini、OpenCode、Qoder、Trae、ZCode；WorkBuddy 仅只读扫描 `.codebuddy/skills`；自定义目录只读；Grok 未接入；HTTPS Git 仓库导入、草稿与磁盘差异已有实现；项目范围由会话记录自动带出 + 手动添加，Claude Code 停用 = 移到应用数据目录的停用区（不写 `skillOverrides`） |
  | 模块 G · 设置 | 86–103 | 常规、外观、网络与代理、数据与存储、通知、关于与更新、清理确认、状态与反馈、手动联网 | 已按新稿实现：`views/SettingsPage.tsx`、`components/settings/`（settingsUi、各分区与 DataDialogs）、`lib/settingsPage.ts`、`lib/settingsApi.ts`，后端 `storage_stats.rs`；默认页面、缩放、密度、减少动效、联网方式、诊断、任务提醒与导入预览已有实现；检查更新默认启用（启动及每 30 分钟检查、后台下载、页头绿色版本按钮、被动安装后自动重启，画板 100/101 的 06），首次安装与 0.1.0→0.1.1 更新已用改名测试包真机验证，见 `docs/verification/2026-10-27-update-install-e2e.md`；全局快捷键与套餐查询凭证已按鼠鼠要求删除，设计稿已同步 |
  | 模块 H · 全局规范与品牌 | 104–110 | 按钮与图标交互六态、Logo 与人物、桌面 / 任务栏 / 托盘图标、启动页 | `components/brand/`、`components/ui/`、`components/tray/`；启动页（109 / 110）已按新稿实现：`frontend/startup.html`、`src/startup.ts` / `startup.css`，窗口在 `lib.rs` 的 `show_startup` |
  | 模块 A2 · 浮动灵动岛 | 111–132 | 浮动灵动岛：Auth / API 收缩与展开、仅 Token、Token 动效、配置细节、平台选择、拖动贴边、右键菜单与分身 | `components/island/`、`views/IslandWindow.tsx`、`views/ContextMenuWindow.tsx` |

  - 浮动灵动岛窗口以 A2 为准。
  - 「现有实现」写的是代码当时的状态，不代表已经和画板一致。新建页面或模块时更新这一列。
  - 编号以画板名称为准。查找画板时用 Pencil 按名称前缀搜索（如 `"40 · "`），不要写死节点 id。
- 颜色使用 `Jolu.pen` 的变量：主窗口 `jolu-*`，灵动岛 `pn-island-*`，浅深色由 `mode` 主题区分，不要凭感觉推导深色。
- 冲突时以 `Jolu.pen` 为准，高于任何文档、旧代码注释和截图。
- `Jolu.pen` 里没有的界面或状态，标注「待确认」并询问鼠鼠；不得自行补全。
- `docs/ui/previews/` 是导出截图（已被 Git 忽略），不是设计依据。
- `Jolu.pen` 用相对路径引用 `docs/ui/brands/`、`docs/logo/`、`frontend/src/assets/platforms/zcode.png` 里的图片。移动或重命名这些文件，必须同时用 Pencil 更新设计稿里的路径。

## 2. 目录与命名

```
backend/                    Tauri + Rust；src/ 按功能平铺模块，capabilities/、icons/、windows/ 为打包资源
frontend/src/
  components/<区域>/        automation、brand、island、panel、providers、sessions、settings、skills、tray、ui、usage
  hooks/                    React hooks
  lib/                      纯逻辑、API 桥接、平台目录；测试与源文件同目录
  views/                    各窗口入口
  mock/                     仅供浏览器预览的示例数据
docs/
  plans/                    实施计划
  specs/                    设计说明
  todo/                     待办
  verification/             验收与执行记录（按需新建）
  ui/                       Jolu.pen 设计稿及其引用的素材
  logo/                     品牌人物与应用图标母版（scripts/icons 的输入）
  assets/                   README 截图与社交预览图
scripts/
  acceptance/  dev/  icons/  installer-art/  release/
```

- 目录名和文档文件名：小写 kebab-case，不使用中文或空格。`README`、`AGENTS`、`CLAUDE` 等约定俗成的文件除外。
- 按日期记录的文档：`YYYY-MM-DD-主题.md`，放进上面对应的目录；不要再新建 `docs/superpowers/`，Superpowers 的计划写到 `docs/plans/`，设计说明写到 `docs/specs/`。
- 前端：组件 `PascalCase.tsx`（一个文件导出多个基础组件时用 camelCase，如 `primitives.tsx`）；hook `useXxx.ts` 放在 `hooks/`；纯逻辑 `camelCase.ts`；测试 `<源文件名>.test.mjs`，与源文件同名同目录。
- 后端 Rust：`snake_case.rs`；脚本：`kebab-case`。
- 发布脚本在 `scripts/release/`，发行工作流是 `.github/workflows/release.yml`：**每次推送 main 都会发一个新的公开版本**，不需要发版的提交在信息中加 `[skip ci]`。规则见 `docs/release-versioning.md`。
- 检查更新默认启用；构建时同时设置 `JOLU_UPDATES_ENABLED=0` / `VITE_JOLU_UPDATES_ENABLED=0` 可停用。Release 必须附正式私钥签名的安装包与 `latest.json`，否则检查会失败。测试公钥和本地端点仅放 output 的覆盖配置，不修改生产配置。
- 安装器使用自定义模板 `backend/windows/installer.nsi`（基于 tauri-cli 2.11.4，改动处标 `Jolu:`）：固定安装到 `%LOCALAPPDATA%\Jolu`，不显示选择目录页。升级 `@tauri-apps/cli` 时须与上游模板对照。卸载钩子会删除名为 `Jolu` 的开机自启项，本机测试安装包时先备份、测后恢复。

## 3. 常用命令（在仓库根目录执行）

```bash
pnpm install && pnpm --dir frontend install
pnpm dev                                    # 只启动前端预览，http://localhost:5173（被占用时顺延）
pnpm tauri dev                              # 启动桌面应用（自动拉起前端；5173 被占用时由 scripts/dev/tauri.mjs 顺延端口并同步 devUrl）
pnpm --dir frontend typecheck
node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs # 前端纯逻辑与共享状态测试
node --test scripts/release/*.test.mjs      # 发布脚本测试
cargo test --manifest-path backend/Cargo.toml
```

桌面验收流程见 `scripts/acceptance/README.md`，只能在隔离目录中运行。

## 4. 开发约束

- 所有代码、配置和文档使用 UTF-8。
- API Key 和认证信息只在后端处理，前端列表只返回掩码；不得写入普通日志、统计导出和诊断报告。
- 未知值、真实的零、能力不支持、查询失败、数据过期要分别表达；正式桌面运行出错时，不得悄悄回退到 `mock/data.ts` 的示例数据。
- 不能根据 UI、图标或套餐名称推断供应商支持登录、余额、Skills 或自动化。
- 不得删除旧账号、历史统计和原始会话文件；数据结构变更需要提供可恢复的迁移。
- 产品名：给用户看的地方写 `Jolu`；包名、标识符、文件前缀和 URL 写 `jolu`；环境变量用 `JOLU_` 前缀。本项目是新仓库，不保留任何旧项目的兼容或迁移逻辑。官网地址暂时置空。
- 涉及第三方库、SDK、CLI 或认证协议时，先用 Context7（`npx ctx7@latest`）核对文档。

## 5. 验证与 Git

- 验证强度跟改动风险匹配：局部修改只跑相关测试；涉及公共接口、核心逻辑或多个模块时，再跑前后端测试和类型检查。
- 改了界面，要用 Pencil MCP 截取对应 `Jolu.pen` 画板，和实际运行效果对照。
- 未经鼠鼠明确授权，不执行 `git add`、提交、推送、切换分支或重置。

### 提交与推送节奏

- 以完整的功能、问题修复或验收阶段为提交单位，不以一次对话、一处文案或几行代码为单位；相关的小改动先积累，阶段完成并通过必要验证后统一提交、推送。
- 阶段边界由助手根据改动关联性、完整性和验证结果判断；鼠鼠连续提出的同一轮界面微调通常属于同一阶段，不要每完成一项就提交。
- 不为凑提交混入无关改动，也不把未完成或未验证的工作打包提交。已有其他修改只纳入明确属于本阶段的部分。
- 鼠鼠已授权按阶段提交、推送时，在授权范围内自行把握时机，不反复请求确认；没有授权时仍遵守上面的 Git 授权规则。鼠鼠明确要求立即提交时按该要求执行。
- 提交时使用 `git-commit-push-zh` 技能，核对本阶段范围、身份与远端状态，以中文说明改动目的和验证结果。
- 小修改完成后简短汇报即可；尚未达到阶段边界时保留工作区修改，不为汇报而专门提交。本规范的零碎更新同样遵守此规则。
