import type { AccountQuotaMetadata, BalanceStateDto, QuotaStateDto, QuotaWindowDto } from "./api"

export interface AccountCardPolicy {
  plan: string | null
  expiresAt: string | null
  expiryIsSnapshot: boolean
  resetCredits: AccountQuotaMetadata["reset_credits"]
  windows: readonly QuotaWindowDto[]
  showQuota: boolean
  showBalance: boolean
  balanceAmount: number | null
  balanceCurrency: string | null
  used: number | null
}

/** 只按连接方式和实际响应决定区域；标签、平台图标与套餐名不代表查询能力。 */
export function accountCardPolicy(
  account: { kind: string; baseUrl?: string | null },
  quota: QuotaStateDto | null,
  balance: BalanceStateDto | null = null,
): AccountCardPolicy {
  const data = quota?.state === "ok" ? quota : null
  const metadata = data?.account
  const plan = data?.plan?.trim() || null
  const expiresAt = metadata?.expires_at && Number.isFinite(new Date(metadata.expires_at).getTime())
    ? metadata.expires_at : null
  const windows = data?.windows ?? []
  const showBalance = account.kind === "api" && Boolean(account.baseUrl?.trim()) && balance?.state !== "unsupported"
  const wallet = showBalance && balance?.state === "ok" ? balance : null

  return {
    plan,
    expiresAt,
    expiryIsSnapshot: expiresAt !== null && metadata?.subscription_source === "local_snapshot",
    resetCredits: metadata?.reset_credits ?? null,
    windows,
    showQuota: data ? windows.length > 0 || plan !== null || expiresAt !== null
      : quota ? quota.state !== "unsupported" : account.kind === "auth",
    showBalance,
    balanceAmount: wallet && Number.isFinite(wallet.balance) ? wallet.balance : null,
    balanceCurrency: wallet?.currency ?? null,
    used: wallet?.used != null && Number.isFinite(wallet.used) ? wallet.used : null,
  }
}
