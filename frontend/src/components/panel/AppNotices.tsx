import { useEffect, useRef } from "react"
import { api, isTauri, listenEvent } from "@/lib/api"
import { noticeToast, parseAppNotice, type NoticeTarget } from "@/lib/appNotices"
import { useToast } from "@/components/ui/Toast"

/** 订阅完成后补读待呈现通知，启动期间先到达的提醒也不会漏掉。 */
export function AppNotices({ onOpenAccount, onOpenTask, deferredTarget, onDeferredHandled }: { onOpenAccount: (id: string) => void; onOpenTask: (id: number) => void; deferredTarget?: NoticeTarget | null; onDeferredHandled?: () => void }) {
  const toast = useToast()
  const actions = useRef({ onOpenAccount, onOpenTask })
  actions.current = { onOpenAccount, onOpenTask }
  const open = (target: NoticeTarget): boolean | void => {
    if (document.querySelector('[role="dialog"]:not([data-state="closed"]), [role="alertdialog"]:not([data-state="closed"])')) {
      toast.show({ scope: `pending:${target.kind}:${target.id}`, tone: "warn", title: "请先关闭当前弹窗", persistent: true, detail: `保存或取消当前操作后，可继续查看${target.kind === "account" ? "账号" : "任务"}`, action: { label: "继续查看", onClick: () => open(target) } })
      return false
    }
    if (target.kind === "account") actions.current.onOpenAccount(target.id)
    else actions.current.onOpenTask(target.id)
  }
  useEffect(() => {
    if (deferredTarget) { open(deferredTarget); onDeferredHandled?.() }
  }, [deferredTarget, onDeferredHandled])
  useEffect(() => {
    if (!isTauri) return
    let stopped = false, reading = false, again = false
    let unlisten: (() => void) | undefined
    const seen = new Set<number>()
    const flush = async () => {
      if (reading) { again = true; return }
      reading = true
      try {
        do {
          again = false
          const notices = await api.takeAppNotices()
          if (stopped) return
          for (const value of notices) {
            const notice = parseAppNotice(value)
            if (!notice || seen.has(notice.id)) continue
            seen.add(notice.id)
            toast.show(noticeToast(notice, {
              account: id => open({ kind: "account", id, nonce: Date.now() }),
              task: id => open({ kind: "task", id, nonce: Date.now() }),
            }))
          }
        } while (again && !stopped)
      } catch { /* 待读队列保留在后端，下次唤醒或主窗口重新挂载时补读。 */ }
      finally { reading = false }
    }
    void listenEvent("app-notice", () => { if (!stopped) void flush() }).then(off => {
      if (stopped) off()
      else { unlisten = off; void flush() }
    }).catch(() => {})
    return () => { stopped = true; unlisten?.() }
  }, [toast])
  return null
}
