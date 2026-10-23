import type { ReactNode } from "react"
import { CircleFadingArrowUp, Github, Moon, PanelLeft, PictureInPicture2, Sun } from "lucide-react"
import { isTauri, openUrl } from "@/lib/api"
import { cn } from "@/lib/utils"
import { resolveDarkTheme } from "@/lib/displayPreferences"
import { UPDATES_ENABLED } from "@/lib/updateState"
import { useSettings } from "@/hooks/useSettings"
import { useSystemDark } from "@/hooks/useSystemDark"
import { useToast } from "@/components/ui/Toast"
import { Tip } from "@/components/ui/Tip"
import { useUpdate } from "@/components/panel/UpdateContext"

export const REPO_URL = "https://github.com/haishishushu/Jolu"

/**
 * 内容区固定页头（Jolu.pen 画板 01「内容区固定页头」，高 64）：
 * 左侧侧栏开关 + 竖分隔线 + 当前模块标题，右侧 GitHub / 灵动岛显隐 / 界面主题 / 检查更新。
 */
export function PageHeader({ title, collapsed, onToggleSidebar }: { title: string; collapsed: boolean; onToggleSidebar: () => void }) {
  return (
    <header className="flex h-16 shrink-0 items-center justify-between gap-4 border-b border-jolu-border px-8">
      <div className="flex min-w-0 items-center gap-4">
        <Tip label={collapsed ? "展开侧栏" : "收起侧栏"}>
          <button
            type="button"
            aria-label={collapsed ? "展开侧栏" : "收起侧栏"}
            aria-expanded={!collapsed}
            onClick={onToggleSidebar}
            className="grid size-8 shrink-0 place-items-center rounded-md text-jolu-text outline-none hover:bg-jolu-stage active:bg-jolu-track focus-visible:outline-2 focus-visible:outline-jolu-accent"
          >
            <PanelLeft aria-hidden className="size-[18px]" />
          </button>
        </Tip>
        <span aria-hidden className="h-5 w-px shrink-0 bg-jolu-border" />
        <h1 className="truncate text-lg font-semibold text-jolu-text">{title}</h1>
      </div>
      <HeaderActions />
    </header>
  )
}

function HeaderActions() {
  const cfg = useSettings()
  const systemDark = useSystemDark()
  const toast = useToast()
  const update = useUpdate()
  const dark = resolveDarkTheme(cfg.settings.theme, systemDark)
  const visible = cfg.settings.island_visible
  const desktopOnly = isTauri ? undefined : "（仅桌面端可用）"

  const onUpdate = () => {
    if (update.info && (update.phase === "idle" || update.phase === "available")) {
      update.startDownload()
      return
    }
    void update.checkNow()
      .then((available) => toast.success(available ? "发现新版本" : "已是最新版本"))
      .catch((reason) => toast.danger("检查更新失败", String(reason)))
  }

  return (
    <div className="flex shrink-0 items-center gap-3">
      <HeaderButton motion="github" label="GitHub" onClick={() => void openUrl(REPO_URL)}>
        <Github aria-hidden className="size-5" />
      </HeaderButton>
      <HeaderButton
        motion="island"
        label={(visible ? "隐藏灵动岛" : "显示灵动岛") + (desktopOnly ?? "")}
        pressed={visible}
        disabled={!isTauri || !cfg.ready}
        onClick={() => void cfg.setIslandVisible(!visible)}
      >
        <PictureInPicture2 aria-hidden className="size-5" />
      </HeaderButton>
      <HeaderButton
        motion={dark ? "moon" : "sun"}
        label={(dark ? "切换到浅色" : "切换到深色") + (desktopOnly ?? "")}
        disabled={!isTauri || !cfg.ready}
        onClick={() => void cfg.setTheme(dark ? "light" : "dark")}
      >
        {dark ? <Moon aria-hidden className="size-5" /> : <Sun aria-hidden className="size-5" />}
      </HeaderButton>
      <HeaderButton
        motion="update"
        label={UPDATES_ENABLED ? "检查更新" : "检查更新暂时停用"}
        disabled={!UPDATES_ENABLED || update.phase === "checking"}
        onClick={onUpdate}
      >
        <CircleFadingArrowUp aria-hidden className="size-5" />
      </HeaderButton>
    </div>
  )
}

function HeaderButton({ motion, label, pressed, disabled, onClick, children }: {
  motion: "github" | "island" | "sun" | "moon" | "update"
  label: string
  pressed?: boolean
  disabled?: boolean
  onClick: () => void
  children: ReactNode
}) {
  return (
    <Tip label={label}>
      {/* 禁用按钮不触发指针事件，外层 span 负责接住悬停提示 */}
      <span className="inline-flex">
        <button
          type="button"
          aria-label={label}
          data-icon-motion={motion}
          aria-pressed={pressed}
          disabled={disabled}
          onClick={onClick}
          className={cn(
            "grid size-9 place-items-center rounded-lg text-jolu-muted outline-none hover:bg-jolu-stage active:bg-jolu-track focus-visible:outline-2 focus-visible:outline-jolu-accent",
            "disabled:pointer-events-none disabled:opacity-45",
          )}
        >
          {children}
        </button>
      </span>
    </Tip>
  )
}
