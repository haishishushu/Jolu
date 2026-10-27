import type { ToastInput } from "./toastQueue"

export type BalanceNotice = { id: number; kind: "balance"; connection_id: string; connection_name: string; platform: string; remaining: number; currency: string; threshold: number }
export type TaskNotice = { id: number; kind: "task"; task_id: number; state: "resumed" | "completed" | "failed"; platform: string; task_name: string; reason?: string | null }
export type AppNotice = BalanceNotice | TaskNotice
export type NoticeTarget = { kind: "account"; id: string; nonce: number } | { kind: "task"; id: number; nonce: number }

export function parseAppNotice(value: unknown): AppNotice | null {
  if (!value || typeof value !== "object") return null
  const v = value as Record<string, unknown>
  const text = (key: string, max = 512) => typeof v[key] === "string" && (v[key] as string).trim().length > 0 && (v[key] as string).length <= max
  if (!Number.isSafeInteger(v.id) || (v.id as number) <= 0) return null
  if (v.kind === "balance" && text("connection_id", 128) && text("connection_name") && text("platform", 64)
    && typeof v.currency === "string" && /^[A-Z]{3}$/.test(v.currency)
    && typeof v.remaining === "number" && Number.isFinite(v.remaining)
    && typeof v.threshold === "number" && Number.isFinite(v.threshold) && v.threshold > 0 && v.remaining < v.threshold) return value as BalanceNotice
  if (v.kind === "task" && Number.isSafeInteger(v.task_id) && (v.task_id as number) > 0
    && ["resumed", "completed", "failed"].includes(v.state as string) && text("platform", 64) && text("task_name")
    && (v.reason == null || typeof v.reason === "string" && v.reason.length <= 2048)) return value as TaskNotice
  return null
}

export function formatNoticeAmount(value: number, currency: string): string {
  const number = (Object.is(value, -0) ? 0 : Math.abs(value)).toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })
  const symbol = currency === "USD" ? "$" : currency === "CNY" ? "¥" : ""
  return `${value < 0 ? "-" : ""}${symbol}${number}`
}

export function noticeToast(notice: AppNotice, open: { account: (id: string) => void | boolean; task: (id: number) => void | boolean }): ToastInput {
  if (notice.kind === "balance") return {
    scope: `balance:${notice.connection_id}`, tone: "warn", title: "余额偏低", balance: { remaining: notice.remaining, currency: notice.currency, threshold: notice.threshold, connectionName: notice.connection_name, platform: notice.platform },
    action: { label: "查看账号", onClick: () => open.account(notice.connection_id) },
  }
  const titles = { resumed: "任务已继续", completed: "任务已完成", failed: "任务失败" }
  return {
    tone: notice.state === "failed" ? "danger" : notice.state === "completed" ? "success" : "info",
    scope: `task:${notice.task_id}`, title: titles[notice.state],
    detail: notice.state === "resumed" ? `${notice.platform} 额度已恢复，「${notice.task_name}」正在原会话中继续。`
      : notice.state === "failed" ? `「${notice.task_name}」：${notice.reason || "请查看运行记录"}` : `「${notice.task_name}」已运行完成。`,
    action: { label: "查看任务", onClick: () => open.task(notice.task_id) },
  }
}
