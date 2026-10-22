/**
 * 额度水位着色 —— 口径改为「剩余」
 *
 * 剩余 > 25%        绿 success    正常
 * 剩余 11% – 25%    琥珀 warn     接近上限
 * 剩余 <= 10%       红 danger     即将耗尽 / 已耗尽（0%）
 *
 * 阈值与原「已用 75%/90%」完全等价，只是展示口径换成剩余。
 * 着色只作用于进度条填充；周期标签（5h 淡紫 / 7d 淡绿）与百分比文字颜色不变。
 */
export type QuotaLevel = "normal" | "warning" | "critical"

export const QUOTA_WARN_THRESHOLD = 25
export const QUOTA_CRITICAL_THRESHOLD = 10

/** 已用 → 剩余（与后端 tray_summary::remaining_percent 同口径） */
export function remainingPercent(usedPercent: number): number {
  return Math.round((100 - Math.min(100, Math.max(0, usedPercent))) * 10) / 10
}

export function quotaLevel(remaining: number): QuotaLevel {
  if (remaining <= QUOTA_CRITICAL_THRESHOLD) return "critical"
  if (remaining <= QUOTA_WARN_THRESHOLD) return "warning"
  return "normal"
}

/** 进度条填充色的 class */
export function quotaBarClass(remaining: number): string {
  switch (quotaLevel(remaining)) {
    case "critical":
      return "bg-danger"
    case "warning":
      return "bg-warn"
    default:
      return "bg-success"
  }
}

