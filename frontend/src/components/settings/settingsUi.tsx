import type { ButtonHTMLAttributes, ReactNode } from "react"
import { Info, LoaderCircle, type LucideIcon } from "lucide-react"
import { cn } from "@/lib/utils"
import { SelectField, type SelectOption } from "@/components/ui/SelectField"

/** 设置分组卡片（画板 86）：左右 20 内边距、圆角 12；有图标时标题行 66 高 */
export function SettingsGroup({ icon: Icon, title, desc, children, className }: {
  icon?: LucideIcon
  title: string
  desc?: string
  children?: ReactNode
  className?: string
}) {
  return (
    <section className={cn("flex w-full flex-col rounded-xl border border-jolu-border bg-jolu-bg px-5", className)}>
      <header className={cn("flex items-center gap-2.5", Icon ? "h-[66px]" : "pb-3.5 pt-[18px]")}>
        {Icon && (
          <span className="grid size-[30px] shrink-0 place-items-center rounded-lg bg-jolu-selected">
            <Icon aria-hidden className="size-4 text-jolu-on" />
          </span>
        )}
        <span className="flex min-w-0 flex-1 flex-col gap-1">
          <h3 className="text-sm font-semibold text-jolu-text">{title}</h3>
          {desc && <span className="text-[11px] text-jolu-muted">{desc}</span>}
        </span>
      </header>
      {children}
    </section>
  )
}

/** 设置行：标题 13/500 + 说明 11，控件靠右；行之间是上边框 */
export function SettingsRow({ title, desc, control, className, descClassName }: {
  title: ReactNode
  desc?: ReactNode
  control?: ReactNode
  className?: string
  descClassName?: string
}) {
  return (
    <div className={cn("flex min-h-[66px] items-center gap-5 border-t border-jolu-border py-3", className)}>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <span className="text-[13px] font-medium text-jolu-text">{title}</span>
        {desc && <span className={cn("text-[11px] text-jolu-muted", descClassName)}>{desc}</span>}
      </div>
      {control && <div className="flex shrink-0 items-center gap-2">{control}</div>}
    </div>
  )
}

export { JoluSwitch as Switch } from "@/components/ui/JoluSwitch"

/** 160×34 的下拉按钮（「上次访问的页面」「5 分钟」等） */
export function SelectButton({ value, options, onChange, label, disabled, className, triggerLabel }: {
  value: string
  options: SelectOption[]
  onChange: (value: string) => void
  label: string
  disabled?: boolean
  className?: string
  triggerLabel?: ReactNode
}) {
  return (
    <SelectField
      label={label}
      value={value}
      options={options}
      disabled={disabled}
      triggerLabel={triggerLabel}
      onValueChange={onChange}
      className={cn("h-[34px] w-40 rounded-lg border-jolu-border bg-jolu-bg px-3 text-xs font-medium text-jolu-text hover:bg-jolu-stage", className)}
    />
  )
}

type Tone = "secondary" | "soft" | "accent" | "danger"

const TONES: Record<Tone, string> = {
  secondary: "border border-jolu-border bg-jolu-bg text-jolu-text hover:bg-jolu-stage",
  soft: "bg-jolu-selected text-jolu-on hover:bg-jolu-pressed",
  accent: "bg-jolu-accent text-[#071B10] hover:brightness-95",
  danger: "bg-jolu-danger-soft text-jolu-danger hover:bg-jolu-danger-pressed",
}

/** 34 高的操作按钮；busy 时显示旋转图标并禁用，防止重复点击 */
export function ActionButton({ tone = "secondary", icon, busy, children, className, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & {
  tone?: Tone
  icon?: ReactNode
  busy?: boolean
}) {
  return (
    <button
      type="button"
      {...props}
      disabled={props.disabled || busy}
      aria-busy={busy || undefined}
      className={cn(
        "motion-button inline-flex h-[34px] shrink-0 items-center gap-[7px] rounded-lg px-3 text-xs font-medium outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-45",
        TONES[tone],
        className,
      )}
    >
      {busy ? <LoaderCircle aria-hidden className="size-3.5 animate-spin" /> : icon}
      {children}
    </button>
  )
}

/** 分段选择（连接方式、列表密度）：底色 side，选中项白底描边 */
export function Segmented<T extends string>({ items, value, onChange, label, disabled }: {
  items: { value: T; label: string }[]
  value: T
  onChange?: (value: T) => void
  label: string
  disabled?: boolean
}) {
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex shrink-0 gap-1 rounded-lg bg-jolu-side p-[3px]">
      {items.map((item) => {
        const active = item.value === value
        return (
          <button
            key={item.value}
            type="button"
            role="radio"
            aria-checked={active}
            disabled={disabled || !onChange}
            onClick={() => onChange?.(item.value)}
            className={cn(
              "h-[30px] rounded-lg border px-3 text-xs font-medium outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed",
              active ? "border-jolu-border bg-jolu-bg text-jolu-on" : "border-transparent text-jolu-text hover:text-jolu-on",
              disabled && !active && "opacity-60",
            )}
          >
            {item.label}
          </button>
        )
      })}
    </div>
  )
}

/** 画了但暂未接入的能力：入口旁标注「待接入」 */
export function PendingBadge({ children = "待接入" }: { children?: ReactNode }) {
  return <span className="inline-flex h-5 shrink-0 items-center rounded-[5px] bg-jolu-side px-1.5 text-[10px] font-normal text-jolu-muted">{children}</span>
}

/** 说明提示：side 底、圆角 9、内边距 14 */
export function Notice({ children, tone = "info", className }: { children: ReactNode; tone?: "info" | "danger"; className?: string }) {
  return (
    <div className={cn("flex items-start gap-2.5 rounded-[9px] p-3.5 text-[11px]", tone === "danger" ? "bg-jolu-danger-soft text-jolu-danger" : "bg-jolu-side text-jolu-muted", className)}>
      <Info aria-hidden className="mt-px size-4 shrink-0" />
      <div className="min-w-0 flex-1">{children}</div>
    </div>
  )
}
