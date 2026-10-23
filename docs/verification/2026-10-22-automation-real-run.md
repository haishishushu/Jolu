# 自动化真实执行验收（任务 1.2）

- 日期：2026-10-22 20:40–20:55（真实运行），2026-10-23 修复后复测
- 环境：Windows 11，`pnpm tauri dev`（开发版，真实数据目录 `%APPDATA%\dev.ningz.jolu`，未用 `JOLU_TEST_DIR`）；Claude Code 2.1.308、codex-cli 0.162.0（均为 npm 全局安装，`claude.cmd` / `codex.cmd`）
- 账号：Claude Code 当前登录的连接「Claude」（API Key），Codex 当前登录的连接「工作账号」（订阅）；`automation_accounts` 均返回 `current: true`
- 驱动：WebView2 远程调试口 9337 + `playwright-core` 的 `connectOverCDP`，在主窗口调用 `automation_create` / `automation_task` / `automation_action` / `automation_run`，每 300 ms 轮询一次状态；运行记录页和任务页在界面上核对
- 任务：提示词「只回复 OK」，工作目录 `output/automation-e2e/`（Git 忽略，位于仓库内，因此 Codex 的 Git 仓库检查能通过），名称以「验收-」开头；每个工具跑 4 次
- 残留检查：每次结束后列出 `jolu.exe` 的全部后代进程和基线之外新增的 npm 版 `claude` / `codex` 进程

## 结果总览

| 项 | Claude Code | Codex |
|---|---|---|
| 手动启动（立即运行） | 通过 | 通过 |
| 定时（两分钟后单次） | 通过 | 通过 |
| 运行中停止 | 通过 | 通过 |
| 续接原会话 | 通过 | 通过 |
| 额度耗尽 → 进入等待并记下重置时间（代码路径） | 发现缺陷，已修复，测试通过 | 发现缺陷，已修复，测试通过 |
| 真实耗尽额度后续跑 | 未测（需真实耗尽额度） | 未测（需真实耗尽额度） |
| 在界面上核对运行记录状态 | 通过 | 通过 |
| 非 Git 目录执行 | 不涉及 | 原先报「运行失败」（鼠鼠已有任务的真实记录）；已按裁决 B 改为「需要处理」，测试通过，未在真机上复测（见「发现的问题 2」） |

## 明细与证据

时间为本机时间（Asia/Shanghai）。「状态序列」是轮询观察到的状态；「时间线」是运行记录里的事件。

### Claude Code

| 项 | 状态序列 | 时间线 | Token（输入 / 缓存读 / 输出） | 结果 | 残留 |
|---|---|---|---|---|---|
| 立即运行（运行 2） | 20:43:13 running → 20:43:19 completed | 进入执行队列 → 开始执行 · 新建会话 → 完成任务 | 37,334 / 0 / 45 | `OK` | 无 |
| 停止（运行 3） | 20:44:16 running → 20:44:18 发送停止 → 20:44:19 cancelled | 进入执行队列 → 开始执行 → 已停止运行 | 全部为 null（界面显示「—」） | — | 无 |
| 定时（运行 4，计划 20:47） | 20:44:32 无运行（next_run_at=20:47:00）→ 20:47:02 running → 20:47:10 completed | 进入执行队列 → 开始执行 → 完成任务；来源 `schedule` | 36,901 / 29,569 / 71 | `OK` | 无 |
| 续接（运行 5，会话 `b6481b3a…` 来自运行 2） | 20:47:51 running → 20:47:57 completed | 进入执行队列 → 开始执行 · 在所选会话中追加任务指令 → 完成任务 | 37,617 / 37,332 / 4 | `OK` | 无 |

- 立即运行：两次轮询之间就从排队进入了执行，所以轮询没观察到 queued，排队状态以时间线里的「进入执行队列」为准。
- 定时：比计划晚 2 秒启动（调度每 30 秒检查一次，或在有操作时被唤醒，所以最多可能晚约 30 秒）。
- 续接：返回的 `session_id` 和所选会话一致；`~/.claude/projects/D--develop-cc-usage-output-automation-e2e/` 下仍然只有两个会话文件，`b6481b3a….jsonl` 从 30 行增加到 41 行，没有新建会话；缓存读 37,332，说明复用了原上下文。

### Codex

| 项 | 状态序列 | 时间线 | Token（输入 / 缓存读 / 输出） | 结果 | 残留 |
|---|---|---|---|---|---|
| 立即运行（运行 6） | 20:48:14 running → 20:48:48 completed | 进入执行队列 → 开始执行 · 新建会话 → 完成任务 | 24,064 / 13,312 / 5 | `OK` | 无 |
| 停止（运行 7） | 20:49:49 running → 20:49:50 发送停止 → 20:49:51 cancelled | 进入执行队列 → 开始执行 → 已停止运行 | 全部为 null | — | 无 |
| 定时（运行 8，计划 20:52） | 20:50:06 无运行 → 20:52:06 queued → 20:52:06 running → 20:52:39 completed | 进入执行队列 → 开始执行 → 完成任务；来源 `schedule` | 24,064 / 13,312 / 5 | `OK` | 无 |
| 续接（运行 9，会话 `01a163a7…` 来自运行 6） | 20:52:56 running → 20:53:22 completed | 进入执行队列 → 开始执行 · 在所选会话中追加任务指令 → 完成任务 | 52,343 / 37,248 / 10 | `OK` | 无 |

- 续接：返回的 `session_id` 不变；`~/.codex/sessions/2026/10/22/rollout-…-01a163a7-….jsonl` 从 15 行增加到 28 行。续接期间修改时间有变化的另一个会话文件是 16:04 创建、属于另一个项目（`pi-desktop-harness`）的 Codex 会话，不是这次新建的。
- 停止：被结束的进程树是 `codex.cmd` → `node codex.js` → `codex.exe`，停止后 npm 路径下没有新增的 `codex.exe` / `node.exe`。

### 进程残留

8 次运行结束后，`jolu.exe` 的后代进程都只有 7 个 `msedgewebview2.exe`。基线之外新出现的 `claude.exe` / `codex.exe` / `node.exe` 已逐个追查父进程，来源分别是 VS Code 扩展（父进程是 `Code.exe`）和基线里已有的 `codex.exe` 拉起的 `chrome-devtools-mcp`，都不是 Jolu 拉起的。

### 界面核对

- 2026-10-22 运行记录页（今天）：任务名、`#RUN-00x`、「Claude · Claude」/「Codex · 工作账号」、「立即执行」/「定时执行」、执行时长、Token（如 52.4K、37.6K）都显示正确；运行 3、7 显示「已取消」，Token 显示「—」；其余显示「已完成」。顶部汇总为今日完成 6、失败 1（这 1 次是鼠鼠已有任务 16:02 的那次，见下文）。
- 2026-10-23 修复后复测（重新编译并启动应用，不发模型请求）：任务页列出 8 个验收任务，状态为「已完成」或「已取消 · 昨天 20:49 停止」，鼠鼠的任务「测试」仍为「已启用 · 今天 16:02 · 上海」。截图保存在 `output/automation-e2e/ui-*.png`（Git 忽略）。

## 发现的问题

### 1. 额度耗尽时判断错误或拿不到重置时间（已修复）

- **现象**：按本机 Claude Code 2.1.308 的实际输出，额度耗尽会被判成「运行失败」，任务不会进入等待；Codex 能判成额度耗尽，但拿不到重置时间。
- **原因**：
  - `limit_reset` 只认 `Claude AI usage limit reached|<时间戳>` 这一种格式，但 2.1.308 的可执行文件里已经没有这个字符串。现在的提示由 `You've hit your <限额名>` 和 ` · resets <时间>` 拼成，限额名包括 `session limit`、`weekly limit`、`Opus limit`、`Sonnet limit` 等。`is_limit` 只匹配 `hit your limit`，所以 `You've hit your session limit` 漏判，结果归为失败。
  - Claude stream-json 会输出结构化的 `rate_limit_event`（`rate_limit_info.status = "rejected"`，`resetsAt` 为 Unix 秒，见官方文档 Agent SDK 的 RateLimitEvent / RateLimitInfo），`Progress::feed` 原先没有解析它。
  - Codex 的提示是 `You’ve hit your usage limit. … or try again at <时间>.`，时间按本机时区格式化为 `3:05 PM`（当天）或 `Oct 23rd, 2026 3:05 PM`（跨天），原先没有解析。
- **修复**（`backend/src/automation.rs`）：
  - `Progress` 新增 `rate_limited`、`limit_resets_at` 两个字段，解析 Claude 的 `rate_limit_event`：`status` 为 `rejected`、且超额用量不可用（`overageStatus` 缺失，或不是 `allowed` / `allowed_warning`）时才置位，并记下重置时间。超额用量可用时会话会继续跑，不算耗尽。
  - `is_limit` 增加关键词 `ve hit your`，两种撇号都能匹配。
  - 新增 `codex_retry_at`，解析 Codex `try again at` 后面的两种时间格式。当天格式只精确到分钟，所以比现在早 5 分钟以内仍按当天算，更早的才按次日算，避免恰好在重置那一分钟里被推迟 24 小时。
  - `classify` 的判断顺序：非 Git 目录 → 文本显示额度耗尽 → 文本显示认证失败 → `rate_limited` 兜底 → 失败。结构化标记只在文本没有给出其他判断时才用，避免早先的 rejected 事件吞掉后来认证过期、网络或 5xx 等真实原因。重置时间依次取 `|时间戳`、`rate_limit_event`、Codex 的 `try again at`。
  - 以上两处（判断顺序、超额状态）是审查后补的，对应测试 `rate_limit_event_does_not_hide_other_failures`，并在 `codex_retry_time_handles_same_day_and_dated_formats` 里补了分钟边界的用例。
- **验证**：新增测试 `classify_quota_exhausted_enters_waiting`、`codex_retry_time_handles_same_day_and_dated_formats`、`rate_limit_event_does_not_hide_other_failures`。修复前，第一个因为 Claude 的输出被判成 `Failed` 而失败；修复后全部通过，后端全量 240 通过、10 个忽略。真机上只确认了修复后能正常编译启动、界面显示正常；额度耗尽这条路径无法人为触发，未在真机上测。
- **样例出处**：已写在测试注释里。提示文本、限额名和时间格式取自本机两个 CLI 可执行文件里的字符串，`rate_limit_event` 的字段取自官方文档；整行 JSON 外壳是按这些字段构造的。

### 2. Codex 在非 Git 目录执行被记为「运行失败」（已按裁决 B 修复）

- **证据**：鼠鼠已有的任务「测试」（Codex，工作目录 `C:\Users\20719\Desktop\develop\秦始皇`，不是 Git 仓库）在 2026-10-22 16:02 那次运行失败，报错 `Not inside a trusted directory and --skip-git-repo-check was not specified.`。
- **原因**：`build_args` 给 Codex 传的是 `exec --json [-m 模型] [resume 会话] -`。Codex 在没有 Git 仓库、也没有被信任过的目录里会拒绝执行（Context7 `/openai/codex` 的 `codex-rs/exec/src/cli.rs` 写明：不加 `--skip-git-repo-check` 又找不到 Git 仓库时，exec 报错退出）。`classify` 不认识这条报错，就把它归为普通失败，界面只显示英文原文，也看不出该怎么处理。
- **裁决**：选 B。保留 Codex 的 Git 目录保护，不加 `--skip-git-repo-check`。另一个方案 A（加这个参数）会失去「改动可以用 Git 撤销」这层保护。
- **修复**（`backend/src/automation.rs`）：
  - 新增 `is_untrusted_dir`，识别 `not inside a trusted directory` 或 `--skip-git-repo-check was not specified`。
  - `classify` 遇到这条报错时，先于额度和认证的判断，返回 `Outcome::UntrustedDir`。
  - `finish` 把运行转为已有的「需要处理」状态（`status = 'attention'`），沿用已有的 `block_kind = 'cwd_missing'`，原因写成「Codex 只能在 Git 仓库或已信任的目录中执行。请把工作目录改为 Git 仓库，或先在 Codex 中信任该目录。」，原始报错写进 `error`，时间线加一条「需要处理」。不写 `ended_at`，用户修好目录后可以「重新排队」。
  - 前端不用改：`cwd_missing` 走提示条的默认分支，显示「需要处理」、原因、「重新排队」和「取消任务」。
- **验证**：新增测试 `codex_untrusted_dir_needs_attention`，样例用的是上面这条真实报错的原文。修复前因为被判成 `Failed` 而失败，修复后通过，后端全量 239 通过、10 个忽略。按控制方要求没有再做真机运行。

## 已知限制

- `ve hit your` 也会匹配 Claude 的 `You've hit your monthly spend limit`（月度花费上限）。这种情况会被当成额度耗尽进入等待，提示里通常没有重置时间，就按递增间隔一直检查，可能等到下个月或无限期等下去。控制方已记为延后处理。
- `is_untrusted_dir` 不区分平台。这条报错目前只有 Codex 会出，所以没有改。

## 待办（真机部分）

- 真实耗尽额度后续跑：等某个工具真的用完额度时，确认运行进入「等待额度」、显示的重置时间和 CLI 提示一致，恢复后在原会话里继续（时间线出现「恢复原会话」）。
- 问题 2：之后找机会让鼠鼠的 Codex 任务在非 Git 目录里跑一次，确认界面显示「需要处理」和上面那条原因。这次报错发生在 Codex 启动阶段，预计不会产生模型请求，但未核实。

## 清理

- 测试任务 8 个（ID 2–9）、运行 8 条、事件 30 条，已在应用退出后按 ID 删除，删除前在同一事务里校验了名称前缀「验收-」。鼠鼠的任务 1 和它的运行没有动。用量记录 `requests` 删除前后都是 62,978 条，按要求保留。
- 自己启动的进程（两次 `pnpm tauri dev` 的进程树，根进程 PID 55728、28400）已结束；9337、5173 端口已释放。驱动脚本和中间结果在 `output/automation-e2e/`（Git 忽略）。
