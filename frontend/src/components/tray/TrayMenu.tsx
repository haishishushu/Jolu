import {
  Bell,
  BellOff,
  Check,
  ChevronRight,
  Eye,
  Info,
  LayoutGrid,
  Loader,
  LogOut,
  Move,
  PanelTopOpen,
  RefreshCw,
  Settings,
} from "lucide-react"
import { cn } from "@/lib/utils"

/**
 * 系统托盘右键菜单 —— §2.6 / 画布 18
 * 宽 268、子菜单 300；菜单项高 32 r6、图标位 16、分隔线上下各 4。
 * 禁用项降低不透明度而非隐藏。
 */
export const TRAY_MENU_WIDTH = 268

type Item =
  | { sep: true }
  | {
      sep?: false
      label: string
      icon?: React.ComponentType<{ className?: string }>
      check?: boolean
      sub?: boolean
      hint?: string
      danger?: boolean
      accent?: boolean
      disabled?: boolean
    }

export interface TrayState {
  refreshing?: boolean
  islandHidden?: boolean
  dnd?: boolean
}

export function trayItems(s: TrayState = {}): Item[] {
  return [
    { label: "打开主面板", icon: PanelTopOpen },
    s.refreshing
      ? { label: "刷新中…", icon: Loader, disabled: true }
      : { label: "立即刷新", icon: RefreshCw },
    { sep: true },
    s.islandHidden
      ? { label: "显示灵动岛", icon: Eye }
      : { label: "显示灵动岛", check: true },
    { label: "切换连接", icon: LayoutGrid, sub: true, disabled: !!s.islandHidden },
    { label: "显示位置", icon: Move, sub: true, disabled: !!s.islandHidden },
    { label: "始终置顶", check: true },
    s.dnd
      ? { label: "免打扰", check: true, accent: true, hint: "已开启" }
      : { label: "免打扰", icon: BellOff },
    { sep: true },
    { label: "重置窗口位置", icon: Move },
    { label: "设置", icon: Settings },
    { label: "关于 Jolu", icon: Info },
    { sep: true },
    { label: "退出", icon: LogOut, danger: true },
  ]
}

export function TrayMenu({ state = {} }: { state?: TrayState }) {
  return (
    <div
      className="flex flex-col gap-px rounded-[8px] border bg-surface p-1 shadow-popover"
      style={{ width: TRAY_MENU_WIDTH }}
    >
      {trayItems(state).map((it, i) =>
        "sep" in it && it.sep ? (
          <div key={i} className="px-2 py-1">
            <div className="h-px w-full bg-[var(--border)]" />
          </div>
        ) : (
          <div
            key={i}
            className={cn(
              "flex h-8 w-full items-center gap-2.5 rounded-[6px] px-2",
              !it.disabled && "hover:bg-surface-3",
              it.disabled && "pointer-events-none opacity-[0.42]",
            )}
          >
            <span className="grid size-4 shrink-0 place-items-center">
              {it.check ? (
                <Check className={cn("size-3.5", it.accent ? "text-accent-blue" : "text-text-primary")} />
              ) : it.icon ? (
                <it.icon
                  className={cn(
                    "size-3.5",
                    it.danger ? "text-danger" : it.accent ? "text-accent-blue" : "text-text-secondary",
                  )}
                />
              ) : null}
            </span>
            <span
              className={cn(
                "min-w-0 flex-1 truncate text-[13px]",
                it.danger ? "text-danger" : it.accent ? "font-semibold text-accent-blue" : "text-text-primary",
              )}
            >
              {it.label}
            </span>
            {it.hint && <span className="text-[11px] text-accent-blue">{it.hint}</span>}
            {it.sub && <ChevronRight className="size-3.5 shrink-0 text-text-tertiary" />}
          </div>
        ),
      )}
    </div>
  )
}

/** 托盘 tooltip：平台、连接、各额度窗口已用百分比、最近更新时间 */
export function TrayTooltip({
  app,
  connection,
  quotas,
  updated,
}: {
  app: string
  connection: string
  quotas: { key: string; value: string }[]
  updated: string
}) {
  return (
    <div className="flex w-[230px] flex-col gap-1 rounded-[8px] border bg-surface px-3 py-2.5 shadow-popover">
      <span className="text-xs font-semibold text-text-primary">{app}</span>
      <span className="break-words text-[11px] text-text-secondary">{connection}</span>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        {quotas.map((q) => (
          <span key={q.key} className="flex items-center gap-1.5">
            <span className="tnum font-mono text-[11px] text-text-tertiary">{q.key}</span>
            <span className="tnum font-mono text-xs font-semibold text-text-primary">{q.value}</span>
          </span>
        ))}
      </div>
      <span className="text-[10px] text-text-tertiary">{updated}</span>
    </div>
  )
}

export { Bell }
