import { useEffect, useState } from "react"
import { isTauri, listenEvent } from "@/lib/api"
import { AUTOMATION_CHANGED } from "@/lib/automationApi"
import { useLiveQuery, type LiveQuery } from "./useLiveQuery"

export type AutomationQuery<T> = LiveQuery<T>

/**
 * 自动化页的查询：在 [`useLiveQuery`] 之上，后端发出 `automation-changed` 时补刷新，
 * `pollMs` 大于 0 时定时刷新（执行中的日志）。
 */
export function useAutomationQuery<T>(key: string, load: () => Promise<T>, demo: () => T, enabled = true, pollMs = 0): AutomationQuery<T> {
  const query = useLiveQuery(key, load, demo, enabled)
  const { refresh } = query

  useEffect(() => {
    if (!enabled || !isTauri) return
    let off: (() => void) | null = null
    let stopped = false
    void listenEvent(AUTOMATION_CHANGED, refresh).then((unlisten) => {
      if (stopped) unlisten()
      else off = unlisten
    }).catch(() => {})
    return () => {
      stopped = true
      off?.()
    }
  }, [enabled, refresh])

  useEffect(() => {
    if (!enabled || !isTauri || pollMs <= 0) return
    const timer = window.setInterval(refresh, pollMs)
    return () => window.clearInterval(timer)
  }, [enabled, pollMs, refresh])

  return query
}

/** 每秒刷新的当前时间，供执行中 / 等待中的计时显示 */
export function useNow(intervalMs = 1000, enabled = true): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!enabled) return
    const timer = window.setInterval(() => setNow(Date.now()), intervalMs)
    return () => window.clearInterval(timer)
  }, [intervalMs, enabled])
  return now
}
