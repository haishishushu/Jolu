import { useRef, type KeyboardEvent } from "react"
import { ChevronDown, ChevronRight, Folder, FolderOpen, RotateCcw } from "lucide-react"
import { cn } from "@/lib/utils"
import { listTime, sessionKey, sessionTitle, statusLabel, visibleRows, type ProjectGroup } from "@/lib/sessionThreads"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { Card, ToolButton, providerName } from "@/components/usage/usageControls"
import type { PlatformId } from "@/types"

/**
 * 项目 → 会话两级列表（画板 40 / 42）。左右键展开收起，上下键移动，Enter 查看（画板 46）。
 * 列表独立滚动；切换会话不改变筛选与展开状态。
 */
export function SessionTree({ groups, expanded, selected, searching, total, onToggle, onSelect, onClear }: {
  groups: ProjectGroup[]
  expanded: ReadonlySet<string>
  selected: string | null
  searching: boolean
  total: number
  onToggle: (project: string, open?: boolean) => void
  onSelect: (key: string) => void
  onClear: () => void
}) {
  const listRef = useRef<HTMLDivElement>(null)
  const rows = visibleRows(groups, expanded)
  const shown = rows.filter((r) => r.kind === "session").length

  const focusRow = (index: number) => {
    const target = listRef.current?.querySelectorAll<HTMLElement>("[data-tree-row]")[index]
    target?.focus()
    target?.scrollIntoView({ block: "nearest" })
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const items = [...(listRef.current?.querySelectorAll<HTMLElement>("[data-tree-row]") ?? [])]
    const index = items.indexOf(document.activeElement as HTMLElement)
    if (index < 0) return
    const row = rows[index]
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault()
      focusRow(Math.min(rows.length - 1, Math.max(0, index + (event.key === "ArrowDown" ? 1 : -1))))
    } else if (event.key === "ArrowRight" && row.kind === "project") {
      event.preventDefault()
      onToggle(row.key, true)
    } else if (event.key === "ArrowLeft") {
      event.preventDefault()
      if (row.kind === "project") onToggle(row.key, false)
      else focusRow(rows.findIndex((r) => r.kind === "project" && r.key === row.project))
    } else if (event.key === "Enter") {
      event.preventDefault()
      if (row.kind === "session") onSelect(row.key)
      else onToggle(row.key)
    }
  }

  return (
    <Card className="flex h-full w-[320px] shrink-0 flex-col gap-1 p-2.5">
      <div className="flex items-center justify-between px-0.5 pb-1">
        <h3 className="text-sm font-semibold text-jolu-text">{searching ? "匹配的项目与会话" : "项目与会话"}</h3>
        <span className="text-[10px] text-jolu-muted">{groups.length} 项目 · {total} 条</span>
      </div>
      <div ref={listRef} role="tree" aria-label="项目与会话" onKeyDown={onKeyDown} className="-mx-1 flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto px-1">
        {groups.map((g) => {
          const open = expanded.has(g.key)
          return (
            <div key={g.key} role="group" className="flex flex-col gap-0.5">
              <button
                type="button"
                data-tree-row
                role="treeitem"
                aria-expanded={open}
                onClick={() => onToggle(g.key)}
                className="flex h-7 shrink-0 items-center gap-1.5 rounded-md bg-jolu-side px-1.5 text-left outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent"
              >
                {open ? <ChevronDown aria-hidden className="size-3 text-jolu-muted" /> : <ChevronRight aria-hidden className="size-3 text-jolu-muted" />}
                {open ? <FolderOpen aria-hidden className="size-[15px] text-jolu-muted" /> : <Folder aria-hidden className="size-[15px] text-jolu-muted" />}
                <span className="min-w-0 flex-1 truncate text-[11px] font-semibold text-jolu-text" title={g.path ?? undefined}>{g.name}</span>
                <span className="tnum text-[10px] text-jolu-muted">{g.sessions.length}</span>
              </button>
              {open && (
                <div className="ml-[7px] flex flex-col gap-0.5 border-l border-jolu-border pl-[7px]">
                  {g.sessions.map((s) => {
                    const key = sessionKey(s)
                    const active = key === selected
                    return (
                      <button
                        key={key}
                        type="button"
                        data-tree-row
                        role="treeitem"
                        aria-selected={active}
                        onClick={() => onSelect(key)}
                        className={cn(
                          "flex h-9 shrink-0 flex-col justify-center gap-0.5 rounded-md px-1.5 text-left [.density-compact_&]:h-8 outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
                          active ? "bg-jolu-selected" : "hover:bg-jolu-side",
                        )}
                      >
                        <span className="flex w-full items-center gap-1.5">
                          <PlatformLogo platform={s.platform as PlatformId} size={12} />
                          <span className={cn("min-w-0 flex-1 truncate text-[11px]", active ? "font-semibold text-jolu-on" : "text-jolu-text")}>{sessionTitle(s)}</span>
                          <span className="tnum shrink-0 text-[9px] text-jolu-muted">{listTime(s.last_ts)}</span>
                        </span>
                        <span className="flex gap-2 pl-[18px] text-[9px]">
                          <span className="text-jolu-muted">{providerName(s.platform)}</span>
                          <span className={s.status === "running" ? "text-jolu-on" : s.status === "failed" ? "text-jolu-danger" : "text-jolu-muted"}>{statusLabel(s.status)}</span>
                        </span>
                      </button>
                    )
                  })}
                </div>
              )}
            </div>
          )
        })}
      </div>
      {searching ? (
        <div className="flex flex-col items-start gap-2 pt-1">
          <span className="text-[10px] text-jolu-muted">仅显示匹配会话，自动展开对应项目。</span>
          <ToolButton bordered icon={<RotateCcw />} onClick={onClear}>清除搜索与筛选</ToolButton>
        </div>
      ) : (
        <span className="pt-1 text-[9px] text-jolu-muted">已展开 {shown} 条会话 · 点击项目展开更多</span>
      )}
    </Card>
  )
}
