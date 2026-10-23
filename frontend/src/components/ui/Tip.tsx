import * as Tooltip from "@radix-ui/react-tooltip"
import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

/** Jolu.pen 画板 104：纯图标按钮用悬浮提示说明用途，延迟 350ms */
export const TIP_DELAY = 350

export function TipProvider({ children }: { children: ReactNode }) {
  return <Tooltip.Provider delayDuration={TIP_DELAY} skipDelayDuration={200}>{children}</Tooltip.Provider>
}

/**
 * 包一层可聚焦的子元素。禁用按钮不触发指针事件，调用方要在外面套一个 span 再传进来。
 */
export function Tip({ label, side = "bottom", children, className }: { label: ReactNode; className?: string; side?: "top" | "right" | "bottom" | "left"; children: ReactNode }) {
  return (
    <Tooltip.Root>
      <Tooltip.Trigger asChild>{children}</Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content
          side={side}
          sideOffset={6}
          collisionPadding={8}
          className={cn("motion-popover z-[120] rounded-md border border-jolu-border bg-jolu-bg px-2 py-1 text-[11px] text-jolu-text shadow-popover", className)}
        >
          {label}
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  )
}
