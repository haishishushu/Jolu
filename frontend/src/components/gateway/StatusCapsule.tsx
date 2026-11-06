import { cn } from "@/lib/utils"

export type CapsuleTone = "on" | "off" | "danger"

/**
 * 「已接入 / 未接入」状态胶囊（鼠鼠 2026-11-06 确认：连接栏 + 来源卡，可点击切换；Jolu.pen 暂无画板，外观待确认）。
 * 已接入：实心绿点 + 品牌绿字；未接入：空心灰点 + 灰字；异常（如已开启但未启动）：红点红字。
 * 传 onClick 时是按钮，结果以接口返回为准（由调用方确认、刷新），这里不先改显示。
 */
export function StatusCapsule({ tone, label, size = "md", title, disabled, busy, onClick }: {
  tone: CapsuleTone
  label: string
  size?: "md" | "sm"
  title?: string
  disabled?: boolean
  busy?: boolean
  onClick?: () => void
}) {
  const className = cn(
    "inline-flex shrink-0 items-center whitespace-nowrap rounded-full border bg-jolu-bg font-semibold outline-none",
    size === "md" ? "h-7 gap-[7px] px-3 text-[12px]" : "h-[22px] gap-[5px] px-2 text-[10.5px]",
    tone === "on" ? "border-jolu-accent/45 text-jolu-on" : tone === "danger" ? "border-jolu-danger/40 text-jolu-danger" : "border-jolu-border text-jolu-muted",
    onClick && "motion-button cursor-pointer transition-colors hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-50",
    busy && "opacity-60",
  )
  const dot = (
    <span aria-hidden className={cn(
      "shrink-0 rounded-full",
      size === "md" ? "size-[7px]" : "size-1.5",
      tone === "on" ? "bg-jolu-accent" : tone === "danger" ? "bg-jolu-danger" : "border-[1.5px] border-jolu-muted",
    )} />
  )
  if (!onClick) return <span className={className} title={title}>{dot}{label}</span>
  return (
    <button type="button" className={className} title={title} aria-label={title ? `${label}：${title}` : label} aria-busy={busy}
      disabled={disabled || busy} onClick={(event) => { event.stopPropagation(); onClick() }}>
      {dot}{label}
    </button>
  )
}
