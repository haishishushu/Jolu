import type { QuotaStateDto } from "./api.ts"
import type { PlatformId } from "../types.ts"
import { providerDescriptor } from "./providers/registry.ts"

export interface RemainingRow {
  label: string
  value: string
  usedText: string | null
  resetsAt: string | null
}

export interface RemainingDisplay {
  title: string
  source: string
  rows: RemainingRow[]
  state: "ok" | "loading" | "unsupported" | "forbidden" | "unauthorized" | "rate_limited" | "failed"
  reason: string
}

/**
 * 未知剩余的占位符：半角连字符 `-`。
 * - 和「真实的 0」严格分开：`0` 表示确实没有剩余，`-` 表示查询成功但没有拿到数值。
 * - 和查询失败 / 能力不支持 / 需要重新登录分开：那几种走 `state` 与 `reason`，不会变成 `-`。
 * - Grok 路径仍用「—」；两处占位符有意不同，避免把「没接进来」和「数值未知」混成同一个符号。
 *   两个符号都声明在 `lib/providers/registry.ts` 的 `labels.remaining.unknownText` 里，
 *   这里的常量只是给界面与测试引用的同一份字面量。
 */
export const UNKNOWN_REMAINING = "-"

function percentage(value: number): string {
  const remaining = Math.round((100 - Math.min(100, Math.max(0, value))) * 10) / 10
  return `${remaining}%`
}

const hasPercent = (value: number | null | undefined): value is number => value != null && Number.isFinite(value)

/**
 * 整句剩余额度的文案与状态。
 *
 * 标题、来源、加载文案、窗口行标签、未知占位与「算不算查询成功」的口径全部来自
 * `lib/providers/registry.ts` 的描述符；这里不再按平台分支。
 */
export function remainingDisplay(platform: PlatformId, kind: "auth" | "api", quota: QuotaStateDto | null): RemainingDisplay {
  const remaining = providerDescriptor(platform)?.labels.remaining
  // 未接入额度描述符的平台：沿用改动前的兜底文案
  if (!remaining) {
    return { title: "剩余额度", source: "在线账号", rows: [], state: "unsupported", reason: "当前未接入在线剩余额度查询。" }
  }
  // 确认查不到在线剩余额度（本机只有消耗记录 / 缺凭证 / 权限不够）：API 与 Auth 同一套说法
  if (remaining.kind === "unsupported") {
    return { title: remaining.title, source: remaining.source, rows: [], state: "unsupported", reason: remaining.reason }
  }
  const { title, source, unknownText, preferRemainingText } = remaining
  // API 连接的额度与订阅额度严格分开，不拿订阅口径顶替
  if (kind === "api") {
    return { title: remaining.api.title, source: remaining.api.source, rows: [], state: "unsupported", reason: remaining.api.reason }
  }
  const base = { title, source }
  if (!quota) return { ...base, rows: [], state: "loading", reason: remaining.loadingReason }
  if (quota.state !== "ok") return { ...base, rows: [], state: quota.state, reason: quota.reason }
  const rows = quota.windows.map((window) => {
    // 来源直接给精确剩余量时用它；Grok 只认已用百分比，不吃 remaining_text
    const exact = preferRemainingText ? window.remaining_text : null
    // 后端 `login_state` 兜底时三个数值字段都是 None：窗口在、数值未知，如实显示未知占位符。
    // 真实剩余为 0 时后端给的是 "0"，原样透出，不参与下面的未知分支。
    return {
      label: remaining.windowLabels[window.key] ?? remaining.windowLabelFallback,
      value: exact ?? (hasPercent(window.used_percent) ? percentage(window.used_percent) : unknownText),
      usedText: window.amount_text ?? (hasPercent(window.used_percent) ? `已用 ${window.used_percent}%` : null),
      resetsAt: window.resets_at,
    }
  })
  // ZCode 看「有没有窗口」：值可能是真实的 0、也可能未知 `-`，都照实显示。
  // Grok 看「有没有数」：全部窗口都拿不到已用百分比时才算不支持。
  // 查询失败与需要重新登录已在上面的 `quota.state !== "ok"` 返回，不会走到这里被改写成占位符。
  const ok = remaining.okWhenValueUnknown ? rows.length > 0 : rows.some((row) => row.value !== unknownText)
  return { ...base, rows, state: ok ? "ok" : "unsupported", reason: ok ? "" : remaining.emptyReason }
}
