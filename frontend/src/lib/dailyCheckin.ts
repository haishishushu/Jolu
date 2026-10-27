import type { CheckinAccountDto, CheckinOutcome, CheckinStatistics, CheckinUpdate } from "./checkinApi.ts"

export interface CheckinAccount extends CheckinAccountDto { account_key: string | null; outcome: CheckinOutcome; checked_at: number | null; statistics: CheckinStatistics | null }
export const CHECKIN_EDITIONS = [
  { id: "workbuddy-cn", platform: "workbuddy", edition: "cn", name: "WorkBuddy", region: "国内版", manual: true, description: "奖励以当前活动返回为准" },
  { id: "qoder-cn", platform: "qoder", edition: "cn", name: "Qoder", region: "国内版", manual: true, description: "北京时间 10:00 开启下一轮" },
  { id: "trae-cn", platform: "trae", edition: "cn", name: "Trae", region: "国内版", manual: true, description: "个人账号 · 北京时间每日更新" },
  { id: "workbuddy-ai", platform: "workbuddy", edition: "ai", name: "WorkBuddy AI", region: "国际版", manual: false, description: "完成有效任务后自动结算，无需手动领取" },
  { id: "qoder-global", platform: "qoder", edition: "global", name: "Qoder", region: "国际版", manual: true, description: "北京时间 10:00 开启下一轮" },
  { id: "trae-global", platform: "trae", edition: "global", name: "Trae", region: "国际版", manual: false, description: "官方当前仅向国内版开放每日积分签到" },
] as const
export type CheckinEdition = (typeof CHECKIN_EDITIONS)[number]

export function checkinAccountLabel(account: Pick<CheckinAccountDto, "account" | "name">, hidden = false): string {
  return hidden ? "账号已隐藏" : account.account?.trim() || "未获取账号"
}

export const STATE_LABELS = { unknown: "待核实", ready: "可签到", claimed: "本轮已签到", inactive: "活动未开放", unsupported: "暂不支持", unauthorized: "登录已过期", forbidden: "无法领取", rate_limited: "请求受限", failed: "查询失败" } as const
export const ERROR_STATES: readonly string[] = ["unauthorized", "forbidden", "rate_limited", "failed", "unknown"]

export function initialCheckinAccount(dto: CheckinAccountDto): CheckinAccount {
  const entry = CHECKIN_EDITIONS.find(e => e.platform === dto.platform && e.edition === dto.edition)
  const outcome: CheckinOutcome = { state: entry && !entry.manual ? "unsupported" : "unknown", message: entry && !entry.manual ? entry.description : "尚未查询，请刷新状态", reward: null, unit: dto.platform === "qoder" ? "Credits" : "积分", period_key: null, reset_at: null }
  return { ...dto, account_key: null, checked_at: null, outcome, statistics: null }
}

export function uniqueCheckinAccounts(accounts: readonly CheckinAccount[]): CheckinAccount[] {
  const seen = new Set<string>()
  return accounts.filter(a => {
    const key = `${a.platform}:${a.edition}:${a.account_key ?? a.id}`
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })
}

export function checkinTargets(accounts: readonly CheckinAccount[], now = Date.now()): CheckinAccount[] {
  return uniqueCheckinAccounts(accounts.filter(a => a.connection_status !== "paused" && a.account_key !== null && currentOutcome(a, now).state === "ready" && CHECKIN_EDITIONS.some(e => e.manual && e.platform === a.platform && e.edition === a.edition)))
}
export function checkinSummary(accounts: readonly CheckinAccount[]) {
  const rows = uniqueCheckinAccounts(accounts.filter(a => a.connection_status !== "paused"))
  return { ready: checkinTargets(rows).length, claimed: rows.filter(a => currentOutcome(a).state === "claimed").length, errors: rows.filter(a => a.checked_at !== null && ERROR_STATES.includes(currentOutcome(a).state)).length }
}
export function rewardLabel(outcome: Pick<CheckinOutcome, "reward" | "unit">): string {
  return outcome.reward !== null && Number.isFinite(outcome.reward) && outcome.reward >= 0 ? `${outcome.reward.toLocaleString("zh-CN", { maximumFractionDigits: 4 })} ${outcome.unit}` : "—"
}
export function currentOutcome(account: CheckinAccount, now = Date.now()): CheckinOutcome {
  const outcome = account.outcome
  // 以官方响应时的剩余秒数计算相对有效期，不用本机绝对时间判断服务端日期。
  const elapsed = account.checked_at === null ? 0 : Math.max(0, now - account.checked_at)
  const expired = outcome.reset_at !== null && outcome.server_time != null && Number.isFinite(outcome.reset_at) && Number.isFinite(outcome.server_time)
    && elapsed >= Math.max(0, outcome.reset_at - outcome.server_time) * 1000
  return expired && (outcome.state === "claimed" || outcome.state === "ready")
    ? { ...outcome, state: "unknown", reward: null, message: "新一轮已开始，请刷新状态" } : outcome
}
export function applyCheckinUpdate(accounts: readonly CheckinAccount[], update: CheckinUpdate): CheckinAccount[] {
  if (!accounts.some(a => a.id === update.id)) return [...accounts]
  return accounts.map(a => a.id === update.id || (update.account_key !== null && a.account_key === update.account_key)
    ? { ...a, account: update.account ?? null, outcome: update.outcome, account_key: update.account_key, checked_at: update.checked_at, statistics: update.statistics ?? null } : a)
}
