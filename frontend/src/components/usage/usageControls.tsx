import { LoadingSurface, type SkeletonLayout } from "@/components/ui/LoadingSurface"
import * as Menu from "@radix-ui/react-dropdown-menu"
import { forwardRef, type ReactNode } from "react"
import { Check, ChartNoAxesCombined, ChevronDown, LoaderCircle, RefreshCw, RotateCcw, Square, SquareCheck } from "lucide-react"
import { cn } from "@/lib/utils"
import { PROVIDERS, providerEntry } from "@/lib/providerCatalog"
import { UNATTRIBUTED_ACCOUNT, accountLabel, accountsTriggerLabel } from "@/lib/usageFilters"
import { JoluButton } from "@/components/ui/JoluButton"
import type { Connection, PlatformId } from "@/types"

/** 统计卡片：12px 圆角、1px 描边（画板 30） */
export function Card({ className, children }: { className?: string; children: ReactNode }) {
  return <section className={cn("min-w-0 rounded-xl border border-jolu-border bg-jolu-bg", className)}>{children}</section>
}

export function CardTitle({ title, note, children }: { title: string; note?: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex min-h-5 items-center justify-between gap-3">
      <h3 className="truncate text-sm font-semibold text-jolu-text">{title}</h3>
      {children ?? (note && <span className="shrink-0 text-[10px] text-jolu-muted">{note}</span>)}
    </div>
  )
}

/** 四项指标卡：名称 11px、数值 25px、说明 10px */
export function MetricCard({ icon, label, value, note }: { icon?: ReactNode; label: string; value: string; note: ReactNode }) {
  return (
    <Card className="flex h-[100px] flex-col gap-2 p-3.5">
      <span className="flex min-w-0 items-center gap-1.5 text-[11px] text-jolu-muted">{icon && <span aria-hidden className="grid size-3.5 shrink-0 place-items-center [&_svg]:size-3.5">{icon}</span>}<span className="truncate">{label}</span></span>
      <span className="tnum truncate text-[25px] font-semibold leading-none text-jolu-text">{value}</span>
      <span className="truncate text-[10px] text-jolu-muted">{note}</span>
    </Card>
  )
}

const FILTER_CLASS =
  "inline-flex h-[34px] shrink-0 items-center gap-2 rounded-lg border border-jolu-border bg-jolu-bg px-3 text-xs text-jolu-text outline-none transition-colors hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent data-[state=open]:bg-jolu-stage disabled:cursor-not-allowed disabled:opacity-45"

/** 筛选按钮：概览 / 分析页图标在前（画板 30），调用明细页箭头在后（画板 34） */
export const FilterTrigger = forwardRef<HTMLButtonElement, React.ButtonHTMLAttributes<HTMLButtonElement> & { leading?: ReactNode; trailingChevron?: boolean }>(
  function FilterTrigger({ leading, trailingChevron, className, children, ...props }, ref) {
    return (
      <button ref={ref} type="button" className={cn(FILTER_CLASS, className)} {...props}>
        {leading ?? (!trailingChevron && <ChevronDown aria-hidden className="size-4 text-jolu-muted" />)}
        <span className="max-w-[180px] truncate">{children}</span>
        {trailingChevron && <ChevronDown aria-hidden className="size-3.5 text-jolu-muted" />}
      </button>
    )
  },
)

/** 浅底工具按钮：刷新、导出 CSV；`bordered` 用于「重置」（画板 34） */
export function ToolButton({ icon, children, bordered, busy, className, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement> & { icon: ReactNode; bordered?: boolean; busy?: boolean }) {
  return (
    <button
      type="button"
      aria-busy={busy || undefined}
      className={cn(
        "motion-button inline-flex h-[34px] shrink-0 items-center gap-1.5 rounded-lg px-3 text-xs font-medium text-jolu-text outline-none transition-colors focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-45",
        bordered ? "border border-jolu-border bg-jolu-bg hover:bg-jolu-stage" : "bg-jolu-side hover:bg-jolu-stage",
        className,
      )}
      {...props}
    >
      <span aria-hidden className={cn("grid size-4 place-items-center text-jolu-muted [&_svg]:size-4", busy && "[&_svg]:animate-spin")}>{icon}</span>
      {children}
    </button>
  )
}

export function RefreshButton({ loading, onClick }: { loading: boolean; onClick: () => void }) {
  // 加载时禁用，避免重复触发（画板 38「交互」）
  return <ToolButton icon={<RefreshCw />} busy={loading} disabled={loading} onClick={onClick}>刷新</ToolButton>
}

export function ResetButton({ onClick, disabled }: { onClick: () => void; disabled?: boolean }) {
  return <ToolButton bordered icon={<RotateCcw />} onClick={onClick} disabled={disabled}>重置</ToolButton>
}

const MENU_CLASS = "motion-popover z-[100] min-w-[var(--radix-dropdown-menu-trigger-width)] max-w-[320px] overflow-y-auto overscroll-contain rounded-xl border border-jolu-border bg-jolu-bg p-1.5 shadow-popover"
const ITEM_CLASS = "relative flex h-8 cursor-default select-none items-center gap-2 rounded-md px-2 text-xs text-jolu-text outline-none data-[highlighted]:bg-jolu-side data-[disabled]:opacity-40"

export interface MenuOption {
  value: string
  label: string
  icon?: ReactNode
}

/** 单选下拉：全部供应商 / 全部模型 / 全部状态 */
export function MenuFilter({ value, options, onChange, label, trailingChevron }: {
  value: string
  options: MenuOption[]
  onChange: (value: string) => void
  label: string
  trailingChevron?: boolean
}) {
  const selected = options.find((option) => option.value === value) ?? options[0]
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger asChild>
        <FilterTrigger aria-label={label} trailingChevron={trailingChevron}>{selected?.label}</FilterTrigger>
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content align="start" sideOffset={6} collisionPadding={12} className={MENU_CLASS} style={{ maxHeight: "min(320px, var(--radix-dropdown-menu-content-available-height))" }}>
          <Menu.RadioGroup value={value} onValueChange={onChange}>
            {options.map((option) => (
              <Menu.RadioItem key={option.value} value={option.value} className={cn(ITEM_CLASS, "pr-7 data-[state=checked]:text-jolu-on")}>
                {option.icon && <span aria-hidden className="grid size-4 place-items-center">{option.icon}</span>}
                <span className="truncate">{option.label}</span>
                <Menu.ItemIndicator className="absolute right-2"><Check aria-hidden className="size-3.5" /></Menu.ItemIndicator>
              </Menu.RadioItem>
            ))}
          </Menu.RadioGroup>
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}

export const ALL = "__all__"

export function providerOptions(): MenuOption[] {
  // 自定义供应商没有本机用量来源，不进用量筛选
  return [{ value: ALL, label: "全部供应商" }, ...PROVIDERS.filter((p) => p.id !== "custom").map((p) => ({ value: p.id, label: p.name }))]
}

/** 账号筛选：多选（画板 38「账号筛选」）；勾选「全部账号」即清空选择 */
export function AccountFilter({ accounts, value, onChange }: {
  accounts: readonly Connection[]
  value: readonly string[]
  onChange: (next: string[]) => void
}) {
  const toggle = (id: string) => onChange(value.includes(id) ? value.filter((v) => v !== id) : [...value, id])
  const row = (id: string, label: string, checked: boolean, onSelect: () => void) => (
    <Menu.CheckboxItem
      key={id}
      checked={checked}
      onSelect={(event) => {
        event.preventDefault()
        onSelect()
      }}
      className={cn(ITEM_CLASS, "h-7", checked && "bg-jolu-selected data-[highlighted]:bg-jolu-selected")}
    >
      {checked ? <SquareCheck aria-hidden className="size-[15px] text-jolu-on" /> : <Square aria-hidden className="size-[15px] text-jolu-muted" />}
      <span className="truncate">{label}</span>
    </Menu.CheckboxItem>
  )
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger asChild>
        <FilterTrigger aria-label="账号筛选">{accountsTriggerLabel(value, accounts)}</FilterTrigger>
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content align="start" sideOffset={6} collisionPadding={12} className={cn(MENU_CLASS, "w-[260px]")} style={{ maxHeight: "min(360px, var(--radix-dropdown-menu-content-available-height))" }}>
          {row("all", "全部账号", value.length === 0, () => onChange([]))}
          {accounts.map((account) => row(account.id, accountLabel(account), value.includes(account.id), () => toggle(account.id)))}
          {row(UNATTRIBUTED_ACCOUNT, "未归属账号", value.includes(UNATTRIBUTED_ACCOUNT), () => toggle(UNATTRIBUTED_ACCOUNT))}
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}

export function providerName(platform: string): string {
  return PROVIDERS.some((p) => p.id === platform) ? providerEntry(platform as PlatformId).name : platform
}

/** 首次加载骨架（画板 38「正在读取本地用量」） */
export function LoadingCard({ title = "正在读取本地用量", layout = "table" }: { title?: string; layout?: SkeletonLayout }) {
  return <LoadingSurface label={title} layout={layout} />
}

/** 筛选无结果（画板 38）；没有可清除的筛选时不显示按钮 */
export function EmptyCard({ onClear }: { onClear?: () => void }) {
  return (
    <Card className="flex flex-col items-start gap-3 p-4">
      <ChartNoAxesCombined aria-hidden className="size-[26px] text-jolu-muted" />
      <span className="text-base font-semibold text-jolu-text">这个范围内暂无用量</span>
      <span className="text-xs text-jolu-muted">尝试扩大日期范围，或清除账号筛选。</span>
      {onClear && <ToolButton icon={<RotateCcw />} onClick={onClear}>清除筛选</ToolButton>}
    </Card>
  )
}

/** 读取失败：没有上次数据时整块显示；有上次数据时只在顶部提示并保留数据 */
export function ErrorNotice({ error, onRetry, stale }: { error: string; onRetry: () => void; stale: boolean }) {
  return (
    <div role="alert" className="flex items-center justify-between gap-3 rounded-[10px] bg-jolu-danger-soft px-4 py-2.5 text-xs text-jolu-danger">
      <span className="min-w-0 truncate">{stale ? `刷新失败，暂时显示上次数据：${error}` : `本地用量读取失败：${error}`}</span>
      <JoluButton size="sm" variant="danger-soft" onClick={onRetry}>重试</JoluButton>
    </div>
  )
}

export function Spinner() {
  return <LoaderCircle aria-hidden className="size-4 animate-spin text-jolu-muted" />
}
