import { useContentMotion, useExitPresence } from "@/lib/motion"
import { useCallback, useEffect, useId, useRef, useState } from "react"
import { api, isTauri, listenEvent, openUrl, type MainIntentDto } from "@/lib/api"
import { pageForIntent, pageLabel, readSidebarCollapsed, resolveOpenPage, writeSidebarCollapsed, type MainPage } from "@/lib/mainNavigation"
import { settingsApi } from "@/lib/settingsApi"
import { applyAppearance, normalizeDensity } from "@/lib/appearance"
import { useSettings } from "@/hooks/useSettings"
import { AppIcon } from "@/components/brand/AppIcon"
import { ToastProvider, ToastViewport } from "@/components/ui/Toast"
import { TipProvider } from "@/components/ui/Tip"
import { PANEL_HEIGHT, PANEL_WIDTH, TitleBar } from "@/components/panel/Chrome"
import { PageHeader, REPO_URL } from "@/components/panel/PageHeader"
import { dateKey } from "@/lib/usageStats"
import { SideNav } from "@/components/panel/SideNav"
import { UpdateProvider } from "@/components/panel/UpdateContext"
import { UpdateDialog } from "@/components/panel/UpdateDialog"
import { IslandView } from "./IslandView"
import { ProvidersView } from "./ProvidersView"
import { SessionsView, type SessionTarget } from "./SessionsView"
import { AutomationView, type AutomationPrefill, type AutomationTarget } from "./AutomationView"
import { SkillsView } from "./SkillsView"
import { UsageView, type UsageRequest } from "./UsageView"
import { SettingsPage } from "./SettingsPage"

/**
 * 主面板窗口（Jolu.pen 模块 A 画板 01–04）：30px 系统标题栏 + 左侧导航 + 内容区。
 * 标题栏、侧栏与内容页头固定，只有主内容区滚动。关闭只隐藏窗口，后台采集继续运行。
 */
export function MainPanelWindow({
  initialPage = "island",
  /** true = 作为真实桌面窗口铺满，不要预览用的固定尺寸与投影 */
  embedded = false,
}: {
  initialPage?: MainPage
  embedded?: boolean
}) {
  const [page, setPage] = useState<MainPage>(initialPage)
  const [collapsed, setCollapsed] = useState(() => readSidebarCollapsed())
  const [navigationRequest, setNavigationRequest] = useState(0)
  /** 托盘 / 菜单要定位的设置分区（旧分区 id，由设置页换算成新分类） */
  const [settingsSection, setSettingsSection] = useState<string | null>(null)
  const [about, setAbout] = useState<string | null>(null)
  const [panelActive, setPanelActive] = useState(true)
  /** 页面之间的定位跳转：用量统计 → 指定会话，会话 → 该会话的调用明细 */
  const [sessionTarget, setSessionTarget] = useState<SessionTarget | null>(null)
  /** 会话线程「创建自动化」带入自动化新建页的会话（画板 72） */
  const [automationPrefill, setAutomationPrefill] = useState<AutomationPrefill | null>(null)
  const [automationTarget, setAutomationTarget] = useState<AutomationTarget | null>(null)
  const clearAutomationTarget = useCallback(() => setAutomationTarget(null), [])
  const [usageRequest, setUsageRequest] = useState<UsageRequest | null>(null)
  const [opening, setOpening] = useState(embedded && isTauri)
  const readySent = useRef(false)
  /** 桌面主窗口：首次挂载按「默认打开页面」定页，定好后才通知后端显示窗口，避免先闪一下灵动岛页 */
  const desktop = embedded && isTauri
  const [pageResolved, setPageResolved] = useState(!desktop)
  const initialPending = useRef(desktop)
  /** 「上次访问的页面」：切页后延迟 1 秒写入，期间再切换就重新计时 */
  const rememberTimer = useRef<number | null>(null)
  const pendingPage = useRef<MainPage | null>(null)
  const savedPage = useRef<string | null>(null)
  /** 各页面的滚动位置：切换模块再回来时保持原位（画板 104） */
  const scrollRef = useRef<HTMLDivElement>(null)
  const scrollPositions = useRef(new Map<MainPage, number>())
  // 页面包含 fixed 弹窗，避免祖先 transform 改变弹窗的定位参照。
  const pageMotion = useContentMotion(page, 180, 4)
  const aboutPresence = useExitPresence(about)
  /** 设置 · 外观（画板 88）：列表密度与减少动效挂在 <html> 上；缩放由后端对 main 窗口 set_zoom */
  const { list_density: listDensity, reduce_motion: reduceMotion } = useSettings().settings

  useEffect(() => {
    const root = document.documentElement
    applyAppearance(root, normalizeDensity(listDensity), reduceMotion)
    // 浏览器预览会在同一文档里切到灵动岛视图，卸载时还原
    return () => applyAppearance(root, "comfortable", false)
  }, [listDensity, reduceMotion])

  const navigate = (next: MainPage) => {
    if (scrollRef.current) scrollPositions.current.set(page, scrollRef.current.scrollTop)
    setPage(next)
  }

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollPositions.current.get(page) ?? 0
  }, [page])

  useEffect(() => {
    if (!embedded || !isTauri || !pageResolved || readySent.current) return
    readySent.current = true
    void api.mainPanelReady().then(() => setOpening(false)).catch((error) => {
      console.error("主面板就绪通知失败", error)
    })
  }, [embedded, pageResolved])

  useEffect(() => {
    if (!desktop || !pageResolved) return
    if (rememberTimer.current !== null) window.clearTimeout(rememberTimer.current)
    rememberTimer.current = null
    if (page === savedPage.current) {
      pendingPage.current = null
      return
    }
    pendingPage.current = page
    rememberTimer.current = window.setTimeout(() => {
      rememberTimer.current = null
      void settingsApi.rememberPage(page).then(() => {
        savedPage.current = page
        if (pendingPage.current === page) pendingPage.current = null
      }).catch((error) => console.error("记录上次访问的页面失败", error))
    }, 1000)
  }, [desktop, page, pageResolved])

  useEffect(() => () => {
    if (rememberTimer.current !== null) window.clearTimeout(rememberTimer.current)
  }, [])

  // 托盘菜单可以直接把主面板带到指定位置
  useEffect(() => {
    const offs: Array<() => void> = []
    let stopped = false
    const finishInitial = () => {
      initialPending.current = false
      if (!stopped) setPageResolved(true)
    }
    const applyIntent = (intent: MainIntentDto | null) => {
      if (!intent || stopped) return
      if (intent.kind === "about") {
        setAbout(intent.version)
        return
      }
      initialPending.current = false
      setAbout(null)
      if (intent.kind === "overview") {
        // 后端快照里的 last_page 可能还没写入最近一次切页（1 秒延迟内），以前端记录为准
        const lastPage = pendingPage.current ?? intent.settings.last_page
        if (intent.settings.last_page) savedPage.current = intent.settings.last_page
        setPage(pageForIntent({ kind: "overview", settings: { default_page: intent.settings.default_page, last_page: lastPage } }))
      } else setPage(pageForIntent(intent))
      if (intent.kind === "settings") setSettingsSection(intent.section)
      if (intent.kind === "automation") setAutomationTarget({ taskId: intent.task_id, nonce: Date.now() })
      if (intent.kind !== "overview") setNavigationRequest((value) => value + 1)
    }
    const consumeIntent = () => void api.takeMainIntent().then(applyIntent).catch(() => {})
    /** 首次挂载：有托盘 / 第二实例留下的跳转意图就按意图打开，否则（含「关于」）读设置决定打开哪一页 */
    const consumeInitial = () => void api.takeMainIntent().then(async (intent) => {
      applyIntent(intent)
      if (!initialPending.current) return
      const settings = await api.getSettings()
      savedPage.current = settings.last_page
      if (!stopped && initialPending.current) setPage(resolveOpenPage(settings.default_page, settings.last_page))
    }).catch((error) => console.error("读取默认打开页面失败", error)).finally(finishInitial)
    void listenEvent("main-intent", consumeIntent).then((un) => {
      if (stopped) un()
      else {
        offs.push(un)
        if (initialPending.current) consumeInitial()
        else consumeIntent()
      }
    }).catch((error) => {
      console.error("主面板跳转监听失败", error)
      finishInitial()
    })
    return () => {
      stopped = true
      offs.forEach((f) => f())
    }
  }, [])

  useEffect(() => {
    if (!isTauri) return
    let stopped = false
    const offs: Array<() => void> = []
    const update = async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window")
      const current = getCurrentWindow()
      const [visible, minimized] = await Promise.all([current.isVisible(), current.isMinimized()])
      if (!stopped) setPanelActive(visible && !minimized && document.visibilityState !== "hidden")
    }
    const onVisibility = () => void update()
    document.addEventListener("visibilitychange", onVisibility)
    void import("@tauri-apps/api/window").then(async ({ getCurrentWindow }) => {
      if (stopped) return
      const current = getCurrentWindow()
      const listeners = await Promise.all([
        current.onResized(() => void update()),
        current.onFocusChanged(() => void update()),
      ])
      if (stopped) listeners.forEach((off) => off())
      else offs.push(...listeners)
      await update()
    }).catch(() => {})
    return () => {
      stopped = true
      document.removeEventListener("visibilitychange", onVisibility)
      offs.forEach((off) => off())
    }
  }, [])

  const toggleSidebar = () => {
    setCollapsed((value) => {
      writeSidebarCollapsed(!value)
      return !value
    })
  }

  return (
    <ToastProvider>
    <UpdateProvider>
    <TipProvider>
    <div
      data-panel-opening={opening}
      className={
        embedded
          ? "desktop-panel-entry relative flex h-full min-h-0 w-full flex-col overflow-hidden bg-jolu-bg"
          : "relative flex flex-col overflow-hidden rounded-2xl border border-jolu-border bg-jolu-bg shadow-window"
      }
      style={embedded ? undefined : { width: PANEL_WIDTH, height: PANEL_HEIGHT, maxWidth: "100%" }}
    >
      <TitleBar />
      <div className="flex min-h-0 flex-1">
        <SideNav page={page} collapsed={collapsed} onNavigate={navigate} />
        <main className="relative flex min-w-0 flex-1 flex-col">
          <PageHeader title={pageLabel(page)} collapsed={collapsed} onToggleSidebar={toggleSidebar} />
          <div ref={scrollRef} data-panel-scroll className="min-h-0 min-w-0 flex-1 overflow-auto">
            <div ref={pageMotion} className="px-8 py-5">
              {page === "island" && <IslandView />}
              {page === "usage" && (
                <UsageView
                  active={panelActive}
                  request={usageRequest}
                  onOpenSession={(target) => {
                    if (target) setSessionTarget({ ...target, nonce: Date.now() })
                    navigate("sessions")
                  }}
                />
              )}
              {page === "sessions" && (
                <SessionsView
                  active={panelActive}
                  target={sessionTarget}
                  onDrillUsage={(s) => {
                    setUsageRequest({ search: s.session_id, range: { start: dateKey(new Date(s.first_ts)), end: dateKey(new Date(s.last_ts)) }, nonce: Date.now() })
                    navigate("usage")
                  }}
                  onCreateAutomation={(session) => {
                    setAutomationPrefill({ session, nonce: Date.now() })
                    navigate("automation")
                  }}
                />
              )}
              {page === "providers" && <ProvidersView />}
              {page === "settings" && <SettingsPage section={settingsSection} navigationRequest={navigationRequest} onOpenSkills={() => navigate("skills")} />}
              {/* 模块 E · 自动化 */}
              {page === "automation" && (
                <AutomationView
                  active={panelActive}
                  prefill={automationPrefill}
                  target={automationTarget}
                  onTargetHandled={clearAutomationTarget}
                  onOpenSession={(platform, sessionId) => {
                    setSessionTarget({ platform, sessionId, nonce: Date.now() })
                    navigate("sessions")
                  }}
                  onOpenProviders={() => navigate("providers")}
                />
              )}
              {/* 模块 F · Skills 管理 */}
              {page === "skills" && <SkillsView active={panelActive} />}
            </div>
          </div>
        </main>
      </div>

      {/* 提示相对整个主窗口上方居中，不随侧栏宽度偏移。 */}
      <ToastViewport className="top-16" />
      {aboutPresence.rendered && <AboutDialog version={aboutPresence.rendered} exiting={aboutPresence.exiting} onClose={() => setAbout(null)} />}
      <UpdateDialog />
    </div>
    </TipProvider>
    </UpdateProvider>
    </ToastProvider>
  )
}

/** 关于（§2.6）：版本号与仓库链接 */
function AboutDialog({ version, onClose, exiting }: { version: string; onClose: () => void; exiting: boolean }) {
  const titleId = useId()
  const dialogRef = useRef<HTMLDivElement>(null)
  const closeRef = useRef(onClose)
  closeRef.current = onClose
  useEffect(() => {
    if (exiting) return
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null
    const dialog = dialogRef.current
    if (!dialog) return
    const items = () => Array.from(dialog.querySelectorAll<HTMLElement>(
      'button:not([disabled]), a[href], [tabindex]:not([tabindex="-1"])',
    ))
    ;(items()[0] ?? dialog).focus()
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault()
        closeRef.current()
        return
      }
      if (event.key !== "Tab") return
      const focusable = items()
      if (!focusable.length) return event.preventDefault()
      const first = focusable[0]
      const last = focusable[focusable.length - 1]
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault()
        last.focus()
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault()
        first.focus()
      }
    }
    document.addEventListener("keydown", onKeyDown)
    return () => {
      document.removeEventListener("keydown", onKeyDown)
      previous?.focus()
    }
  }, [exiting])
  return (
    <div inert={exiting} data-state={exiting ? "closed" : "open"} className="motion-overlay absolute inset-0 z-20 grid place-items-center bg-black/25 p-8">
      <div ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1} className="motion-dialog flex w-[380px] max-w-full flex-col gap-4 rounded-xl border bg-surface p-5 shadow-dialog">
        <div className="flex items-center gap-2.5">
          <AppIcon size={32} />
          <div className="flex flex-col">
            <span id={titleId} className="text-[13px] font-semibold text-text-primary">Jolu</span>
            <span className="tnum font-mono text-[11px] text-text-tertiary">v{version}</span>
          </div>
        </div>
        <p className="text-[11px] leading-[1.6] text-text-tertiary">
          在本机运行的 AI 编程工具管理器。统计来自本地会话记录与服务商接口，凭证只保存在本机。
        </p>
        <button
          type="button"
          onClick={() => void openUrl(REPO_URL)}
          className="self-start text-[11px] text-accent-blue underline"
        >
          {REPO_URL}
        </button>
        <div className="flex justify-end">
          <button
            type="button"
            onClick={onClose}
            className="rounded-[8px] border bg-surface px-4 py-2 text-xs text-text-primary"
          >
            关闭
          </button>
        </div>
      </div>
    </div>
  )
}
