import * as Dialog from "@radix-ui/react-dialog"
import { useState } from "react"
import { X } from "lucide-react"
import { JoluButton } from "@/components/ui/JoluButton"

/** 取消任务确认（画板 68 第 09 格）：停止后续等待与调度，保留运行记录和已完成的文件修改 */
export function CancelTaskDialog({ name, running, onClose, onConfirm }: { name: string; running: boolean; onClose: () => void; onConfirm: () => Promise<void> }) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const confirm = async () => {
    setBusy(true)
    setError(null)
    try {
      await onConfirm()
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
      setBusy(false)
    }
  }
  return (
    <Dialog.Root open onOpenChange={(open) => !open && !busy && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/40" />
        <Dialog.Content
          aria-describedby={undefined}
          className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex w-[440px] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-3 rounded-2xl border border-jolu-border bg-jolu-bg p-6 shadow-dialog outline-none"
        >
          <Dialog.Title className="text-sm font-semibold text-jolu-text">取消「{name}」？</Dialog.Title>
          <p className="text-[11px] text-jolu-muted">
            停止后续等待与调度，保留运行记录和已经完成的文件修改。{running ? "正在执行的运行会被停止。" : ""}
          </p>
          {error && <p role="alert" className="text-[11px] text-jolu-danger">{error}</p>}
          <div className="flex justify-end gap-2">
            <JoluButton size="sm" variant="outline" disabled={busy} onClick={onClose}>继续等待</JoluButton>
            <JoluButton size="sm" variant="danger" disabled={busy} onClick={() => void confirm()}><X aria-hidden />{busy ? "正在取消…" : "确认取消"}</JoluButton>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}
