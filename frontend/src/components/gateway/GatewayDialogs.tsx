import { useRef, useState } from "react"
import * as RadixDialog from "@radix-ui/react-dialog"
import { TriangleAlert, X } from "lucide-react"
import { cn } from "@/lib/utils"
import type { CatalogModelDto, GatewayGroupDto, GroupKind, WiringStatusDto } from "@/lib/gatewayApi"
import { agentLabel } from "@/lib/gatewayPage"
import { useToast } from "@/components/ui/Toast"
import { ConfirmDialog as ActionConfirmDialog } from "@/components/ui/ConfirmDialog"
import { JoluButton } from "@/components/ui/JoluButton"
import type { Connection } from "@/types"

/**
 * 统一网关的弹窗与管理行（沿用模块 J 旧画板的弹窗与 E1–E10 状态）。
 * 来源进出模型池、改名与菜单 id、分组管理在「管理来源」弹窗（SourceManageDialog.tsx）。
 */

export function Modal({ title, icon, tone, children, footer, onClose, busy = false, width = 540 }: { title: string; icon?: React.ReactNode; tone?: "danger"; children: React.ReactNode; footer: React.ReactNode; onClose: () => void; busy?: boolean; width?: number }) {
  const trigger = useRef(document.activeElement)
  return <RadixDialog.Root open onOpenChange={value => { if (!value && !busy) onClose() }}>
    <RadixDialog.Portal>
      <RadixDialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/40" />
      <RadixDialog.Content aria-describedby={undefined} aria-busy={busy} style={{ width }}
        onPointerDownOutside={event => event.preventDefault()}
        onEscapeKeyDown={event => { if (busy) event.preventDefault() }}
        onCloseAutoFocus={event => { event.preventDefault(); if (trigger.current instanceof HTMLElement && trigger.current.isConnected) trigger.current.focus() }}
        className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex max-h-[calc(100dvh-70px)] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 overflow-y-auto rounded-2xl border border-jolu-border bg-jolu-cc-surface p-6 shadow-dialog outline-none">
        <div className="flex items-center gap-2">
          {icon}
          <RadixDialog.Title className={cn("min-w-0 flex-1 text-base font-semibold", tone === "danger" ? "text-jolu-danger" : "text-jolu-text")}>{title}</RadixDialog.Title>
          <RadixDialog.Close aria-label="关闭" disabled={busy} className="grid size-8 shrink-0 place-items-center rounded-lg text-jolu-muted outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-40"><X aria-hidden className="size-4" /></RadixDialog.Close>
        </div>
        {children}
        <div className="flex flex-wrap justify-end gap-2">{footer}</div>
      </RadixDialog.Content>
    </RadixDialog.Portal>
  </RadixDialog.Root>
}

export function Button({ label, primary, danger, disabled, onClick }: { label: string; primary?: boolean; danger?: boolean; disabled?: boolean; onClick: () => void }) {
  return <JoluButton variant={primary ? (danger ? "danger" : "primary") : "outline"} disabled={disabled} onClick={onClick}>{label}</JoluButton>
}

export function Warning({ children, tone = "danger" }: { children: React.ReactNode; tone?: "danger" | "muted" }) {
  return <div className={cn("flex items-start gap-2 rounded-lg px-3 py-2.5 text-[10px]", tone === "danger" ? "bg-jolu-danger-soft text-jolu-danger" : "bg-jolu-side text-jolu-muted")}><TriangleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" /><span>{children}</span></div>
}

export function MessageDialog({ title, tone, message, action, onClose }: { title: string; tone?: "danger"; message: string; action?: { label: string; onClick: () => void }; onClose: () => void }) {
  return (
    <Modal title={title} tone={tone} icon={<TriangleAlert className={cn("h-4 w-4", tone === "danger" ? "text-jolu-danger" : "text-jolu-muted")} />} onClose={onClose} footer={<><Button label="关闭" onClick={onClose} />{action && <Button label={action.label} primary onClick={action.onClick} />}</>}>
      <Warning>{message}</Warning>
    </Modal>
  )
}

export function RenameDialog({ connection, busy, onClose, onSave }: { connection: Connection; busy: boolean; onClose: () => void; onSave: (name: string, slug: string) => void }) {
  const [name, setName] = useState(connection.baseName)
  const [slug, setSlug] = useState(connection.slug ?? "")
  const reserved = ["route", "auto", "jolu", "group", "default"].includes(slug.trim())
  const invalid = !!slug && (!/^[a-z0-9-]+$/.test(slug) || slug.startsWith("-") || reserved)
  return (
    <Modal busy={busy} title="重命名来源" onClose={onClose} footer={<><Button label="取消" disabled={busy} onClick={onClose} /><Button label="保存" primary disabled={busy || invalid || !name.trim()} onClick={() => onSave(name.trim(), slug.trim())} /></>}>
      <div className="flex flex-col gap-1">
        <span className="text-[9.5px] text-jolu-muted">显示名称</span>
        <input value={name} onChange={(event) => setName(event.target.value)} className="jolu-control h-9 rounded-lg border border-jolu-border bg-jolu-side px-3 text-[11px] text-jolu-text outline-none focus:border-jolu-accent" />
        <span className="text-[8.5px] text-jolu-muted">只改显示名；已接入的工具不受影响。</span>
      </div>
      <div className="flex flex-col gap-1">
        <span className="text-[9.5px] text-jolu-muted">菜单 id</span>
        <input value={slug} onChange={(event) => setSlug(event.target.value)} className={cn("h-9 rounded-lg border bg-jolu-side px-3 text-[11px] text-jolu-text outline-none", invalid ? "border-jolu-danger" : "border-jolu-border focus:border-jolu-accent")} />
        {invalid ? <span className="text-[8.5px] text-jolu-danger">{reserved ? `「${slug}」是保留字，建议改用 ${slug}-src` : "只能使用小写字母、数字和连字符，不能以连字符开头。"}</span> : <span className="text-[8.5px] text-jolu-muted">Agent 配置里存的是这个 id；改名不会改它，改 id 后旧 id 会返回 404 并说明新名字。</span>}
      </div>
    </Modal>
  )
}

export function WiringDialog({ agents, busy, onClose, onWire, onUnwire, onAccept }: { agents: WiringStatusDto[]; busy: boolean; onClose: () => void; onWire: (agent: string) => void; onUnwire: (agent: string) => void; onAccept: (agent: string) => void }) {
  return (
    <Modal busy={busy} title="Agent 接线" onClose={onClose} footer={<Button label="关闭" disabled={busy} onClick={onClose} />}>
      <p className="text-[9.5px] text-jolu-muted">状态按配置文件里实际写入的值判断；重新应用不会重新拍快照。</p>
      <div className="flex flex-col gap-1.5">
        {agents.map((entry) => (
          <div key={entry.agent} className="flex items-center gap-2 rounded-lg border border-jolu-border bg-jolu-bg px-2.5 py-2">
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className="text-[10.5px] text-jolu-text">{agentLabel(entry.agent)}</span>
              <span className={cn("truncate text-[8.5px]", entry.wired ? "text-jolu-on" : entry.drift ? "text-jolu-danger" : "text-jolu-muted")}>
                {entry.wired ? "已接入" : entry.drift === "foreign" ? "已被另一个 Jolu 接管" : entry.drift === "removed" ? "配置里已移除" : entry.drift === "changed" ? "配置被改写" : entry.drift === "pending" ? "上次接线未完成" : entry.reason ?? "未接入"}
              </span>
            </div>
            {entry.wired ? <Button label="断开" disabled={busy} onClick={() => onUnwire(entry.agent)} /> : entry.drift ? (<><Button label="重新应用" primary disabled={busy} onClick={() => onWire(entry.agent)} /><Button label="接受现状" disabled={busy} onClick={() => onAccept(entry.agent)} /></>) : (<Button label="接线" primary disabled={busy || (!entry.config_path && !!entry.reason)} onClick={() => onWire(entry.agent)} />)}
          </div>
        ))}
      </div>
      <span className="text-[8.5px] text-jolu-muted">同一工具的第二份安装（WSL、便携版）暂不支持，只在主安装上接线。</span>
    </Modal>
  )
}

export function CodexDialog({ mode, returnFocusTo, onClose, onConfirm }: { mode: "oauth" | "provider"; busy: boolean; returnFocusTo?: HTMLElement | null; onClose: () => void; onConfirm: () => Promise<boolean> }) {
  const oauth = mode === "oauth"
  return (
    <ActionConfirmDialog returnFocusTo={returnFocusTo} title="接入 Codex" description="确认下方接线方式后更新 Codex 本机配置。" confirmLabel="接入" onClose={onClose} onConfirm={onConfirm}>
      <p className="text-[10px] text-jolu-muted">Jolu 会把 Codex 的请求转到本机模型入口；只写入 config.toml。</p>
      <div className={cn("flex flex-col gap-1.5 rounded-lg border p-3", oauth ? "border-jolu-on bg-jolu-status-success-bg" : "border-jolu-border bg-jolu-side")}>
        <span className="text-[11px] font-semibold text-jolu-text">{oauth ? "保留 ChatGPT 登录" : "使用网关 Key"}</span>
        <span className="text-[9px] text-jolu-muted">{oauth ? "检测到 ChatGPT 登录：只改请求地址（openai_base_url），不写 Key、不动 auth.json。" : "没有检测到 ChatGPT 登录：写自定义 provider 和 jolu-codex 令牌，不改其他 provider。"}</span>
      </div>
      {!oauth && <Warning tone="muted">如果以后登录 ChatGPT，重新接线即可切回保留登录的接法。</Warning>}
      <Warning tone="muted">Codex 发 Responses 请求；只支持 Chat Completions 的上游会在请求时返回 404。</Warning>
    </ActionConfirmDialog>
  )
}

export function GeminiDialog({ returnFocusTo, onClose, onConfirm }: { busy: boolean; returnFocusTo?: HTMLElement | null; onClose: () => void; onConfirm: () => Promise<boolean> }) {
  return (
    <ActionConfirmDialog returnFocusTo={returnFocusTo} title="接入 Gemini CLI" description="接线期间 Gemini 将使用 Jolu 网关 Key，断开时恢复原登录方式。" confirmLabel="确认接入" onClose={onClose} onConfirm={onConfirm}>
      <Warning>接线期间 Gemini 会改用 Jolu 网关 Key，不再使用 Google 账号登录。OAuth 凭据文件不会被删除。</Warning>
      <div className="flex flex-col rounded-lg bg-jolu-side px-3">
        <Line label="当前登录方式" value="Google 账号 · oauth-personal" />
        <Line label="接线期间" value="网关 Key · gemini-api-key" danger />
        <Line label="断开时" value="恢复原登录方式与原 Key" />
      </div>
      <span className="text-[8.5px] text-jolu-muted">原值会存进接线记录；如果断开时还原失败，会在「Agent 接线」里提示并提供重试。</span>
    </ActionConfirmDialog>
  )
}

export function CatalogDialog({ models, onClose }: { models: CatalogModelDto[]; onClose: () => void }) {
  const toast = useToast()
  return (
    <Modal title="模型池里的模型" onClose={onClose} footer={<Button label="关闭" onClick={onClose} />}>
      <p className="text-[9.5px] text-jolu-muted">复制「来源/模型」填入调用工具。订阅模型菜单不代表已完成真实调用验收。</p>
      <div className="flex max-h-[50vh] flex-col gap-1 overflow-y-auto">
        {models.map((model) => (
          <div key={model.id} className="flex items-center gap-2 rounded-lg bg-jolu-side px-2.5 py-1.5">
            <span className="truncate text-[10px] text-jolu-text">{model.id}</span>
            <button type="button" className="shrink-0 text-[9px] text-jolu-on" onClick={() => { void navigator.clipboard.writeText(model.id).then(() => toast.success("模型名已复制"), () => toast.danger("复制失败，请重试")) }}>复制模型名</button>
            <div className="flex-1" />
            <span className={cn("text-[8.5px]", model.status === "connected" ? "text-jolu-muted" : "text-jolu-danger")}>{model.status === "connected" ? model.source : "未连接"}</span>
          </div>
        ))}
      </div>
    </Modal>
  )
}

export function Line({ label, value, danger }: { label: string; value: string; danger?: boolean }) {
  return (
    <div className="flex h-8 items-center gap-2 border-b border-jolu-table-divider text-[9.5px] last:border-0">
      <span className="text-jolu-muted">{label}</span>
      <div className="flex-1" />
      <span className={cn("font-semibold", danger ? "text-jolu-danger" : "text-jolu-text")}>{value}</span>
    </div>
  )
}

/** 分组按置顶优先、再按保存的顺序排列（只影响排列，不改路由、接线和 Agent 菜单）。 */
export function orderGroups(groups: GatewayGroupDto[], pinned: string[], kind: GroupKind): GatewayGroupDto[] {
  return [...groups].sort((a, b) => {
    const aPin = pinned.includes(`${kind}:${a.id}`) ? 0 : 1
    const bPin = pinned.includes(`${kind}:${b.id}`) ? 0 : 1
    return aPin - bPin || a.sort - b.sort
  })
}
