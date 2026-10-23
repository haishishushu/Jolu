import { cn } from "@/lib/utils"
import * as Dialog from '@radix-ui/react-dialog'
import type { ReactNode } from 'react'
import { X } from 'lucide-react'
export const handoffButton = 'inline-flex shrink-0 whitespace-nowrap h-[34px] items-center justify-center gap-1.5 rounded-lg border border-jolu-border bg-jolu-bg px-3 text-xs font-medium text-jolu-text hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45 disabled:cursor-not-allowed'
export const handoffPrimary = cn(handoffButton, 'border-transparent bg-jolu-accent text-[#071B10] hover:bg-jolu-accent hover:brightness-95')
export const handoffInput = 'w-full rounded-lg border border-jolu-border bg-jolu-bg px-3 py-2 text-xs text-jolu-text outline-none focus:border-jolu-accent disabled:opacity-45'
export function HandoffShell({ title, children, onClose, chat = false }: {title:string; children:ReactNode; onClose:()=>void; chat?:boolean}) {
  return <Dialog.Root open onOpenChange={open => !open && onClose()}><Dialog.Portal>
    <Dialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/40" />
    <Dialog.Content aria-describedby={undefined} className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex max-h-[calc(100dvh-70px)] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 overflow-y-auto rounded-2xl border border-jolu-border bg-jolu-bg p-6 shadow-dialog outline-none" style={{width:chat?780:740,height:chat?840:undefined}}>
      <div className="flex items-center gap-3"><Dialog.Title className="min-w-0 flex-1 truncate text-[19px] font-semibold text-jolu-text">{title}</Dialog.Title><button aria-label="关闭" onClick={onClose} className={handoffButton}><X className="size-4" /></button></div>{children}
    </Dialog.Content>
  </Dialog.Portal></Dialog.Root>
}
export function HandoffNotice({children, danger=false}: {children:ReactNode;danger?:boolean}) {
  return <div role={danger?'alert':undefined} className={`rounded-lg p-3 text-xs leading-6 ${danger?'bg-jolu-danger/10 text-jolu-danger':'bg-jolu-selected text-jolu-on'}`}>{children}</div>
}
