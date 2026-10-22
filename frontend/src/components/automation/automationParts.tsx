import * as Menu from "@radix-ui/react-dropdown-menu"
import type { ReactNode } from "react"
import { Ellipsis, Info, ListOrdered, Pause, Pencil, Play, RotateCcw, Square, X, type LucideIcon } from "lucide-react"
import { cn } from "@/lib/utils"
import { taskStatus, type Tone } from "@/lib/automation"
import type { TaskAction, TaskItemDto } from "@/lib/automationApi"

export const TONE: Record<Tone, string> = {
  on: "text-jolu-on",
  text: "text-jolu-text",
  danger: "text-jolu-danger",
  muted: "text-jolu-muted",
}

/** 卡片标题行：14px 标题 + 右侧 10px 口径（画板 62 / 66） */
export function SectionTitle({ title, note, children }: { title: string; note?: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex min-h-5 items-center justify-between gap-3">
      <h3 className="truncate text-sm font-semibold text-jolu-text">{title}</h3>
      {children ?? (note && <span className="shrink-0 text-[10px] text-jolu-muted">{note}</span>)}
    </div>
  )
}

/** 键值行：标签在左，值在右 */
export function KeyValue({ label, value, title }: { label: string; value: ReactNode; title?: string }) {
  return (
    <div className="flex items-center gap-2.5 text-[11px]">
      <span className="shrink-0 text-jolu-muted">{label}</span>
      <span className="flex-1" />
      <span className="min-w-0 truncate text-right text-jolu-text" title={title}>{value}</span>
    </div>
  )
}

/** 提示框：选中绿底（等待额度）/ 浅底说明 / 危险（画板 60–68） */
export function Notice({ icon: Icon = Info, title, children, tone = "side", className }: {
  icon?: LucideIcon
  title: ReactNode
  children?: ReactNode
  tone?: "selected" | "side" | "danger"
  className?: string
}) {
  return (
    <div
      className={cn(
        "flex flex-col gap-[7px] rounded-xl border p-3.5",
        tone === "selected" && "border-jolu-selected bg-jolu-selected",
        tone === "side" && "border-jolu-border bg-jolu-side",
        tone === "danger" && "border-jolu-danger-soft bg-jolu-danger-soft",
        className,
      )}
    >
      <div className="flex items-center gap-2.5">
        <Icon aria-hidden className={cn("size-4 shrink-0", tone === "danger" ? "text-jolu-danger" : tone === "selected" ? "text-jolu-on" : "text-jolu-muted")} />
        <span className={cn("text-xs font-semibold", tone === "danger" ? "text-jolu-danger" : tone === "selected" ? "text-jolu-on" : "text-jolu-text")}>{title}</span>
      </div>
      {children && <div className={cn("whitespace-pre-line text-[11px] leading-[1.6]", tone === "selected" ? "text-jolu-on" : tone === "danger" ? "text-jolu-danger" : "text-jolu-muted")}>{children}</div>}
    </div>
  )
}

/** 页签：任务 / 运行记录（画板 58 / 64） */
export function AutomationTabs({ value, onChange }: { value: "tasks" | "runs"; onChange: (v: "tasks" | "runs") => void }) {
  return (
    <div role="tablist" aria-label="自动化页签" className="flex h-[38px] gap-6 border-b border-jolu-border">
      {([["tasks", "任务"], ["runs", "运行记录"]] as const).map(([key, label]) => (
        <button
          key={key}
          type="button"
          role="tab"
          aria-selected={value === key}
          onClick={() => onChange(key)}
          className={cn(
            "-mb-px h-[38px] border-b-2 text-[13px] font-semibold outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
            value === key ? "border-jolu-accent text-jolu-on" : "border-transparent text-jolu-muted hover:text-jolu-text",
          )}
        >
          {label}
        </button>
      ))}
    </div>
  )
}

/** 额度进度条（画板 62 右栏） */
export function QuotaBar({ label, value, note, percent, blocking }: { label: string; value: string; note?: string; percent: number | null; blocking: boolean }) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex justify-between gap-2 text-[11px]">
        <span className="text-jolu-text">{label}</span>
        <span className="tnum text-jolu-muted">{value}</span>
      </div>
      <span className="h-1.5 rounded-[3px] bg-jolu-track">
        <span className={cn("block h-full rounded-[3px]", blocking ? "bg-jolu-on" : "bg-jolu-accent")} style={{ width: `${Math.min(100, Math.max(percent === null ? 0 : 2, percent ?? 0))}%` }} />
      </span>
      {note && <span className="text-[10px] text-jolu-muted">{note}</span>}
    </div>
  )
}

const MENU_CLASS = "motion-popover z-[100] min-w-[168px] rounded-xl border border-jolu-border bg-jolu-bg p-1.5 shadow-popover"
const ITEM_CLASS = "flex h-8 cursor-default select-none items-center gap-2 rounded-md px-2 text-[11px] text-jolu-text outline-none data-[highlighted]:bg-jolu-side data-[disabled]:opacity-40 [&_svg]:size-3.5 [&_svg]:text-jolu-muted"

/**
 * 任务更多操作（画板 68 第 08 格）：编辑任务 / 暂停等待 / 查看账号队列 / 取消任务；
 * 已暂停时为「恢复任务」，运行中为「停止运行」。没有进行中的运行时可立即运行（已错过为「补跑」、失败为「重试」）。
 */
export function TaskMenu({ item, now, onAction, onEdit, onQueue, onCancel, trigger }: {
  item: TaskItemDto
  now: number
  onAction: (action: TaskAction) => void
  onEdit: () => void
  onQueue: () => void
  onCancel: () => void
  trigger?: ReactNode
}) {
  const running = item.active?.status === "running"
  const cancelled = item.state === "cancelled"
  const status = taskStatus(item, now)
  const canRun = !cancelled && item.state !== "paused" && (!item.active || item.active.status === "attention")
  const runLabel = item.active?.status === "attention" ? "重新排队" : item.missed_at ? "补跑" : status.label === "失败" ? "重试" : "立即运行"
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger asChild>
        {trigger ?? (
          <button type="button" aria-label={`${item.name} 的更多操作`} className="grid size-8 shrink-0 place-items-center rounded-lg bg-jolu-side text-jolu-muted outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent data-[state=open]:bg-jolu-stage">
            <Ellipsis aria-hidden className="size-4" />
          </button>
        )}
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content align="end" sideOffset={6} collisionPadding={12} className={MENU_CLASS}>
          <Menu.Item className={ITEM_CLASS} disabled={cancelled} onSelect={onEdit}><Pencil aria-hidden />编辑任务</Menu.Item>
          {running ? (
            <Menu.Item className={ITEM_CLASS} onSelect={() => onAction("stop")}><Square aria-hidden />停止运行</Menu.Item>
          ) : item.state === "paused" ? (
            <Menu.Item className={ITEM_CLASS} onSelect={() => onAction("resume")}><Play aria-hidden />恢复任务</Menu.Item>
          ) : (
            <Menu.Item className={ITEM_CLASS} disabled={item.state !== "active"} onSelect={() => onAction("pause")}><Pause aria-hidden />{item.active?.status === "waiting" ? "暂停等待" : "暂停任务"}</Menu.Item>
          )}
          {canRun && <Menu.Item className={ITEM_CLASS} onSelect={() => onAction(item.active ? "retry" : "run")}><RotateCcw aria-hidden />{runLabel}</Menu.Item>}
          <Menu.Item className={ITEM_CLASS} onSelect={onQueue}><ListOrdered aria-hidden />查看账号队列</Menu.Item>
          <Menu.Item className={ITEM_CLASS} disabled={cancelled} onSelect={onCancel}><X aria-hidden />取消任务</Menu.Item>
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}
