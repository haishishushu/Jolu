/** 主窗口操作提示的队列与计时：只接收显式时刻，便于确定性验证。 */
export type ToastTone = "success" | "info" | "warn" | "danger"
export type PauseReason = "pointer" | "focus"
export type ToastInput = {
  tone: ToastTone
  title: string
  detail?: string
  scope?: string
  persistent?: boolean
  action?: { label: string; onClick: () => void | boolean | Promise<void | boolean> }
  balance?: { remaining: number; currency: string; threshold: number; connectionName: string; platform: string }
}
export type ToastItem = ToastInput & {
  id: number
  count: number
  duration: number
  remaining: number
  startedAt: number | null
  paused: PauseReason[]
  exiting: boolean
}

export const TOAST_DURATION: Record<ToastTone, number> = {
  success: 3000, info: 3000, warn: 6000, danger: Infinity,
}
export const TOAST_EXIT_MS = 140
export const MAX_VISIBLE_TOASTS = 3

/** 退场中的卡片仍占一个显示位，动画结束后才让排队卡片补位。 */
export function visibleToasts(list: ToastItem[]): ToastItem[] {
  return list.slice(0, MAX_VISIBLE_TOASTS)
}

function activate(list: ToastItem[], now: number): ToastItem[] {
  return list.map((item, index) => index < MAX_VISIBLE_TOASTS && item.startedAt === null
    && !item.exiting && item.paused.length === 0 ? { ...item, startedAt: now } : item)
}

export function toastRemaining(item: ToastItem, now: number): number {
  return Math.max(0, item.remaining - (item.startedAt === null ? 0 : Math.max(0, now - item.startedAt)))
}

export function pushToast(list: ToastItem[], input: ToastInput, id: number, now: number): ToastItem[] {
  const duplicate = list.find(item => !item.exiting && item.scope === input.scope && item.tone === input.tone
    && item.title === input.title && item.detail === input.detail && item.action?.label === input.action?.label
    && item.persistent === input.persistent && JSON.stringify(item.balance) === JSON.stringify(input.balance))
  const duration = input.persistent ? Infinity : TOAST_DURATION[input.tone]
  if (duplicate) {
    return activate(list.map(item => item.id === duplicate.id ? {
      ...item, ...input, count: item.count + 1, remaining: duration, startedAt: null,
    } : item), now)
  }
  return activate([...list, {
    ...input, id, count: 1, duration, remaining: duration, startedAt: null, paused: [], exiting: false,
  }], now)
}

export function pauseToast(list: ToastItem[], id: number, reason: PauseReason, paused: boolean, now: number): ToastItem[] {
  return activate(list.map(item => {
    if (item.id !== id || item.exiting || item.paused.includes(reason) === paused) return item
    return {
      ...item, remaining: toastRemaining(item, now), startedAt: null,
      paused: paused ? [...item.paused, reason] : item.paused.filter(value => value !== reason),
    }
  }), now)
}

export function markToastExiting(list: ToastItem[], id: number): ToastItem[] {
  return list.map(item => item.id === id && !item.exiting ? { ...item, exiting: true } : item)
}

export function removeToast(list: ToastItem[], id: number, now: number): ToastItem[] {
  return activate(list.filter(item => item.id !== id), now)
}
