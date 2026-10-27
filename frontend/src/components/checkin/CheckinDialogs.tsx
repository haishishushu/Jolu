import type { ReactNode } from "react"
import * as Dialog from "@radix-ui/react-dialog"
import { LoaderCircle, Trash2, X } from "lucide-react"
import { useDialogClose } from "@/hooks/useDialogClose"
import { CHECKIN_EDITIONS } from "@/lib/dailyCheckin"
import { connectionProviderId, type ProviderId } from "@/lib/providerCatalog"
import { JoluButton } from "@/components/ui/JoluButton"
import { PlatformLogo } from "@/components/brand/PlatformLogo"

function CheckinDialog({ title, description, open, onClose, footer, children }: {
  title: string; description: string; open: boolean; onClose: () => void; footer: string; children: ReactNode
}) {
  return <Dialog.Root open={open} onOpenChange={value => !value && onClose()}>
    <Dialog.Portal>
      <Dialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/40" />
      <Dialog.Content className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex max-h-[calc(100dvh-70px)] w-[640px] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 rounded-2xl border border-jolu-border bg-jolu-bg p-6 shadow-dialog outline-none">
        <div className="flex shrink-0 items-center justify-between gap-3">
          <Dialog.Title className="text-xl font-semibold text-jolu-text">{title}</Dialog.Title>
          <Dialog.Close aria-label="关闭弹窗" className="grid size-[30px] shrink-0 place-items-center rounded-lg text-jolu-muted outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent"><X aria-hidden className="size-4" /></Dialog.Close>
        </div>
        <Dialog.Description className="shrink-0 text-xs leading-relaxed text-jolu-muted">{description}</Dialog.Description>
        <div className="min-h-0 overflow-y-auto">{children}</div>
        <div className="flex shrink-0 justify-end"><JoluButton variant="outline" onClick={onClose}>{footer}</JoluButton></div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>
}

export function CheckinProviderDialog({ onClose, onPick }: { onClose: () => void; onPick: (provider: ProviderId) => void }) {
  const { open, close, finish } = useDialogClose(onClose)
  return <CheckinDialog title="添加账号" description="选择供应商与版本，继续连接账号。" open={open} onClose={close} footer="取消">
    <div className="grid grid-cols-1 gap-3 min-[520px]:grid-cols-2">
      {CHECKIN_EDITIONS.map(entry => {
        const provider = connectionProviderId({ platformId: entry.platform, edition: entry.edition })
        return <button key={entry.id} type="button" aria-label={`${entry.name} ${entry.region}`} disabled={!entry.manual || !provider} onClick={() => provider && finish(() => onPick(provider))} className="flex min-h-[100px] min-w-0 flex-col gap-2.5 rounded-[10px] border border-jolu-border p-3.5 text-left outline-none hover:border-jolu-accent hover:bg-jolu-selected focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-45 disabled:hover:border-jolu-border disabled:hover:bg-transparent">
          <span className="flex items-center gap-2.5 text-sm font-medium text-jolu-text"><PlatformLogo platform={entry.platform} size={22} /><span>{entry.name} {entry.region}</span></span>
          <span className="text-[11px] leading-relaxed text-jolu-muted">{entry.description}</span>
        </button>
      })}
    </div>
  </CheckinDialog>
}

export function CheckinRemoveAccountDialog({ account, busy, error, onClose, onConfirm }: {
  account: string; busy: boolean; error: string | null; onClose: () => void; onConfirm: () => void
}) {
  return <Dialog.Root open onOpenChange={open => { if (!open && !busy) onClose() }}>
    <Dialog.Portal>
      <Dialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/40" />
      <Dialog.Content aria-describedby={undefined} className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex w-[480px] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 rounded-2xl border border-jolu-border bg-jolu-bg p-6 shadow-dialog outline-none">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0"><Dialog.Title className="text-base font-semibold text-jolu-text">删除账号？</Dialog.Title><p className="mt-1 break-all text-xs text-jolu-muted">{account}</p></div>
          <Dialog.Close aria-label="关闭删除确认" disabled={busy} className="grid size-7 shrink-0 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-40"><X aria-hidden className="size-4" /></Dialog.Close>
        </div>
        <p className="text-xs leading-relaxed text-jolu-text">将从 Jolu 移除此账号连接及保存的凭据，签到记录保留。远端账号与原工具登录不受影响。</p>
        {error && <p role="alert" className="break-words text-xs text-jolu-danger">{error}</p>}
        <div className="flex justify-end gap-2.5"><JoluButton variant="outline" disabled={busy} onClick={onClose}>取消</JoluButton><JoluButton variant="danger" disabled={busy} onClick={onConfirm}>{busy ? <LoaderCircle aria-hidden className="animate-spin" /> : <Trash2 aria-hidden />}删除账号</JoluButton></div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>
}

