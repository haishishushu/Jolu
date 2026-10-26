/**
 * 账号管理卡片的显示规则（Jolu.pen 画板 07–10、23）。纯函数，便于 node --test 验证。
 */
import type { AccountQuotaMetadata, QuotaWindowDto } from "./api"
import { connectionProviderId } from "./providerCatalog.ts"

/** 查询覆盖所有供应商；具体字段是否提供由服务响应决定。 */
export function accountQueries(account: { id: string; platformId: string; kind: string; status: string; baseUrl?: string | null }) {
  if (account.status === "paused") return { quota: null, balance: null }
  return {
    quota: account.id,
    balance: account.kind === "api" && account.baseUrl ? account.id : null,
  }
}

export function quotaValue(window: { remaining_text?: string | null; used_percent?: number | null }): string {
  if (window.remaining_text?.trim()) return window.remaining_text
  const used = window.used_percent
  return used != null && Number.isFinite(used) && used >= 0 && used <= 100
    ? `剩余 ${Math.round(100 - used)}%` : "未提供"
}

export function subscriptionDisplay(account?: AccountQuotaMetadata | null) {
  const snapshot = account?.subscription_source === "local_snapshot"
  const date = account?.expires_at ? new Date(account.expires_at) : null
  const observed = account?.observed_at ? new Date(account.observed_at) : null
  const recordedAt = observed && Number.isFinite(observed.getTime())
    ? `最近记录：${observed.getFullYear()}-${pad(observed.getMonth() + 1)}-${pad(observed.getDate())} ${pad(observed.getHours())}:${pad(observed.getMinutes())}。` : ""
  const expires = date && Number.isFinite(date.getTime())
    ? `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}` : "未提供"
  return {
    expires,
    note: snapshot ? `${recordedAt}到期时间来自本机登录记录，续费后可能延迟更新，请以官网为准。` : null,
  }
}

export function resetCreditsText(credits?: AccountQuotaMetadata["reset_credits"], compact = false) {
  if (!credits) return "未提供"
  const count = credits.available_count
  const usable = credits.applicable_available_count
  const available = count == null ? "可用次数未提供" : `可用 ${count} 次`
  return compact ? available : `${available} · ${usable == null ? "适用次数未提供" : `当前适用 ${usable} 次`}`
}

/** 官方订阅的两个主窗口保持固定顺序；缺失数据显式为空，额外模型窗口完整保留。 */
export function accountQuotaWindows(platform: string, windows: readonly QuotaWindowDto[]): QuotaWindowDto[] {
  if (platform !== "claude" && platform !== "codex") return [...windows]
  const keys = ["5h", "7d"]
  return [
    ...keys.map(key => windows.find(window => window.key === key) ?? {
      key, window_name: quotaTitle(key, key), used_percent: null, amount_text: null, resets_at: null,
    }),
    ...windows.filter(window => !keys.includes(window.key)),
  ]
}

/** 额度窗口标题：5h →「5 小时额度」，7d →「周额度」，其余沿用来源给的名称 */
export function quotaTitle(key: string, windowName: string): string {
  if (key === "5h") return "5 小时额度"
  if (key === "7d") return "周额度"
  return windowName
}

const pad = (n: number) => String(n).padStart(2, "0")

/** 重置时间：今天显示「今天 18:30 重置」，其他日期「10/26 09:00 重置」；缺失或无效返回 null */
export function formatResetAt(iso: string | null, now = new Date()): string | null {
  if (!iso) return null
  const at = new Date(iso)
  if (Number.isNaN(at.getTime())) return null
  const time = `${pad(at.getHours())}:${pad(at.getMinutes())}`
  const sameDay = at.getFullYear() === now.getFullYear() && at.getMonth() === now.getMonth() && at.getDate() === now.getDate()
  return sameDay ? `今天 ${time} 重置` : `${pad(at.getMonth() + 1)}/${pad(at.getDate())} ${time} 重置`
}

/** 同步时间：一分钟内「刚刚同步」，一小时内「N 分钟前」，否则显示时分 */
export function formatSyncedAt(at: Date | null, now = new Date(), prefix = ""): string {
  if (!at) return `${prefix}尚未同步`
  const minutes = Math.floor((now.getTime() - at.getTime()) / 60_000)
  if (minutes < 1) return `${prefix}刚刚同步`
  if (minutes < 60) return `${prefix}${minutes} 分钟前同步`
  return `${prefix}${pad(at.getHours())}:${pad(at.getMinutes())} 同步`
}

/** 金额：带两位小数与币种；美元加 $ 前缀 */
export function formatMoney(amount: number, currency: string): string {
  const fixed = amount.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })
  return currency.toUpperCase() === "USD" ? `$${fixed}` : fixed
}

export interface AccountLike {
  id?: string
  edition?: string | null
  accountEmail?: string | null
  platformId: string
  baseName: string
  name: string
  label: string
  kind?: string
}

/** 账号搜索与供应商筛选共同生效（画板 25「供应商筛选」）；搜索匹配名称与标识，不区分大小写 */
export function filterAccounts<T extends AccountLike>(accounts: readonly T[], platform: string | null, query: string, kind: "auth" | "api" | null = null): T[] {
  const q = query.trim().toLowerCase()
  return accounts.filter((account) =>
    (!kind || account.kind === kind) &&
    (!platform || connectionProviderId(account) === platform) &&
    (!q || [account.baseName, account.name, account.label, account.accountEmail ?? ""].some((field) => field.toLowerCase().includes(q))))
}

/** 错误正文仍可阅读，只遮蔽其中可能回显的邮箱及地址。 */
export function redactAccountMessage(message: string): string {
  return message.replace(/https?:\/\/[^\s，。；）)]+/gi, "[地址已隐藏]")
    .replace(/[A-Z0-9._%+-]+@[A-Z0-9.-]+/gi, "[账号已隐藏]")
}
