# 账号、会话与运行体验整合交付

## 改动总结

- 账号管理：固定区分 5 小时和周额度；订阅历史记录显示来源与观察时间；Codex 重置卡查询保留真实零。完整授权用当前 Windows 用户加密保存，切换时同时处理默认 profile、服务地址和授权来源，并提供加密备份恢复脚本。
- 会话与多工具：消息支持 Markdown；压缩会话支持生成、修订、预览和导出；多平台 Skills、ZCode/OpenCode 连接及 OpenCode 本机用量采集接入。WorkBuddy 仅使用 `.codebuddy/skills`。
- 更新与隔离：下载取消、重试、稍后安装、运行任务保护、自动检查偏好；隔离验收不读取真实日志/会话，不写真实 CLI 配置。
- 交互与性能：异步结果按当前视图归属，修复设置推送被旧请求覆盖；大文本差异限制计算量；七页按需加载，首次显示骨架，资源异常保留侧栏操作并提供明确恢复入口。
- 文档：纠正旧的评测授权、OpenCode 真机验证与 Token 统计口径，保留尚未验证的真实服务和安装边界。
- 后续完整性修补：Claude 完整订阅授权与身份快照恢复；ZCode 手填模型需明确确认并验证通过再保存；交接文档重名时默认另存为首个可用后缀名称；恢复单账号查询时仅通知相关账号刷新。

## 当前证据

- `cargo test --manifest-path backend/Cargo.toml --features handoff-eval --quiet`：最终修补后 484 通过、0 失败、12 忽略，退出码 0；覆盖 Claude 完整身份、首次 API 配置创建、Codex profile/授权来源与交接重名建议。较早的 471、483 项结果为中间阶段。
- 前端类型检查与最新生产构建通过。上阶段 198 项逻辑/共享状态测试及发布脚本 12 项通过；本阶段新增 ZCode 真实弹窗合成回归通过，覆盖四组地址/协议、用量确认、失败保留输入、验证失效及获取期间输入禁用，没有重复无关测试。
- 独立应用标识的隔离桌面七页冒烟通过，无控制台 error、未捕获异常或读取/扫描/刷新失败。该轮先于最后的 profile 守卫；后续守卫通过文件级回归验证，不以隔离 UI 替代真实登录切换。
- 生产预览七页、延迟加载、资源加载失败恢复及会话到用量/自动化的参数传递通过。可复跑脚本见 `scripts/acceptance/lazy-pages-preview.e2e.mjs` 和 `lazy-jumps-preview.e2e.mjs`。
- 账号卡片合成渲染验证四列、两类额度窗口、快照日期、重置卡真实零及隐私显示。Pencil 画板与页面对照记录见本日对应报告。
- `3b237f7` 阶段补充真实组件合成回归：Skills 快速切文件/保存防重、任务跨账号额度异步隔离，以及账号单卡刷新成功/失败均通过。该次仅改测试与文档；之后的功能修补另行验证和重新打包。
- 主窗口 chunk 661.43 → 71.56 kB；这只是入口资源体积，不代表总下载量或启动耗时同比下降。
- 5000 行全异合成文本的差异计算约 123 → 2 ms；大块改写回退为整段差异，降低精细度以限制界面线程工作量。

## 安装包

- 普通 NSIS 安装包构建成功，退出码 0。命令使用 `--config` 临时设置 `bundle.createUpdaterArtifacts=false`，生产公钥和配置文件保持原样，默认更新仍关闭。
- 最新修补产物：`backend/target/release/bundle/nsis/Jolu_0.1.0_x64-setup.exe`，4,697,393 字节，2026-10-23 23:08:54；构建退出码 0，包含本轮 Claude、ZCode 与交接重名处理。
- SHA256：`757B18260C1C450EABFE2D1A6A0B51FE9F3113B26F55FDBFB7EFD2C94FFC8793`。
- 首轮默认命令已生成安装器，但因缺少正式私钥在更新签名阶段退出 1；之后用上述普通安装包配置重新构建并成功，不把首轮失败隐藏为成功。
- 未安装或覆盖本机应用。STATIC_VCRUNTIME 弃用、MSVC 导入库输出和两处动静态 import 混用提示仍存在，没有隐藏警告。

## 尚待外部验证

- 实时订阅接口返回 403，当前只提供有来源的历史到期记录；不能保证实时订阅状态。
- Claude 官方 `/api/oauth/profile` 身份核实、完整快照与三文件恢复已实现并通过合成专项；本机缺完整订阅 OAuth 登录，真实预检为 0 次 HTTP 请求，真实身份接口与跨账号切换仍未验证。证据见本日账号后端验收记录；Codex 非文件凭据存储明确拒绝写入切换。
- 真实桌面最后视觉复核遇到 Windows 锁屏，已请求解锁并暂停原生桌面操作。
- 正式签名、Release/latest.json、安装升级/重启及通知中心实际点击，仍遵守鼠鼠此前“先记待办”的决定。
- ZCode Key、OpenCode 登录、长会话评测样本等外部条件见 `docs/todo/2026-10-22-unverified-integrations.md`。

完整目标尚不能据此宣称全部完成。本记录是已实现并验证代码阶段的交付，不是正式发布验收。

## 提交清单

完整阶段候选 100 个文件；测试产物、安装包、本机数据及设计稿不纳入。

```text
AGENTS.md
README.en.md
README.md
backend/Cargo.lock
backend/Cargo.toml
backend/build.rs
backend/src/auth_snapshot.rs
backend/src/automation.rs
backend/src/bin/handoff_eval.rs
backend/src/cli_apply.rs
backend/src/coding_plan.rs
backend/src/collector.rs
backend/src/connections.rs
backend/src/creds.rs
backend/src/current_connection.rs
backend/src/db.rs
backend/src/grok_quota.rs
backend/src/handoff_eval.rs
backend/src/lib.rs
backend/src/login_detect.rs
backend/src/native_sources.rs
backend/src/notify.rs
backend/src/opencode.rs
backend/src/opencode_usage.rs
backend/src/platforms.rs
backend/src/quota.rs
backend/src/session_handoff.rs
backend/src/session_threads.rs
backend/src/settings.rs
backend/src/skills.rs
backend/src/updater.rs
backend/src/validation.rs
docs/plans/2026-10-21-jolu-development-roadmap.md
docs/plans/2026-10-23-account-completion-audit.md
docs/todo/2026-10-22-unverified-integrations.md
docs/verification/2026-10-23-account-backend.md
docs/verification/2026-10-23-account-interface.md
docs/verification/2026-10-23-current-connection-audit.md
docs/verification/2026-10-23-integration-stage.md
docs/verification/2026-10-23-isolation-and-notification.md
docs/verification/2026-10-23-project-audit.md
docs/verification/2026-10-23-real-validation.md
docs/verification/2026-10-23-remaining-tasks.md
docs/verification/2026-10-23-update-audit.md
frontend/package.json
frontend/pnpm-lock.yaml
frontend/src/components/automation/TaskDetail.tsx
frontend/src/components/panel/PageHeader.tsx
frontend/src/components/panel/UpdateContext.tsx
frontend/src/components/panel/UpdateContextBase.ts
frontend/src/components/panel/UpdateDialog.tsx
frontend/src/components/panel/lazyPage.tsx
frontend/src/components/providers/AccountsPanel.tsx
frontend/src/components/providers/ConnectDialog.tsx
frontend/src/components/sessions/CompressChat.tsx
frontend/src/components/sessions/CompressDialog.tsx
frontend/src/components/sessions/HandoffPreview.tsx
frontend/src/components/sessions/MessageMarkdown.tsx
frontend/src/components/sessions/SessionDetail.tsx
frontend/src/components/sessions/handoffParts.tsx
frontend/src/components/settings/AboutSection.tsx
frontend/src/components/skills/SkillDetail.tsx
frontend/src/components/skills/SkillEditor.tsx
frontend/src/components/skills/SkillsTable.tsx
frontend/src/components/skills/skillDialogs.tsx
frontend/src/components/skills/skillParts.tsx
frontend/src/hooks/useConnections.ts
frontend/src/hooks/useSettings.test.mjs
frontend/src/hooks/useSettings.ts
frontend/src/lib/accountActions.test.mjs
frontend/src/lib/accountActions.ts
frontend/src/lib/accountDisplay.test.mjs
frontend/src/lib/accountDisplay.ts
frontend/src/lib/activeRequest.test.mjs
frontend/src/lib/activeRequest.ts
frontend/src/lib/api.ts
frontend/src/lib/handoffApi.ts
frontend/src/lib/platformCatalog.json
frontend/src/lib/providerCatalog.test.mjs
frontend/src/lib/providerCatalog.ts
frontend/src/lib/sessionHandoff.test.mjs
frontend/src/lib/sessionHandoff.ts
frontend/src/lib/skillsApi.ts
frontend/src/lib/skillsManager.ts
frontend/src/lib/textDiff.test.mjs
frontend/src/lib/textDiff.ts
frontend/src/lib/updateState.ts
frontend/src/startup.css
frontend/src/views/MainPanelWindow.tsx
frontend/src/views/SessionsView.tsx
scripts/acceptance/README.md
scripts/acceptance/account-metadata.cjs
scripts/acceptance/lazy-jumps-preview.e2e.mjs
scripts/acceptance/lazy-pages-preview.e2e.mjs
scripts/acceptance/update-flow.cjs
scripts/handoff-eval/README.md
scripts/release/local-update-e2e.md
scripts/release/verify-release.mjs
scripts/release/verify-release.test.mjs
scripts/restore-cli-backup.ps1
```
