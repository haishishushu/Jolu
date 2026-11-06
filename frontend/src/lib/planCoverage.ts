import { isPlanWindowKey } from "./providers/registry.ts"

/**
 * 判断当前连接是不是套餐制。
 *
 * 口径：额度接口返回了描述符声明为订阅窗口（`countsAsPlan`）的窗口，就是套餐
 * （官方订阅、sub2api 网关、各家 Coding Plan 都走这个形态）；查不到这些窗口的按
 * API Key 按量计费处理。窗口 key 的唯一出处是 `lib/providers/registry.ts`：
 * 当前仍是 5 小时 / 周两个窗口，日额度、月额度、积分额度都不算。
 */

/** 参数放宽为只读 key：额度窗口在 DTO、后端映射、界面类型里字段名不同，套餐判定只认 key */
export function isPlanCovered(windows: readonly { key: string }[] | null | undefined): boolean {
  return Boolean(windows?.some((window) => isPlanWindowKey(window.key)))
}
