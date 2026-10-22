import { cn } from "@/lib/utils"

export interface AppIconProps {
  size?: number
  className?: string
}

/** 与桌面、任务栏和托盘共用 Jolu 人物母版，浅深主题保持同一形象。 */
export function AppIcon({ size = 16, className }: AppIconProps) {
  return (
    <img
      src="/app-icon.png"
      alt="Jolu"
      width={size}
      height={size}
      className={cn("shrink-0 object-contain", className)}
      draggable={false}
    />
  )
}
