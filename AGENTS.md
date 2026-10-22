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
  | 模块 B · 供应商 | 05–29 | 直接连接、账号管理、订阅连接、API Key 与模型获取、OpenCode、账号与连接的异常状态；29 是平台能力规则 | 已按新稿实现：`views/ProvidersView.tsx`、`components/providers/`、`lib/providerCatalog.ts`；OpenCode、自定义供应商、ZCode / Qoder / WorkBuddy 的 API Key 标「待接入」 |
  | 模块 C · 用量统计 | 30–39 | 概览、用量分析、调用明细、日期范围弹窗、交互状态 | 已按新稿实现：`views/UsageView.tsx`、`components/usage/`、`lib/usageStats.ts`，后端 `usage_stats.rs`；会话项目名、额度提醒与按账号同步失败状态待确认 |
  | 模块 D · 会话线程 | 40–57 | 会话列表与消息、搜索、在原工具继续、压缩会话与 Markdown 交接、从会话创建自动化 | 已按新稿实现：`views/SessionsView.tsx`、`components/sessions/`、`lib/sessionThreads.ts`，后端 `session_threads.rs`；压缩会话（画板 48–55）标「待接入」，创建自动化目前只跳转到自动化页 |
  | 模块 E · 自动化 | 58–73 | 任务列表、新建任务、等待额度与续跑、运行记录与详情、执行会话选择 | 已按新稿实现：`views/AutomationView.tsx`、`components/automation/`、`lib/automation.ts` / `automationApi.ts`，后端 `automation.rs`（Claude Code / Codex 非交互执行与本机调度）；Skills 选择标「待接入」，执行账号须为原工具当前登录的账号 |
  | 模块 F · Skills 管理 | 74–85 | Skills 列表、供应商选择、详情与文件、新建 / 编辑、导入与冲突检查 | 已按新稿实现：`views/SkillsView.tsx`、`components/skills/`、`lib/skillsManager.ts` / `skillsApi.ts`，后端 `skills.rs`；只接入 Claude Code 与 Codex，其他供应商、自定义目录、Git 仓库导入、查看差异标「待接入」；项目范围由会话记录自动带出 + 手动添加，Claude Code 停用 = 移到应用数据目录的停用区（不写 `skillOverrides`） |
  | 模块 G · 设置 | 86–103 | 常规、外观、网络与代理、数据与存储、通知、关于与更新、清理确认、状态与反馈、手动联网 | 已按新稿实现：`views/SettingsPage.tsx`、`components/settings/`（settingsUi、各分区与 DataDialogs）、`lib/settingsPage.ts`、`lib/settingsApi.ts`，后端 `storage_stats.rs`；默认打开页面、界面缩放、列表密度、减少动效、应用联网方式、连接诊断、任务提醒、导入预览标「待接入」，检查更新随 `UPDATES_ENABLED` 停用；全局快捷键与套餐查询凭证已按鼠鼠要求删除，设计稿已同步 |
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
  acceptance/  icons/  installer-art/  release/
```

- 目录名和文档文件名：小写 kebab-case，不使用中文或空格。`README`、`AGENTS`、`CLAUDE` 等约定俗成的文件除外。
- 按日期记录的文档：`YYYY-MM-DD-主题.md`，放进上面对应的目录；不要再新建 `docs/superpowers/`，Superpowers 的计划写到 `docs/plans/`，设计说明写到 `docs/specs/`。
- 前端：组件 `PascalCase.tsx`（一个文件导出多个基础组件时用 camelCase，如 `primitives.tsx`）；hook `useXxx.ts` 放在 `hooks/`；纯逻辑 `camelCase.ts`；测试 `<源文件名>.test.mjs`，与源文件同名同目录。
- 后端 Rust：`snake_case.rs`；脚本：`kebab-case`。
- 发布脚本在 `scripts/release/`。恢复 GitHub Actions 工作流时，脚本路径要改成这个目录。
- 检查更新目前停用：`backend/src/updater.rs` 和 `frontend/src/lib/updateState.ts` 各有一个 `UPDATES_ENABLED = false`。首个带 `latest.json` 的 Release 发布后，两处同时改为 `true`。

## 3. 常用命令（在仓库根目录执行）

```bash
pnpm install && pnpm --dir frontend install
pnpm dev                                    # 只启动前端预览，http://localhost:5173
pnpm tauri dev                              # 启动桌面应用（自动拉起前端）
pnpm --dir frontend typecheck
node --test frontend/src/lib/*.test.mjs     # 前端纯逻辑单元测试
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
