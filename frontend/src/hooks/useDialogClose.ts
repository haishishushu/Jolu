import { useCallback, useEffect, useRef, useState } from "react"
import { reducedMotion } from "@/lib/motion"

/** 先让 Radix 进入 closed 状态，再通知父级卸载；重复关闭不会多次执行回调。 */
export function useDialogClose(onDismiss: () => void) {
  const [open, setOpen] = useState(true)
  const pending = useRef(false)
  const dismiss = useRef(onDismiss)
  dismiss.current = onDismiss
  const completion = useRef<(() => void) | null>(null)
  const finish = useCallback((action: () => void) => {
    if (pending.current) return
    pending.current = true
    completion.current = action
    setOpen(false)
  }, [])
  const close = useCallback(() => finish(() => dismiss.current()), [finish])
  useEffect(() => {
    if (open) return
    const timer = window.setTimeout(() => completion.current?.(), reducedMotion() ? 0 : 140)
    return () => window.clearTimeout(timer)
  }, [open])
  return { open, close, finish }
}
