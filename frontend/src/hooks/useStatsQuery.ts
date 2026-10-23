import { useEffect } from "react"
import { isTauri } from "@/lib/api"
import { onUsageChanged } from "@/lib/usageChanges"
import { useLiveQuery, type LiveQuery } from "./useLiveQuery"

export type StatsQuery<T> = LiveQuery<T>

/**
 * 用量统计与会话线程页的查询：在 [`useLiveQuery`] 之上，本机用量有变化时自动补刷新。
 */
export function useStatsQuery<T>(key: string, load: () => Promise<T>, demo: () => T, platform: string | null, enabled = true): StatsQuery<T> {
  const query = useLiveQuery(key, load, demo, enabled)
  const { revalidate: refresh } = query

  useEffect(() => {
    if (!enabled || !isTauri) return
    return onUsageChanged(platform ?? "all", refresh)
  }, [platform, refresh, enabled])

  return query
}
