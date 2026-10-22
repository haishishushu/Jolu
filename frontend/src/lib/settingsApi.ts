import { invoke, isTauri } from "./api"

/** 设置 · 数据与存储（画板 92）：请求记录与会话索引的条数 */
export interface StorageCountsDto {
  requests: number
  sessions: number
}

export const settingsApi = {
  storageCounts: () => invoke<StorageCountsDto>("storage_counts"),
  /** 当前应用版本；浏览器预览没有桌面运行时，返回 null 由界面显示「—」 */
  appVersion: async (): Promise<string | null> => {
    if (!isTauri) return null
    const { getVersion } = await import("@tauri-apps/api/app")
    return getVersion()
  },
}
