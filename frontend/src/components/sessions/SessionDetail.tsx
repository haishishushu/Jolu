import { useContentMotion } from "@/lib/motion"
import { LoadingSurface } from "@/components/ui/LoadingSurface"
import * as Menu from "@radix-ui/react-dropdown-menu"
import { useEffect, useRef, useState } from "react"
import { ArrowUpRight, Bot, ChartColumn, ChevronDown, ChevronRight, ChevronsUp, Copy, Ellipsis, Eye, FileCode, FileDown, Folder } from "lucide-react"
import { cn } from "@/lib/utils"
import { api, isTauri, type MessageDto, type MessagePageDto, type SessionItemDto } from "@/lib/api"
import { canContinue, consumption, middleEllipsis, projectName, sessionTitle, spanLabel, statusLabel } from "@/lib/sessionThreads"
import { formatTokens, formatUsd, modelDisplayName } from "@/lib/usageStats"
import { accountLabel } from "@/lib/usageFilters"
import { demoMessages } from "@/mock/sessionThreads"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { Tip } from "@/components/ui/Tip"
import { useToast } from "@/components/ui/Toast"
import { providerName } from "@/components/usage/usageControls"
import type { Connection, PlatformId } from "@/types"

type DetailTab = "messages" | "info"

/** 会话详情（画板 40 / 42）：消息记录与会话信息两个页签，正文只读 */
export function SessionDetail({ session, connections, onDrillUsage, onContinue, onCreateAutomation }: {
  session: SessionItemDto
  connections: readonly Connection[]
  onDrillUsage: () => void
  onContinue: () => void
  onCreateAutomation: () => void
}) {
  const toast = useToast()
  const [tab, setTab] = useState<DetailTab>("messages")
  const [page, setPage] = useState<MessagePageDto | null>(null)
  const [messages, setMessages] = useState<MessageDto[]>([])
  const [error, setError] = useState<string | null>(null)
  const [loadingMore, setLoadingMore] = useState(false)
  const bodyMotion = useContentMotion(`${session.session_id}-${tab}-${!!page}`)
  const revision = useRef(0)
  const platform = session.platform as PlatformId
  const name = providerName(session.platform)
  const account = session.connection_id ? connections.find((c) => c.id === session.connection_id) : null

  const load = () => {
    const current = ++revision.current
    setError(null)
    if (!isTauri) {
      const demo = demoMessages(session.platform, session.session_id)
      setPage(demo)
      setMessages(demo.messages)
      return
    }
    setPage(null)
    api.sessionMessages(session.platform, session.session_id, null)
      .then((result) => {
        if (current !== revision.current) return
        setPage(result)
        setMessages(result.messages)
      })
      .catch((reason: unknown) => current === revision.current && setError(reason instanceof Error ? reason.message : String(reason)))
  }

  useEffect(load, [session.platform, session.session_id])

  const loadEarlier = () => {
    if (!page || !page.offset || !isTauri) return
    setLoadingMore(true)
    const current = revision.current
    api.sessionMessages(session.platform, session.session_id, page.offset)
      .then((result) => {
        if (current !== revision.current) return
        setPage({ ...result, total: page.total })
        setMessages((value) => [...result.messages, ...value])
      })
      .catch((reason: unknown) => toast.danger("读取更早消息失败", reason instanceof Error ? reason.message : String(reason)))
      .finally(() => setLoadingMore(false))
  }

  const copy = (text: string, what: string) => {
    void navigator.clipboard.writeText(text).then(() => toast.success("已复制", what), () => toast.danger("复制失败", "请手动选择文字复制"))
  }

  const continueButton = (
    <button
      type="button"
      disabled={!canContinue(session.platform) || !session.has_source}
      onClick={onContinue}
      className="inline-flex h-[34px] shrink-0 items-center gap-1.5 rounded-lg bg-jolu-accent px-3 text-xs font-medium text-[#071B10] outline-none hover:brightness-95 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-45"
    >
      在 {name} 中继续
      <ArrowUpRight aria-hidden className="size-3.5" />
    </button>
  )

  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col gap-3 rounded-xl border border-jolu-border bg-jolu-bg p-5">
      <div className="flex items-start gap-2.5">
        <div className="flex min-w-0 flex-1 flex-col gap-[7px]">
          <h3 className="truncate text-lg font-semibold text-jolu-text" title={sessionTitle(session)}>{sessionTitle(session)}</h3>
          <span className="truncate whitespace-pre text-[11px] text-jolu-muted">{name}  /  {projectName(session.cwd)}  /  {spanLabel(session.first_ts, session.last_ts)}</span>
        </div>
        <Menu.Root modal={false}>
          <Menu.Trigger aria-label="更多会话操作" className="grid size-[30px] shrink-0 place-items-center rounded-lg bg-jolu-side text-jolu-muted outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent data-[state=open]:bg-jolu-stage">
            <Ellipsis aria-hidden className="size-4" />
          </Menu.Trigger>
          <Menu.Portal>
            <Menu.Content align="end" sideOffset={6} className="motion-popover z-[100] w-[200px] rounded-xl border border-jolu-border bg-jolu-bg p-1.5 shadow-popover">
              <MenuItem icon={<Copy />} onSelect={() => copy(session.session_id, "会话 ID")}>复制会话 ID</MenuItem>
              <MenuItem icon={<Folder />} disabled={!session.cwd} onSelect={() => session.cwd && copy(session.cwd, "项目路径")}>复制项目路径</MenuItem>
              <MenuItem icon={<ChartColumn />} onSelect={onDrillUsage}>查看用量明细</MenuItem>
              <MenuItem icon={<ArrowUpRight />} disabled={!canContinue(session.platform) || !session.has_source} onSelect={onContinue}>在原工具中继续</MenuItem>
            </Menu.Content>
          </Menu.Portal>
        </Menu.Root>
      </div>

      <div className="flex h-7 items-center gap-2">
        <Chip on={session.status === "done" || session.status === "running"}>{statusLabel(session.status)}</Chip>
        <Chip>{account ? accountLabel(account) : "未归属账号"}</Chip>
        <Chip>{page ? (page.available ? `${page.total} 条消息` : "无消息正文") : "— 条消息"}</Chip>
        <Chip>{formatTokens(consumption(session))} Token</Chip>
      </div>

      <div role="tablist" className="flex h-8 shrink-0 gap-6 border-b border-jolu-border">
        {(["messages", "info"] as const).map((t) => (
          <button
            key={t}
            type="button"
            role="tab"
            aria-selected={tab === t}
            onClick={() => setTab(t)}
            className={cn("-mb-px border-b-2 text-xs font-medium outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent", tab === t ? "border-jolu-accent text-jolu-on" : "border-transparent text-jolu-muted hover:text-jolu-text")}
          >
            {t === "messages" ? "消息记录" : "会话信息"}
          </button>
        ))}
      </div>

      <div ref={bodyMotion} className="-mr-2 min-h-0 flex-1 overflow-y-auto pr-2">
        {tab === "messages" ? (
          error ? (
            <StateBox title="会话源暂时不可读" note="读取失败不代表会话结束；旧记录保留。" action={<SmallButton onClick={load}>重新读取</SmallButton>} detail={error} />
          ) : !page ? (
            <LoadingSurface layout="messages" label="正在读取消息" />
          ) : !page.available ? (
            <StateBox
              title="这个来源未保留消息正文"
              note="会话标题和 Token 仍可查看。不会将统计记录拼成对话内容。"
              action={<SmallButton accent onClick={() => setTab("info")}>查看会话信息<ChevronRight aria-hidden className="size-3.5" /></SmallButton>}
            />
          ) : (
            <div className="flex flex-col gap-3">
              {page.offset > 0 && (
                <button type="button" onClick={loadEarlier} disabled={loadingMore} className="flex h-6 items-center justify-center gap-1 text-[10px] text-jolu-muted outline-none hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent">
                  <ChevronsUp aria-hidden className="size-3" />
                  {loadingMore ? "正在读取…" : `加载更早的 ${page.offset} 条消息`}
                </button>
              )}
              {messages.map((m, index) => <MessageView key={index} message={m} platform={platform} name={name} />)}
              {!messages.length && <span className="text-xs text-jolu-muted">源文件里没有可显示的消息</span>}
            </div>
          )
        ) : (
          <InfoPanel session={session} page={page} accountText={account ? accountLabel(account) : "未归属 · 本机记录未提供账号身份"} onCopy={copy} onDrillUsage={onDrillUsage} />
        )}
      </div>

      <div className="flex h-[34px] shrink-0 items-center gap-3">
        <Eye aria-hidden className="size-[15px] text-jolu-muted" />
        <span className="text-[11px] text-jolu-muted">本机会话 · 只读</span>
        <span className="flex-1" />
        <button type="button" onClick={onCreateAutomation} className="inline-flex h-[34px] items-center gap-1.5 rounded-lg bg-jolu-selected px-3 text-xs font-medium text-jolu-on outline-none hover:bg-jolu-pressed focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <Bot aria-hidden className="size-4" />
          创建自动化
        </button>
        {/* 压缩会话需要调用模型整理交接文档（画板 48–55），执行链路尚未接入 */}
        <Tip label="压缩会话待接入">
          <span className="flex">
            <button type="button" disabled className="inline-flex h-[34px] items-center gap-1.5 rounded-lg bg-jolu-selected px-3 text-xs font-medium text-jolu-on opacity-45">
              <FileDown aria-hidden className="size-4" />
              压缩会话
            </button>
          </span>
        </Tip>
        {canContinue(session.platform) && session.has_source
          ? continueButton
          : <Tip label={session.has_source ? `${name} 暂无可靠的继续入口` : "没有可读的源文件，无法定位会话"}><span className="flex">{continueButton}</span></Tip>}
      </div>
    </section>
  )
}

function MenuItem({ icon, children, onSelect, disabled }: { icon: React.ReactNode; children: React.ReactNode; onSelect: () => void; disabled?: boolean }) {
  return (
    <Menu.Item disabled={disabled} onSelect={onSelect} className="flex h-8 cursor-default select-none items-center gap-2 rounded-md px-2 text-xs text-jolu-text outline-none data-[highlighted]:bg-jolu-selected data-[disabled]:opacity-40 [&_svg]:size-[15px] [&_svg]:text-jolu-muted data-[highlighted]:[&_svg]:text-jolu-on">
      {icon}
      {children}
    </Menu.Item>
  )
}

function Chip({ on, children }: { on?: boolean; children: React.ReactNode }) {
  return <span className={cn("inline-flex h-[22px] items-center rounded-[5px] px-2 text-[10px]", on ? "bg-jolu-selected text-jolu-on" : "bg-jolu-stage text-jolu-muted")}>{children}</span>
}

function SmallButton({ accent, onClick, children }: { accent?: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button type="button" onClick={onClick} className={cn("inline-flex h-[34px] items-center gap-1.5 rounded-lg px-3 text-xs font-medium outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent", accent ? "bg-jolu-selected text-jolu-on hover:bg-jolu-pressed" : "bg-jolu-side text-jolu-text hover:bg-jolu-stage")}>
      {children}
    </button>
  )
}

function StateBox({ title, note, action, detail }: { title: string; note: string; action: React.ReactNode; detail?: string }) {
  return (
    <div className="flex flex-col items-start gap-3 rounded-xl border border-jolu-border p-4">
      <span className="text-base font-semibold text-jolu-text">{title}</span>
      <span className="text-xs text-jolu-muted">{note}</span>
      {detail && <span className="text-[11px] text-jolu-danger">{detail}</span>}
      {action}
    </div>
  )
}

function clock(ts: number | null): string {
  if (ts === null) return ""
  const d = new Date(ts)
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`
}

function MessageView({ message, platform, name }: { message: MessageDto; platform: PlatformId; name: string }) {
  if (message.role === "user") {
    return (
      <div className="flex flex-col gap-2 rounded-[9px] bg-jolu-side p-3">
        <div className="flex items-center justify-between">
          <span className="text-sm font-semibold text-jolu-text">你</span>
          <span className="tnum text-[10px] text-jolu-muted">{clock(message.ts)}</span>
        </div>
        <p className="whitespace-pre-wrap break-words text-xs leading-[1.6] text-jolu-text">{message.text}</p>
      </div>
    )
  }
  return (
    <div className="flex flex-col gap-2.5 py-1">
      <div className="flex items-center gap-[7px]">
        <PlatformLogo platform={platform} size={17} />
        <span className="text-xs font-semibold text-jolu-text">{name}</span>
        <span className="tnum text-[10px] text-jolu-muted">{clock(message.ts)}</span>
      </div>
      {message.text && <p className="whitespace-pre-wrap break-words text-xs leading-[1.6] text-jolu-text">{message.text}</p>}
      {message.tools.map((tool, index) => <ToolRow key={index} tool={tool} />)}
    </div>
  )
}

function ToolRow({ tool }: { tool: MessageDto["tools"][number] }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="flex flex-col gap-1.5">
      <button type="button" aria-expanded={open} onClick={() => setOpen(!open)} className="flex h-[34px] items-center gap-[7px] rounded-[7px] bg-jolu-side px-2.5 text-left outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent">
        {open ? <ChevronDown aria-hidden className="size-3.5 text-jolu-muted" /> : <ChevronRight aria-hidden className="size-3.5 text-jolu-muted" />}
        <FileCode aria-hidden className="size-3.5 text-jolu-muted" />
        <span className="min-w-0 flex-1 truncate text-[11px] text-jolu-text">{tool.summary ? `${tool.name} · ${tool.summary}` : tool.name}</span>
        <span className={cn("text-[10px]", tool.output ? "text-jolu-muted" : "text-jolu-muted/70")}>{tool.output ? "已完成" : "无输出记录"}</span>
      </button>
      {open && (
        <div className="rounded-[7px] bg-jolu-side p-2.5">
          {/* 缺少工具输出时只显示调用摘要，不补写结果（画板 46） */}
          <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] text-jolu-muted">{tool.output ?? "来源没有记录这次调用的输出"}</pre>
        </div>
      )}
    </div>
  )
}

function InfoPanel({ session, page, accountText, onCopy, onDrillUsage }: {
  session: SessionItemDto
  page: MessagePageDto | null
  accountText: string
  onCopy: (text: string, what: string) => void
  onDrillUsage: () => void
}) {
  const row = (label: string, value: React.ReactNode, copyText?: string, what?: string) => (
    <div className="flex min-h-[30px] items-center gap-3">
      <span className="w-[60px] shrink-0 text-[11px] text-jolu-muted">{label}</span>
      <span className="min-w-0 flex-1 truncate text-[11px] text-jolu-text">{value}</span>
      {copyText && (
        <button type="button" aria-label={`复制${what}`} onClick={() => onCopy(copyText, what!)} className="grid size-[30px] place-items-center rounded-lg bg-jolu-side text-jolu-muted outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <Copy aria-hidden className="size-4" />
        </button>
      )}
    </div>
  )
  const models = page?.models ?? []
  const split: [string, number | null][] = [["普通输入", session.fresh_input], ["输出", session.output], ["缓存读取", session.cache_read], ["缓存写入", session.cache_write]]
  return (
    <div className="flex flex-col gap-3.5">
      <div className="flex flex-col gap-[9px] rounded-[9px] border border-jolu-border p-3">
        {row("会话 ID", <span title={session.session_id}>{middleEllipsis(session.session_id)}</span>, session.session_id, "会话 ID")}
        {row("项目目录", session.cwd ?? "未识别", session.cwd ?? undefined, "项目目录")}
        {row("数据来源", session.has_source ? `本机 ${providerName(session.platform)} 记录` : `本机 ${providerName(session.platform)} 用量记录（无正文）`)}
        {row("模型", models.length ? (
          <span className="flex items-center gap-1.5"><PlatformLogo platform={session.platform as PlatformId} size={16} />{models.map(modelDisplayName).join("、")}</span>
        ) : "—")}
        {row("账号归属", accountText)}
      </div>
      <div className="flex gap-2.5">
        {split.map(([label, value]) => (
          <div key={label} className="flex min-w-0 flex-1 flex-col gap-1.5 rounded-lg bg-jolu-side p-2.5">
            <span className="text-[10px] text-jolu-muted">{label}</span>
            <span className={cn("tnum text-[17px] font-semibold", value === null ? "text-jolu-muted" : "text-jolu-text")}>{formatTokens(value)}</span>
          </div>
        ))}
      </div>
      <div className="flex items-center gap-2.5">
        <span className="text-[11px] text-jolu-muted">
          估算费用 {page && page.priced_requests ? formatUsd(page.cost) : "—"} · {session.requests} 次调用
          {page && page.priced_requests < page.requests ? `（${page.requests - page.priced_requests} 次未计价）` : ""}
        </span>
        <span className="flex-1" />
        <SmallButton accent onClick={onDrillUsage}>查看用量明细<ChevronRight aria-hidden className="size-3.5" /></SmallButton>
      </div>
      <p className="text-[10px] text-jolu-muted">缺少模型价格或计量字段时显示 —；账号归属不使用当前登录账号推断。</p>
    </div>
  )
}
