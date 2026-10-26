/**
 * 用量统计的账号筛选（画板 38「账号筛选」）：多选；切换供应商后清除不适用的账号。
 * 「未归属账号」与后端 `usage_stats::UNATTRIBUTED` 对应，指没有归账到任何连接的本机记录。
 */
import { connectionProviderEntry } from "./providerCatalog.ts"
import type { Connection, PlatformId } from "../types"

export const UNATTRIBUTED_ACCOUNT = "__unattributed__"

type Account = Pick<Connection, "id" | "platformId" | "baseName"> & { edition?: string | null }

export function accountLabel(account: Account): string {
  return `${account.baseName} · ${connectionProviderEntry(account).name}`
}

export function accountsTriggerLabel(value: readonly string[], accounts: readonly Account[]): string {
  if (!value.length) return "全部账号"
  if (value.length === 1) {
    if (value[0] === UNATTRIBUTED_ACCOUNT) return "未归属账号"
    const found = accounts.find((a) => a.id === value[0])
    return found ? accountLabel(found) : "1 个账号"
  }
  return `${value.length} 个账号`
}

/** 供应商筛选下可选的账号 */
export function accountsFor<T extends Account>(accounts: readonly T[], platform: PlatformId | null): T[] {
  return platform ? accounts.filter((a) => a.platformId === platform) : [...accounts]
}

/** 切换供应商后保留仍适用的选择；「未归属账号」对任何供应商都适用 */
export function keepApplicable(value: readonly string[], accounts: readonly Account[], platform: PlatformId | null): string[] {
  const allowed = new Set(accountsFor(accounts, platform).map((a) => a.id))
  return value.filter((id) => id === UNATTRIBUTED_ACCOUNT || allowed.has(id))
}
