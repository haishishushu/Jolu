import { useEffect, useState } from "react"
import { api, isTauri } from "./api"

/**
 * 费用币种换算（模块 C 扩展）：成本基准是 USD，展示时按当日汇率换算。
 *
 * 汇率由后端拉取（open.er-api.com，24 小时缓存）；失败时后端返回 fallback=true
 * 的内置参考价——界面如实标注「参考汇率」，不把失败伪装成最新价。
 */

export type CostCurrency = "USD" | "CNY"

export interface FxState {
  currency: CostCurrency
  /** 1 USD 兑目标币种；USD 时恒为 1 */
  rate: number
  /** 汇率来源说明（日期或「内置参考价」） */
  asOf: string
  fallback: boolean
  loading: boolean
}

const EMPTY: FxState = { currency: "USD", rate: 1, asOf: "基准币种", fallback: false, loading: false }

/** 按设置币种拉汇率；USD 恒为基准，不发请求。 */
export function useCostFx(currency: CostCurrency): FxState {
  const [state, setState] = useState<FxState>(() =>
    currency === "USD" ? EMPTY : { currency, rate: 1, asOf: "读取中…", fallback: false, loading: true },
  )

  useEffect(() => {
    if (currency === "USD" || !isTauri) {
      setState(EMPTY)
      return
    }
    let stopped = false
    setState({ currency, rate: 1, asOf: "读取中…", fallback: false, loading: true })
    api
      .fxRate(currency)
      .then((dto) => {
        if (!stopped) {
          setState({ currency, rate: dto.rate, asOf: dto.as_of, fallback: dto.fallback, loading: false })
        }
      })
      .catch(() => {
        if (!stopped) {
          setState({ currency, rate: 1, asOf: "基准币种", fallback: false, loading: false })
        }
      })
    return () => {
      stopped = true
    }
  }, [currency])

  return state
}

/** 把 USD 金额换算到目标币种。rate=1（USD）时原值返回。
 * 纯函数实现放在 `usageStats.ts`（无 React 依赖，便于 `node --test` 直接验证）。 */
export { convertFromUsd } from "./usageStats"
