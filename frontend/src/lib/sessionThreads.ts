/**
 * 会话线程页（Jolu.pen 模块 D，画板 40–57）的纯逻辑：项目分组、时间文案、状态与顶部统计。
 */
import type { SessionItemDto, SessionStatus } from "./api.ts"

export const UNKNOWN_PROJECT = "未识别项目"

/** 项目名取目录最后一段：D:/develop/jolu → jolu */
export function projectName(cwd: string | null): string {
  if (!cwd) return UNKNOWN_PROJECT
  const parts = cwd.split(/[\\/]+/).filter(Boolean)
  return parts[parts.length - 1] ?? cwd
}

/**
 * 项目分组键：Windows 路径不区分大小写，`\` 与 `/` 视为同一分隔符，去掉末尾分隔符；
 * 未识别项目为 ""。同一目录的 `d:\x` 与 `D:/x` 归入同一项目。
 */
export function projectKey(cwd: string | null): string {
  if (!cwd) return ""
  return cwd.replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase()
}

export interface ProjectGroup {
  /** 规范化后的项目键（[`projectKey`]）；未识别项目为 "" */
  key: string
  /** 组内第一条会话的目录原值，用于悬浮提示；未识别项目为 null */
  path: string | null
  name: string
  sessions: SessionItemDto[]
  lastTs: number
}

/** 按项目分组：组内按最近更新排序，组按最近会话排序，「未识别项目」放最后 */
export function groupByProject(sessions: readonly SessionItemDto[], sort: SessionSort = "recent"): ProjectGroup[] {
  const map = new Map<string, ProjectGroup>()
  for (const s of sessions) {
    const key = projectKey(s.cwd)
    const group = map.get(key) ?? { key, path: s.cwd, name: projectName(s.cwd), sessions: [], lastTs: 0 }
    group.sessions.push(s)
    group.lastTs = Math.max(group.lastTs, s.last_ts)
    map.set(key, group)
  }
  const groups = [...map.values()]
  for (const g of groups) g.sessions.sort(sorter(sort))
  return groups.sort((a, b) => {
    if (!a.key !== !b.key) return a.key ? -1 : 1
    if (sort === "tokens") return groupTokens(b) - groupTokens(a)
    return b.lastTs - a.lastTs
  })
}

export type SessionSort = "recent" | "tokens"

export function consumption(s: Pick<SessionItemDto, "fresh_input" | "cache_read" | "cache_write" | "output">): number {
  return (s.fresh_input ?? 0) + (s.cache_read ?? 0) + (s.cache_write ?? 0) + (s.output ?? 0)
}

function groupTokens(g: ProjectGroup): number {
  return g.sessions.reduce((sum, s) => sum + consumption(s), 0)
}

function sorter(sort: SessionSort) {
  return sort === "tokens"
    ? (a: SessionItemDto, b: SessionItemDto) => consumption(b) - consumption(a)
    : (a: SessionItemDto, b: SessionItemDto) => b.last_ts - a.last_ts
}

function sameDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate()
}

const pad = (n: number) => String(n).padStart(2, "0")
const hm = (d: Date) => `${pad(d.getHours())}:${pad(d.getMinutes())}`
const md = (d: Date) => `${pad(d.getMonth() + 1)}/${pad(d.getDate())}`

function dayWord(d: Date, now: Date): string {
  if (sameDay(d, now)) return "今天"
  const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1)
  if (sameDay(d, yesterday)) return "昨天"
  return d.getFullYear() === now.getFullYear() ? md(d) : `${d.getFullYear()}/${md(d)}`
}

/** 列表右侧时间：今天写时刻，昨天写「昨天」，更早写日期 */
export function listTime(ts: number, now: Date = new Date()): string {
  const d = new Date(ts)
  const word = dayWord(d, now)
  return word === "今天" ? hm(d) : word
}

/** 详情副标题里的时间段：今天 12:12—12:38；跨天时两端都写日期 */
export function spanLabel(first: number, last: number, now: Date = new Date()): string {
  const a = new Date(first)
  const b = new Date(last)
  if (sameDay(a, b)) return `${dayWord(a, now)} ${hm(a)}—${hm(b)}`
  return `${dayWord(a, now)} ${hm(a)}—${dayWord(b, now)} ${hm(b)}`
}

/** 状态文案：运行中 / 已完成 / 失败需要来源事件支持；只有写入时间时写「最近活跃」（画板 46） */
export function statusLabel(status: SessionStatus): string {
  return { running: "运行中", done: "已完成", failed: "失败", recent: "最近活跃", usage_only: "仅用量" }[status]
}

export function sessionTitle(s: Pick<SessionItemDto, "title" | "session_id">): string {
  return s.title ?? `会话 ${s.session_id.slice(0, 8)}`
}

/** 长 ID 中间省略：session_demo_82ac…91f0 */
export function middleEllipsis(id: string, head = 12, tail = 4): string {
  return id.length <= head + tail + 1 ? id : `${id.slice(0, head)}…${id.slice(-tail)}`
}

/**
 * 顶部统计（画板 40）：输入含缓存输入，消耗 = 输入 + 输出，缓存命中 = 缓存读取 ÷ 输入。
 * 任一会话缺某个分项，对应统计显示「—」。
 */
/**
 * 顶部统计只汇总 Token 四项都已知的会话，`unknown` 是被排除的会话数；
 * 全部未知时各项为 null（显示「—」），没有会话时为真实的 0。
 */
export function sessionTotals(sessions: readonly SessionItemDto[]) {
  const known = sessions.filter((s) => s.fresh_input !== null && s.cache_read !== null && s.cache_write !== null && s.output !== null)
  const unknown = sessions.length - known.length
  const sum = (pick: (s: SessionItemDto) => number | null) =>
    sessions.length > 0 && known.length === 0 ? null : known.reduce((total, s) => total + (pick(s) ?? 0), 0)
  const fresh = sum((s) => s.fresh_input)
  const read = sum((s) => s.cache_read)
  const write = sum((s) => s.cache_write)
  const output = sum((s) => s.output)
  const cache = read === null || write === null ? null : read + write
  const input = fresh === null || cache === null ? null : fresh + cache
  return {
    threads: sessions.length,
    unknown,
    input,
    cache,
    output,
    consumption: input === null || output === null ? null : input + output,
    hitRate: input && read !== null ? read / input : null,
  }
}

export type TimePreset = "all" | "today" | "7d" | "30d"

export const TIME_PRESETS: { value: TimePreset; label: string }[] = [
  { value: "all", label: "全部时间" },
  { value: "today", label: "今天" },
  { value: "7d", label: "近 7 天" },
  { value: "30d", label: "近 30 天" },
]

export function sinceFor(preset: TimePreset, now: Date = new Date()): number | null {
  if (preset === "all") return null
  const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
  return preset === "today" ? midnight : midnight - (preset === "7d" ? 6 : 29) * 86_400_000
}

/** 只有这两个来源有可靠的命令行续聊入口 */
export function canContinue(platform: string): boolean {
  return platform === "claude" || platform === "codex"
}

/** 键盘导航用的可见行：项目行与展开项目下的会话行 */
export type TreeRow = { kind: "project"; key: string } | { kind: "session"; key: string; project: string }

export function sessionKey(s: Pick<SessionItemDto, "platform" | "session_id">): string {
  return `${s.platform}:${s.session_id}`
}

export function visibleRows(groups: readonly ProjectGroup[], expanded: ReadonlySet<string>): TreeRow[] {
  const rows: TreeRow[] = []
  for (const g of groups) {
    rows.push({ kind: "project", key: g.key })
    if (expanded.has(g.key)) for (const s of g.sessions) rows.push({ kind: "session", key: sessionKey(s), project: g.key })
  }
  return rows
}
