import type { AccountQuotaMetadata, QuotaStateDto, QuotaWindowDto } from "./api.ts"
import type { CheckinStatistics, OfficialCheckinStatistics } from "./checkinApi.ts"
import { rewardLabel } from "./dailyCheckin.ts"

export interface CheckinQuotaRow {
  key: string
  label: string
  total: string
  remaining: string
  reason: string | null
  remainingLabel?: string
  used?: string | null
}

export interface CheckinQuotaDisplay {
  rows: CheckinQuotaRow[]
  message: string | null
  stale: boolean
}

type QuotaFailure = Exclude<QuotaStateDto, { state: "ok" }>
const NUMBER = "(?:\\d{1,3}(?:,\\d{3})+|\\d+)(?:\\.\\d+)?"
const UNIT = "([^\\d%;；/]*)"
const TOTAL_RATIO = new RegExp(`^${NUMBER}\\s*/\\s*(${NUMBER})\\s*${UNIT}$`)
const TOTAL_ONLY = new RegExp(`^总额度\\s*(${NUMBER})\\s*${UNIT}(?:[；;]\\s*已用未知)?$`)
const UNLIMITED_USED = new RegExp(`^已用\\s*${NUMBER}\\s*${UNIT}[；;]\\s*不限量$`)
const REMAINING = new RegExp(`^(?:剩余\\s*)?(${NUMBER})\\s*${UNIT}$`)
const QUOTA_STATES = { unsupported: "不支持查询", forbidden: "无权限", unauthorized: "登录已过期", rate_limited: "请求受限", failed: "查询失败" } as const

function absoluteAmount(match: RegExpMatchArray | null): string | null {
  if (!match || !Number.isFinite(Number(match[1].replaceAll(",", "")))) return null
  return [match[1], match[2].trim()].filter(Boolean).join(" ")
}

function quotaValues(key: string, label: string, total: string | null, remaining: string | null, percentageOnly = false): CheckinQuotaRow {
  const reason = total === null && remaining === null
    ? percentageOnly ? "仅提供使用百分比，未提供总量和精确剩余" : "来源未提供总量和精确剩余"
    : total === null ? "来源未提供总额度"
    : remaining === null ? "来源未提供精确剩余" : null
  return { key, label, total: total ?? "—", remaining: remaining ?? "—", reason }
}

function quotaRow(window: QuotaWindowDto): CheckinQuotaRow {
  const amount = window.amount_text?.trim() ?? ""
  const unlimited = amount.match(UNLIMITED_USED)
  const total = amount === "不限量" ? "不限量"
    : unlimited ? `不限量${unlimited[1].trim() ? `（${unlimited[1].trim()}）` : ""}`
    : absoluteAmount(amount.match(TOTAL_RATIO) ?? amount.match(TOTAL_ONLY))
  const remaining = absoluteAmount(window.remaining_text?.trim().match(REMAINING) ?? null)
  return quotaValues(window.key, window.window_name, total, remaining, window.used_percent !== null && Number.isFinite(window.used_percent))
}

function rangeAmount(value: number | null, unit: string): string | null {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) return null
  return [(value === 0 ? 0 : value).toLocaleString("zh-CN", { maximumSignificantDigits: 21 }), unit.trim()].filter(Boolean).join(" ")
}

function rawQuotaRow(range: NonNullable<AccountQuotaMetadata["amount_ranges"]>[number]): CheckinQuotaRow {
  const direct = rangeAmount(range.remaining, range.unit)
  const used = rangeAmount(range.used ?? null, range.unit)
  const total = range.unlimited ? `不限量${range.unit.trim() ? `（${range.unit.trim()}）` : ""}` : rangeAmount(range.total, range.unit)
  const calculated = !range.unlimited && typeof range.total === "number" && Number.isFinite(range.total) && range.total >= 0
    && typeof range.used === "number" && Number.isFinite(range.used) && range.used >= 0 && range.used <= range.total
    && typeof range.calculated_remaining === "number" && Number.isFinite(range.calculated_remaining)
    && range.calculated_remaining >= 0 && range.calculated_remaining <= range.total
    && Math.abs(range.calculated_remaining - (range.total - range.used)) <= 1e-9 * Math.max(1, range.total)
      ? rangeAmount(range.calculated_remaining, range.unit) : null
  const isCalculated = direct === null && calculated !== null
  const row = quotaValues(range.key, range.name, total, direct ?? calculated)
  return { ...row, used, remainingLabel: isCalculated ? "可用额度（计算）" : "可用额度",
    reason: isCalculated ? "可用额度由总额度减已用额度计算" : row.reason }
}

function failureMessage(failure: QuotaFailure): string {
  return `${QUOTA_STATES[failure.state]}${failure.reason.trim() ? `：${failure.reason}` : ""}`
}

/** 只使用来源明确提供的绝对数；既不从百分比推算，也不把不同窗口相加。 */
export function checkinQuotaDisplay(quota: {
  state: QuotaStateDto | null
  failure?: QuotaFailure | null
  loading?: boolean
  stale?: boolean
}): CheckinQuotaDisplay {
  const empty = { key: "unknown", label: "账户额度", total: "—", remaining: "—", reason: null }
  if (!quota.state) return { rows: [empty], message: quota.loading ? "正在查询额度…" : "未获取", stale: false }
  if (quota.state.state !== "ok") return { rows: [empty], message: failureMessage(quota.state), stale: false }
  const ranges = quota.state.account?.amount_ranges
  let rows: CheckinQuotaRow[]
  if (Array.isArray(ranges)) {
    // 数组（含空数组）声明了原始数据范围，缺失项不可回退到旧窗口的推算或汇总值。
    rows = ranges.map(rawQuotaRow)
  } else {
    const allRows = quota.state.windows.map(quotaRow)
    // Qoder 的 overall 只提供汇总百分比；有独立额度范围时不重复显示这行未知金额。
    const scopedRows = allRows.filter(row => row.key !== "qoder:overall")
    rows = scopedRows.length ? scopedRows : allRows
  }
  return {
    rows: rows.length ? rows : [{ ...empty, reason: "来源未提供总量和精确剩余" }],
    message: quota.stale
      ? `数据可能已过期${quota.failure ? ` · ${failureMessage(quota.failure)}` : ""}`
      : quota.loading ? "正在更新额度…" : null,
    stale: quota.stale ?? false,
  }
}

/** 主卡只显示明确汇总或单一范围；多范围留到详情，不悄悄相加。 */
export function checkinQuotaSummary(quota: Parameters<typeof checkinQuotaDisplay>[0]) {
  const display = checkinQuotaDisplay(quota)
  const row = display.rows.find(item => item.key === "trae:credits") ?? (display.rows.length === 1 ? display.rows[0] : null)
  return {
    total: row?.total ?? "多项额度",
    remaining: row?.remaining ?? "多项额度",
    remainingLabel: row?.remainingLabel ?? "可用额度",
    scope: row?.label ?? `共 ${display.rows.length} 项额度`,
    status: display.stale ? "数据过期" : quota.state && quota.state.state !== "ok" ? QUOTA_STATES[quota.state.state]
      : quota.loading ? quota.state ? "更新中…" : "查询中…" : !quota.state ? "未获取" : null,
  }
}

export function checkinPeriodDisplay(statistics: CheckinStatistics | null, period: "week" | "month" | "all", official?: OfficialCheckinStatistics | null): { days: string; reward: string } {
  const current = official?.[period]
  if (current) return {
    days: current.days !== null && Number.isSafeInteger(current.days) && current.days >= 0 ? `${current.days.toLocaleString("zh-CN")} 天` : "—",
    reward: official.unit.trim() ? rewardLabel({ reward: current.reward, unit: official.unit }) : "—",
  }
  if (!statistics) return { days: "未获取", reward: "—" }
  if (!statistics.has_records) return { days: "暂无记录", reward: "—" }
  return {
    days: `${statistics[period].days.toLocaleString("zh-CN")} 天`,
    reward: statistics.unit.trim() ? rewardLabel({ reward: statistics[period].reward, unit: statistics.unit }) : "—",
  }
}
