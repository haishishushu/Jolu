import { ConfirmDialog } from "@/components/ui/ConfirmDialog"

/** 画板 68/69：保留用量统计；失败留在弹窗中，允许重试。 */
export function DeleteTaskDialog({ name, onClose, onConfirm }: { name: string; onClose: () => void; onConfirm: () => Promise<void> }) {
  return <ConfirmDialog title={`删除任务「${name}」？`}
    description="任务和它的运行记录会一起删除，已产生的用量统计保留。此操作无法撤销。"
    confirmLabel="删除" busyLabel="正在删除…" width={440} danger onClose={onClose} onConfirm={onConfirm} />
}
