import { useRef, useState } from "react"
import * as Dialog from "@radix-ui/react-dialog"
import { Check, LoaderCircle, Trash2, X } from "lucide-react"
import { JoluButton } from "@/components/ui/JoluButton"
import type { AccountSelection } from "@/hooks/useAccountSelection"
import type { BatchAccountDeleteResult } from "@/lib/accountActions"

export interface BatchAccountTarget {
  id: string
  label: string
  provider: string
  current?: boolean
}

export function AccountBatchToolbar({ selection, total, disabled = false, onDelete }: {
  selection: AccountSelection
  total: number
  disabled?: boolean
  onDelete: () => void
}) {
  const count = selection.selectedIds.length
  if (!selection.active) return <JoluButton variant="outline" disabled={disabled || total === 0} onClick={selection.enter}>批量管理</JoluButton>
  return <div className="flex flex-wrap items-center gap-2" role="group" aria-label="批量管理账号">
    <JoluButton size="sm" variant="neutral" disabled={disabled || total === 0 || count === total} onClick={selection.selectAll}>全选当前列表</JoluButton>
    <JoluButton size="sm" variant="neutral" disabled={disabled || count === 0} onClick={selection.clear}>清空已选</JoluButton>
    <span className="text-xs text-jolu-muted" aria-live="polite">已选 {count} / {total}</span>
    <JoluButton size="sm" variant="danger-soft" disabled={disabled || count === 0} onClick={onDelete}><Trash2 aria-hidden />删除所选（{count}）</JoluButton>
    <JoluButton size="sm" variant="outline" disabled={disabled} onClick={selection.exit}>退出</JoluButton>
  </div>
}

export function BatchAccountDeleteDialog({ targets, hidden = false, onClose, onConfirm }: {
  targets: readonly BatchAccountTarget[]
  hidden?: boolean
  onClose: () => void
  onConfirm: (ids: string[]) => Promise<BatchAccountDeleteResult>
}) {
  // 确认期间固定目标快照；父级列表刷新或筛选变化不能更换本次删除对象。
  const [snapshot] = useState(() => targets.filter((target, index) => targets.findIndex(item => item.id === target.id) === index))
  const [busy, setBusy] = useState(false)
  const running = useRef(false)
  const [deletedIds, setDeletedIds] = useState<string[]>([])
  const [failed, setFailed] = useState<BatchAccountDeleteResult["failed"] | null>(null)
  const deleted = new Set(deletedIds)
  const failures = new Map(failed?.map(item => [item.id, item.message]))
  const pendingIds = failed ? failed.map(item => item.id) : snapshot.map(item => item.id)
  const close = () => { if (!running.current) onClose() }

  const confirm = async () => {
    if (running.current || pendingIds.length === 0) return
    running.current = true
    setBusy(true)
    try {
      const result = await onConfirm(pendingIds)
      const succeeded = new Set([...deletedIds, ...result.deletedIds])
      setDeletedIds([...succeeded])
      setFailed(result.failed)
      if (result.failed.length === 0) onClose()
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      setFailed(pendingIds.map(id => ({ id, message })))
    } finally {
      running.current = false
      setBusy(false)
    }
  }

  return <Dialog.Root open onOpenChange={open => { if (!open) close() }}>
    <Dialog.Portal>
      <Dialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/40" />
      <Dialog.Content aria-describedby={undefined} onEscapeKeyDown={event => { if (running.current) event.preventDefault() }} onPointerDownOutside={event => { if (running.current) event.preventDefault() }} className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex max-h-[calc(100dvh-70px)] w-[480px] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 rounded-2xl border border-jolu-border bg-jolu-bg p-6 shadow-dialog outline-none">
        <div className="flex shrink-0 items-start justify-between gap-3">
          <div className="min-w-0">
            <Dialog.Title className="text-base font-semibold text-jolu-text">删除所选账号？</Dialog.Title>
            <p className="mt-1 text-xs text-jolu-muted">共 {snapshot.length} 个账号连接{failed && ` · 已删除 ${deletedIds.length} 个 · 失败 ${failed.length} 个`}</p>
          </div>
          <Dialog.Close asChild><button type="button" aria-label="关闭删除确认" disabled={busy} className="grid size-7 shrink-0 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-40"><X aria-hidden className="size-4" /></button></Dialog.Close>
        </div>
        <p className="shrink-0 text-xs leading-relaxed text-jolu-text">仅从 Jolu 移除所选连接及保存的凭据，签到与用量历史保留。远端账号、API Key 和原工具登录不受影响。</p>
        {snapshot.some(target => target.current) && <p className="shrink-0 rounded-lg bg-jolu-side px-3 py-2 text-[11px] leading-relaxed text-jolu-muted">所选连接包含当前使用账号。删除后不会自动切换原工具或选择替代账号。</p>}
        <ul className="min-h-0 overflow-y-auto rounded-lg border border-jolu-border" aria-label="待删除的账号连接">
          {snapshot.map((target, index) => <li key={target.id} className="flex items-start gap-2 border-b border-jolu-border px-3 py-2.5 last:border-b-0">
            {deleted.has(target.id) && <Check aria-hidden className="mt-0.5 size-3.5 shrink-0 text-jolu-on" />}
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs"><span className="break-all font-medium text-jolu-text">{hidden ? `账号 ${index + 1}` : target.label}</span>{target.current && <span className="text-[10px] text-jolu-muted">当前使用</span>}</div>
              <p className="mt-1 text-[11px] text-jolu-muted">{target.provider}{deleted.has(target.id) && " · 已删除"}</p>
              {failures.has(target.id) && <p role="alert" className="mt-1 break-words text-[11px] leading-relaxed text-jolu-danger">{hidden ? "删除失败，请显示账号信息后查看原因。" : failures.get(target.id)}</p>}
            </div>
          </li>)}
        </ul>
        {failed && failed.length > 0 && <p className="shrink-0 text-xs text-jolu-danger" role="status">部分连接删除失败，已删除的连接不会再次提交。</p>}
        <div className="flex shrink-0 flex-wrap justify-end gap-2.5">
          <JoluButton variant="outline" disabled={busy} onClick={close}>{failed ? "关闭" : "取消"}</JoluButton>
          <JoluButton variant="danger" disabled={busy || pendingIds.length === 0} onClick={() => void confirm()}>{busy ? <LoaderCircle aria-hidden className="animate-spin" /> : <Trash2 aria-hidden />}{busy ? "正在删除…" : failed ? `重试失败项（${pendingIds.length}）` : `删除 ${snapshot.length} 个账号`}</JoluButton>
        </div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>
}
