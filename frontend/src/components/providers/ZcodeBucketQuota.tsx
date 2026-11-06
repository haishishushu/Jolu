import { useEffect, useMemo, useState } from "react"
import type { QuotaWindowDto } from "@/lib/api"
import { quotaBucketView, type QuotaBucketView } from "@/lib/quotaBuckets"
import { cn } from "@/lib/utils"

/** `5h 12m` → `5h 12m 后重置`；已过期交给来源的 `等待更新`，不再拼「后重置」。 */
function resetLabel(reset: string | null): string {
  if (!reset) return "—"
  return reset === "等待更新" ? reset : `${reset} 后重置`
}

/**
 * 尺寸档位：
 * - `card`：主窗口账号卡片（默认，外观与改造前逐字一致）；
 * - `island`：380 宽灵动岛展开卡片，按 A3 的字号（窗口标签 11px、数值 13px）。
 *
 * 只改字号与间距，不新增颜色变量：两档都用既有 `jolu-*`。
 */
export type ZcodeBucketQuotaSize = "card" | "island"

interface BucketDensity {
  /** 区块头「日额度」 */
  header: string
  /** 区块头右侧重置倒计时 */
  headerReset: string
  /** 小卡桶名 */
  name: string
  /** 小卡主值（百分比） */
  value: string
  /** 小卡的 `已用 / 总量` 与 `剩 N` */
  detail: string
  /** 小卡外壳 */
  card: string
  /** 小卡网格间距 */
  grid: string
  /** 进度条高度 */
  track: string
}

const DENSITY: Readonly<Record<ZcodeBucketQuotaSize, BucketDensity>> = {
  card: {
    header: "text-[10px]",
    headerReset: "text-[10px]",
    name: "text-[10px]",
    value: "text-[11px]",
    detail: "text-[10px]",
    card: "gap-1 rounded-lg px-2 py-1.5",
    grid: "gap-2",
    track: "h-[5px]",
  },
  island: {
    header: "text-[11px]",
    headerReset: "text-[11px]",
    name: "text-[11px]",
    value: "text-[13px]",
    detail: "text-[11px]",
    card: "gap-1 rounded-lg px-2 py-1.5",
    grid: "gap-2",
    track: "h-[5px]",
  },
}

/**
 * ZCode 的按桶额度（方案 A）：一个后端窗口一张小卡。
 *
 * 与灵动岛同一套口径（纯逻辑在 `lib/quotaBuckets.ts`）：百分比优先、数字压缩、
 * 每桶一张带进度条的小卡。颜色只用主窗口既有的 `jolu-*` 变量；进度条复用 `QuotaBar`
 * 的轨道 / 填充类，没有新增样式。未知（无百分比）不画进度条，真实 0% 画空轨道并写 `0%`。
 *
 * 两种数据入口：
 * - `windows`：后端 DTO，组件自己按 30 秒刷新倒计时（主窗口账号卡）；
 * - `view`：调用方已经算好的视图（灵动岛的窗口只有格式化好的倒计时，用
 *   `quotaBucketViewFromWindows` 先算好再传），此时组件不另起定时器。
 *
 * 倒计时每 30 秒刷新一次：额度查询按 `refresh_minutes` 轮询，倒计时不能跟着等那么久。
 */
export function ZcodeBucketQuota({
  windows,
  view: providedView,
  plan,
  size = "card",
  columns,
  className,
}: {
  /** 后端额度窗口；与 `view` 二选一 */
  windows?: readonly QuotaWindowDto[]
  /** 已经算好的按桶视图；给了就不再从 `windows` 计算，也不起定时器 */
  view?: QuotaBucketView | null
  /** 套餐名（`日额度 · 套餐名`）；灵动岛只写「日额度」时传 null */
  plan: string | null
  /** 尺寸档位，默认 `card`（主窗口） */
  size?: ZcodeBucketQuotaSize
  /** 列数覆盖：灵动岛图例栏窄，固定逐行传 1；不传沿用 `view.columns` */
  columns?: number
  className?: string
}) {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    // 视图由调用方算好时，倒计时也跟着它的数据刷新走，这里不再自转定时器。
    if (providedView !== undefined) return
    const timer = window.setInterval(() => setNow(Date.now()), 30_000)
    return () => window.clearInterval(timer)
  }, [providedView])
  const view = useMemo(
    () => (providedView !== undefined ? providedView : quotaBucketView(windows ?? [], now)),
    [providedView, windows, now],
  )
  if (!view) return null
  const density = DENSITY[size]
  const cols = columns ?? view.columns
  return (
    <div
      className={cn("flex min-w-0 flex-col gap-2", className)}
      data-zcode-buckets
      data-zcode-buckets-size={size}
      data-zcode-plan={plan ?? undefined}
    >
      <div className="flex min-w-0 items-baseline justify-between gap-2">
        <span className={cn("min-w-0 truncate font-medium text-jolu-text", density.header)} title={plan ?? undefined}>
          日额度{plan ? ` · ${plan}` : ""}
        </span>
        <span className={cn("shrink-0 tabular-nums text-jolu-muted", density.headerReset)}>{resetLabel(view.reset)}</span>
      </div>
      <div className={cn("grid min-w-0", density.grid, cols === 1 ? "grid-cols-1" : "grid-cols-2")}>
        {view.buckets.map((bucket) => (
          <div
            key={bucket.key}
            data-quota-bucket={bucket.key}
            className={cn("flex min-w-0 flex-col border border-jolu-border bg-jolu-side", density.card)}
          >
            <div className="flex min-w-0 items-baseline justify-between gap-1.5">
              <span className={cn("min-w-0 truncate text-jolu-muted", density.name)} title={bucket.name}>{bucket.name}</span>
              <span className={cn("shrink-0 font-semibold tabular-nums", density.value, bucket.tone === "danger" ? "text-jolu-danger" : "text-jolu-text")}>
                {bucket.value}
              </span>
            </div>
            {bucket.usedTotal && bucket.value !== bucket.usedTotal && (
              <span className={cn("truncate tabular-nums text-jolu-muted", density.detail)} title={bucket.usedTotal}>{bucket.usedTotal}</span>
            )}
            {bucket.percent !== null && (
              <div className={cn("overflow-hidden rounded-[3px] bg-jolu-track", density.track)}>
                <div
                  className={cn("motion-quota-bar h-full rounded-[3px]", bucket.tone === "danger" ? "bg-jolu-danger" : "bg-jolu-accent")}
                  style={{ width: `${bucket.percent}%` }}
                />
              </div>
            )}
            <span className={cn("truncate tabular-nums text-jolu-muted", density.detail)}>{bucket.remaining ? `剩 ${bucket.remaining}` : "剩 —"}</span>
          </div>
        ))}
      </div>
    </div>
  )
}
