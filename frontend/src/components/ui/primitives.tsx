import * as React from "react"
import { Check } from "lucide-react"
import { cn } from "@/lib/utils"
import { useSlidingIndicator } from "@/hooks/useSlidingIndicator"

/* ---------------------------------------------------------------------------
   基础原语 —— 尺寸与配色以 docs/ui/Jolu.pen 为准，勿套用 shadcn 默认值
   （改动时按 Jolu.pen 模块 H 交互规范核对）
--------------------------------------------------------------------------- */

/** 徽标：padding [2,6~8]，圆角 6，字号 10~11 */
export function Chip({
  tone = "neutral",
  mono,
  className,
  children,
}: {
  tone?: "neutral" | "success" | "warn" | "danger" | "accent" | "purple"
  mono?: boolean
  className?: string
  children: React.ReactNode
}) {
  const tones: Record<string, string> = {
    neutral: "bg-neutral-soft text-text-secondary",
    success: "bg-success-soft text-success-text",
    warn: "bg-warn-soft text-warn",
    danger: "bg-danger-soft text-danger",
    accent: "bg-accent-blue-soft text-accent-blue",
    purple: "bg-purple-soft text-purple-text",
  }
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center rounded-[6px] px-1.5 py-0.5 text-[10px] font-medium leading-normal",
        mono && "font-mono tnum",
        tones[tone],
        className,
      )}
    >
      {children}
    </span>
  )
}

/** 接入类型徽标（§3）：官方订阅绿 Auth、API Key 蓝 API */
export function ConnectionKindChip({ kind, local = false }: { kind: "auth" | "api"; local?: boolean }) {
  if (local) return <Chip tone="neutral">本机</Chip>
  if (kind === "auth") return <Chip tone="success" mono>Auth</Chip>
  return <Chip tone="accent" mono>API</Chip>
}

/** 连接状态点 + 文字 */
export function StatusDot({ tone }: { tone: "success" | "warn" | "danger" | "neutral" }) {
  const tones: Record<string, string> = {
    success: "bg-success",
    warn: "bg-warn",
    danger: "bg-danger",
    neutral: "bg-text-tertiary",
  }
  return <span className={cn("size-1.5 shrink-0 rounded-full", tones[tone])} />
}

export function Checkbox({ checked }: { checked: boolean }) {
  return (
    <span
      className={cn(
        "grid size-[14px] shrink-0 place-items-center rounded-[4px] border",
        checked ? "border-accent-blue bg-accent-blue" : "border-border-strong bg-surface",
      )}
    >
      {checked && <Check className="size-2.5 text-white" strokeWidth={3} />}
    </span>
  )
}

/** 分段控件：容器 surface-3 圆角 9 内边距 2；选中项 surface + 描边 圆角 7 */
export function Segmented<T extends string>({
  items,
  value,
  onChange,
  className,
}: {
  items: { value: T; label: string; disabled?: boolean }[]
  value: T
  onChange?: (v: T) => void
  className?: string
}) {
  // 滑动底板与设置分区导航、平台标签共用同一套测量逻辑
  const { rootRef: root, plateRef: indicator } = useSlidingIndicator<HTMLDivElement, HTMLSpanElement>({
    selector: 'button[aria-pressed="true"]',
    itemsKey: JSON.stringify(items.map((item) => item.value)),
    value,
  })
  return (
    <div ref={root} className={cn("relative isolate inline-flex gap-0.5 rounded-[9px] bg-surface-3 p-0.5", className)}>
      <span ref={indicator} aria-hidden="true" data-motion-indicator className="motion-indicator pointer-events-none absolute left-0 top-0 -z-10 rounded-[7px] border bg-surface" />
      {items.map((it) => {
        const active = it.value === value
        return (
          <button
            key={it.value}
            type="button"
            disabled={it.disabled}
            aria-pressed={active}
            onClick={() => onChange?.(it.value)}
            className={cn(
              "relative grid rounded-[7px] border border-transparent px-3 py-1.5 text-xs transition-colors",
              active
                ? "font-semibold text-text-primary"
                : "text-text-secondary hover:text-text-primary",
              it.disabled && "pointer-events-none opacity-40",
            )}
          >
            <span aria-hidden="true" className="invisible col-start-1 row-start-1 font-semibold">{it.label}</span>
            <span className="col-start-1 row-start-1">{it.label}</span>
          </button>
        )
      })}
    </div>
  )
}

export function SectionTitle({ children }: { children: React.ReactNode }) {
  return <h2 className="text-sm font-semibold text-text-primary">{children}</h2>
}

export function Divider({ className }: { className?: string }) {
  return <div className={cn("h-px w-full bg-[var(--border)]", className)} />
}
