import * as Dialog from "@radix-ui/react-dialog"
import { useRef, useState, type ReactNode } from "react"
import { LoaderCircle, TriangleAlert, X } from "lucide-react"
import { useDialogClose } from "@/hooks/useDialogClose"
import { JoluButton } from "./JoluButton"

/** 共用确认：确认前不执行，成功才关闭；失败和执行中保留对象与上下文。 */
export function ConfirmDialog({ title, description, confirmLabel, cancelLabel = "取消", busyLabel = "正在处理…", width = 480, compact = false, returnFocusTo, danger = false, children, onConfirm, onClose }: {
  title: string
  description: ReactNode
  confirmLabel: string
  cancelLabel?: string
  busyLabel?: string
  width?: number
  compact?: boolean
  returnFocusTo?: HTMLElement | null
  danger?: boolean
  children?: ReactNode
  onConfirm: () => void | boolean | Promise<void | boolean>
  onClose: () => void
}) {
  const { open, close } = useDialogClose(onClose)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const running = useRef(false)
  const cancelRef = useRef<HTMLButtonElement>(null)
  const trigger = useRef(returnFocusTo ?? document.activeElement)
  const dismiss = () => { if (!running.current) close() }
  const confirm = async () => {
    if (running.current) return
    running.current = true
    setBusy(true)
    setError(null)
    try {
      const result = await onConfirm()
      if (result === false) throw new Error("操作未完成，请检查原因后重试。")
      close()
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
      running.current = false
      setBusy(false)
    }
  }
  return <Dialog.Root open={open} onOpenChange={value => { if (!value) dismiss() }}>
    <Dialog.Portal>
      <Dialog.Overlay className={`motion-overlay fixed inset-x-0 bottom-0 ${compact ? "top-0" : "top-[30px]"} z-[150] bg-black/40`} />
      <Dialog.Content
        onOpenAutoFocus={event => { event.preventDefault(); cancelRef.current?.focus() }}
        onCloseAutoFocus={event => { event.preventDefault(); if (trigger.current instanceof HTMLElement && trigger.current.isConnected) trigger.current.focus() }}
        onPointerDownOutside={event => event.preventDefault()}
        onEscapeKeyDown={event => { if (running.current) event.preventDefault() }}
        className={`motion-dialog fixed left-1/2 z-[150] flex -translate-x-1/2 -translate-y-1/2 flex-col gap-4 overflow-y-auto rounded-2xl border border-jolu-border bg-jolu-cc-surface shadow-dialog outline-none ${compact ? "top-1/2 max-h-[calc(100dvh-12px)] max-w-[calc(100vw-12px)] p-4" : "top-[calc(50%+15px)] max-h-[calc(100dvh-70px)] max-w-[calc(100vw-48px)] p-6"}`}
        aria-busy={busy}
        style={{ width }}
      >
        <div className="flex items-center gap-3">
          <Dialog.Title className="min-w-0 flex-1 text-base font-semibold text-jolu-text">{title}</Dialog.Title>
          <button type="button" aria-label="关闭确认框" disabled={busy} onClick={dismiss} className="grid size-8 shrink-0 place-items-center rounded-lg text-jolu-muted outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-40"><X aria-hidden className="size-4" /></button>
        </div>
        <Dialog.Description className="flex items-start gap-2 rounded-lg bg-jolu-stage px-3 py-2.5 text-xs leading-relaxed text-jolu-muted">
          <TriangleAlert aria-hidden className="mt-0.5 size-4 shrink-0" /><span>{description}</span>
        </Dialog.Description>
        {children}
        {error && <p role="alert" className="rounded-lg bg-jolu-danger-soft px-3 py-2.5 text-xs leading-relaxed text-jolu-danger">{error}</p>}
        <div className="flex flex-wrap justify-end gap-2">
          <JoluButton ref={cancelRef} variant="outline" disabled={busy} onClick={dismiss}>{cancelLabel}</JoluButton>
          <JoluButton variant={danger ? "danger" : "primary"} disabled={busy} onClick={() => void confirm()}>{busy && <LoaderCircle aria-hidden className="animate-spin" />}{busy ? busyLabel : confirmLabel}</JoluButton>
        </div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>
}
