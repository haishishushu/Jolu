import { invoke, isTauri, type AppSettings } from "./api"
import type { DefaultPage, MainPage } from "./mainNavigation"
import type { Density } from "./appearance"
import type { NetworkDiagnosisDto, NetworkMode } from "./networkSettings"

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
  /** 设置 · 外观（画板 88）：界面缩放、列表密度、减少动效；缩放由后端应用到主窗口 */
  setAppearancePreferences: (zoom: number, density: Density, reduceMotion: boolean) =>
    invoke<AppSettings>("set_appearance_preferences", { zoom, density, reduceMotion }),
  /** 设置 · 网络与代理（画板 90 / 102）：保存应用联网方式；非手动代理传 null 保留上次的代理地址 */
  setNetwork: (mode: NetworkMode, proxyUrl: string | null) => invoke<AppSettings>("set_network", { mode, proxyUrl }),
  /** 用草稿参数做连接诊断，不保存设置；不发模型请求、不带凭证 */
  diagnoseNetwork: (mode: NetworkMode, proxyUrl: string | null) => invoke<NetworkDiagnosisDto>("diagnose_network", { mode, proxyUrl }),
  /** 设置 · 通知（画板 94）：任务提醒的两个开关，传 null 的保持不变 */
  setTaskNotify: (resumed: boolean | null, finished: boolean | null) => invoke<AppSettings>("set_task_notify", { resumed, finished }),
}
