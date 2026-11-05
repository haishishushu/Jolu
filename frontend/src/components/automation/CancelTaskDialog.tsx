import { ConfirmDialog } from "@/components/ui/ConfirmDialog"

/** 画板 68：停止调度，保留运行记录和已经完成的文件修改。 */
export function CancelTaskDialog({ name, running, onClose, onConfirm }: { name: string; running: boolean; onClose: () => void; onConfirm: () => Promise<void> }) {
  return <ConfirmDialog title={`取消「${name}」？`}
    description={`停止后续等待与调度，保留运行记录和已经完成的文件修改。${running ? "正在执行的运行会被停止。" : ""}`}
    confirmLabel="确认取消" cancelLabel="继续等待" busyLabel="正在取消…" width={440} danger onClose={onClose} onConfirm={onConfirm} />
}
