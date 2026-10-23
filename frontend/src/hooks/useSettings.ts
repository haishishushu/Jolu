import { useCallback, useRef, useState, useSyncExternalStore } from "react"
import { api, isTauri, listenEvent, type AppSettings } from "@/lib/api"
import type { DefaultPage } from "@/lib/mainNavigation"
import { settingsApi } from "@/lib/settingsApi"
import { normalizeDensity } from "@/lib/appearance"
import type { NetworkMode } from "@/lib/networkSettings"

/**
 * 持久化设置（§2.6 / §7.3）
 *
 * 灵动岛平台与免打扰在托盘菜单和设置界面里必须是**同一个值**，
 * 因此这里不各存一份：Rust 持有真值，前端读它、改它，并监听变更事件。
 */
const DEFAULTS: AppSettings = {
  silent_startup: true,
  island_platform: "claude",
  island_kind: "api",
  island_connection_id: null,
  island_connection_name: null,
  island_source_id: "local:claude",
  always_on_top: true,
  theme: "light",
  island_opacity: 100,
  island_scale: 100,
  island_shrink_scale: 100,
  refresh_minutes: 5,
  balance_alert_threshold: null,
  balance_alert_currency: "USD",
  dock_enabled: true,
  retention_days: null,
  dock: { edge: null, offset: 0, monitor: null },
  dnd: false,
  island_visible: true,
  island_clones: [],
  proxy_enabled: false,
  proxy_port: 12731,
  proxy_fallback_direct: true,
  proxy_claude_upstream: null,
  proxy_codex_upstream: null,
  default_page: "last",
  last_page: null,
  main_zoom: 100,
  list_density: "comfortable",
  reduce_motion: false,
  network_mode: "system",
  network_proxy_url: null,
  notify_task_resumed: true,
  notify_task_finished: true,
}

interface SharedSettings {
  settings: AppSettings
  ready: boolean
  loading: boolean
  error: string | null
}

/**
 * 每个窗口只保留一份设置快照：额度、余额、用量等 hook 都要读刷新间隔，
 * 各自读取和监听会让一个灵动岛窗口同时挂着六七份相同的订阅与初始请求。
 */
let shared: SharedSettings = { settings: DEFAULTS, ready: !isTauri, loading: isTauri, error: null }
/** 每收到一次推送递增；读取或保存期间若有更新的推送，较旧的响应不能覆盖它。 */
let revision = 0
let subscribed = false
const listeners = new Set<() => void>()

function publish(patch: Partial<SharedSettings>) {
  shared = { ...shared, ...patch }
  listeners.forEach((listener) => listener())
}

async function reloadShared() {
  if (!isTauri) return
  const started = revision
  publish({ loading: true })
  try {
    const next = await api.getSettings()
    if (revision === started) publish({ settings: next, error: null, ready: true })
  } catch (reason) { publish({ error: `设置读取失败：${String(reason)}` }) }
  finally { publish({ loading: false }) }
}

function ensureSubscribed() {
  if (subscribed || !isTauri) return
  subscribed = true
  // 先订阅再读初值；读取期间收到的新快照不能被较旧的响应覆盖。
  void listenEvent<AppSettings>("settings-changed", (next) => {
    revision++
    publish({ settings: next, ready: true })
  }).then(() => reloadShared()).catch((reason) => {
    subscribed = false
    publish({ error: `设置同步失败：${String(reason)}`, loading: false })
  })
}

function subscribe(listener: () => void) {
  ensureSubscribed()
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

// 串行执行不同设置，避免组合配置的读改写互相覆盖。
let saveQueue: Promise<unknown> = Promise.resolve()
const snapshot = () => shared

export function useSettings() {
  const state = useSyncExternalStore(subscribe, snapshot)
  const [pending, setPending] = useState<Set<string>>(new Set())
  const saving = pending.size > 0
  const isSaving = (key: string) => pending.has(key)
  const [saveError, setSaveError] = useState<string | null>(null)
  const busy = useRef(new Set<string>())

  const save = useCallback(async (key: string, action: () => Promise<AppSettings>) => {
    if (busy.current.has(key)) return false
    busy.current.add(key)
    setPending(new Set(busy.current))
    setSaveError(null)
    const operation = saveQueue.then(async () => {
      const started = revision
      try {
        const next = await action()
        if (revision === started) publish({ settings: next })
        return true
      } catch (reason) {
        setSaveError(`设置保存失败：${String(reason)}`)
        return false
      }
    })
    saveQueue = operation
    try { return await operation }
    finally {
      busy.current.delete(key)
      setPending(new Set(busy.current))
    }
  }, [])

  const setIslandPlatform = useCallback((platform: string) => save("island_platform", () => api.setIslandPlatform(platform)), [save])
  const setIslandConnection = useCallback((id: string | null, island: string | null = null) => save("island_connection", () => api.setIslandConnection(id, island)), [save])
  const setIslandSource = useCallback((id: string | null) => save("island_source", () => api.setIslandSource(id)), [save])
  const setDnd = useCallback((on: boolean) => save("dnd", () => api.setDnd(on)), [save])
  const setDockEnabled = useCallback((on: boolean) => save("dock_enabled", () => api.setDockEnabled(on)), [save])
  const setTopmost = useCallback((on: boolean) => save("always_on_top", () => api.islandTopmost(on)), [save])
  const setIslandVisible = useCallback((visible: boolean) => save("island_visible", () => api.setIslandVisibility(visible)), [save])
  const setSilentStartup = useCallback((on: boolean) => save("silent_startup", () => api.setSilentStartup(on)), [save])
  const setProxy = useCallback(
    (port: number, enabled: boolean, fallbackDirect: boolean) => save("proxy", () => api.setProxy(port, enabled, fallbackDirect)),
    [save],
  )
  const setTheme = useCallback((theme: "light" | "dark" | "system") => save("theme", () => api.setTheme(theme)), [save])
  const setRetentionDays = useCallback((days: number | null) => save("retention_days", () => api.setRetentionDays(days)), [save])
  const setDisplayPreferences = useCallback((patch: Partial<Pick<AppSettings, "island_opacity" | "island_scale" | "island_shrink_scale" | "refresh_minutes">>) => save(Object.keys(patch).sort().join(","), async () => {
    const next = { ...await api.getSettings(), ...patch }
    return api.setDisplayPreferences(next.island_opacity, next.island_scale, next.island_shrink_scale, next.refresh_minutes)
  }), [save])
  const setDefaultPage = useCallback((page: DefaultPage) => save("default_page", () => settingsApi.setDefaultPage(page)), [save])
  const setAppearance = useCallback((patch: Partial<Pick<AppSettings, "main_zoom" | "list_density" | "reduce_motion">>) => save(Object.keys(patch).sort().join(","), async () => {
    const next = { ...await api.getSettings(), ...patch }
    return settingsApi.setAppearancePreferences(next.main_zoom, normalizeDensity(next.list_density), next.reduce_motion)
  }), [save])
  /** 成功返回保存后的设置（手动代理地址已由后端规整），失败返回 null */
  const setNetwork = useCallback(async (mode: NetworkMode, proxyUrl: string | null) => {
    let saved = null as AppSettings | null
    const ok = await save("network", async () => (saved = await settingsApi.setNetwork(mode, proxyUrl)))
    return ok ? saved : null
  }, [save])

  /** 设置 · 通知（画板 94）：任务提醒开关，只改传入的那一个 */
  const setTaskNotify = useCallback((patch: { resumed?: boolean; finished?: boolean }) => save(patch.resumed !== undefined ? "notify_task_resumed" : "notify_task_finished", () => settingsApi.setTaskNotify(patch.resumed ?? null, patch.finished ?? null)), [save])

  const { settings, ready, loading } = state
  const error = saveError ?? state.error
  return { settings, ready, loading, saving, isSaving, error, reload: reloadShared, setIslandPlatform, setIslandConnection, setIslandSource, setDnd, setDockEnabled, setTopmost, setIslandVisible, setSilentStartup, setProxy, setTheme, setRetentionDays, setDisplayPreferences, setDefaultPage, setAppearance, setNetwork, setTaskNotify }
}

