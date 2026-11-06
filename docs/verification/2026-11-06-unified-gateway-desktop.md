# 统一网关真实桌面验收（2026-11-06）

## 范围

统一网关（J6）「模型来源 → 最近会话」的数据闭环：真实 CLI 请求经 Jolu 网关时的会话识别、运行状态、来源关联、路由流水与界面展示。

## 环境

- 隔离 Jolu 实例：`start.ps1 -Executable output/acceptance/target-5193/debug/jolu.exe`（独立 identifier、5193 前端、隔离数据目录，不采集本机日志）。
- 合成模型上游：`scripts/acceptance/model-upstream.py`，回环 18766，回复前等待 4 秒。
- 真实 CLI（各自独立主目录 `output/acceptance/cli-home/<工具>`，去掉本机工具环境变量）：
  Claude Code 2.1.318、Codex CLI 0.162.0、OpenCode 1.19.4、Gemini CLI 0.65.0。
- 驱动脚本：`scripts/acceptance/gateway-unified-desktop.e2e.mjs`。

## 结果：15 项全部通过

| 检查 | 结果 |
| --- | --- |
| 请求进行中界面显示「运行中」 | 通过（在途 2 个，亮线与流光，见 `02-running.png`） |
| 四个 CLI 经网关拿到回复并正常退出 | 通过（退出码均为 0，输出 `OK`） |
| 请求全部结束后没有残留在途请求 | 通过 |
| Claude Code 会话 ID 与 `CLAUDE_CONFIG_DIR/projects/*/<id>.jsonl` 一致，并连到 Anthropic 来源 | 通过 |
| Codex 会话 ID 与 `rollout-…-<id>.jsonl` 一致，并连到 OpenAI 来源 | 通过 |
| OpenCode 会话 ID（`opencode:ses_…`）能在 OpenCode 本地数据中找到 | 通过 |
| Gemini 请求写入路由流水并识别为 Gemini CLI | 通过 |
| Gemini 请求不带会话标识，不生成会话 | 通过 |
| 最近会话显示 Claude Code / Codex / OpenCode 会话卡 | 通过 |
| 路由流水显示后端分页统计 | 通过 |

报告与截图：`output/acceptance/gateway-unified-desktop/`（已被 Git 忽略）。

## 实测证据

- OpenCode 1.19.4 每个模型请求带 `x-opencode-session-id`（与 `x-session-affinity`、`x-session-id` 同值）。
- Gemini CLI 0.65.0（API Key 模式）请求头只有 `User-Agent: CloudCodeVSCode/0.65.0 (…proxy_client=geminicli)`、SDK 版本与 Key，请求体只有 `contents / generationConfig / systemInstruction / tools`，**没有会话标识**；官方源码里 `sessionId` 只传给 Google 登录（Code Assist）路径。

## 验收中发现并已修复的问题

1. Gemini 入口不写路由流水、不发运行事件、丢弃请求头：已改为与其他入口一致。
2. Gemini CLI 0.65 的 UA 不再以 `GeminiCLI` 开头，导致识别不到 Agent：已兼容 `proxy_client=geminicli`。
3. 没有标题时兜底标题显示成「会话 opencode」：改为取去掉统计前缀后的原始 ID。
4. 共用滚动区在页面刷新后被浏览器自带的滚动锚定推下约 20px：关闭 `overflow-anchor`，离顶部 24px 内视为在顶部。

## 后续决定

鼠鼠选定方案 A：Gemini 这类不带会话标识的请求只在来源卡上显示「运行中 · 工具名」，不进最近会话（Jolu.pen `J6-04` 05；预览验收已覆盖）。

## 限制

- 隔离模式不采集本机日志，会话标题、项目与 Token 显示为兜底标题与「—」；真实环境中由会话线程统计补齐。
- 验收时临时把手动接入 Key 写入剪贴板读取，随后恢复原剪贴板文本；原剪贴板若是图片等非文本内容不会被恢复。
- 自动化运行中重复开启转发会尝试再次绑定同一端口而报错；界面开关只在关闭时开启，不受影响，脚本已改为先判断状态。
