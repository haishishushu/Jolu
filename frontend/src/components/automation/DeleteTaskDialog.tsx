import * as Dialog from "@radix-ui/react-dialog"
import { useState } from "react"
import { CircleAlert, Trash2 } from "lucide-react"
import { JoluButton } from "@/components/ui/JoluButton"

/**
 * 删除任务确认（画板 68 / 69 第 10、11 格）：任务和运行记录一起删除，用量统计保留。
 * 成功后由调用方关闭弹窗和任务详情并刷新列表；失败时不关闭，在弹窗内显示后端返回的原因，可再次点击「删除」重试。
 */
export function DeleteTaskDialog({ name, onClose, onConfirm }: { name: string; onClose: () => void; onConfirm: () => Promise<void> }) {
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
          <Dialog.Title className="text-sm font-semibold text-jolu-text">删除任务「{name}」？</Dialog.Title>
          <p className="text-[11px] leading-[1.6] text-jolu-muted">任务和它的运行记录会一起删除，已产生的用量统计保留。此操作无法撤销。</p>
          {error && (
            <div role="alert" className="flex items-center gap-2 rounded-lg bg-jolu-status-error-bg px-2.5 py-2">
              <CircleAlert aria-hidden className="size-3.5 shrink-0 text-jolu-danger" />
              <span className="min-w-0 flex-1 text-[11px] text-jolu-danger">删除失败：{error}</span>
            </div>
          )}
          <div className="flex justify-end gap-2">
            <JoluButton size="sm" variant="outline" disabled={busy} onClick={onClose}>取消</JoluButton>
            <JoluButton size="sm" variant="danger" disabled={busy} onClick={() => void confirm()}><Trash2 aria-hidden />{busy ? "正在删除…" : "删除"}</JoluButton>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}
