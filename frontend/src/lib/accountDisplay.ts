/**
 * 账号管理卡片的显示规则（Jolu.pen 画板 07–10、23）。纯函数，便于 node --test 验证。
 */

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
  platformId: string
  baseName: string
  name: string
  label: string
}

/** 账号搜索与供应商筛选共同生效（画板 25「供应商筛选」）；搜索匹配名称与标识，不区分大小写 */
export function filterAccounts<T extends AccountLike>(accounts: readonly T[], platform: string | null, query: string): T[] {
  const q = query.trim().toLowerCase()
  return accounts.filter((account) =>
    (!platform || account.platformId === platform) &&
    (!q || [account.baseName, account.name, account.label].some((field) => field.toLowerCase().includes(q))))
}
