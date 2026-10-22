import { useCallback, useEffect, useRef, useState } from "react"
import { isTauri } from "@/lib/api"

export interface LiveQuery<T> {
  data: T | null
  /** 首次加载没有数据时显示骨架；再次加载保留上次数据 */
  loading: boolean
  error: string | null
  refresh: () => void
}

/**
 * 主窗口各页共用的查询：`key` 变化即重新查询，过期响应直接丢弃。
 * `refresh` 在请求进行中只记一次补刷新，等当前请求返回后再查，
 * 避免后台数据持续变化时新请求不断顶掉旧请求、页面一直停在加载中。
 * 浏览器预览直接用示例数据；桌面端出错不回落示例。
 */
export function useLiveQuery<T>(key: string, load: () => Promise<T>, demo: () => T, enabled = true): LiveQuery<T> {
  const [data, setData] = useState<T | null>(null)
  const [loading, setLoading] = useState(isTauri)
  const [error, setError] = useState<string | null>(null)
  const [tick, setTick] = useState(0)
  const loadRef = useRef(load)
  const demoRef = useRef(demo)
  loadRef.current = load
  demoRef.current = demo
  const revision = useRef(0)
  const inFlight = useRef(false)
  const queued = useRef(false)

  useEffect(() => {
    if (!enabled) return
    const current = ++revision.current
    if (!isTauri) {
      setData(demoRef.current())
      setLoading(false)
      return
    }
    inFlight.current = true
    queued.current = false
    setLoading(true)
    loadRef.current()
      .then((value) => {
        if (current !== revision.current) return
        setData(value)
        setError(null)
      })
      .catch((reason: unknown) => {
        if (current !== revision.current) return
        setError(reason instanceof Error ? reason.message : String(reason))
      })
      .finally(() => {
        if (current !== revision.current) return
        inFlight.current = false
        setLoading(false)
        if (queued.current) {
          queued.current = false
          setTick((value) => value + 1)
        }
      })
  }, [key, tick, enabled])

  const refresh = useCallback(() => {
    if (inFlight.current) queued.current = true
    else setTick((value) => value + 1)
  }, [])

  return { data, loading, error, refresh }
}
