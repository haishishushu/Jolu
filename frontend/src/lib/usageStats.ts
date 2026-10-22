/**
 * 用量统计页（Jolu.pen 模块 C，画板 30–39）的纯逻辑：日期范围、数值格式、日历与状态文案。
 * 数值缺失一律返回「—」，不当成 0（画板 38「统计口径」）。
 */
import { shortModelName } from "./modelName.ts"

export const DASH = "—"

/** 本地日期键 YYYY-MM-DD */
export function dateKey(date: Date): string {
  const y = date.getFullYear()
  const m = String(date.getMonth() + 1).padStart(2, "0")
  const d = String(date.getDate()).padStart(2, "0")
  return `${y}-${m}-${d}`
}

export function parseDateKey(key: string): Date {
  const [y, m, d] = key.split("-").map(Number)
  return new Date(y, m - 1, d)
}

export function addDays(key: string, days: number): string {
  const date = parseDateKey(key)
  date.setDate(date.getDate() + days)
  return dateKey(date)
}

/** 日期范围：本地日期，起止都包含 */
export interface DateRange {
  start: string
  end: string
}

export function todayRange(now: Date = new Date()): DateRange {
  const key = dateKey(now)
  return { start: key, end: key }
}

/** 转成后端的 [start, end) 毫秒区间；结束日是今天时统计到当前时刻（画板 36 说明） */
export function rangeToMillis(range: DateRange, now: Date = new Date()): { start: number; end: number } {
  const start = parseDateKey(range.start).getTime()
  const nextDay = parseDateKey(addDays(range.end, 1)).getTime()
  const end = Math.min(nextDay, now.getTime())
  return { start, end: Math.max(end, start + 1) }
}

export function rangeDays(range: DateRange): number {
  return Math.round((parseDateKey(range.end).getTime() - parseDateKey(range.start).getTime()) / 86_400_000) + 1
}

function monthDay(key: string): string {
  const [, m, d] = key.split("-")
  return `${m}/${d}`
}

/** 筛选按钮文案：今天 / 10/15 / 10/15—10/21 */
export function rangeLabel(range: DateRange, now: Date = new Date()): string {
  const today = dateKey(now)
  if (range.start === today && range.end === today) return "今天"
  if (range.start === range.end) return monthDay(range.start)
  return `${monthDay(range.start)}—${monthDay(range.end)}`
}

/** 弹窗草稿是否可确认：两端都有且开始不晚于结束，允许同一天（画板 38） */
export function canConfirmRange(start: string | null, end: string | null): boolean {
  return !!start && !!end && start <= end
}

/** 日历点选：第一次点选定开始；已有完整范围或点在开始之前时重新开始 */
export function pickDate(draft: { start: string | null; end: string | null }, key: string): { start: string | null; end: string | null } {
  if (!draft.start || draft.end) return { start: key, end: null }
  if (key < draft.start) return { start: key, end: null }
  return { start: draft.start, end: key }
}

/** 月历：周一开头，每周 7 格，不属于本月的格子为 null */
export function monthGrid(year: number, month: number): (string | null)[][] {
  const first = new Date(year, month, 1)
  const offset = (first.getDay() + 6) % 7
  const days = new Date(year, month + 1, 0).getDate()
  const cells: (string | null)[] = Array.from({ length: offset }, () => null)
  for (let d = 1; d <= days; d++) cells.push(dateKey(new Date(year, month, d)))
  while (cells.length % 7) cells.push(null)
  const weeks: (string | null)[][] = []
  for (let i = 0; i < cells.length; i += 7) weeks.push(cells.slice(i, i + 7))
  return weeks
}

/** 时区说明：Asia/Shanghai → 上海；没有中文名的时区直接显示 IANA 名 */
const ZONE_NAMES: Record<string, string> = {
  "Asia/Shanghai": "上海",
  "Asia/Chongqing": "重庆",
  "Asia/Urumqi": "乌鲁木齐",
  "Asia/Hong_Kong": "香港",
  "Asia/Macau": "澳门",
  "Asia/Taipei": "台北",
  "Asia/Tokyo": "东京",
  "Asia/Seoul": "首尔",
  "Asia/Singapore": "新加坡",
  "Europe/London": "伦敦",
  "America/New_York": "纽约",
  "America/Los_Angeles": "洛杉矶",
}

export function zoneName(zone: string): string {
  return ZONE_NAMES[zone] ?? zone
}

export function utcOffsetLabel(date: Date = new Date()): string {
  const minutes = -date.getTimezoneOffset()
  const sign = minutes >= 0 ? "+" : "-"
  const abs = Math.abs(minutes)
  const h = Math.floor(abs / 60)
  const m = abs % 60
  return `UTC${sign}${h}${m ? `:${String(m).padStart(2, "0")}` : ""}`
}

export function localZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "本地"
  } catch {
    return "本地"
  }
}

/** Token：≥1M 写 12.48 M，≥1K 写 8.4 K，其余写整数 */
export function formatTokens(n: number | null | undefined): string {
  if (n === null || n === undefined) return DASH
  const abs = Math.abs(n)
  if (abs >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(2)} B`
  if (abs >= 1_000_000) return `${(n / 1_000_000).toFixed(2)} M`
  if (abs >= 1_000) return `${(n / 1_000).toFixed(1)} K`
  return String(n)
}

/** 估算费用：≥1 美元两位小数，不足 1 美元三位小数（单次调用常见 $0.026） */
export function formatUsd(n: number | null | undefined): string {
  if (n === null || n === undefined) return DASH
  if (n === 0) return "$0.00"
  return n >= 1 ? `$${n.toFixed(2)}` : `$${n.toFixed(3)}`
}

export function formatPercent(rate: number | null | undefined, digits = 1): string {
  if (rate === null || rate === undefined || !Number.isFinite(rate)) return DASH
  return `${(rate * 100).toFixed(digits)}%`
}

/** 较上一段同长度区间的变化；上一段没有记录或为 0 时不比较 */
export function changeText(current: number | null, previous: number | null): string | null {
  if (current === null || previous === null || previous <= 0) return null
  const pct = ((current - previous) / previous) * 100
  return `${pct >= 0 ? "+" : ""}${pct.toFixed(1)}%`
}

/** 秒：模型表写「1.6 s」，调用表写「0.4s」 */
export function formatSeconds(ms: number | null | undefined, spaced = false): string {
  if (ms === null || ms === undefined) return DASH
  return `${(ms / 1000).toFixed(1)}${spaced ? " " : ""}s`
}

export function formatMinutes(minutes: number): string {
  const h = Math.floor(minutes / 60)
  const m = minutes % 60
  return h ? `${h} 小时 ${m} 分` : `${m} 分`
}

export function bucketLabel(bucket: "2h" | "day" | "week"): string {
  return bucket === "2h" ? "每 2 小时" : bucket === "day" ? "每天" : "每周"
}

/** 输出 TPS：输出 Token ÷（总耗时 − 首字）。任一缺失或差值不为正时未知 */
export function outputTps(row: { output: number | null; duration_ms: number | null; first_token_ms: number | null }): number | null {
  if (row.output === null || row.duration_ms === null || row.first_token_ms === null) return null
  const seconds = (row.duration_ms - row.first_token_ms) / 1000
  if (seconds <= 0 || row.output <= 0) return null
  return row.output / seconds
}

const EFFORT_LABELS: Record<string, string> = { minimal: "最低", low: "低", medium: "中", high: "高", xhigh: "极高", max: "最高" }

export function effortLabel(effort: string | null): string {
  if (!effort) return DASH
  return EFFORT_LABELS[effort.toLowerCase()] ?? effort
}

/** 状态胶囊：没有状态码的本机记录按成功（能写入用量即已返回） */
export function callStatus(row: { failed: boolean; status_code: number | null }): { label: string; ok: boolean } {
  if (!row.failed) return { label: "成功", ok: true }
  if (row.status_code === 429) return { label: "429 限流", ok: false }
  if (row.status_code === 408 || row.status_code === 504) return { label: "超时", ok: false }
  if (row.status_code !== null) return { label: `失败 ${row.status_code}`, ok: false }
  return { label: "失败", ok: false }
}

/** 模型展示名：claude-sonnet-4-5 → Claude Sonnet 4.5，gpt-5-mini → GPT-5 mini；完整名放悬浮提示 */
export function modelDisplayName(model: string | null): string {
  if (!model) return "未知模型"
  const name = shortModelName(model)
  const parts = name.split("-").filter(Boolean)
  if (parts[0]?.toLowerCase() === "gpt" && parts.length > 1) {
    return [`GPT-${parts[1]}`, ...parts.slice(2)].join(" ")
  }
  const words: string[] = []
  for (const part of parts) {
    const last = words[words.length - 1]
    if (/^\d+(\.\d+)?$/.test(part) && last !== undefined && /^\d+(\.\d+)?$/.test(last) && !last.includes(".")) {
      words[words.length - 1] = `${last}.${part}`
    } else {
      words.push(/^[a-z]/.test(part) ? part[0].toUpperCase() + part.slice(1) : part)
    }
  }
  return words.join(" ")
}

/** 分页页码：首尾常驻，当前页附近展开，其余折叠成省略号 */
export function pageWindow(page: number, pageCount: number): (number | "…")[] {
  if (pageCount <= 7) return Array.from({ length: pageCount }, (_, i) => i + 1)
  const pages = new Set([1, pageCount, page - 1, page, page + 1])
  if (page <= 4) [2, 3, 4, 5].forEach((p) => pages.add(p))
  if (page >= pageCount - 3) [pageCount - 4, pageCount - 3, pageCount - 2, pageCount - 1].forEach((p) => pages.add(p))
  const sorted = [...pages].filter((p) => p >= 1 && p <= pageCount).sort((a, b) => a - b)
  const out: (number | "…")[] = []
  sorted.forEach((p, i) => {
    if (i && p - sorted[i - 1] > 1) out.push("…")
    out.push(p)
  })
  return out
}

/** 全年日历：按周分列（周一在上），每列 7 格；超出最后一天的格子为 null */
export function calendarWeeks(firstDate: string, lastDate: string): (string | null)[][] {
  const weeks: (string | null)[][] = []
  let key = firstDate
  while (key <= lastDate) {
    const week: (string | null)[] = []
    for (let i = 0; i < 7; i++) {
      week.push(key <= lastDate ? key : null)
      key = addDays(key, 1)
    }
    weeks.push(week)
  }
  return weeks
}

/** 月份标签放在该月第一个周一所在的列（与画板 32 一致）；第一列显示起始月份 */
export function monthLabels(weeks: (string | null)[][]): { column: number; label: string }[] {
  const out: { column: number; label: string }[] = []
  weeks.forEach((week, column) => {
    const monday = week[0]
    if (!monday) return
    const month = monday.slice(0, 7)
    const previous = column ? weeks[column - 1][0]?.slice(0, 7) : null
    if (column === 0 || month !== previous) out.push({ column, label: `${Number(monday.slice(5, 7))}月` })
  })
  // 第一列和下一个月标签相邻时去掉第一列，避免文字重叠
  if (out.length > 1 && out[1].column - out[0].column < 2) out.shift()
  return out
}

/** 活跃等级 0–4：有用量的日期按 Token 四分位分档 */
export function activityLevel(tokens: number, sortedNonZero: number[]): 0 | 1 | 2 | 3 | 4 {
  if (tokens <= 0 || !sortedNonZero.length) return 0
  const at = (q: number) => sortedNonZero[Math.min(sortedNonZero.length - 1, Math.floor(q * sortedNonZero.length))]
  if (tokens <= at(0.25)) return 1
  if (tokens <= at(0.5)) return 2
  if (tokens <= at(0.75)) return 3
  return 4
}

/** 占比：分母为 0 或未知时返回 null */
export function share(part: number | null, whole: number | null): number | null {
  if (part === null || whole === null || whole <= 0) return null
  return part / whole
}
