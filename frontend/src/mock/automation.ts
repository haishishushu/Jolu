/**
 * 仅供浏览器预览的自动化示例数据（对应 Jolu.pen 画板 58–67 的示例）。
 * 桌面端读取失败时不得回落到这里。
 */
import type {
  AutomationAccountDto, AutomationOverviewDto, AutomationPlatform, QuotaVerdictDto, RunDetailDto, RunDto, RunPageDto, SkillOptionDto, TaskDetailDto, TaskItemDto,
} from "@/lib/automationApi"

const MIN = 60_000

function base(now: number) {
  const d = new Date(now)
  return (h: number, m: number, s = 0, dayOffset = 0) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + dayOffset, h, m, s).getTime()
}

function run(patch: Partial<RunDto> & Pick<RunDto, "id" | "task_id" | "task_name" | "platform" | "connection_id" | "status" | "created_at">): RunDto {
  return {
    source: "now", block_kind: null, reason: null, resets_at: null, last_check_at: null, next_check_at: null, check_failures: 0, resume_pending: false,
    session_id: null, account: null, model: null, started_at: null, ended_at: null, exec_ms: 0, wait_ms: 0, exec_started_at: null, wait_started_at: null,
    waits: 0, input_tokens: null, cache_read_tokens: null, output_tokens: null, result: null, files: [], error: null, ...patch,
  }
}

function tasks(now: number): TaskItemDto[] {
  const t = base(now)
  const common = { model: null, session_id: null, skills: [], schedule: null, auto_resume: true, wait_deadline: null, state: "active" as const, next_run_at: null, missed_at: null, last: null, queue_ahead: null, serial_queue: true }
  return [
    {
      ...common, id: 24, name: "完善登录页面", platform: "claude", connection_id: "demo-momo", cwd: "D:/develop/jolu", model: "claude-sonnet-4-5",
      session_id: "session_demo_82ac4b1e91f0", prompt: "完善登录页面的表单校验和错误提示，沿用当前绿色主题。完成后运行相关测试，并汇总改动与验证结果。",
      skills: ["项目约定", "测试检查"], launch: "quota", created_at: t(16, 2), updated_at: t(16, 48),
      active: run({
        id: 93, task_id: 24, task_name: "完善登录页面", platform: "claude", connection_id: "demo-momo", status: "waiting", source: "quota",
        block_kind: "quota", reason: "5 小时额度已用完", resets_at: t(18, 30), last_check_at: t(16, 48), next_check_at: t(18, 30), resume_pending: true,
        session_id: "session_demo_82ac4b1e91f0", account: "Momo", created_at: t(16, 2), started_at: t(16, 2, 8), exec_ms: 14 * MIN, wait_started_at: t(16, 16, 8), waits: 1,
      }),
    },
    {
      ...common, id: 25, name: "补充接口测试", platform: "codex", connection_id: "demo-alex", cwd: "D:/develop/api-proxy", prompt: "为代理接口补充单元测试。",
      launch: "now", created_at: t(16, 30), updated_at: t(16, 44),
      active: run({ id: 92, task_id: 25, task_name: "补充接口测试", platform: "codex", connection_id: "demo-alex", status: "running", account: "Alex", created_at: now - 5 * MIN, started_at: now - 272_000, exec_started_at: now - 272_000 }),
    },
    {
      ...common, id: 26, name: "整理项目文档", platform: "claude", connection_id: "demo-momo", cwd: "D:/develop/jolu-docs", prompt: "整理项目文档目录。",
      launch: "quota", created_at: t(16, 20), updated_at: t(16, 40), queue_ahead: 1,
      active: run({ id: 94, task_id: 26, task_name: "整理项目文档", platform: "claude", connection_id: "demo-momo", status: "queued", source: "quota", account: "Momo", created_at: t(16, 20) }),
    },
    {
      ...common, id: 27, name: "生成项目日报", platform: "codex", connection_id: "demo-alex", cwd: "D:/develop/jolu", prompt: "汇总今天的提交并生成日报。",
      launch: "schedule", schedule: { freq: "daily", time: "09:00", weekday: null, date: null, timezone: "Asia/Shanghai" }, next_run_at: t(9, 0, 0, 1),
      created_at: t(9, 0, 0, -7), updated_at: t(9, 3), active: null,
    },
    {
      ...common, id: 28, name: "检查缓存统计", platform: "claude", connection_id: "demo-team", cwd: "D:/develop/jolu", prompt: "核对缓存命中统计。",
      launch: "now", created_at: t(8, 42), updated_at: t(8, 43),
      active: run({ id: 95, task_id: 28, task_name: "检查缓存统计", platform: "claude", connection_id: "demo-team", status: "waiting", block_kind: "quota", reason: "周额度已用完", resets_at: t(9, 0, 0, 2), resume_pending: false, account: "Team", created_at: t(8, 42), wait_started_at: t(8, 42) }),
    },
    {
      ...common, id: 29, name: "整理发布说明", platform: "codex", connection_id: "demo-alex", cwd: "D:/develop/jolu-docs", prompt: "整理本周发布说明。",
      launch: "now", state: "paused", created_at: t(8, 0), updated_at: t(8, 20), active: null,
    },
  ]
}

export function demoOverview(now: number): AutomationOverviewDto {
  const t = base(now)
  return { tasks: tasks(now), enabled: 5, running: 1, running_note: "Codex · Alex", waiting: 2, next_reset: t(18, 30), done_today: 8, failed_today: 1 }
}

export function demoTaskDetail(id: number, now: number): TaskDetailDto {
  const t = base(now)
  const item = tasks(now).find((x) => x.id === id) ?? tasks(now)[0]
  return {
    item,
    queue: [{ task_id: 24, name: "完善登录页面", status: "waiting" }, { task_id: 26, name: "整理项目文档", status: "queued" }].filter((q) => q.task_id === item.id || item.connection_id === "demo-momo"),
    events: [
      { ts: t(16, 2, 8), kind: "start", title: "开始执行", detail: "在所选会话中追加任务指令" },
      { ts: t(16, 16, 8), kind: "pause", title: "额度耗尽，保留进度", detail: "检查表单校验与错误提示" },
      { ts: t(16, 48), kind: "check", title: "检查额度", detail: "5 小时额度已用完" },
    ],
  }
}

function runRows(now: number): RunDto[] {
  const t = base(now)
  return [
    run({ id: 92, task_id: 25, task_name: "补充接口测试", platform: "codex", connection_id: "demo-alex", status: "running", account: "Alex", created_at: t(18, 42, 16), started_at: t(18, 42, 16), exec_started_at: now - 272_000, input_tokens: 14_200, output_tokens: 4_400 }),
    run({
      id: 91, task_id: 24, task_name: "完善登录页面", platform: "claude", connection_id: "demo-momo", status: "completed", source: "quota", account: "Momo", model: "claude-sonnet-4-5",
      session_id: "session_demo_82ac4b1e91f0", created_at: t(16, 2, 8), started_at: t(16, 2, 8), ended_at: t(18, 34, 53), exec_ms: 1_122_000, wait_ms: 8_043_000, waits: 1,
      input_tokens: 46_200, cache_read_tokens: 38_000, output_tokens: 8_600,
      result: "已完善登录表单校验，并统一错误提示样式。额度恢复后延续原会话，保留此前完成的修改。\n- 补充邮箱、密码必填与格式校验\n- 统一输入框错误态及绿色聚焦样式\n- 完成相关测试并汇总验证结果",
      files: ["src/LoginForm.tsx", "src/LoginForm.test.tsx"],
    }),
    run({ id: 90, task_id: 26, task_name: "整理项目文档", platform: "claude", connection_id: "demo-momo", status: "completed", source: "quota", account: "Momo", created_at: t(18, 37, 28), started_at: t(18, 37, 28), ended_at: t(18, 42, 44), exec_ms: 316_000, input_tokens: 18_000, output_tokens: 4_400 }),
    run({ id: 89, task_id: 27, task_name: "生成项目日报", platform: "codex", connection_id: "demo-alex", status: "completed", source: "schedule", account: "Alex", created_at: t(9, 0), started_at: t(9, 0), ended_at: t(9, 3, 12), exec_ms: 192_000, input_tokens: 10_000, output_tokens: 2_200 }),
    run({ id: 88, task_id: 28, task_name: "检查缓存统计", platform: "claude", connection_id: "demo-team", status: "failed", account: "Team", created_at: t(8, 42, 6), started_at: t(8, 42, 6), ended_at: t(8, 42, 14), exec_ms: 8_000, error: "登录失效，需要处理" }),
    run({ id: 87, task_id: 30, task_name: "整理测试清单", platform: "codex", connection_id: "demo-alex", status: "cancelled", source: "manual", account: "Alex", created_at: t(8, 20), started_at: t(8, 20), ended_at: t(8, 22, 46), exec_ms: 166_000, input_tokens: 8_000, output_tokens: 1_600 }),
  ]
}

export function demoRuns(page: number, now: number): RunPageDto {
  return {
    rows: runRows(now), total: 11, page, pages: 2, completed: 8, completed_waited: 2, running: 1, failed: 1, failed_note: "登录失效，需要处理", tokens: 428_600,
    tasks: tasks(now).map((t) => ({ task_id: t.id, name: t.name, status: t.state })),
  }
}

export function demoRunDetail(id: number, now: number): RunDetailDto {
  const t = base(now)
  const r = runRows(now).find((x) => x.id === id) ?? runRows(now)[1]
  const task = tasks(now).find((x) => x.id === r.task_id) ?? null
  return {
    run: r,
    task,
    events: [
      { ts: t(16, 2, 8), kind: "start", title: "开始执行", detail: "读取项目上下文与登录组件" },
      { ts: t(16, 16, 8), kind: "pause", title: "额度耗尽，保留进度", detail: "等待 5 小时额度恢复" },
      { ts: t(18, 30, 11), kind: "check", title: "检查额度恢复", detail: "5 小时与周额度均可用" },
      { ts: t(18, 30, 15), kind: "start", title: "恢复原会话", detail: "继续未完成的校验与测试" },
      { ts: t(18, 34, 53), kind: "done", title: "完成任务", detail: "结果已关联到原会话" },
    ],
    logs: [
      { ts: t(18, 30, 11), kind: "log", title: "额度检查通过，恢复原会话", detail: null },
      { ts: t(18, 30, 15), kind: "log", title: "继续检查表单错误提示", detail: null },
      { ts: t(18, 32, 6), kind: "log", title: "执行相关测试", detail: null },
      { ts: t(18, 34, 53), kind: "log", title: "返回结果，任务完成", detail: null },
    ],
  }
}

export function demoAccounts(platform: AutomationPlatform): AutomationAccountDto[] {
  return platform === "claude"
    ? [{ id: "demo-momo", name: "Momo", kind: "auth", status: "active", current: true }, { id: "demo-team", name: "Team", kind: "auth", status: "active", current: false }]
    : [{ id: "demo-alex", name: "Alex", kind: "auth", status: "active", current: true }]
}

export function demoQuota(now: number): QuotaVerdictDto {
  const t = base(now)
  return {
    state: "blocked", reason: "5 小时窗口已用完", resets_at: t(18, 30),
    windows: [
      { key: "5h", name: "5 小时窗口", used_percent: 100, resets_at: t(18, 30), blocking: true },
      { key: "7d", name: "7 天窗口", used_percent: 58, resets_at: t(9, 0, 0, 2), blocking: false },
    ],
  }
}

export const DEMO_MODELS: Record<AutomationPlatform, string[]> = {
  claude: ["claude-sonnet-4-5", "claude-opus-4-1"],
  codex: ["gpt-5-codex", "gpt-5"],
}

/** 「Skills（可选）」候选示例：名称沿用画板 60 的「项目约定」「测试检查」；填了工作目录才带出项目级 */
export function demoSkills(platform: AutomationPlatform, cwd: string | null): SkillOptionDto[] {
  const global: SkillOptionDto[] = platform === "claude"
    ? [{ name: "测试检查", description: "修改后运行相关测试并汇总结果", scope: "global", project: null }, { name: "api-review", description: "核对接口契约与错误码", scope: "global", project: null }]
    : [{ name: "测试检查", description: "修改后运行相关测试并汇总结果", scope: "global", project: null }]
  return cwd ? [...global, { name: "项目约定", description: "遵守仓库的目录、命名与提交约定", scope: "project", project: cwd }] : global
}
