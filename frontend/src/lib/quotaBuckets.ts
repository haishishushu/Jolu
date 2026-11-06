import type { QuotaWindowDto } from "./api.ts"
import type { QuotaWindow } from "../types.ts"
import { compactQuotaNumber } from "./capsuleQuotaChips.ts"
import { formatQuotaCountdown } from "./quotaMap.ts"
import { isPlanWindowKey } from "./providers/registry.ts"

/**
 * 「按桶」额度视图（ZCode 本机登录 / BigModel 套餐那份日额度桶）。
 *
 * 一个后端窗口 = 一张小卡：桶名 + 已用百分比（主值）+ `已用 / 总量` + 进度条 + `剩 N`。
 * 主窗口账号卡（`ZcodeBucketQuota`）与灵动岛展开卡片共用这一份口径：
 * - 主值是**已用百分比**（`used_percent`），与额度环、胶囊芯片一致，不把「剩余」当主值；
 * - 数字压缩复用 `capsuleQuotaChips.compactQuotaNumber`（一位小数），不另写一套；
 * - 倒计时复用 `quotaMap.formatQuotaCountdown`（`还剩 5h 12m`），上屏时去掉 `还剩`；
 * - 只有 `used_percent` 未知时才回落到 `已用 / 总量`，两者都未知写 `—`；
 * - 未知（`null`）不画进度条，真实 0% 画空轨道 + `0%`。
 *
 * 只由「非套餐窗口」（`!isPlanWindowKey`）组成视图：5h / 7d 的 Claude / Codex 卡片保持
 * 原有的 `QuotaBar`。调用方还会按平台收口（当前只有 ZCode 走这里），避免 Grok 的
 * 周 / 月 / 积分窗口被当成桶。
 *
 * 两种入口：
 * - [`quotaBucketView`] 吃后端 DTO（带 `resets_at` ISO 时间），主窗口用；
 * - [`quotaBucketViewFromWindows`] 吃界面已经转好的 `QuotaWindow`（只有格式化好的
 *   `resetCountdown`，没有 ISO 时间），灵动岛用——两边落到同一个 `buildBucketView`。
 */

/** 小卡列数上限；超过 2 个桶改为逐行（沿用灵动岛「最多两块」的收口） */
export const MAX_BUCKET_COLUMNS = 2

export interface QuotaBucket {
  /** 后端窗口 key（bucket_id），只用于 React key 与测试，不上屏 */
  key: string
  /** 桶名：后端 `window_name`，缺失时写 `—` */
  name: string
  /** 主值：`40%`（百分比优先）→ `1.2M / 3.0M` → `—` */
  value: string
  /** `1.2M / 3.0M`；来源没给分母时为 null */
  usedTotal: string | null
  /** 剩余量（不含「剩」前缀）；未知 null（上屏写 `剩 —`） */
  remaining: string | null
  /** 已用百分比（0–100 收口）；未知 null */
  percent: number | null
  /** 真实 0%：画空进度条并写 `0%`，与未知严格分开 */
  zero: boolean
  /** ≥100% 的告警色；其余 null（沿用 jolu-danger，不新增颜色变量） */
  tone: "danger" | null
  /** 该桶的重置倒计时（`5h 12m`），未知 null */
  reset: string | null
}

export interface QuotaBucketView {
  buckets: QuotaBucket[]
  /** 1 = 逐行，2 = 两列；桶数 > 2 时为 1 */
  columns: number
  /** 全区块最近一次重置倒计时（`5h 12m`），未知 null */
  reset: string | null
}

/**
 * 两种入口归一后的「原始桶」：只保留组视图需要的字段。
 * 倒计时两种形态二选一——DTO 给 `resetsAt`（ISO），`QuotaWindow` 给已经格式化好的
 * `resetText`；后者优先，避免把 `还剩 5h 12m` 反解析回时间再算一遍。
 */
interface BucketSource {
  key: string
  name: string | null | undefined
  usedPercent: number | null | undefined
  amountText: string | null | undefined
  remainingText: string | null | undefined
  resetsAt?: string | null
  resetText?: string | null
}

function clampPercent(value: number): number {
  return Math.min(100, Math.max(0, value))
}

function knownPercent(value: number | null | undefined): number | null {
  return value != null && Number.isFinite(value) ? clampPercent(value) : null
}

/**
 * 取一段文字里的第一个数量并按既有压缩函数收口。
 * 来源已经写成 `3M` / `12.5K` 时保留原单位，避免二次换算（与 `capsuleQuotaChips` 同一规则）。
 */
function firstQuantity(text: string | null | undefined): string | null {
  if (!text) return null
  const match = text.trim().match(/(\d[\d,]*(?:\.\d+)?)\s*([KkMmBb])?/)
  if (!match) return null
  const value = Number(match[1].replace(/,/g, ""))
  if (!Number.isFinite(value)) return null
  const unit = match[2]?.toUpperCase()
  return unit ? `${match[1]}${unit}` : compactQuotaNumber(value)
}

/** `0 / 3000000 token` → `0 / 3.0M`；没有 `/`、取不到数时返回 null（不猜分母）。 */
function usedOfTotal(amountText: string | null | undefined): string | null {
  if (!amountText) return null
  const parts = amountText.split("/")
  if (parts.length !== 2) return null
  const used = firstQuantity(parts[0])
  const total = firstQuantity(parts[1])
  if (used === null || total === null) return null
  return `${used} / ${total}`
}

/** `还剩 5h 12m` → `5h 12m`；`等待更新` 原样保留（它不是「没提供时间」）。 */
function stripCountdown(text: string | null | undefined): string | null {
  if (!text) return null
  return text.replace(/^还剩\s*/, "")
}

/** 一个桶的重置倒计时：来源已格式化就用它，否则按 ISO 时刻现算。 */
function resetOf(source: BucketSource, now: number): string | null {
  if (source.resetText) return stripCountdown(source.resetText)
  return stripCountdown(formatQuotaCountdown(source.resetsAt ?? null, now))
}

/**
 * 组装按桶视图。没有非套餐窗口时返回 null，调用方继续用原有表达。
 *
 * 顺序沿用后端给的 bucket 顺序（模型顺序），不按水位重排：同一份额度在灵动岛与主窗口
 * 里保持同一顺序；桶数超过 [`MAX_BUCKET_COLUMNS`] 时只改布局（逐行），不丢桶。
 */
function buildBucketView(sources: readonly BucketSource[], now: number): QuotaBucketView | null {
  if (sources.length === 0) return null

  const buckets = sources.map((source): QuotaBucket => {
    const percent = knownPercent(source.usedPercent)
    const usedTotal = usedOfTotal(source.amountText)
    const remaining = firstQuantity(source.remainingText)
    const shownPercent = percent === null ? null : Math.round(percent)
    return {
      key: source.key,
      name: source.name?.trim() || "—",
      value: shownPercent !== null ? `${shownPercent}%` : usedTotal ?? "—",
      usedTotal,
      remaining,
      percent,
      zero: shownPercent === 0,
      tone: shownPercent !== null && shownPercent >= 100 ? "danger" : null,
      reset: resetOf(source, now),
    }
  })

  // 区块头的重置时间取最近一次（桶的重置时刻可能不同）；全部解析失败时回落到
  // 第一个已知的倒计时（灵动岛的窗口只有格式化好的读数，没有 ISO 时刻可比大小）。
  const nextReset = sources
    .map((source) => (source.resetsAt ? Date.parse(source.resetsAt) : Number.NaN))
    .filter((time) => Number.isFinite(time))
    .sort((a, b) => a - b)[0]
  const reset = nextReset === undefined
    ? sources.map((source) => resetOf(source, now)).find((label) => label !== null) ?? null
    : stripCountdown(formatQuotaCountdown(new Date(nextReset).toISOString(), now))

  return {
    buckets,
    columns: buckets.length > MAX_BUCKET_COLUMNS ? 1 : buckets.length,
    reset,
  }
}

/** 主窗口入口：后端 DTO 窗口（带 `resets_at`）。 */
export function quotaBucketView(
  windows: readonly QuotaWindowDto[] | null | undefined,
  now = Date.now(),
): QuotaBucketView | null {
  const sources = (windows ?? [])
    .filter((window) => !isPlanWindowKey(window.key))
    .map((window): BucketSource => ({
      key: window.key,
      name: window.window_name,
      usedPercent: window.used_percent,
      amountText: window.amount_text,
      remainingText: window.remaining_text ?? null,
      resetsAt: window.resets_at,
    }))
  return buildBucketView(sources, now)
}

/**
 * 灵动岛入口：界面已经转好的 `QuotaWindow`。
 *
 * 它只带格式化后的 `resetCountdown`（`还剩 2h 9m`），没有 ISO 时刻，所以倒计时直接用现成读数；
 * 其余口径（百分比优先、压缩、未知不画条）与主窗口完全一致。
 */
export function quotaBucketViewFromWindows(
  windows: readonly QuotaWindow[] | null | undefined,
  now = Date.now(),
): QuotaBucketView | null {
  const sources = (windows ?? [])
    .filter((window) => !isPlanWindowKey(window.key))
    .map((window): BucketSource => ({
      key: window.key,
      name: window.windowName,
      usedPercent: window.usedPercent,
      amountText: window.usedText,
      remainingText: window.remainingText ?? null,
      resetText: window.resetCountdown,
    }))
  return buildBucketView(sources, now)
}
