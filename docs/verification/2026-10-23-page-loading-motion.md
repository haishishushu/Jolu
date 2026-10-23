# 主窗口加载与交互验收（2026-10-23）

## 本阶段结果

- 七个主页面统一加载占位与内容过渡；150ms 后显示骨架，快请求不延迟内容。
- 指标、表格、卡片、设置、会话列表与消息使用对应结构骨架；后台刷新保留同一查询的数据。
- 窗口内缓存最多 24 项、30 秒过期，返回页面立即展示可用快照并重新请求；查询身份切换读取对应缓存，否则显示骨架，旧响应不覆盖新身份。
- 主窗口按钮统一颜色、描边、透明度反馈；共用操作按钮轻微按压，导航与列表不缩放。
- 标签页与详情切换平滑过渡；主要弹窗取消、确认、保存成功及 Esc 进入统一退出流程。
- 系统与应用内减少动效取消骨架扫光、位移和缩放；不改浮动灵动岛的专用动效。

## 验证证据

- `pnpm --dir frontend typecheck`：通过；另外从暂存区导出独立前端版本，类型检查和 15 项相关测试通过。
- `node --test frontend/src/lib/*.test.mjs`：185 项通过。
- `pnpm --dir frontend build`：通过；现有主面板 chunk 超过 500kB 的提示仍存在。
- 浏览器预览：七个主页面、可见按钮过渡、统计三个页签、浅深色、减少动效、控制台错误检查通过。
- 受控请求夹具：慢请求骨架、就绪显示、切换身份、失败退出骨架、返回缓存、旧响应丢弃、刷新保留内容及重挂载缓存通过。
- 弹窗夹具：日期取消/确认、设置导出取消、Esc 关闭、减少动效关闭通过。
- 独立只读审查发现并修复跨身份数据暂留、取消按钮绕过退出及退出时长不一致；返回会话入口也已统一。
- 已查看 Pencil 01 / 05 / 38 / 58 / 74 / 86 / 104 画板；保持既有业务页面布局，加载与动效依据鼠鼠本轮确认方案。

## 验证边界

界面验证使用本地浏览器预览与受控异步夹具，没有在真实账号上逐个执行删除、付费调用或文件写入。此次未重新打桌面安装包。截图与临时验证脚本位于被忽略的 output/acceptance/。

## 本轮文件范围

- `frontend/src/index.css`
- `frontend/src/hooks/useAutomationQuery.ts`
- `frontend/src/hooks/useConnections.ts`
- `frontend/src/hooks/useLiveQuery.ts`
- `frontend/src/lib/motion.ts`
- `frontend/src/views/AutomationView.tsx`
- `frontend/src/views/IslandView.tsx`
- `frontend/src/views/MainPanelWindow.tsx`
- `frontend/src/views/ProvidersView.tsx`
- `frontend/src/views/SessionsView.tsx`
- `frontend/src/views/SettingsPage.tsx`
- `frontend/src/views/SkillsView.tsx`
- `frontend/src/views/UsageView.tsx`
- `frontend/src/components/automation/CancelTaskDialog.tsx`
- `frontend/src/components/automation/DeleteTaskDialog.tsx`
- `frontend/src/components/automation/SkillPicker.tsx`
- `frontend/src/components/automation/TaskDialog.tsx`
- `frontend/src/components/providers/ConnectDialog.tsx`
- `frontend/src/components/sessions/SessionDetail.tsx`
- `frontend/src/components/sessions/sessionDialogs.tsx`
- `frontend/src/components/settings/DataDialogs.tsx`
- `frontend/src/components/settings/settingsUi.tsx`
- `frontend/src/components/skills/SkillDetail.tsx`
- `frontend/src/components/skills/skillDialogs.tsx`
- `frontend/src/components/skills/skillParts.tsx`
- `frontend/src/components/ui/JoluButton.tsx`
- `frontend/src/components/usage/ActivityCalendar.tsx`
- `frontend/src/components/usage/AnalysisPanel.tsx`
- `frontend/src/components/usage/DateRangeDialog.tsx`
- `frontend/src/components/usage/usageControls.tsx`
- `frontend/src/hooks/useDialogClose.ts`
- `frontend/src/lib/queryCache.test.mjs`
- `frontend/src/lib/queryCache.ts`
- `frontend/src/components/ui/LoadingSurface.tsx`

另纳入本计划、验收记录和 AGENTS.md 的提交节奏规则。已有其他未提交功能不属于本次范围。
