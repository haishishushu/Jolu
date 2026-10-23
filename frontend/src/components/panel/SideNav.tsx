import { BookOpen, Bot, ChartColumn, Link2, MessageSquareText, PictureInPicture2, SlidersHorizontal, type LucideIcon } from "lucide-react"
import { cn } from "@/lib/utils"
import { PRIMARY_PAGES, SETTINGS_PAGE, type MainPage, type MainPageItem } from "@/lib/mainNavigation"
import { Tip } from "@/components/ui/Tip"

/** 图标取自 Jolu.pen 画板 01 / 03（lucide） */
const ICONS: Record<MainPage, LucideIcon> = {
  island: PictureInPicture2,
  providers: Link2,
  usage: ChartColumn,
  sessions: MessageSquareText,
  automation: Bot,
  skills: BookOpen,
  settings: SlidersHorizontal,
}

/**
 * 主窗口左侧导航（Jolu.pen 画板 01 展开 208px / 画板 03 收起 64px）。
 * 整行可点击；悬停中性浅底，当前模块绿色；收起后悬停图标显示模块名（画板 104）。
 */
export function SideNav({ page, collapsed, onNavigate }: { page: MainPage; collapsed: boolean; onNavigate: (page: MainPage) => void }) {
  const item = (entry: MainPageItem) => (
    <NavItem key={entry.id} entry={entry} active={entry.id === page} collapsed={collapsed} onClick={() => onNavigate(entry.id)} />
  )
  return (
    <nav
      aria-label="主导航"
      data-collapsed={collapsed}
      className={cn(
        "flex h-full shrink-0 flex-col gap-1.5 overflow-hidden border-r border-jolu-border bg-jolu-side pb-5 pt-7 transition-[width,padding] duration-[180ms] ease-[var(--motion-ease)]",
        collapsed ? "w-16 px-2.5" : "w-[208px] px-3.5",
      )}
    >
      {PRIMARY_PAGES.map(item)}
      <span aria-hidden className="flex-1" />
      {item(SETTINGS_PAGE)}
    </nav>
  )
}

function NavItem({ entry, active, collapsed, onClick }: { entry: MainPageItem; active: boolean; collapsed: boolean; onClick: () => void }) {
  const Icon = ICONS[entry.id]
  const button = (
    <button
      type="button"
      data-icon-motion={entry.id}
      aria-current={active ? "page" : undefined}
      aria-label={collapsed ? entry.label : undefined}
      onClick={onClick}
      className={cn(
        collapsed ? "px-3" : "px-3.5",
        "flex h-11 shrink-0 items-center gap-3 overflow-hidden whitespace-nowrap rounded-lg text-left text-sm outline-none focus-visible:outline-2 focus-visible:outline-offset-0 focus-visible:outline-jolu-accent",
        active
          ? "bg-jolu-selected font-semibold text-jolu-on active:bg-jolu-pressed"
          : "text-jolu-text hover:bg-jolu-stage active:bg-jolu-track",
      )}
    >
      <Icon aria-hidden className={cn("size-5 shrink-0", active ? "text-jolu-on" : "text-jolu-muted")} />
      <span className={cn("transition-opacity duration-[120ms]", collapsed && "opacity-0")}>{entry.label}</span>
    </button>
  )
  return collapsed ? <Tip label={entry.label} side="right">{button}</Tip> : button
}
