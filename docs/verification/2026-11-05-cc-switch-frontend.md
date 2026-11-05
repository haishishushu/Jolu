# CC Switch 风格前端验收

## 结果

已按 `docs/ui/Jolu.pen` 把全部现有前端模块同步为实色表面、细边和短投影；保留原业务逻辑、浅深色、绿色品牌及既有数据状态。先完成设计说明和实施计划，再编写产品代码。前端实现阶段未更改后端、依赖或 Git 状态；设计稿的工作区修改来自上一阶段，本轮仅通过 Pencil 读取和截图。

范围：主窗口、供应商与账号、签到、用量统计、会话、自动化、Skills、设置、模型入口、浮动窗口、右键菜单和启动页。目录、品牌画板及历史备选不等于额外的运行页面，模型入口仍使用当前 V3。后续同步远端已实现的胶囊灵动岛，保留经典/胶囊双样式及其独立容器，详见下方同步记录。

## 改动总结

- 从 Pencil 提取并同步 `jolu-cc-*` 浅深色变量，组件显式应用 `jolu-card`、`jolu-control`、`jolu-tablist` 与 `jolu-tab`。卡片、按钮、输入框、菜单、通知和弹窗共享材质，保持原布局及语义状态。
- 供应商、统计、自动化及连接表单采用灰轨道分段页签；侧栏和页头控件同步细边与圆角。浮动岛短投影更新，布局和功能不变。启动页 109 浅色为白色，110 深色按画板保留 `#101211`，不套用主窗口深色底。
- 新增 `ConfirmDialog.tsx`：取消不执行、默认聚焦取消、危险确认不因遮罩点击关闭、同步 ref 阻止同一事件循环重复提交；执行中阻止关闭，成功退出，失败原位显示原因并可重试。
- 自动化删除/取消复用确认组件；保留账号和批量删除、Skills 移除、导入冲突与数据清理的原有确认。账号和 Skill 移除增加同步互斥。
- 模型入口的浮层迁移到 Radix。补齐 Agent 接线/断开/接受外部配置、关闭有接线的模型转发、刷新网关 Key 的确认；Codex/Gemini 的专用说明保留，操作成功后才关闭。紧急直连仍直接执行。
- 插入 Skills 基础模板覆盖草稿前确认；设置页及右键菜单销毁灵动岛前确认，取消不执行。右键菜单使用紧凑确认布局。

## 执行结果

| 检查 | 结果 |
| --- | --- |
| `pnpm --dir frontend typecheck` | 通过 |
| `pnpm --dir frontend build` | 最终通过，2096 个模块；仅生成前端 dist，不是安装包 |
| accountActions / gatewayPage / skillsManager / automation / mainNavigation 单元测试 | 41 项通过，0 失败 |
| `node scripts/acceptance/cc-switch-ui.e2e.mjs` | 9 主模块 × 浅深色共 18 组通过；另含账号、连接弹窗、5 个额外设置分区双主题、窄窗口、浮动岛展开/收起和启动页材质 |
| `node scripts/acceptance/confirm-dialog.e2e.mjs` | 取消不执行、初始焦点、焦点恢复、遮罩保护、防重复、执行中不可退出、失败重试、模板覆盖及紧凑右键销毁确认通过 |
| `node scripts/acceptance/gateway-confirmations.e2e.mjs` | 断开、Codex/Gemini 接线失败重试、接受配置、关闭转发取消及 Key 刷新确认通过；未调用真实原生桥接 |
| `git diff --check -- frontend scripts docs/plans docs/specs` | 通过 |

前端构建保留两条已有的静态/动态混合 import 分块提示：Tauri window 与 skills mock；不影响构建结果，本轮未改变依赖或加载架构。

浏览器自动化使用独立无头 Edge、本机 5197 开发服务和已有 `output/acceptance/node` 内的 playwright-core。涉及写操作的 API 都用合成 Promise 替换；启动页只隔离原生桥接验证原始 HTML/CSS，不宣称启动流程已实机验收。

## 视觉证据

通过 Pencil 重新截取供应商、设置、会话、自动化、Skills、签到、模型分组确认及浮动岛画板，对照实际运行截图。检查内容区横向溢出、页签裁切、卡片/选中页签投影、液态模糊残留、表单与弹窗双主题表面；已查看代表性实际截图。

截图和机器检查结果位于 `output/acceptance/cc-switch/`（忽略的验收产物）：

- `providers-light.png` / `providers-dark.png`：供应商；`accounts-*`、`connect-*`：账号与连接表单。
- `island-*`、`usage-*`、`sessions-*`、`automation-*`、`skills-*`、`checkin-*`、`gateway-*`、`settings-*`：主页面。
- `settings-appearance-*`、`settings-network-*`、`settings-data-*`、`settings-notifications-*`、`settings-about-*`：设置分区。
- `floating-*` / `floating-expanded-*`：浮动岛；`startup-*`：启动页材质。
- `confirm-dark.png`、`gateway-key-confirm-light.png`、`clone-confirm-compact.png`：确认与失败保留。
- `ui-results.json`、`build.log`：自动检查数据与最终构建日志。

## 独立审查及修复

按代码审查技能由只读审查者复核未提交变更。未发现其他可靠的 Critical / Important 问题，指出链式弹窗和下拉菜单触发元素卸载时可能丢失焦点。

已实测“插入模板 → 基础模板 → 取消”后焦点落到 body，新增断言先失败，再加入显式稳定的 `returnFocusTo`。模板确认回到“插入模板”，Agent 接线确认回到“接入设置”，灵动岛销毁确认回到该行更多按钮。模板和模型入口焦点断言随后通过。此问题属于文档明确要求的焦点恢复，已修复，未留待办。

## 验证边界

本轮验收证明前端表现和合成异步交互，不代表真实账号登录、真实工具配置写入、Windows 原生菜单失焦/窗口销毁或安装升级已经实机验收。未运行真实危险操作、未制作安装包、未提交或推送。

## 远端同步后的复验与提交范围

鼠鼠明确授权处理重叠改动并提交。远端的 `44487b1`（新增胶囊样式并解耦窗口）和 `960d352`（本轮增量与写入状态）已经快进同步到本地，再恢复本阶段改动；两处重叠修改自动合并成功，没有遗留冲突标记。

独立文件备份在 `output/backups/2026-11-05-cc-switch-merge/`，原始 stash `a681323fc1bb4c662c3643d5b43a81b91d621790` 保留，便于恢复。胶囊样式、预览组件、独立容器和后端保持远端版本；样式选择入口及防图片拖拽规则也已保留。

合并后复验：类型检查通过，原 41 项加胶囊相关单测共 80 项通过。浏览器使用全新 5198 服务复验，避免旧 Vite HMR 时间戳导致测试注入命中另一份模块；旧服务中曾出现此环境问题（确认框显示 not-tauri），没有据此修改产品代码。合并后最终构建和三个浏览器验收全部通过（18 组主模块主题、设置分区、连接表单、模板覆盖、紧凑销毁确认与网关确认）。最终构建日志为 `output/acceptance/cc-switch/build-after-sync.log`。

提交身份：`还是鼠鼠 <187837522+haishishushu@users.noreply.github.com>`。目标 `main → origin/main`，使用 `[skip ci]`，不触发安装包发行。未加入依赖、缓存、预览图片、备份目录或疑似敏感文件。

本次提交清单（56 个文件，M 为修改，A 为新增）：

```text
A docs/plans/2026-11-05-cc-switch-frontend.md
A docs/specs/2026-11-05-cc-switch-frontend.md
M docs/ui/Jolu.pen
A docs/verification/2026-11-05-cc-switch-frontend.md
M frontend/src/components/automation/CancelTaskDialog.tsx
M frontend/src/components/automation/DeleteTaskDialog.tsx
M frontend/src/components/automation/SkillPicker.tsx
M frontend/src/components/automation/TaskDialog.tsx
M frontend/src/components/automation/automationParts.tsx
M frontend/src/components/checkin/CheckinAccountStats.tsx
M frontend/src/components/checkin/CheckinDialogs.tsx
M frontend/src/components/gateway/GatewayAccessBar.tsx
M frontend/src/components/panel/PageHeader.tsx
M frontend/src/components/panel/SideNav.tsx
M frontend/src/components/panel/UpdateDialog.tsx
M frontend/src/components/providers/AccountBatchControls.tsx
M frontend/src/components/providers/AccountsPanel.tsx
M frontend/src/components/providers/ConnectDialog.tsx
M frontend/src/components/sessions/SessionDetail.tsx
M frontend/src/components/sessions/handoffParts.tsx
M frontend/src/components/sessions/sessionDialogs.tsx
M frontend/src/components/settings/AboutSection.tsx
M frontend/src/components/settings/DataDialogs.tsx
M frontend/src/components/settings/DataSection.tsx
M frontend/src/components/settings/NetworkSection.tsx
M frontend/src/components/settings/settingsUi.tsx
M frontend/src/components/skills/SkillDetail.tsx
M frontend/src/components/skills/SkillEditor.tsx
M frontend/src/components/skills/SkillsTable.tsx
M frontend/src/components/skills/skillDialogs.tsx
M frontend/src/components/skills/skillParts.tsx
A frontend/src/components/ui/ConfirmDialog.tsx
M frontend/src/components/ui/JoluButton.tsx
M frontend/src/components/ui/LoadingSurface.tsx
M frontend/src/components/ui/SelectField.tsx
M frontend/src/components/ui/Tip.tsx
M frontend/src/components/ui/Toast.tsx
M frontend/src/components/ui/primitives.tsx
M frontend/src/components/usage/CallsPanel.tsx
M frontend/src/components/usage/DateRangeDialog.tsx
M frontend/src/components/usage/TrendChart.tsx
M frontend/src/components/usage/usageControls.tsx
M frontend/src/index.css
M frontend/src/startup.css
M frontend/src/views/CheckinView.tsx
M frontend/src/views/ContextMenuWindow.tsx
M frontend/src/views/GatewayView.tsx
M frontend/src/views/IslandView.tsx
M frontend/src/views/MainPanelWindow.tsx
M frontend/src/views/ProvidersView.tsx
M frontend/src/views/SessionsView.tsx
M frontend/src/views/SkillsView.tsx
M frontend/src/views/UsageView.tsx
A scripts/acceptance/cc-switch-ui.e2e.mjs
A scripts/acceptance/confirm-dialog.e2e.mjs
A scripts/acceptance/gateway-confirmations.e2e.mjs
```
