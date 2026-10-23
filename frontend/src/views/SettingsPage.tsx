import { useCallback, useEffect, useRef, useState } from "react"
import { Bell, CircleAlert, CircleDot, Database, Info, LoaderCircle, Network, Palette, SlidersHorizontal, type LucideIcon } from "lucide-react"
import { cn } from "@/lib/utils"
import { SAVE_STATE_LABEL, SETTINGS_TABS, saveState, tabForSection, type SettingsTab } from "@/lib/settingsPage"
import { useSettings } from "@/hooks/useSettings"
import { GeneralSection } from "@/components/settings/GeneralSection"
import { AppearanceSection } from "@/components/settings/AppearanceSection"
import { NetworkSection } from "@/components/settings/NetworkSection"
import { DataSection } from "@/components/settings/DataSection"
import { NotificationSection } from "@/components/settings/NotificationSection"
import { AboutSection } from "@/components/settings/AboutSection"

const ICONS: Record<SettingsTab, LucideIcon> = {
  general: SlidersHorizontal,
  appearance: Palette,
  network: Network,
  data: Database,
  notifications: Bell,
  about: Info,
}

/**
 * 设置（Jolu.pen 模块 G · 画板 86–103）：页头保存状态 + 左侧 168 宽分类导航 + 当前分类内容。
 * `section` 来自托盘 / 菜单的跳转意图（旧分区 id），每次 navigationRequest 变化时重新定位。
 */
export function SettingsPage({ section, navigationRequest, onOpenSkills }: {
  section: string | null
  navigationRequest: number
  onOpenSkills: () => void
}) {
  const cfg = useSettings()
  const [tab, setTab] = useState<SettingsTab>(() => tabForSection(section))
  const [dirtyKeys, setDirtyKeys] = useState<ReadonlySet<string>>(new Set())

  useEffect(() => { if (navigationRequest > 0) setTab(tabForSection(section)) }, [navigationRequest, section])
  // 切换分类后回到页面顶部，避免停在上一分类的滚动位置
  const rootRef = useRef<HTMLDivElement>(null)
  useEffect(() => { rootRef.current?.closest<HTMLElement>("[data-panel-scroll]")?.scrollTo({ top: 0 }) }, [tab])

  const setDirty = useCallback((key: string, dirty: boolean) => {
    setDirtyKeys((prev) => {
      if (prev.has(key) === dirty) return prev
      const next = new Set(prev)
      if (dirty) next.add(key)
      else next.delete(key)
      return next
    })
  }, [])

  const state = saveState({ saving: cfg.saving, error: cfg.error, dirty: dirtyKeys.size > 0 })
  const current = SETTINGS_TABS.find((t) => t.id === tab) ?? SETTINGS_TABS[0]

  return (
    <div ref={rootRef} className="flex w-full flex-col gap-6 py-1">
      <header className="flex min-h-14 items-center justify-between gap-4">
        <div className="flex flex-col gap-[7px]">
          <h1 className="text-2xl font-semibold text-jolu-text">设置</h1>
          <p className="text-xs text-jolu-muted">让 Jolu 按你的习惯工作</p>
        </div>
        <SaveStatus state={state} />
      </header>

      {cfg.error && (
        <div role="alert" className="-mt-2 flex items-center justify-between gap-3 rounded-[9px] bg-jolu-danger-soft px-3.5 py-2.5 text-[11px] text-jolu-danger">
          <span className="flex items-center gap-2"><CircleAlert aria-hidden className="size-4 shrink-0" />{cfg.error}。已保留原值。</span>
          <button type="button" disabled={cfg.loading || cfg.saving} onClick={() => void cfg.reload()} className="shrink-0 rounded-md px-2 py-1 font-medium outline-none hover:bg-jolu-danger-pressed focus-visible:outline-2 focus-visible:outline-jolu-accent">
            重新读取
          </button>
        </div>
      )}

      <div className="flex min-h-[560px] gap-7">
        <nav aria-label="设置分类" className="sticky top-0 flex w-[168px] shrink-0 flex-col gap-1.5 self-start">
          {SETTINGS_TABS.map((t) => {
            const Icon = ICONS[t.id]
            const active = t.id === tab
            return (
              <button
                key={t.id}
                type="button"
                aria-current={active ? "page" : undefined}
                onClick={() => setTab(t.id)}
                className={cn(
                  "flex h-[42px] items-center gap-2.5 rounded-lg px-3 text-left text-[13px] outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
                  active ? "bg-jolu-selected font-semibold text-jolu-on" : "text-jolu-text hover:bg-jolu-side",
                )}
              >
                <Icon aria-hidden className={cn("size-[17px]", active ? "text-jolu-on" : "text-jolu-muted")} />
                {t.label}
              </button>
            )
          })}
          <span className="mt-6 text-[10px] text-jolu-muted">仅保存到此设备</span>
          <span className="text-[10px] text-jolu-muted">Jolu · 桌面端</span>
        </nav>

        <section aria-label={current.label} className="flex min-w-0 flex-1 flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <h2 className="text-[22px] font-semibold text-jolu-text">{current.label}</h2>
            <p className="text-xs text-jolu-muted">{current.desc}</p>
          </div>
          {cfg.loading && !cfg.ready && <p role="status" className="text-xs text-jolu-muted">正在读取设置…</p>}
          {tab === "general" && <GeneralSection cfg={cfg} />}
          {tab === "appearance" && <AppearanceSection cfg={cfg} />}
          {tab === "network" && <NetworkSection cfg={cfg} setDirty={setDirty} />}
          {tab === "data" && <DataSection cfg={cfg} onOpenSkills={onOpenSkills} />}
          {tab === "notifications" && <NotificationSection cfg={cfg} setDirty={setDirty} />}
          {tab === "about" && <AboutSection />}
        </section>
      </div>
    </div>
  )
}

/** 页头保存状态（画板 86 / 100 / 102） */
function SaveStatus({ state }: { state: ReturnType<typeof saveState> }) {
  if (state === "saved") return null
  const Icon = state === "saving" ? LoaderCircle : state === "dirty" ? CircleDot : CircleAlert
  return (
    <span role="status" className={cn(
      "flex items-center gap-1.5 text-[11px]",
      state === "failed" ? "text-jolu-danger" : "text-jolu-muted",
    )}>
      <Icon aria-hidden className={cn("size-3.5", state === "saving" && "animate-spin")} />
      {SAVE_STATE_LABEL[state]}
    </span>
  )
}
