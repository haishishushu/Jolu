import { forwardRef, type ButtonHTMLAttributes } from "react"
import { cn } from "@/lib/utils"

/**
 * Jolu.pen 画板 104「操作按钮 · 统一视觉与交互」：圆角 8 · 图文间距 6 · 常规 36 / 紧凑 32。
 *   primary 主要操作（绿底深字）· soft 次要绿色操作 · neutral 中性浅底 · outline 描边（取消）
 *   danger-soft 停止 / 取消任务 · danger 最终删除确认
 */
export type JoluButtonVariant = "primary" | "soft" | "neutral" | "outline" | "danger-soft" | "danger"

const VARIANTS: Record<JoluButtonVariant, string> = {
  primary: "bg-jolu-accent text-[#071B10] hover:brightness-[0.96] active:brightness-[0.92]",
  soft: "bg-jolu-selected text-jolu-on hover:brightness-[0.97] active:bg-jolu-pressed",
  neutral: "bg-jolu-side text-jolu-text hover:bg-jolu-stage active:bg-jolu-track [&_svg]:text-jolu-muted",
  outline: "border border-jolu-border bg-jolu-bg text-jolu-text hover:bg-jolu-stage active:bg-jolu-track",
  "danger-soft": "bg-jolu-danger-soft text-jolu-danger hover:brightness-[0.98] active:bg-jolu-danger-pressed",
  danger: "bg-jolu-danger-solid text-white hover:brightness-[0.95] active:brightness-[0.9]",
}

export const JoluButton = forwardRef<HTMLButtonElement, ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: JoluButtonVariant
  size?: "md" | "sm"
}>(function JoluButton({ variant = "neutral", size = "md", className, type = "button", ...props }, ref) {
  return (
    <button
      ref={ref}
      type={type}
      className={cn(
        "inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap rounded-lg text-xs font-medium outline-none",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jolu-accent disabled:pointer-events-none disabled:opacity-45 [&_svg]:size-4 [&_svg]:shrink-0",
        size === "md" ? "h-9 px-3.5" : "h-8 px-2.5",
        VARIANTS[variant],
        className,
      )}
      {...props}
    />
  )
})
