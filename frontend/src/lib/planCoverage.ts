/**
 * 判断当前连接是不是套餐制。
 *
 * 口径：额度接口返回了 5 小时或周窗口的，就是套餐（官方订阅、sub2api 网关、
 * 各家 Coding Plan 都走这个形态）；查不到这两个窗口的按 API Key 按量计费处理。
 */

/** 只有 5h 与 7d 代表「5 小时 / 周额度」；月额度、积分额度是别的形态，不算 */
const PLAN_WINDOW_KEYS = ["5h", "7d"]

/** 参数放宽为只读 key：额度窗口在 DTO、后端映射、界面类型里字段名不同，套餐判定只认 key */
export function isPlanCovered(windows: readonly { key: string }[] | null | undefined): boolean {
  return Boolean(windows?.some((window) => PLAN_WINDOW_KEYS.includes(window.key)))
}

