/**
 * 自动化（Jolu.pen 模块 E，画板 58–73）的纯逻辑：状态与启动方式文案、时长格式、定时计算与表单校验。
 * Token 与时间缺失一律显示「—」，不以 0 代替（画板 64 底部说明）。
 */
import type {
  AutomationPlatform, LaunchMode, QuotaVerdictDto, QuotaWindowDto, RunDto, RunSource, RunStatus, ScheduleDto, SkillOptionDto, TaskItemDto,
} from "./automationApi.ts"

export const DASH = "—"

export function toolName(platform: string): string {
  return platform === "codex" ? "Codex" : platform === "claude" ? "Claude Code" : DASH
}

/** 运行记录里的短名：「Claude · Momo」 */
export function shortTool(platform: string): string {
  return platform === "codex" ? "Codex" : platform === "claude" ? "Claude" : DASH
}

/** 账号文案：「Momo · 订阅」/「Key 1 · API Key」 */
export function accountText(name: string | null | undefined, kind?: string | null): string {
  if (!name) return "未知账号"
  if (!kind) return name
  return `${name} · ${kind === "api" ? "API Key" : "订阅"}`
}

export function pad(n: number, width = 2): string {
  return String(n).padStart(width, "0")
}

export function taskCode(id: number): string {
  return `#AT-${pad(id, 3)}`
}

export function runCode(id: number): string {
  return `#RUN-${pad(id, 3)}`
}

function startOfDay(ts: number): number {
  const d = new Date(ts)
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()
}

export function hhmm(ts: number): string {
  const d = new Date(ts)
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`
}

export function hhmmss(ts: number | null | undefined): string {
  if (ts === null || ts === undefined) return DASH
  const d = new Date(ts)
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

/** 「今天 18:30」「明天 09:00」「昨天 14:32」「10/24 09:00」 */
export function dayTime(ts: number | null | undefined, now: number): string {
  if (ts === null || ts === undefined) return DASH
  const days = Math.round((startOfDay(ts) - startOfDay(now)) / 86_400_000)
  const prefix = days === 0 ? "今天" : days === 1 ? "明天" : days === -1 ? "昨天" : `${new Date(ts).getMonth() + 1}/${pad(new Date(ts).getDate())}`
  return `${prefix} ${hhmm(ts)}`
}

/** 表格里的计时：不足 1 小时「04:32」，否则「02:14:03」；0 显示「—」 */
export function clock(ms: number | null | undefined, zeroDash = true): string {
  if (ms === null || ms === undefined || (zeroDash && ms <= 0)) return DASH
  const total = Math.max(0, Math.floor(ms / 1000))
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = total % 60
  return h ? `${pad(h)}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`
}

/** 指标里的时长：「18 分 42 秒」「2 时 14 分」「42 秒」 */
export function durationText(ms: number | null | undefined): string {
  if (ms === null || ms === undefined) return DASH
  const total = Math.max(0, Math.round(ms / 1000))
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = total % 60
  if (h) return `${h} 时 ${m} 分`
  if (m) return `${m} 分 ${s} 秒`
  return `${s} 秒`
}

/** 「约 1 小时 42 分钟后」 */
export function untilText(ts: number, now: number): string {
  const minutes = Math.max(0, Math.round((ts - now) / 60_000))
  if (minutes < 1) return "即将到达"
  const h = Math.floor(minutes / 60)
  const m = minutes % 60
  return `约 ${h ? `${h} 小时 ` : ""}${m ? `${m} 分钟` : ""}后`.replace(" 后", "后")
}

export function runningMs(run: RunDto, now: number): number {
  return run.exec_ms + (run.status === "running" && run.exec_started_at ? Math.max(0, now - run.exec_started_at) : 0)
}

export function waitingMs(run: RunDto, now: number): number {
  return run.wait_ms + (run.wait_started_at ? Math.max(0, now - run.wait_started_at) : 0)
}

/** 本次运行消耗：输入（含缓存）+ 输出；两者都未知时为 null */
export function runTokens(run: Pick<RunDto, "input_tokens" | "output_tokens">): number | null {
  if (run.input_tokens === null && run.output_tokens === null) return null
  return (run.input_tokens ?? 0) + (run.output_tokens ?? 0)
}

/* ─────────────────────────── 启动方式 ─────────────────────────── */

export const WEEKDAYS = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]

export function scheduleLabel(s: ScheduleDto): string {
  if (s.freq === "daily") return `每天 ${s.time}`
  if (s.freq === "weekly") return `每${WEEKDAYS[(s.weekday ?? 1) - 1] ?? "周"} ${s.time}`
  if (s.date) {
    const [, m, d] = s.date.split("-")
    return `${Number(m)}/${d} ${s.time}`
  }
  return s.time
}

export function launchLabel(task: { launch: LaunchMode; schedule: ScheduleDto | null }): string {
  if (task.launch === "now") return "立即执行"
  if (task.launch === "quota") return "额度恢复后"
  return task.schedule ? scheduleLabel(task.schedule) : "定时执行"
}

export function sourceLabel(source: RunSource): string {
  return { now: "立即执行", schedule: "定时执行", quota: "额度恢复后", manual: "手动执行" }[source] ?? source
}

/** 浏览器本地时区下的下一次触发（与后端 `next_fire` 同一口径） */
export function nextFire(s: ScheduleDto, after: number): number | null {
  const match = /^(\d{2}):(\d{2})$/.exec(s.time)
  if (!match) return null
  const [h, m] = [Number(match[1]), Number(match[2])]
  const at = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate(), h, m).getTime()
  if (s.freq === "once") {
    if (!s.date) return null
    const [y, mo, d] = s.date.split("-").map(Number)
    const ts = new Date(y, mo - 1, d, h, m).getTime()
    return ts > after ? ts : null
  }
  const base = new Date(after)
  for (let i = 0; i <= 8; i++) {
    const day = new Date(base.getFullYear(), base.getMonth(), base.getDate() + i)
    const weekday = ((day.getDay() + 6) % 7) + 1
    if (s.freq === "weekly" && weekday !== s.weekday) continue
    const ts = at(day)
    if (ts > after) return ts
  }
  return null
}

/* ─────────────────────────── 状态 ─────────────────────────── */

export type Tone = "on" | "text" | "danger" | "muted"
export type TaskFilter = "all" | "running" | "waiting" | "paused" | "done"

export interface TaskStatus {
  label: string
  next: string
  tone: Tone
  filter: Exclude<TaskFilter, "all"> | null
  /** 行内主按钮 */
  action: "查看运行" | "查看等待" | "查看队列" | "查看任务"
}

function firstClause(text: string | null | undefined, fallback: string): string {
  if (!text) return fallback
  const clause = text.split(/[；;。]/)[0]?.trim()
  return clause || fallback
}

export function waitNext(run: RunDto, now: number): string {
  if (run.block_kind === "quota_unknown") return run.resets_at ? `预计${dayTime(run.resets_at, now)} 尝试继续` : "恢复时间未知"
  if (run.block_kind === "check_failed") return "检查失败 · 稍后重试"
  if (run.resets_at) {
    const weekly = run.reason?.includes("周") || run.reason?.includes("7 天")
    return weekly ? `周额度 · ${dayTime(run.resets_at, now).replace("今天 ", "")}` : `预计${dayTime(run.resets_at, now)}`
  }
  return "等待检查额度"
}

export function taskStatus(item: TaskItemDto, now: number, zone = ""): TaskStatus {
  const run = item.active
  if (run?.status === "running") return { label: "执行中", next: `正在执行 · ${clock(runningMs(run, now), false)}`, tone: "on", filter: "running", action: "查看运行" }
  if (item.state === "paused") return { label: "已暂停", next: "暂停期间不自动运行", tone: "text", filter: "paused", action: "查看任务" }
  if (run?.status === "waiting") return { label: "等待额度", next: waitNext(run, now), tone: "on", filter: "waiting", action: "查看等待" }
  if (run?.status === "queued") {
    const ahead = item.queue_ahead ?? 0
    return { label: "排队中", next: ahead ? `同账号前方 ${ahead} 个任务` : "即将开始", tone: "text", filter: null, action: "查看队列" }
  }
  if (run?.status === "attention") return { label: "需要处理", next: firstClause(run.reason, "查看详情后处理"), tone: "danger", filter: null, action: "查看任务" }
  if (item.state === "cancelled") return { label: "已取消", next: "已停止调度", tone: "muted", filter: null, action: "查看任务" }
  if (item.missed_at) return { label: "已错过", next: `${dayTime(item.missed_at, now)} · 可补跑`, tone: "danger", filter: null, action: "查看任务" }
  if (item.launch === "schedule" && item.state === "active" && item.next_run_at) {
    return { label: "已启用", next: `${dayTime(item.next_run_at, now)}${zone ? ` · ${zone}` : ""}`, tone: "text", filter: null, action: "查看任务" }
  }
  const last = item.last
  if (last?.status === "failed") return { label: "失败", next: firstClause(last.error, "查看运行记录"), tone: "danger", filter: null, action: "查看任务" }
  if (last?.status === "cancelled") return { label: "已取消", next: last.ended_at ? `${dayTime(last.ended_at, now)} 停止` : "已停止运行", tone: "muted", filter: null, action: "查看任务" }
  if (item.state === "finished" || last?.status === "completed") {
    return { label: "已完成", next: last?.ended_at ? `${dayTime(last.ended_at, now)} 完成` : "已完成", tone: "on", filter: "done", action: "查看任务" }
  }
  return { label: "已启用", next: "尚未开始", tone: "text", filter: null, action: "查看任务" }
}

/** 删除任务（画板 68 第 12 格）：执行中的任务需先停止；传入进行中运行的状态，没有运行时传任务状态 */
export function canDeleteTask(status: string): boolean {
  return status !== "running"
}

export const DELETE_BLOCKED_TIP = "运行中的任务需先停止"

export function runStatusLabel(status: RunStatus): { label: string; tone: Tone } {
  switch (status) {
    case "queued": return { label: "排队中", tone: "text" }
    case "waiting": return { label: "等待额度", tone: "on" }
    case "running": return { label: "执行中", tone: "on" }
    case "attention": return { label: "需要处理", tone: "danger" }
    case "completed": return { label: "已完成", tone: "on" }
    case "failed": return { label: "失败", tone: "danger" }
    case "cancelled": return { label: "已取消", tone: "muted" }
  }
}

/** 任务筛选：状态页签 + 账号 + 名称 / 项目搜索 */
export function filterTasks(items: readonly TaskItemDto[], filter: TaskFilter, account: string | null, search: string, now: number): TaskItemDto[] {
  const q = search.trim().toLowerCase()
  return items.filter((item) => {
    if (filter !== "all" && taskStatus(item, now).filter !== filter) return false
    if (account && item.connection_id !== account) return false
    if (q && !`${item.name} ${item.cwd} ${item.prompt}`.toLowerCase().includes(q)) return false
    return true
  })
}

export function countByFilter(items: readonly TaskItemDto[], now: number): Record<TaskFilter, number> {
  const out: Record<TaskFilter, number> = { all: items.length, running: 0, waiting: 0, paused: 0, done: 0 }
  for (const item of items) {
    const f = taskStatus(item, now).filter
    if (f) out[f] += 1
  }
  return out
}

/** 等待 / 处理状态的提示条（画板 62 顶部、画板 68 第 04–06 格） */
export function blockBanner(run: RunDto, now: number): { title: string; body: string; danger: boolean } {
  const reason = run.reason ?? ""
  switch (run.block_kind) {
    case "quota":
      if (!run.resume_pending) {
        return { title: "等待额度恢复后启动", body: `${reason || "额度暂不可用"}${run.resets_at ? `；预计${dayTime(run.resets_at, now)} 恢复` : ""}。届时检查实际可用额度后启动。`, danger: false }
      }
      return {
        title: "等待额度 · 已保留任务与进度",
        body: `额度暂不可用${run.resets_at ? `；预计${dayTime(run.resets_at, now)} 恢复` : ""}。恢复时间到达后，会再次检查 5 小时与周额度。`,
        danger: false,
      }
    case "quota_unknown":
      return { title: "恢复时间未知", body: `${reason || "暂未获取重置时间"}，任务保持等待。`, danger: false }
    case "check_failed":
      return { title: reason || "检查失败", body: "保留上次额度并标记过期；逐步延长重试间隔。检查失败不当作额度耗尽。", danger: false }
    case "reauth":
      return { title: "需要重新登录", body: "停止额度检查，保留任务；重新认证并检查账号后才能继续。", danger: true }
    case "resume_unsupported":
    case "interrupted":
      return { title: "无法自动续跑 · 需要手动继续", body: reason || "原会话失效、工具未安装或需要交互确认时保留进度，不自动从头重跑。", danger: true }
    case "timeout":
      return { title: "等待超时", body: reason, danger: true }
    default:
      return { title: run.status === "attention" ? "需要处理" : "等待额度", body: reason || "查看详情后处理", danger: run.status === "attention" }
  }
}

/* ─────────────────────────── 额度 ─────────────────────────── */

export function windowLabel(w: Pick<QuotaWindowDto, "key" | "name">): string {
  if (w.key === "5h") return "5 小时额度"
  if (w.key === "7d") return "周额度"
  if (w.key.startsWith("7d-")) return `周额度 · ${w.name.split("·").pop()?.trim() ?? w.key.slice(3)}`
  return w.name
}

/** 新建任务里「额度恢复后执行」的提示（画板 60） */
export function quotaTip(v: QuotaVerdictDto, account: string, now: number): { title: string; body: string; disabled: boolean } {
  const others = v.windows.filter((w) => !w.blocking && w.used_percent !== null)
    .map((w) => `${windowLabel(w)}剩余 ${Math.max(0, Math.round(100 - (w.used_percent ?? 0)))}%`)
  switch (v.state) {
    case "blocked": {
      const names = v.windows.filter((w) => w.blocking).map(windowLabel)
      const when = v.resets_at ? `预计${dayTime(v.resets_at, now)} 恢复` : "恢复时间未知"
      return {
        title: `等待 ${names.join("、") || "额度"}恢复`,
        body: `${account} 的 ${names.join("、")}已用完${others.length ? `，${others.join("，")}` : ""}。\n${when}；届时检查实际可用额度后启动。`,
        disabled: false,
      }
    }
    case "available":
      return { title: "额度当前可用", body: `${account}${others.length ? ` · ${others.join("，")}` : ""}。创建后检查额度，可用时立即启动。`, disabled: false }
    case "unknown":
      return { title: "暂时无法读取额度", body: `${v.reason ?? "检查失败"}。检查失败不当作额度耗尽，创建后会间隔重试。`, disabled: false }
    case "reauth":
      return { title: "需要重新登录", body: `${account} 的登录已失效，请重新认证后再选择此启动方式。`, disabled: true }
    case "unsupported":
      return { title: "不支持读取额度", body: `${v.reason ?? "该账号不提供订阅额度"}。可改用立即执行或定时执行。`, disabled: true }
  }
}

/* ─────────────────────────── 表单 ─────────────────────────── */

export interface TaskForm {
  name: string
  platform: AutomationPlatform
  connectionId: string
  /** 空字符串表示跟随工具默认模型 */
  model: string
  cwd: string
  sessionId: string | null
  prompt: string
  launch: LaunchMode
  freq: ScheduleDto["freq"]
  time: string
  weekday: number
  date: string
  timezone: string
  autoResume: boolean
  /** datetime-local 字符串；空为不限制 */
  deadline: string
  /** 同账号排队（D-09） */
  serialQueue: boolean
  /** 选中的 Skills 名称（D-08），最多 10 个 */
  skills: string[]
}

export type FormErrors = Partial<Record<"name" | "connectionId" | "cwd" | "prompt" | "time" | "deadline" | "launch", string>>

export function validateForm(f: TaskForm, now: number, quotaDisabled: string | null = null): FormErrors {
  const e: FormErrors = {}
  if (!f.name.trim()) e.name = "请填写任务名称"
  else if ([...f.name.trim()].length > 80) e.name = "任务名称不能超过 80 个字符"
  if (!f.connectionId) e.connectionId = "请选择账号后再创建任务。"
  if (!f.cwd.trim()) e.cwd = "请选择项目目录"
  if (!f.prompt.trim()) e.prompt = "请填写执行内容"
  if (f.launch === "schedule") {
    if (!/^\d{2}:\d{2}$/.test(f.time)) e.time = "请选择执行时间"
    else if (nextFire(toSchedule(f), now) === null) e.time = f.freq === "once" ? "单次执行时间已经过去" : "请选择执行时间"
  }
  if (f.launch === "quota" && quotaDisabled) e.launch = quotaDisabled
  if (f.deadline) {
    const ts = new Date(f.deadline).getTime()
    if (!Number.isFinite(ts) || ts <= now) e.deadline = "等待截止时间应晚于现在"
  }
  return e
}

export function toSchedule(f: TaskForm): ScheduleDto {
  return { freq: f.freq, time: f.time, weekday: f.freq === "weekly" ? f.weekday : null, date: f.freq === "once" ? f.date : null, timezone: f.timezone }
}

/** 来自会话的任务名（画板 72）：「继续开发：<会话标题>」 */
export function sessionTaskName(title: string | null | undefined): string {
  const t = (title ?? "").trim()
  return t ? `继续开发：${[...t].slice(0, 70).join("")}` : "继续开发"
}

export function shortId(id: string): string {
  return id.length > 22 ? `${id.slice(0, 12)}…${id.slice(-4)}` : id
}

export function projectOf(cwd: string | null | undefined): string {
  if (!cwd) return "未识别项目"
  const parts = cwd.replace(/[\\/]+$/, "").split(/[\\/]/)
  return parts[parts.length - 1] || cwd
}

/** 运行记录的时间范围 */
export const RUN_RANGES = [
  { value: "today", label: "今天" },
  { value: "7d", label: "近 7 天" },
  { value: "30d", label: "近 30 天" },
  { value: "all", label: "全部时间" },
] as const

export const RUN_STATUSES: { value: RunStatus; label: string }[] = [
  { value: "running", label: "执行中" },
  { value: "waiting", label: "等待额度" },
  { value: "queued", label: "排队中" },
  { value: "attention", label: "需要处理" },
  { value: "completed", label: "已完成" },
  { value: "failed", label: "失败" },
  { value: "cancelled", label: "已取消" },
]

/* ─────────────────────────── Skills（D-08） ─────────────────────────── */

/** 一个任务最多选择的 Skills 数，与后端 `MAX_SKILLS` 一致 */
export const MAX_SKILLS = 10

/** 「Skills（可选）」字段的摘要（画板 60）：「已选择 2 项 · 项目约定、测试检查」，超过 2 项写「a、b 等」 */
export function skillSummary(names: string[]): string {
  if (!names.length) return ""
  return `已选择 ${names.length} 项 · ${names.slice(0, 2).join("、")}${names.length > 2 ? " 等" : ""}`
}

/** 切换工具或工作目录后，只保留仍在候选里的选择，顺序不变 */
export function usableSkills(selected: readonly string[], options: readonly SkillOptionDto[]): string[] {
  return selected.filter((name) => options.some((o) => o.name === name))
}
