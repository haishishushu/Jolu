/**
 * 主面板 Toast 队列的纯逻辑。
 *
 * 这里只算「队列该变成什么样」，不碰 DOM、不设计时器：
 * 计时交给 ToastProvider 的 effect，退场动画交给 CSS。
 * 好处是替换、重复提示续时这些容易出错的规则可以直接单测。
 */

export type ToastTone = "success" | "info" | "warn" | "danger"

export type ToastInput = {
  tone: ToastTone
  title: string
  /** 失败原因等补充信息，单独一行小字 */
  detail?: string
}

export type ToastItem = {
  id: number
  tone: ToastTone
  title: string
  detail?: string
  duration: number
  /** 计时起点；每次新提示或重复提示均重新计时 */
  createdAt: number
  /** 退场动画期间仍留在列表里，动画跑完才真正移除 */
  exiting: boolean
}

/** 所有状态统一显示 5 秒。 */
export const TOAST_DURATION: Record<ToastTone, number> = {
  success: 5000,
  info: 5000,
  warn: 5000,
  danger: 5000,
}

/** 退场动画时长，与 index.css 的 .motion-toast[data-state="closed"] 同一数值 */
export const TOAST_EXIT_MS = 140

/** 未进入退场的条目，即视觉上真正占位的那些 */
export function visibleToasts(list: ToastItem[]): ToastItem[] {
  return list.filter((item) => !item.exiting)
}

function sameContent(item: ToastItem, input: ToastInput): boolean {
  return item.tone === input.tone && item.title === input.title && item.detail === input.detail
}

/** 新提示立即替换旧提示；同内容复用节点并重置计时，避免闪烁。 */
export function pushToast(list: ToastItem[], input: ToastInput, id: number, now: number): ToastItem[] {
  const duplicate = list.find((item) => !item.exiting && sameContent(item, input))
  return [{
    id: duplicate?.id ?? id,
    tone: input.tone,
    title: input.title,
    detail: input.detail,
    duration: TOAST_DURATION[input.tone],
    createdAt: now,
    exiting: false,
  }]
}

/** 转入退场：超时与程序关闭共用这一条路径 */
export function markToastExiting(list: ToastItem[], id: number): ToastItem[] {
  return list.map((item) => (item.id === id && !item.exiting ? { ...item, exiting: true } : item))
}

/** 退场动画结束后真正移除 */
export function removeToast(list: ToastItem[], id: number): ToastItem[] {
  return list.filter((item) => item.id !== id)
}

/** 还要停留多久 */
export function toastRemaining(item: ToastItem, now: number): number {
  return Math.max(0, item.createdAt + item.duration - now)
}
