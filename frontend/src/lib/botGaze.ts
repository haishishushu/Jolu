import type { BotExpr } from "./botExpression.ts"

/**
 * bot 眼睛的视线（设计说明 §6.2）。偏移单位是脸的直径比例。
 */
export interface Vec { x: number; y: number }

export const GAZE_MAX_X = 0.13
export const GAZE_MAX_Y = 0.1
/** 每帧向目标逼近的比例；减少动效时直接对准 */
export const GAZE_SMOOTH = 0.22

/** 睡着只轻微转动，贴边偷看与被拖动减弱 */
const AMP: Partial<Record<BotExpr, number>> = { sleep: 0.3, peek: 0.4, drag: 0.6 }
export const gazeAmp = (expr: BotExpr): number => AMP[expr] ?? 1

/** 表情自带的视线偏移：贴边偷看时眼睛挤向朝屏幕内的一侧 */
export function gazeBase(expr: BotExpr, edge: "top" | "bottom" | "left" | "right" | null): Vec {
  if (expr !== "peek" || !edge) return { x: 0, y: 0 }
  return edge === "left" ? { x: 0.2, y: 0 } : edge === "right" ? { x: -0.2, y: 0 }
    : edge === "top" ? { x: 0, y: 0.15 } : { x: 0, y: -0.15 }
}

/**
 * 视线目标 = 表情偏移 + 指向指针的偏移。
 * 指针离脸越远越接近最大偏移，达到最大值的距离为 max(80px, 脸宽 × 2.5)。
 * pointer 为 null（指针不在或位置未知）时只剩表情偏移。
 */
export function gazeTarget(
  face: { cx: number; cy: number; width: number },
  pointer: Vec | null,
  options: { base: Vec; amp: number },
): Vec {
  let { x, y } = options.base
  if (pointer && options.amp > 0) {
    const dx = pointer.x - face.cx, dy = pointer.y - face.cy
    const distance = Math.hypot(dx, dy)
    if (distance > 0.5) {
      const reach = Math.min(1, distance / Math.max(80, face.width * 2.5))
      x += (dx / distance) * GAZE_MAX_X * reach * options.amp
      y += (dy / distance) * GAZE_MAX_Y * reach * options.amp
    }
  }
  return { x, y }
}

/** 插值一步；与目标足够接近时直接落在目标上并标记已稳定 */
export function gazeStep(current: Vec, target: Vec, smooth: number): { next: Vec; settled: boolean } {
  const next = { x: current.x + (target.x - current.x) * smooth, y: current.y + (target.y - current.y) * smooth }
  const settled = Math.abs(target.x - next.x) < 0.0008 && Math.abs(target.y - next.y) < 0.0008
  return { next: settled ? { ...target } : next, settled }
}

export const gazeSmooth = (reduced: boolean): number => (reduced ? 1 : GAZE_SMOOTH)
