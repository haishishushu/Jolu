/**
 * 自动化（模块 E）的后端桥接，对应 backend/src/automation.rs。
 * Token 缺失为 null，不以 0 代替；时间均为毫秒时间戳。
 */
import { invoke } from "./api"

export type AutomationPlatform = "claude" | "codex"
export type LaunchMode = "now" | "schedule" | "quota"
export type ScheduleFreq = "once" | "daily" | "weekly"
export type TaskState = "active" | "paused" | "finished" | "cancelled"
export type RunStatus = "queued" | "waiting" | "running" | "attention" | "completed" | "failed" | "cancelled"
export type RunSource = "now" | "schedule" | "quota" | "manual"
export type TaskAction = "pause" | "resume" | "cancel" | "stop" | "run" | "retry" | "check"

export interface ScheduleDto {
  freq: ScheduleFreq
  /** HH:MM，按本机时区 */
  time: string
  /** 1 = 周一 … 7 = 周日 */
  weekday: number | null
  /** YYYY-MM-DD，仅单次 */
  date: string | null
  timezone: string
}

export interface TaskInputDto {
  name: string
  platform: AutomationPlatform
  connection_id: string
  model: string | null
  cwd: string
  /** 为空表示新建会话 */
  session_id: string | null
  prompt: string
  skills: string[]
  launch: LaunchMode
  schedule: ScheduleDto | null
  auto_resume: boolean
  wait_deadline: number | null
  /** 同账号排队（D-09），默认开启；关闭后可与同账号任务并行 */
  serial_queue: boolean
}

export interface TaskDto {
  id: number
  name: string
  platform: AutomationPlatform
  connection_id: string
  model: string | null
  cwd: string
  session_id: string | null
  prompt: string
  skills: string[]
  launch: LaunchMode
  schedule: ScheduleDto | null
  auto_resume: boolean
  wait_deadline: number | null
  state: TaskState
  next_run_at: number | null
  missed_at: number | null
  created_at: number
  updated_at: number
  serial_queue: boolean
}

export interface RunDto {
  id: number
  task_id: number
  task_name: string
  platform: AutomationPlatform
  connection_id: string
  source: RunSource
  status: RunStatus
  /** quota / quota_unknown / check_failed / reauth / unsupported / timeout / account_mismatch / tool_missing / cwd_missing / resume_unsupported / interrupted / skill_missing */
  block_kind: string | null
  reason: string | null
  resets_at: number | null
  last_check_at: number | null
  next_check_at: number | null
  check_failures: number
  resume_pending: boolean
  session_id: string | null
  account: string | null
  model: string | null
  created_at: number
  started_at: number | null
  ended_at: number | null
  exec_ms: number
  wait_ms: number
  exec_started_at: number | null
  wait_started_at: number | null
  waits: number
  input_tokens: number | null
  cache_read_tokens: number | null
  output_tokens: number | null
  result: string | null
  files: string[]
  error: string | null
}

export interface TaskItemDto extends TaskDto {
  active: RunDto | null
  last: RunDto | null
  queue_ahead: number | null
}

export interface AutomationOverviewDto {
  tasks: TaskItemDto[]
  enabled: number
  running: number
  running_note: string | null
  waiting: number
  next_reset: number | null
  done_today: number
  failed_today: number
}

export interface QueueItemDto {
  task_id: number
  name: string
  status: string
}

export interface AutomationEventDto {
  ts: number
  kind: string
  title: string
  detail: string | null
}

export interface TaskDetailDto {
  item: TaskItemDto
  queue: QueueItemDto[]
  events: AutomationEventDto[]
}

export type RunRange = "today" | "7d" | "30d" | "all"

export interface RunFilterDto {
  range: RunRange
  status: RunStatus | null
  connection: string | null
  task: number | null
}

export interface RunPageDto {
  rows: RunDto[]
  total: number
  page: number
  pages: number
  completed: number
  completed_waited: number
  running: number
  failed: number
  failed_note: string | null
  tokens: number | null
  tasks: QueueItemDto[]
}

export interface RunDetailDto {
  run: RunDto
  task: TaskDto | null
  events: AutomationEventDto[]
  logs: AutomationEventDto[]
}

export interface AutomationAccountDto {
  id: string
  name: string
  kind: string
  status: string
  /** 该工具当前登录的连接；只有它能执行 */
  current: boolean
}

export interface QuotaWindowDto {
  key: string
  name: string
  used_percent: number | null
  resets_at: number | null
  blocking: boolean
}

export interface QuotaVerdictDto {
  state: "available" | "blocked" | "unknown" | "reauth" | "unsupported"
  reason: string | null
  resets_at: number | null
  windows: QuotaWindowDto[]
}

export interface ToolInfoDto {
  platform: AutomationPlatform
  found: boolean
}

/** 「Skills（可选）」的候选项：目标工具已启用的全局 Skills + 工作目录下的项目 Skills（D-08） */
export interface SkillOptionDto {
  name: string
  description: string | null
  /** global / project / system / synced */
  scope: string
  /** 项目级 Skill 所在的项目目录 */
  project: string | null
}

/** 后端在任务或运行状态变化后发出 */
export const AUTOMATION_CHANGED = "automation-changed"

export const automationApi = {
  overview: () => invoke<AutomationOverviewDto>("automation_overview"),
  task: (id: number) => invoke<TaskDetailDto>("automation_task", { id }),
  create: (input: TaskInputDto) => invoke<number>("automation_create", { input }),
  update: (id: number, input: TaskInputDto) => invoke<void>("automation_update", { id, input }),
  action: (id: number, action: TaskAction) => invoke<void>("automation_action", { id, action }),
  runs: (filter: RunFilterDto, page: number) => invoke<RunPageDto>("automation_runs", { filter, page }),
  run: (id: number) => invoke<RunDetailDto>("automation_run", { id }),
  accounts: (platform: AutomationPlatform) => invoke<AutomationAccountDto[]>("automation_accounts", { platform }),
  models: (platform: AutomationPlatform, connectionId: string | null) => invoke<string[]>("automation_models", { platform, connectionId }),
  quota: (connectionId: string, force: boolean) => invoke<QuotaVerdictDto>("automation_quota", { connectionId, force }),
  tools: () => invoke<ToolInfoDto[]>("automation_tools"),
  pickDir: () => invoke<string | null>("automation_pick_dir"),
  /** 删除任务及其运行记录与事件，用量统计保留；执行中的任务后端拒绝（任务 2.9） */
  deleteTask: (taskId: number) => invoke<void>("automation_delete", { taskId }),
  /** 目标工具已启用的 Skills；`cwd` 为空时只列全局（任务 3.3） */
  skills: (platform: AutomationPlatform, cwd: string | null) => invoke<SkillOptionDto[]>("automation_skills", { platform, cwd }),
}
