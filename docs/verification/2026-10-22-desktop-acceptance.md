# 桌面验收：Skills 读写、深色主题、灵动岛分身、安装包（任务 1.3）

- 日期：2026-10-23 09:26–10:10（本机时间）
- 环境：Windows 11，`backend/target/debug/jolu.exe`（debug 构建）+ `pnpm --dir frontend dev`，由 `scripts/acceptance/start.ps1` 启动隔离环境：`JOLU_TEST_DIR=output/acceptance/profile/<随机目录>`，Skills 目录在 `<隔离目录>/skills-home`，合成网关 `127.0.0.1:18765`，WebView2 远程调试口 9337
- 驱动：`playwright-core` 的 `connectOverCDP`，在主窗口和灵动岛页面上点击、填表；系统原生的「选择文件夹 / 打开文件」对话框用 Win32 消息操作（找到应用进程的 `#32770` 对话框，向编辑框 1152 / 1148 发 `WM_SETTEXT`，再向「选择文件夹 / 打开」按钮发 `BM_CLICK`），不改产品代码
- 未接触鼠鼠的真实数据目录、`~/.claude/skills`、`~/.codex`；没有发模型请求，没有消耗额度
- 临时脚本与截图在 `output/acceptance/task13/`、`output/acceptance/dark/`（Git 已忽略）

## 结果总览

| 步骤 | 项 | 结果 |
|---|---|---|
| 1 Skills | 新建（Claude Code · 全局） | 通过 |
| 1 Skills | 编辑（改描述，保存前备份） | 通过 |
| 1 Skills | Claude 停用 → 启用 | 通过 |
| 1 Skills | Codex 新建 → 停用 → 启用，核对 `config.toml` | 通过 |
| 1 Skills | 从本地目录导入 3 个 | 通过 |
| 1 Skills | 再次导入：同名冲突识别 | 通过 |
| 1 Skills | 冲突选「跳过」 | 通过 |
| 1 Skills | 冲突选「覆盖」 | 通过（有一条待确认的行为，见观察 1） |
| 1 Skills | 跨盘停用 / 启用 | 改为后端测试，通过（真机同盘，无法复现） |
| 2 深色主题 | 七个页面与深色画板对照 | 发现 1 处明确偏差，已修复；其余差异记录在下表 |
| 3 灵动岛分身 | 两条连接各开一个岛，额度各自显示 | 通过（隔离开发版 + 合成网关） |
| 4 安装包 | Windows Sandbox 安装检查 | 未测（本机无 Windows Sandbox），已写入 `docs/todo/2026-10-23-installer-sandbox-check.md` |

## 步骤 1：Skills 读写

脚本：`output/acceptance/task13/skills-flow.mjs`，最终一次完整运行在全新隔离目录 `profile/31bc7b73…` 中进行，15 项全部通过，日志 `output/acceptance/task13/skills-flow.log`。

| 操作 | 界面操作 | 磁盘核对 |
|---|---|---|
| 新建 | 「新建 Skill」→ 名称 `accept-demo`、描述、勾选「保存后启用」→「保存 Skill」 | `skills-home/.claude/skills/accept-demo/SKILL.md` 存在，含 `name: accept-demo` 与描述 |
| 编辑 | 行尾「更多操作」→「编辑 Skill」→ 改描述 → 保存 | SKILL.md 只含新描述；`skill-backups/files/` 下有原文件备份 |
| Claude 停用 | 行内开关 | 原目录消失；`skills-disabled/<时间戳>-claude/accept-demo/SKILL.md` 内容为编辑后版本；`skills.json` 的 `disabled` 有 1 条；没有生成 `.claude/settings.json`（不写 `skillOverrides`） |
| Claude 启用 | 行内开关 | 移回 `skills-home/.claude/skills/accept-demo/`，停用区目录消失，`disabled` 清空 |
| Codex 新建 | 切换供应商为 Codex → 新建 `accept-codex` | `skills-home/.agents/skills/accept-codex/SKILL.md` |
| Codex 停用 | 行内开关 | 文件不动；`.codex/config.toml` 新增 `[[skills.config]]`，`path` 指向这份 SKILL.md，`enabled = false` |
| Codex 启用 | 行内开关 | `config.toml` 中该条目移除（文件变为空）；`skill-backups/config/` 有修改前备份 |
| 首次导入 | 「导入 Skill」→「选择目录」（原生对话框）→ 只保留 brainstorming、writing-plans、systematic-debugging，勾选「导入后启用」 | 扫描「15 可导入 · 0 冲突 · 0 异常」；3 个目录落到 `.claude/skills/`，SKILL.md 与来源逐字一致 |
| 再次导入 | 先在隔离副本 brainstorming、writing-plans 的 SKILL.md 末尾加标记；再次导入，writing-plans 改为「覆盖」，另两个保持默认「跳过」 | 扫描「12 可导入 · 3 冲突 · 0 异常」，3 项都标「目标已有同名 Skill」；按钮「导入 1 项」 |
| 跳过 | — | brainstorming 保留标记，systematic-debugging 未变 |
| 覆盖 | — | writing-plans 内容恢复为来源版本；`skill-backups/overwritten/<时间戳>-claude-writing-plans/` 是带标记的旧版本 |
| 导入到 Codex（只扫描） | 目标供应商改为 Codex | 「15 可导入 · 0 冲突 · 0 异常」，superpowers 的 SKILL.md 都带 `name`，没有被判为无效 |
| 来源目录 | — | 导入前后对来源目录全部文件的「相对路径 + 修改时间 + 内容」求哈希，一致，未被修改 |

导入来源由控制方指定：本机 superpowers 插件自带的 Skills 目录 `C:\Users\20719\.claude\plugins\cache\claude-plugins-official\superpowers\7.0.0\skills`，只读。

### 跨盘停用（后端测试）

隔离目录与 Skills 目录都在 D 盘，真机无法复现跨盘。新增测试 `skills::tests::claude_park_and_unpark_across_volumes`：Skills 目录放在仓库所在的 D 盘（`backend/target/` 下），停用区放在 `std::env::temp_dir()`（C 盘）。`fs::rename` 跨卷失败，走「复制 + 校验 + 删除原目录」路径。停用与启用来回各一次，SKILL.md、子目录里的文本文件、二进制文件内容都不变，`skills.json` 的 `disabled` 最后清空。如果两个目录在同一个盘，测试会输出「未测」并跳过。

```
cargo test --manifest-path backend/Cargo.toml claude_park_and_unpark_across_volumes
test skills::tests::claude_park_and_unpark_across_volumes ... ok
```

### 观察（未改，待鼠鼠确认）

1. **再次导入时「覆盖」会把已启用的 Skill 移进停用区。** 弹窗里的「导入后启用」每次都默认关闭，覆盖完成后按「关闭」处理，Claude Code 就把它移到 Jolu 停用区。这和弹窗文案「默认关闭；关闭时 Claude Code 放入 Jolu 停用区」一致，但用户覆盖一份正在用的 Skill 时，多半预期它保持启用。是否改为「覆盖时保留原启用状态」，待确认。
2. **停用区里的副本不算同名冲突。** 以「不启用」方式导入 test-driven-development 后，它在停用区里；再扫描同一来源，它显示「可导入」，再导入会产生第二份副本，之后启用时会因「原位置已有同名目录」失败。`lib/skillsManager.ts` 的 `hasNameConflict` 注释写明这是既定设计（启用时由后端再检查原位置），因此不改，只记录。
3. **测试过程中应用卡死一次。** 早期脚本在第一个原生选目录框还没出现时又点了一次「选择目录」，屏幕上叠了两个模态对话框；小的用消息取消它们后，Windows 事件日志记录 `jolu.exe` AppHang（09:46:03），应用被系统关闭。之后按正常流程一次只开一个对话框，重启后完整跑了两遍，没有复现。是否需要防止连点打开两个对话框，待确认。

## 步骤 2：深色主题

做法：在应用里调用 `set_theme("dark")`（不切换系统主题），逐页截主窗口（1600×1093）。用 Pencil 按名称找深色画板并导出 PNG 对照（导出到 `output/acceptance/dark/pen/`，未修改 `Jolu.pen`）。会话和用量页先通过「导入数据」的原生文件框导入了 24 条合成统计（`output/acceptance/task13/dark-fixture.json`），以便看到有数据的状态。

截图：`output/acceptance/dark/01-island.png` … `07-settings.png`；有数据的 `data-03-usage.png`、`data-04-sessions.png`；修复后的 `after-02-providers.png`、`after-03-usage.png`、`after-04-sessions.png`。

| 页面 | 深色画板 | 差异 | 处理 |
|---|---|---|---|
| 灵动岛 | 02 · 灵动岛 · 深色 | 设计稿未选中的导航项、顶栏图标按钮显示灰色底块，运行时没有 | 不改：节点填充是 `#00000000`（透明），浅色稿 01 同样如此，灰块是导出渲染的效果 |
| 供应商 | 06 · 供应商 · 直接连接 · 深色 | 页面滚动时右侧原生滚动条是浅色（白底灰块），深色界面上很突兀 | **已修复**（见下） |
| 供应商 | 同上 | 「添加自定义」「连接 OpenCode」禁用、OpenCode 标「待接入」 | 不改：AGENTS 已登记为「待接入」 |
| 用量统计 | 31 · 用量统计概览 · 深色 | 配色、卡片、图表线色一致；浅色滚动条同上 | 滚动条已随修复解决 |
| 会话线程 | 41 · 会话列表与消息 · 深色 | 「最近更新」按钮设计稿用 `arrow-down-wide-narrow` 图标，运行时是下拉箭头；浅色稿 40 同样如此，与深色无关 | 不改：运行时它是下拉菜单，属于实现取舍，待确认 |
| 会话线程 | 同上 | 设计稿 41、59、75、87 的顶栏主题按钮画的是太阳，02、06、31 画的是月亮，设计稿内部不一致；运行时深色下统一显示月亮 | 不改，待确认设计稿以哪个为准 |
| 自动化 | 59 · 任务列表 · 深色 | 设计稿统计卡片在「任务 / 运行记录」标签上方，运行时在标签下方 | 不改：统计卡片只属于「任务」标签（「运行记录」页有自己的标题和统计），与深色无关，待确认 |
| Skills 管理 | 75 · Skills 管理 · 深色 | 「全部范围」设计稿前面有文件夹图标；表格各列设计稿居中，运行时左对齐 | 不改：与深色无关的细节，待确认 |
| 设置 | 87 · 设置 / 常规 · 深色 | 「仅保存到此设备 / Jolu · 桌面端」设计稿贴在左栏底部，运行时紧跟在分区导航下方；「静默启动」灰显、「默认打开页面」标「待接入」 | 不改：前者是布局取舍；后两者分别因隔离环境未开机启动、AGENTS 已登记「待接入」 |

各页文字都清晰可读，背景、卡片、边框、强调色都取自 `jolu-*` 深色变量（`body` 背景 `rgb(13,13,13)` = `--jolu-bg` 深色值 `#0d0d0d`）。

### 修复：深色时原生滚动条仍为浅色

- 原因：样式里没有设置 `color-scheme`，WebView2 的原生滚动条和表单控件始终按浅色绘制。
- 改动：`frontend/src/index.css` 在 `@layer base` 中加入 `html.dark:not(.island-window) { color-scheme: dark; }`。只作用于主面板；灵动岛、右键菜单、托盘摘要窗口（根元素带 `island-window`）保持现状。
- 证据：修复前 `02-providers.png`、`data-03-usage.png`、`data-04-sessions.png` 右侧是浅色滚动条；修复后 `after-*.png` 是深色滚动条。修复后读取各窗口根元素的计算值：主窗口 `colorScheme: dark`，本体与分身灵动岛均为 `normal`。
- 这是纯样式改动，没有可写的单元测试，以修复前后截图为证。

## 步骤 3：灵动岛分身

脚本：`output/acceptance/task13/clones.mjs`、`primary-expand.mjs`、`clone-expand.mjs`。

1. 在灵动岛页面调用 `api.addConnection` 新增两条 Claude API Key 连接，Base URL 都是合成网关：「网关 A（余额 12.5）」用 `test-valid`，「网关 B（余额 0）」用 `test-zero`。后端 `connection_balance` 分别返回 `ok 12.5 USD`、`ok 0 USD`。
2. 本体选连接 A，`island_action("clone_create")` 开一个分身，分身改选连接 B。`settings.json`：`island_connection_id` = A，`island_clones[0].connection_id` = B。
3. 双击展开两个岛（本体是左侧停靠，先悬停探出再双击）：
   - 本体：「连接 网关 A（余额 12.5）· test****alid」「网关余额 12.50 USD」（绿色）
   - 分身：「连接 网关 B（余额 0）· test****zero」「网关余额 0.00 USD」（红色，表示耗尽）
   - 截图：`output/acceptance/task13/clone-primary-full.png`、`clone-clone-full.png`
4. 通过 IPC 请求追踪确认分身只用连接 B 查询 `connection_quota` / `connection_balance`。
5. 主窗口「灵动岛」页的「连接与分身」表也显示本体 = 网关 A、分身 1 = 网关 B（`output/acceptance/dark/01-island.png`）。

说明：探出的精简卡片只显示 Token 和估算费用，余额只在双击展开后显示，这是 A2 画板的既定布局。网关额度窗口（`test-quota`）这次没有覆盖。安装版 + 真实账号的确认仍留在 `docs/todo/2026-10-01-island-clones.md` 的 V-4。

## 步骤 4：安装包

未测（本机无 Windows Sandbox）：`C:\Windows\System32\WindowsSandbox.exe` 不存在，按控制方裁决不安装。待办：`docs/todo/2026-10-23-installer-sandbox-check.md`。

## 测试

- `cargo test --manifest-path backend/Cargo.toml`：241 通过，0 失败，10 忽略（含新增 `claude_park_and_unpark_across_volumes`）
- `node --test frontend/src/lib/*.test.mjs`：144 通过，0 失败
- `pnpm --dir frontend typecheck`：通过

## 收尾

只停止了本次启动的进程：隔离应用 30428（09:46 因 AppHang 被系统关闭）、41628、17992，网关 33996、28712、43076，前端开发服务器（后台任务）。9337、18765、5173 端口均已释放。隔离目录保留在 `output/acceptance/profile/` 下。
