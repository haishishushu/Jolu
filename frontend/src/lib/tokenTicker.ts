/**
 * bot 的「眼睛 ⇄ Token」自动轮播（设计说明 §6.3）。
 * 只描述节拍：首轮先看眼睛 2s，之后 Token 3s、眼睛 4s 循环。
 */
export type TickerFace = "eyes" | "tick"

export const TICKER_FIRST_MS = 2000
export const TICKER_TICK_MS = 3000
export const TICKER_EYES_MS = 4000

export interface TickerStep { face: TickerFace; delay: number }

/** 开始（或暂停后恢复）轮播：总是从眼睛开始，先停留 2s */
export const tickerStart = (): TickerStep => ({ face: "eyes", delay: TICKER_FIRST_MS })

/** 当前面停留结束后翻到另一面，delay 是新一面的停留时长 */
export const tickerNext = (face: TickerFace): TickerStep =>
  face === "eyes" ? { face: "tick", delay: TICKER_TICK_MS } : { face: "eyes", delay: TICKER_EYES_MS }

/**
 * 小脸放不下完整数字时的短格式，最多 4 个字符：860 / 8.2k / 17k / 999k / 1.2M / 12M。
 * 先按舍入后的值选单位，避免 999,600 被写成 1000k。
 */
export function formatShort(n: number): string {
  const value = Math.max(0, Math.round(n))
  if (value < 1000) return String(value)
  const units: [number, string][] = [[1000, "k"], [1_000_000, "M"], [1_000_000_000, "B"]]
  for (const [size, suffix] of units) {
    const scaled = value / size
    if (scaled >= 1000 && suffix !== "B") continue
    const decimal = scaled.toFixed(1)
    // 保留一位小数只在个位数时；舍入后进位到 10.0 或 1000 就改用整数或上一级单位
    if (scaled < 10 && decimal !== "10.0") return `${decimal}${suffix}`
    const whole = Math.round(scaled)
    if (whole < 1000 || suffix === "B") return `${whole}${suffix}`
  }
  return String(value)
}
