import * as Dialog from "@radix-ui/react-dialog"
import { useEffect, useState, type ReactNode } from "react"
import { ArrowUpRight, Bot, ChevronRight, CircleAlert, CircleCheck, LoaderCircle, X } from "lucide-react"
import { cn } from "@/lib/utils"
import { api, isTauri, type ContinueCheckDto, type SessionItemDto } from "@/lib/api"
import { middleEllipsis, projectName, sessionTitle } from "@/lib/sessionThreads"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { providerName } from "@/components/usage/usageControls"
import type { PlatformId } from "@/types"

function Shell({ width, onClose, children, busy }: { width: number; onClose: () => void; children: ReactNode; busy?: boolean }) {
  return (
    <Dialog.Root open onOpenChange={(open) => !open && !busy && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/40" />
        <Dialog.Content
          aria-describedby={undefined}
          className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex max-h-[calc(100dvh-70px)] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 overflow-y-auto rounded-2xl border border-jolu-border bg-jolu-bg p-6 shadow-dialog outline-none"
          style={{ width }}
        >
          {children}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

const secondary = "inline-flex h-[34px] items-center rounded-lg border border-jolu-border bg-jolu-bg px-3 text-xs font-medium text-jolu-text outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45"
const primary = "inline-flex h-[34px] items-center gap-1.5 rounded-lg bg-jolu-accent px-3 text-xs font-medium text-[#071B10] outline-none hover:brightness-95 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-45"

/**
 * 在原工具继续（画板 44）：检查原工具与项目目录，确认后在新的控制台窗口里续聊。
 * 沿用原工具当前的登录状态；打开失败保留当前会话，可重试或复制 ID（画板 46）。
 */
export function ContinueDialog({ session, accountKnown, onClose, onDone }: {
  session: SessionItemDto
  accountKnown: boolean
  onClose: () => void
  onDone: () => void
}) {
  const name = providerName(session.platform)
  const [check, setCheck] = useState<ContinueCheckDto | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [opening, setOpening] = useState(false)

  useEffect(() => {
    if (!isTauri) {
      setCheck({ supported: true, tool_found: true, cwd: session.cwd, cwd_accessible: true })
      return
    }
    api.sessionContinueCheck(session.platform, session.session_id).then(setCheck, (reason: unknown) => setError(String(reason)))
  }, [session.platform, session.session_id, session.cwd])

  const ready = !!check && check.supported && check.tool_found && check.cwd_accessible
  const open = async () => {
    if (!isTauri) return onDone()
    setOpening(true)
    setError(null)
    try {
      await api.sessionContinue(session.platform, session.session_id)
      onDone()
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setOpening(false)
    }
  }

  const item = (ok: boolean | null, label: string) => (
    <span className="flex items-center gap-1.5 text-[11px] text-jolu-muted">
      {ok === null ? <LoaderCircle aria-hidden className="size-3.5 animate-spin" /> : ok ? <CircleCheck aria-hidden className="size-3.5 text-jolu-on" /> : <CircleAlert aria-hidden className="size-3.5 text-jolu-danger" />}
      {label}
    </span>
  )

  return (
    <Shell width={540} onClose={onClose} busy={opening}>
      <div className="flex items-center gap-2.5">
        <PlatformLogo platform={session.platform as PlatformId} size={24} />
        <Dialog.Title className="flex-1 text-[19px] font-semibold text-jolu-text">在 {name} 中继续</Dialog.Title>
        <Dialog.Close aria-label="关闭" disabled={opening} className="grid size-[30px] place-items-center rounded-lg bg-jolu-side text-jolu-muted outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <X aria-hidden className="size-4" />
        </Dialog.Close>
      </div>
      <p className="text-xs text-jolu-muted">打开原工具并定位这条会话，后续消息仍由原工具处理。</p>
      <div className="flex flex-col gap-2.5 rounded-xl bg-jolu-side p-3.5">
        <span className="text-sm font-semibold text-jolu-text">{sessionTitle(session)}</span>
        {([["会话", middleEllipsis(session.session_id)], ["目录", check?.cwd ?? session.cwd ?? "未识别"], ["来源", `本机 ${name} 记录`]] as const).map(([label, value]) => (
          <span key={label} className="flex gap-3 text-[11px]"><span className="w-6 shrink-0 text-jolu-muted">{label}</span><span className="min-w-0 truncate text-jolu-text" title={value}>{value}</span></span>
        ))}
      </div>
      <div className="flex gap-[18px]">
        {item(check ? check.tool_found : null, check && !check.tool_found ? `未检测到 ${name}` : `已检测到 ${name}`)}
        {item(check ? check.cwd_accessible : null, check && !check.cwd_accessible ? "项目目录不可访问" : "项目目录可访问")}
      </div>
      <div className="flex flex-col gap-1.5 rounded-lg bg-jolu-selected p-3">
        <span className="text-xs font-medium text-jolu-on">沿用原工具当前的登录状态</span>
        <span className="text-[11px] text-jolu-on">{accountKnown ? "Jolu 不会据此自动切换原工具账号。" : "这条历史会话未归属账号。Jolu 不会据此自动切换原工具账号。"}</span>
      </div>
      {(error || (check && !ready)) && (
        <p role="alert" className="text-[11px] text-jolu-danger">
          {error ?? (check && !check.tool_found ? `请安装或配置 ${name} 后重试。当前仍可查看记录与复制会话 ID。` : "项目目录不可访问，无法在原目录打开。")}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <button type="button" className={secondary} disabled={opening} onClick={onClose}>取消</button>
        <button type="button" className={primary} disabled={!ready || opening} onClick={() => void open()}>
          {opening ? `正在打开 ${name}…` : `打开 ${name}`}
          <ArrowUpRight aria-hidden className="size-3.5" />
        </button>
      </div>
    </Shell>
  )
}

/** 从会话创建自动化（画板 56）：确认后前往「自动化 → 新建任务」，不会立即执行 */
export function AutomationConfirmDialog({ session, onClose, onConfirm }: { session: SessionItemDto; onClose: () => void; onConfirm: () => void }) {
  return (
    <Shell width={520} onClose={onClose}>
      <div className="flex items-center gap-2.5">
        <Bot aria-hidden className="size-[22px] text-jolu-on" />
        <Dialog.Title className="text-xl font-semibold text-jolu-text">为此会话创建自动化任务？</Dialog.Title>
      </div>
      <p className="text-xs text-jolu-muted">将前往“自动化 → 新建任务”，并带入当前会话和项目。</p>
      <div className="flex flex-col gap-1.5 rounded-xl bg-jolu-side p-3.5">
        <span className="text-sm font-semibold text-jolu-text">{sessionTitle(session)}</span>
        <span className="text-xs text-jolu-muted">{providerName(session.platform)} · {projectName(session.cwd)}</span>
        <span className="text-[10px] text-jolu-muted" title={session.session_id}>{middleEllipsis(session.session_id)}</span>
      </div>
      <div className={cn("flex flex-col gap-1.5 rounded-lg bg-jolu-selected p-3")}>
        <span className="text-xs text-jolu-on">确认后仅打开任务配置，任务创建后才会执行。</span>
        <span className="text-[11px] text-jolu-on">请在新建页面选择执行账号、填写任务内容与启动方式；已有会话内容不会被清空。</span>
      </div>
      <div className="flex justify-end gap-2">
        <button type="button" className={secondary} onClick={onClose}>取消</button>
        <button type="button" className={primary} onClick={onConfirm}>继续新建<ChevronRight aria-hidden className="size-3.5" /></button>
      </div>
    </Shell>
  )
}
