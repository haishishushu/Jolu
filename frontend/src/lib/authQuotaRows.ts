import type { QuotaWindow } from "../types"
import { PLACEHOLDER_WINDOWS, isPlaceholderWindowKey, providerDescriptor } from "./providers/registry.ts"

/**
 * Auth 保留额度布局，未知值不能伪装成 API 指标或 0%。
 *
 * 占位行的名字、配色与「哪些 key 是命名窗口」都在 `lib/providers/registry.ts`：
 * 当前补的是 5h / 7d 两行。
 */
export function authQuotaRows(windows?: QuotaWindow[], missingText: string | null = null): QuotaWindow[] {
  const defaults: QuotaWindow[] = PLACEHOLDER_WINDOWS.map((empty) => ({
    key: empty.key,
    windowName: empty.name,
    usedPercent: null,
    usedText: missingText,
    resetCountdown: null,
    badgeTone: empty.badgeTone,
  }))
  if (missingText === null && windows?.some(w => !isPlaceholderWindowKey(w.key))) return windows.slice(0, 2)
  return defaults.map(empty => windows?.find(w => w.key === empty.key) ?? empty)
}

/**
 * 展示用额度行（胶囊与经典岛共用）。
 *
 * 描述符声明了 `placeholderWindows: false` 的来源（Grok）直接用后端原始窗口：
 * 它只有周 / 月 / 积分窗口，补 5h / 7d 占位会凭空多出一条「5 小时」。
 * 其余平台沿用 `authQuotaRows` 的占位补全。
 */
export function quotaRowsFor(platform: string, windows?: QuotaWindow[]): QuotaWindow[] {
  return providerDescriptor(platform)?.placeholderWindows === false ? windows ?? [] : authQuotaRows(windows)
}
