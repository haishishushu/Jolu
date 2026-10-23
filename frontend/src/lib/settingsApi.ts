import { invoke, isTauri, type AppSettings } from "./api"
import type { DefaultPage, MainPage } from "./mainNavigation"

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
  /** 设置 · 常规（画板 86）：默认打开页面 */
  setDefaultPage: (page: DefaultPage) => invoke<AppSettings>("set_default_page", { page }),
  /** 只写入上次访问的页面，不广播设置变更 */
  rememberPage: (page: MainPage) => invoke<void>("remember_page", { page }),
}
