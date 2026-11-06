import type { QuotaWindow } from "../types.ts"
import type { RingGeometry } from "./providers/types.ts"
import { isPlanWindowKey, windowRingGeometry, windowSpec } from "./providers/registry.ts"

/**
 * 胶囊额度环：**哪些窗口参与环**（本文件前半）与**刻度环怎么摆**（后半）。
 *
 * 几何按窗口声明，不再按平台：`lib/providers/registry.ts` 的窗口条目里
 * `ringGeometry` 给出 `arc`（5h / 7d 订阅窗口）或 `ticks`（日额度 / 积分桶），
 * 未声明的 key 用平台缺省（ZCode 的 `bucket_id` 落到 `ticks`），`null` = 不画环。
 *
 * 这与 `planCoverage`（算不算套餐窗口）是两件事：
 * - `planCoverage` 回答「这条连接是不是套餐制」，仍然只认 5h / 7d；
 * - 这里回答「这个窗口要不要画环、画成哪种几何」，日额度桶也算。
 *
 * 三条硬语义：
 * - **未知不画环**（`usedPercent === null` 的窗口直接不参与），未知不等于 0；
 * - **真实 0% 照画**（弧段环留空轨道，刻度环 24 段全留轨道色 + 起点标记）；
 * - **超过 100% 按 100% 收口**，由 `Bot` 的既有 clamp 与 `tickedSegments` 一起保证。
 */

/** 一个环最多两个：外圈在前、内圈在后 */
export const MAX_RING_COUNT = 2

/** 环对应的额度窗口 + 它的几何 */
export interface QuotaRing {
  /** 环数据来源：后端额度窗口 */
  window: QuotaWindow
  /** 环几何（来自窗口声明，未声明的 key 用平台缺省） */
  geometry: RingGeometry
}

interface Candidate extends QuotaRing {
  /** 窗口在入参里的序号：同一水位时保持后端给的顺序，结果稳定 */
  index: number
}

/** 已用百分比是否可用（未知 / NaN 都不算数，不参与环） */
function knownPercent(window: QuotaWindow): number | null {
  const percent = window.usedPercent
  return typeof percent === "number" && Number.isFinite(percent) ? percent : null
}

/**
 * 水位排序：紧张的在先，未知排后，同水位保持入参顺序。
 * 未知的窗口在选择阶段就被剔除，这里保留「未知排后」是为了让排序本身也说得清语义。
 */
function byTension(a: Candidate, b: Candidate): number {
  const pa = knownPercent(a.window)
  const pb = knownPercent(b.window)
  if (pa === null || pb === null) {
    if (pa === pb) return a.index - b.index
    return pa === null ? 1 : -1
  }
  if (pa !== pb) return pb - pa
  return a.index - b.index
}

/** 订阅窗口的环序：周额度在外、5 小时在内（与改造前的内外圈一致） */
const PLAN_KIND_ORDER: Readonly<Record<string, number>> = { "weekly-7d": 0, "rolling-5h": 1 }

/**
 * 环序：订阅窗口在前（外圈），日额度 / 积分在后（内圈）；组内订阅按窗口长度、
 * 其余按水位从紧到松。
 */
function byRingOrder(a: Candidate, b: Candidate): number {
  const planA = isPlanWindowKey(a.window.key)
  const planB = isPlanWindowKey(b.window.key)
  if (planA !== planB) return planA ? -1 : 1
  if (planA) {
    const rankA = PLAN_KIND_ORDER[windowSpec(a.window.key)?.kind ?? ""] ?? 0
    const rankB = PLAN_KIND_ORDER[windowSpec(b.window.key)?.kind ?? ""] ?? 0
    if (rankA !== rankB) return rankA - rankB
  }
  return byTension(a, b)
}

/**
 * 选出参与环的窗口（最多 `MAX_RING_COUNT` 个，外圈在前）。
 *
 * - 可环化：窗口声明了 `ringGeometry`（未声明的 key 用平台缺省，`null` 则不画环）；
 * - 未知（没有已用百分比）不参与：不画环，也不画成 0%；
 * - 候选超过两个时按「最紧张优先」取（未知排后），再按环序排：订阅窗口在前、日额度在后。
 *
 * 例：同一连接上 5h 40% + 7d 88% + 日额度 95% → 取 7d（弧段环，外圈）+ 日额度（刻度环，内圈）。
 */
export function quotaRings(
  platform: string | null | undefined,
  windows: readonly QuotaWindow[] | null | undefined,
): QuotaRing[] {
  const seen = new Set<string>()
  const candidates: Candidate[] = []
  for (const [index, window] of (windows ?? []).entries()) {
    const geometry = windowRingGeometry(window.key, platform)
    if (geometry === null) continue
    if (knownPercent(window) === null) continue
    // 同一个 key 只认第一条（后端去重之外再兜一层，避免两个环画同一个窗口）
    if (seen.has(window.key)) continue
    seen.add(window.key)
    candidates.push({ window, geometry, index })
  }
  const chosen = [...candidates].sort(byTension).slice(0, MAX_RING_COUNT).sort(byRingOrder)
  return chosen.map(({ window, geometry }) => ({ window, geometry }))
}

// ---------------------------------------------------------------------------
// 刻度环几何（Jolu.pen A3-13 / A3-14）：24 段、每段 15°、半径＝所在环半径，
// 32px 外径 + 弧段环宽 2 时正好是「半径 15、线宽 1.44、段长 2.6、起点标记 1.6」。
// ---------------------------------------------------------------------------

/** 段数：一圈 24 段，每段 15° */
export const TICK_COUNT = 24

/** 刻度线宽 / 环宽 = 0.72（A3-13：弧段环 2 × 0.72 = 1.44） */
const TICK_THICKNESS_RATIO = 0.72
/** 段长 / 环宽 = 1.3（A3-13：2 × 1.3 = 2.6） */
const TICK_LENGTH_RATIO = 1.3
/** 起点标记半径 / 环宽 = 0.4（A3-13：2 × 0.4 = 0.8，直径 1.6） */
const TICK_MARKER_RADIUS_RATIO = 0.4
/** 起点标记圆心比环半径再靠近圆心 0.1 × 环宽（A3-13：15 − 0.2 = 14.8，即 32px 里的 (16, 1.2)） */
const TICK_MARKER_INSET_RATIO = 0.1

/**
 * 点亮段数：`0` → 0 段（全轨道），`62` → 15 段（0.62 × 24 = 14.88 四舍五入），
 * `100` → 24 段；超过 100 按 100 收口，负数按 0 收口；未知（`null` / NaN）不点亮。
 */
export function tickedSegments(percent: number | null | undefined): number {
  if (percent === null || percent === undefined || !Number.isFinite(percent)) return 0
  const clamped = Math.max(0, Math.min(100, percent))
  return Math.round((clamped / 100) * TICK_COUNT)
}

/** 一段刻度：`x` / `y` 为未旋转时的左上角，`angle` 为绕段中心旋转的度数 */
export interface TickSegment {
  /** 段中心 x（= 旋转中心） */
  cx: number
  /** 段中心 y（= 旋转中心） */
  cy: number
  /** 未旋转时的左上角 x */
  x: number
  /** 未旋转时的左上角 y */
  y: number
  /** 线宽（切向） */
  width: number
  /** 段长（径向） */
  height: number
  /**
   * 绕段中心旋转的度数：等于 12 点起顺时针角度 t，段的长轴因此沿半径、摆成放射状。
   * 画板写的是 `rotation: 180 − θ`（Pencil 的 x/y 摆放按逆时针旋转算，长轴沿半径）；
   * SVG 的正角是顺时针，等价的线方向就是 `t`（矩形对 180° 对称，两者是同一根长轴）。
   */
  angle: number
  /** 是否是已用段 */
  on: boolean
}

/**
 * 24 段刻度的位置与朝向。
 *
 * 与画板同一套算法：第 i 段的角度 `t = i × 15°`，段中心落在
 * `(c + R·sin t, c − R·cos t)`（12 点起顺时针），长轴绕段中心转到 `t`（放射状），
 * 于是段长方向沿半径、以 `R` 为圆心居中。`radius` 传所在环的半径，
 * 刻度环画在最内侧时自然缩进，不会盖住弧段环。
 */
export function tickRingSegments(args: {
  /** bot 的外径 */
  size: number
  /** 段中心所在半径 */
  radius: number
  /** 该环的线宽（弧段环宽；聚光时按 1.35 放大） */
  ringWidth: number
  /** 已用百分比；未知传 null（一段都不点亮，全轨道） */
  percent: number | null | undefined
}): TickSegment[] {
  const { size, radius, ringWidth, percent } = args
  const c = size / 2
  const thickness = ringWidth * TICK_THICKNESS_RATIO
  const length = ringWidth * TICK_LENGTH_RATIO
  const filled = tickedSegments(percent)
  const segments: TickSegment[] = []
  for (let i = 0; i < TICK_COUNT; i += 1) {
    const angleDeg = (i * 360) / TICK_COUNT
    const rad = (angleDeg * Math.PI) / 180
    const cx = c + radius * Math.sin(rad)
    const cy = c - radius * Math.cos(rad)
    segments.push({
      cx,
      cy,
      x: cx - thickness / 2,
      y: cy - length / 2,
      width: thickness,
      height: length,
      angle: angleDeg,
      on: i < filled,
    })
  }
  return segments
}

/** 12 点方向的起点标记（一个实心小圆），标出填充方向 */
export function tickRingMarker(args: { size: number; radius: number; ringWidth: number }) {
  const { size, radius, ringWidth } = args
  const c = size / 2
  // 收口到 3 位小数：A3-13 的 (16, 1.2) / r 0.8 不带上二进制的尾巴
  const round = (value: number) => Math.round(value * 1000) / 1000
  return {
    cx: round(c),
    cy: round(c - (radius - ringWidth * TICK_MARKER_INSET_RATIO)),
    r: round(ringWidth * TICK_MARKER_RADIUS_RATIO),
  }
}
