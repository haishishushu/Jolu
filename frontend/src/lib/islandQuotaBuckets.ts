import type { PlatformId, QuotaWindow } from "../types.ts"
import { quotaBucketViewFromWindows, type QuotaBucketView } from "./quotaBuckets.ts"
import { isPlanWindowKey } from "./providers/registry.ts"

/**
 * 灵动岛展开卡片要不要走「按桶」额度（`ZcodeBucketQuota` 的那套小卡）。
 *
 * 分流口径（与主窗口 `AccountsPanel` 的 `hasBuckets` 同一来源，但更严一档）：
 *
 * 1. **平台**：只有 `zcode` 的额度是按 `bucket_id` 返回的日额度桶（本机登录 / BigModel 套餐）。
 *    Claude / Codex 的 5h / 7d、Grok 的周 / 月 / 积分仍是套餐窗口，保持原样。
 * 2. **接入方式**：只有 `auth`（订阅 / 本机登录）。API Key 计量走今日 Token 与费用那一套，
 *    没有桶可画。
 * 3. **查询与连接状态**：`queryFailed`（失败 / 401 / 需要重新登录）与 `unavailable`
 *    （未连接 / 连接过期 / 来源不支持）时保留原有表达，不让失败被画成「未知的桶」。
 *    没有窗口（`quotas` 为空）同理。
 * 4. **窗口组成**：列表里只要混有套餐窗口（5h / 7d），就保留原来的整句——桶卡不吞套餐行。
 *    纯桶列表（当前 ZCode 本机登录的真实形状）才整体换成桶卡。
 *
 * 命中返回视图（与主窗口同一份 `quotaBuckets` 结果），否则返回 null 表示「继续显示
 * `remainingMetrics` 整句」。
 */
export interface IslandBucketQuotaInput {
  platform: PlatformId
  kind: "auth" | "api"
  /** 灵动岛拿到的界面窗口（`IslandData.quotas`），可能为空 / 未定义 */
  windows: readonly QuotaWindow[] | null | undefined
  /** 额度查询失败（含 401 / 需重登）：走原有表达 */
  queryFailed?: boolean | null
  /** 未连接 / 连接过期 / 来源不支持：走原有表达 */
  unavailable?: boolean | null
}

export function islandBucketQuotaView(
  { platform, kind, windows, queryFailed, unavailable }: IslandBucketQuotaInput,
  now = Date.now(),
): QuotaBucketView | null {
  if (platform !== "zcode") return null
  if (kind !== "auth") return null
  if (unavailable || queryFailed) return null
  const list = windows ?? []
  if (list.length === 0) return null
  // 混有套餐窗口时整体回落整句：桶卡只画非套餐窗口，会把这几个窗口的行吞掉。
  if (list.some((window) => isPlanWindowKey(window.key))) return null
  return quotaBucketViewFromWindows(list, now)
}
