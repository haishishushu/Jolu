/** 只缓存当前窗口内的展示数据；30 秒过期、最多 24 项，返回页面后仍立即重新校验。 */
export function createQueryCache(limit = 24, ttl = 30_000) {
  const entries = new Map<string, { value: unknown; at: number }>()
  return {
    get<T>(key: string, now = Date.now()): T | null {
      const entry = entries.get(key)
      if (!entry) return null
      if (now - entry.at >= ttl) { entries.delete(key); return null }
      return entry.value as T
    },
    set(key: string, value: unknown, now = Date.now()) {
      entries.delete(key)
      entries.set(key, { value, at: now })
      while (entries.size > limit) entries.delete(entries.keys().next().value!)
    },
  }
}
export const queryCache = createQueryCache()
