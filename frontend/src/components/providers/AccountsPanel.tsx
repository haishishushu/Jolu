import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react"
import * as Dialog from "@radix-ui/react-dialog"
import * as Menu from "@radix-ui/react-dropdown-menu"
import { AlertTriangle, Blocks, Check, ChevronDown, CircleAlert, EyeClosed, Globe, KeyRound, Mail, LoaderCircle, Pencil, Plus, RefreshCw, ScanEye, Search, Ticket, Trash2, X } from "lucide-react"
import { api, isTauri, type QuotaWindowDto } from "@/lib/api"
import { cn } from "@/lib/utils"
import { PROVIDERS, connectionProviderId, connectionProviderEntry, editSaveState, fallsBackToManualModel, modelFetchErrorText, protocolLabel, providerEntry, requestGate, canStartVerify, modelBusyAfter, validateConnectionName, type ProviderId, type ModelBusy } from "@/lib/providerCatalog"
import { accountQueries, quotaValue, subscriptionDisplay, resetCreditsText, filterAccounts, formatMoney, formatResetAt, formatSyncedAt, quotaTitle, redactAccountMessage } from "@/lib/accountDisplay"
import { accountCardPolicy } from "@/lib/accountCardPolicy"
import { remainingPercent } from "@/lib/quota"
import { removeAccounts, removeAccountWithReplacement } from "@/lib/accountActions"
import { useConnections } from "@/hooks/useConnections"
import { useAccountSelection } from "@/hooks/useAccountSelection"
import { useSettings } from "@/hooks/useSettings"
import { useBalance, useQuota, type BalanceResult, type QuotaResult } from "@/hooks/useQuota"
import { SelectField } from "@/components/ui/SelectField"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { JoluButton } from "@/components/ui/JoluButton"
import { ModelPicker } from "@/components/providers/ConnectDialog"
import { AccountBatchToolbar, BatchAccountDeleteDialog, type BatchAccountTarget } from "@/components/providers/AccountBatchControls"
import { Tip } from "@/components/ui/Tip"
import { useToast } from "@/components/ui/Toast"
import type { Connection, ConnectionKind, PlatformId } from "@/types"

const PrivacyContext = createContext(false)
const PRIVACY_KEY = "jolu.accounts.hide-sensitive"
/** 马赛克使用生成的纹理，原文不进入 DOM、title 或无障碍标签。 */
function PrivacyMosaic({ label, length }: { label: string; length: number }) {
  return <span role="img" aria-label={`${label}已隐藏`} className="privacy-mosaic" style={{ width: `${Math.min(30, Math.max(8, length))}ch` }} />
}

/** 显隐保留字段图标和行高，隐藏态没有可复制或悬浮显示的原文。 */
function SensitiveValue({ value, label, icon: Icon = Mail,compact=false }: { value: string | null | undefined; label: string; icon?: typeof Mail;compact?:boolean }) {
  const hidden = useContext(PrivacyContext)
  return <span className={cn("flex min-w-0 shrink-0 items-center gap-2 rounded-md bg-jolu-side px-2 text-[11px] text-jolu-muted",compact?"h-5":"h-7")}>
    <Icon aria-hidden className="size-3.5 shrink-0" />
    {hidden && value ? <PrivacyMosaic label={label} length={value.length} />
      : <span className="min-w-0 truncate" title={value || undefined}>{value || `${label}未获取`}</span>}
  </span>
}

function displayName(account: Connection, hidden: boolean) {
  return hidden && (account.baseName.includes("@") || (!!account.accountEmail && account.baseName.includes(account.accountEmail))) ? "账号" : account.baseName
}

/** 「设为当前」只对能写入本机 CLI 配置的平台开放 */
const CLI_PLATFORMS: readonly PlatformId[] = ["claude", "codex"]
/** 筛选栏直接露出的供应商，其余收进「更多供应商」（画板 07 / 09） */
const QUICK_FILTERS: readonly ProviderId[] = ["claude", "codex"]

type Conn = ReturnType<typeof useConnections>

/**
 * 账号管理（Jolu.pen 画板 07–10、23、25）：订阅账号看额度，API Key 账号看余额与消费。
 * 「当前使用」以本机 CLI 实际在用的凭证为准，按供应商分别设置。
 */
export function AccountsPanel({ conn, onConnect, target, onTargetHandled }: {
  conn: Conn
  onConnect: (platform: ProviderId) => void
  target?: { connectionId: string; nonce: number } | null
  onTargetHandled?: () => void
}) {
  const [hidden, setHidden] = useState(() => {
    try { return localStorage.getItem(PRIVACY_KEY) === "true" } catch { return false }
  })
  const togglePrivacy = () => {
    setHidden(value => {
      const next = !value
      try { localStorage.setItem(PRIVACY_KEY, String(next)) } catch { /* 存储不可用时仍支持本次切换 */ }
      return next
    })
    if (!hidden) setQuery("")
  }
  const toast = useToast()
  const cfg = useSettings()
  const [platform, setPlatform] = useState<ProviderId | null>(null)
  const [kind, setKind] = useState<ConnectionKind | null>(null)
  const [query, setQuery] = useState("")
  const [current, setCurrent] = useState<string[]>([])
  const [removing, setRemoving] = useState<Connection | null>(null)
  const [promoting, setPromoting] = useState<Connection | null>(null)
  const [editing, setEditing] = useState<Connection | null>(null)
  const [batchTargets, setBatchTargets] = useState<BatchAccountTarget[] | null>(null)
  const [batchBusy, setBatchBusy] = useState(false)
  const batchRunning = useRef(false)
  const gridRef=useRef<HTMLDivElement>(null)
  const handledTarget = useRef<number | null>(null)
  const [locatedId, setLocatedId] = useState<string | null>(null)
  useEffect(()=>{gridRef.current?.scrollTo({top:0})},[platform,query,kind])

  useEffect(() => {
    if (!isTauri) return
    let stopped = false
    void api.currentConnections().then((ids) => { if (!stopped) setCurrent(ids) }).catch(() => { if (!stopped) setCurrent([]) })
    return () => { stopped = true }
  }, [conn.connections])

  const accounts = conn.connections
  const visible = useMemo(() => filterAccounts(accounts, platform, query, kind), [accounts, platform, query,kind])
  const selection = useAccountSelection(visible)
  const batchToolbar = <AccountBatchToolbar selection={selection} total={visible.length} disabled={batchBusy || conn.loading || !!conn.error || !!removing || !!promoting || !!editing} onDelete={() => {
    if (batchRunning.current) return
    setBatchTargets(visible.filter(account => selection.selected.has(account.id)).map(account => ({
      id: account.id, label: account.baseName, provider: connectionProviderEntry(account).name, current: current.includes(account.id),
    })))
  }} />
  const currentCount = accounts.filter((a) => current.includes(a.id)).length
  useEffect(() => {
    if (target) { setPlatform(null); setKind(null); setQuery("") }
  }, [target])
  useEffect(() => {
    if (!target || handledTarget.current === target.nonce || conn.loading || conn.error || platform || kind || query) return
    if (!accounts.some(account => account.id === target.connectionId)) {
      handledTarget.current = target.nonce
      toast.warn("该账号已不存在", "账号可能已被删除，请在账号管理中检查")
      onTargetHandled?.()
      return
    }
    const element = [...(gridRef.current?.querySelectorAll<HTMLElement>("[data-account-id]") ?? [])].find(card => card.dataset.accountId === target.connectionId)
    if (!element) return
    element.scrollIntoView({ block: "nearest", behavior: cfg.settings.reduce_motion ? "auto" : "smooth" })
    setLocatedId(target.connectionId)
    handledTarget.current = target.nonce
    onTargetHandled?.()
  }, [target, conn.loading, conn.error, accounts, platform, kind, query, toast, onTargetHandled, cfg.settings.reduce_motion])
  useEffect(() => {
    if (!locatedId) return
    const timer = window.setTimeout(() => setLocatedId(null), 4000)
    return () => window.clearTimeout(timer)
  }, [locatedId])

  if (!conn.loading && !conn.error && !accounts.length && !batchTargets) {
    return (
      <>
        <Header total={0} current={0} onAdd={() => onConnect("claude")} hidden={hidden} onTogglePrivacy={togglePrivacy} />
        <EmptyState title="还没有账号" desc="添加账号后，账号的额度、余额与消费会显示在这里。" action={<JoluButton variant="primary" onClick={() => onConnect("claude")}><Plus aria-hidden className="!text-[#071B10]" />添加账号</JoluButton>} />
      </>
    )
  }

  return (
    <PrivacyContext.Provider value={hidden}>
      <div className="flex min-h-0 flex-1 flex-col gap-3" data-account-panel>
      <Header total={accounts.length} current={currentCount} onAdd={() => onConnect(platform ?? "claude")} hidden={hidden} onTogglePrivacy={togglePrivacy} addDisabled={selection.active || batchBusy} disabled={batchBusy} batchControl={!selection.active ? batchToolbar : undefined} />
      {selection.active && batchToolbar}
      <fieldset disabled={batchBusy} className="flex shrink-0 flex-wrap items-center gap-2">
        <label className="flex h-[34px] w-[220px] items-center gap-2 rounded-lg border border-jolu-border px-3 focus-within:border-jolu-accent">
          <Search aria-hidden className="size-4 shrink-0 text-jolu-muted" />
          <input type={hidden ? "password" : "text"} autoComplete="off" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索用户名、邮箱、连接名称" aria-label="搜索账号" className="min-w-0 flex-1 bg-transparent text-xs text-jolu-text outline-none placeholder:text-jolu-muted" />
        </label>
        <div role="group" aria-label="账号类型" className="flex items-center gap-2">
          <FilterChip active={kind === null} onClick={() => setKind(null)}>全部类型</FilterChip>
          <FilterChip active={kind === "auth"} onClick={() => setKind("auth")}>Auth官方订阅</FilterChip>
          <FilterChip active={kind === "api"} onClick={() => setKind("api")}>第三方ApiKey</FilterChip>
        </div>
        <FilterChip active={platform === null} onClick={() => setPlatform(null)}>全部供应商</FilterChip>
        {QUICK_FILTERS.map((id) => (
          <FilterChip key={id} active={platform === id} onClick={() => setPlatform(id)}><span aria-hidden className="grid size-4 shrink-0 place-items-center"><PlatformLogo platform={providerEntry(id).platformId} size={16} /></span>{providerEntry(id).name}</FilterChip>
        ))}
        <MoreFilter accounts={accounts.filter(account=>!kind || account.kind===kind)} value={platform} onChange={setPlatform} />
      </fieldset>

      {conn.error && (
        <div role="alert" className="flex items-center justify-between gap-3 rounded-[10px] bg-jolu-danger-soft px-4 py-2.5 text-xs text-jolu-danger">
          <span>账号列表读取失败：{hidden ? redactAccountMessage(conn.error) : conn.error}</span>
          <JoluButton size="sm" variant="danger-soft" disabled={batchBusy} onClick={() => void conn.reload()}>重新读取</JoluButton>
        </div>
      )}
      {conn.loading && !accounts.length && <p role="status" className="text-xs text-jolu-muted">正在读取账号…</p>}

      {visible.length ? (
        <div className="relative min-h-0 flex-1">
        <div ref={gridRef} role="list" aria-label="账号列表" data-account-grid className="absolute inset-0 grid grid-cols-4 content-start gap-3 overflow-y-auto pr-1" style={{gridAutoRows:"calc((100% - 12px) / 2)",scrollbarGutter:"stable"}}>
          {visible.map((account, index) => (
            <AccountCard
              key={`${account.id}:${account.status}`}
              account={account}
              current={current.includes(account.id)}
              located={locatedId === account.id}
              selecting={selection.active}
              selected={selection.selected.has(account.id)}
              disabled={batchBusy}
              selectionLabel={hidden ? `选择账号 ${index + 1}` : `选择账号 ${account.baseName}`}
              onToggle={() => { if (!batchRunning.current) selection.toggle(account.id) }}
              onRemove={() => setRemoving(account)}
              onPromote={() => setPromoting(account)}
              onEdit={() => setEditing(account)}
              onResume={() => {
                void conn.setPaused(account.id, false)
                  .then(() => toast.success(`已恢复 ${account.baseName} 的查询`))
                  .catch((reason) => toast.danger("恢复查询失败", String(reason)))
              }}
            />
          ))}
        </div>
        </div>
      ) : accounts.length > 0 && (
        <EmptyState
          title="没有找到匹配的账号"
          desc="切换账号类型、供应商，或清除搜索条件。"
          action={<>
            <JoluButton variant="neutral" disabled={batchBusy} onClick={() => { setQuery(""); setPlatform(null); setKind(null) }}>清除筛选</JoluButton>
            <JoluButton variant="soft" disabled={selection.active || batchBusy} onClick={() => onConnect(platform ?? "claude")}><Plus aria-hidden />添加账号</JoluButton>
          </>}
        />
      )}
      <p className="flex shrink-0 items-center justify-between gap-3 text-[10px] text-jolu-muted"><span>仅显示供应商实际提供的信息；完整额度与服务信息见详情。</span><span className="shrink-0">共 {visible.length} 个{visible.length > 8 ? " · 滚轮查看更多" : ""}</span></p>

      {batchTargets && <BatchAccountDeleteDialog targets={batchTargets} hidden={hidden} onClose={() => { if (!batchRunning.current) setBatchTargets(null) }} onConfirm={async ids => {
        if (batchRunning.current) throw new Error("正在删除账号，请稍候")
        const allowed = new Set(batchTargets.map(account => account.id))
        if (ids.some(id => !allowed.has(id))) throw new Error("删除目标已变化，请关闭确认后重新选择")
        batchRunning.current = true
        setBatchBusy(true)
        try {
          const result = await removeAccounts(ids, api.removeConnection)
          selection.discard(result.deletedIds)
          await conn.reload()
          return result
        } finally {
          batchRunning.current = false
          setBatchBusy(false)
        }
      }} />}

      {removing && (
        <RemoveDialog
          account={removing}
          current={current.includes(removing.id)}
          alternatives={accounts.filter((a) => connectionProviderId(removing) !== null && connectionProviderId(a) === connectionProviderId(removing) && a.id !== removing.id)}
          islandUsers={islandUsers(removing.id, cfg.settings)}
          onClose={() => setRemoving(null)}
          onConfirm={async (replacement) => {
            await removeAccountWithReplacement(removing.id, replacement, { enable: api.enableConnection, remove: conn.remove })
            toast.success(`已删除 ${removing.baseName}`, replacement ? "已切换到替代账号；历史统计保留" : "配置与凭据已移除，历史统计保留")
            setRemoving(null)
            await conn.reload()
          }}
        />
      )}
      {promoting && (
        <PromoteDialog
          account={promoting}
          onClose={() => setPromoting(null)}
          onConfirm={async () => {
            const note = await api.enableConnection(promoting.id)
            toast.success(`已将「${promoting.baseName}」设为当前账号`, note)
            setPromoting(null)
            await conn.reload()
          }}
        />
      )}
      {editing && (
        <EditDialog
          account={editing}
          others={accounts.filter((a) => a.id !== editing.id)}
          onClose={() => setEditing(null)}
          onSave={async (name, secret, model) => {
            if (secret && model) {
              // 自定义供应商更换密钥：密钥与重新获取 / 验证过的模型一起保存（画板 27）
              await api.updateConnection({ id: editing.id, name, base_url: editing.baseUrl, secret, model, effort: editing.effort, context_1m: editing.context1m })
              await conn.reload()
            } else {
              if (name !== editing.baseName) await conn.rename(editing.id, name)
              if (secret) await conn.replaceApiKey(editing.id, secret)
            }
            toast.success(`已保存 ${name}`, secret ? (model ? "密钥与默认模型已更新" : "密钥已更换并通过验证") : undefined)
            setEditing(null)
          }}
        />
      )}
      </div>
    </PrivacyContext.Provider>
  )
}

function islandUsers(id: string, settings: ReturnType<typeof useSettings>["settings"]): string[] {
  const users: string[] = []
  if (settings.island_connection_id === id) users.push("灵动岛本体")
  settings.island_clones.forEach((clone, index) => { if (clone.connection_id === id) users.push(`灵动岛分身 ${index + 1}`) })
  return users
}

function Header({ total, current, onAdd, hidden, onTogglePrivacy, batchControl, addDisabled = false, disabled = false }: { total: number; current: number; onAdd: () => void; hidden: boolean; onTogglePrivacy: () => void; batchControl?: ReactNode; addDisabled?: boolean; disabled?: boolean }) {
  return (
    <div className="flex min-h-9 shrink-0 flex-wrap items-center justify-between gap-3">
      <div className="flex items-baseline gap-3">
        <h2 className="text-[22px] font-semibold text-jolu-text">账号管理</h2>
        <span className="text-xs text-jolu-muted">{total} 个账号 · {current} 个当前使用</span>
      </div>
      <div className="flex items-center gap-2">
        {batchControl}
        <JoluButton variant="outline" disabled={disabled} aria-pressed={hidden} onClick={onTogglePrivacy}>
          {hidden ? <ScanEye aria-hidden strokeWidth={1.6} /> : <EyeClosed aria-hidden strokeWidth={1.6} />}{hidden ? "显示敏感信息" : "隐藏敏感信息"}
        </JoluButton>
        <JoluButton variant="primary" disabled={disabled || addDisabled} onClick={onAdd}><Plus aria-hidden className="!text-[#071B10]" />添加账号</JoluButton>
      </div>
    </div>
  )
}

function FilterChip({ active, onClick, children }: { active: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "flex h-[34px] items-center gap-2 rounded-lg border px-3 text-[13px] font-medium outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
        active ? "border-jolu-selected bg-jolu-selected text-jolu-on" : "border-jolu-border bg-jolu-bg text-jolu-text hover:bg-jolu-stage",
      )}
    >
      {children}
    </button>
  )
}

/** 「更多供应商」：全部选项三列网格，带账号数量；0 个账号的供应商仍可选（画板 25） */
function MoreFilter({ accounts, value, onChange }: { accounts: readonly Connection[]; value: ProviderId | null; onChange: (platform: ProviderId | null) => void }) {
  const count = (id: ProviderId) => accounts.filter((a) => connectionProviderId(a) === id).length
  const inMore = value !== null && !QUICK_FILTERS.includes(value)
  const item = (key: string, label: string, selected: boolean, onSelect: () => void, platform?: PlatformId) => (
    <Menu.Item
      key={key}
      onSelect={onSelect}
      className={cn(
        "flex h-8 min-w-0 cursor-default select-none items-center gap-2 rounded-md border px-2 text-xs outline-none",
        selected ? "border-jolu-accent bg-jolu-selected font-medium text-jolu-on" : "border-jolu-border text-jolu-text data-[highlighted]:bg-jolu-side",
      )}
    >
      <span aria-hidden className="grid size-4 shrink-0 place-items-center">{platform ? <PlatformLogo platform={platform} size={16} /> : <Blocks className="size-4" />}</span>
      <span className="min-w-0 truncate" title={label}>{label}</span>
    </Menu.Item>
  )
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger
        className={cn(
          "flex h-[34px] items-center gap-2 rounded-lg border px-3 text-[13px] font-medium outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent data-[state=open]:bg-jolu-stage",
          inMore ? "border-jolu-selected bg-jolu-selected text-jolu-on" : "border-jolu-border bg-jolu-bg text-jolu-text hover:bg-jolu-stage",
        )}
      >
        <ChevronDown aria-hidden className="size-4 text-jolu-muted" />
        {inMore && <span aria-hidden className="grid size-4 shrink-0 place-items-center"><PlatformLogo platform={providerEntry(value).platformId} size={16} /></span>}
        {inMore ? providerEntry(value).name : "更多供应商"}
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content align="start" sideOffset={6} collisionPadding={12} style={{ maxHeight: "var(--radix-dropdown-menu-content-available-height)" }} className="motion-popover z-[100] grid w-[528px] max-w-[calc(100vw-24px)] overflow-y-auto grid-cols-3 gap-2 rounded-xl border border-jolu-border bg-jolu-bg p-3 shadow-popover">
          {item("all", `全部供应商 · ${accounts.length}`, value === null, () => onChange(null))}
          {PROVIDERS.map((entry) => item(entry.id, `${entry.name} · ${count(entry.id)}`, value === entry.id, () => onChange(entry.id), entry.platformId))}
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}

function EmptyState({ title, desc, action }: { title: string; desc: string; action: ReactNode }) {
  return (
    <div className="flex flex-col items-start gap-3 rounded-xl border border-jolu-border p-6">
      <span className="text-base font-semibold text-jolu-text">{title}</span>
      <span className="text-xs text-jolu-muted">{desc}</span>
      <div className="flex gap-2.5">{action}</div>
    </div>
  )
}

function AccountCard({ account, current, located, selecting = false, selected = false, disabled = false, selectionLabel, onToggle, onRemove, onPromote, onEdit, onResume }: {
  account: Connection
  current: boolean
  located?: boolean
  selecting?: boolean
  selected?: boolean
  disabled?: boolean
  selectionLabel?: string
  onToggle?: () => void
  onRemove: () => void
  onPromote: () => void
  onEdit: () => void
  onResume: () => void
}) {
  const hidden = useContext(PrivacyContext)
  const [detailsOpen,setDetailsOpen]=useState(false)
  const entry = connectionProviderEntry(account)
  const paused = account.status === "paused"
  const canPromote = !current && CLI_PLATFORMS.includes(account.platformId) && !account.id.startsWith("browser:")
  const queries = accountQueries(account)
  const quota = useQuota(queries.quota)
  const balance = useBalance(queries.balance)
  const policy = accountCardPolicy(account,quota.state,balance.state)
  const isApi = account.kind === "api"
  const email=account.accountEmail ?? (quota.state?.state === "ok" ? quota.state.account?.email : null)
  const refresh = !queries.quota && !queries.balance ? null : {
    ...(policy.showQuota ? quota : balance),
    loading: quota.loading || balance.loading,
    stale: quota.stale || balance.stale,
    refresh: async () => { await Promise.all([queries.quota && quota.refresh(), queries.balance && balance.refresh()]) },
  }
  const emptyReason=isApi && balance.state && balance.state.state!=="ok" ? balance.state.reason : quota.state && quota.state.state!=="ok" ? quota.state.reason : "供应商未返回可展示的额度或余额"
  const name = displayName(account, hidden)
  return (
    <article role="listitem" data-account-card data-account-id={account.id} data-account-located={located ? "true" : undefined} className={cn("flex h-full min-h-0 min-w-0 flex-col gap-1.5 rounded-xl border bg-jolu-bg p-3", current ? "border-jolu-accent" : "border-jolu-border", located && "ring-2 ring-jolu-accent ring-inset")}>
      <div className="flex h-8 shrink-0 items-center gap-2">
        {selecting && <input type="checkbox" aria-label={selectionLabel ?? "选择账号"} checked={selected} disabled={disabled} onChange={onToggle} className="size-4 shrink-0 cursor-pointer accent-jolu-accent disabled:cursor-default" />}
        <span className="grid size-6 shrink-0 place-items-center"><PlatformLogo platform={account.platformId} size={22} /></span>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="truncate text-[13px] font-semibold text-jolu-text" title={hidden ? undefined : account.baseName}>{name !== account.baseName ? <PrivacyMosaic label="账号名称" length={account.baseName.length} /> : name}</span>
          <span className="truncate text-[10px] text-jolu-muted">{entry.name} · {isApi ? `ApiKey${account.protocol ? ` · ${protocolLabel(account.protocol)}` : ""}` : "Auth 官方订阅"}</span>
        </div>
        {current && <Badge tone="on">当前使用</Badge>}
      </div>
      {account.kind === "auth" && email && <SensitiveValue label="邮箱 / 账号" value={email} compact />}
      {isApi && account.baseUrl && <SensitiveValue label="服务地址" icon={Globe} value={account.baseUrl} compact />}
      <div className="flex min-h-0 flex-1 flex-col gap-1.5 overflow-hidden" data-account-summary>
        {(!isApi || policy.expiresAt) && <SubscriptionExpiry account={account} quota={quota} compact />}
        {paused ? <div className="flex flex-col items-start gap-2 rounded-lg bg-jolu-side p-2.5 text-[11px] text-jolu-muted"><span>已暂停查询，配置与历史保留。</span><JoluButton size="sm" variant="soft" disabled={!isTauri || selecting || disabled} onClick={onResume}>恢复查询</JoluButton></div> : <>
          {policy.showBalance && <ApiMetrics account={account} balance={balance} compact />}
          {policy.showQuota && (!isApi || !policy.showBalance) && <SubscriptionMetrics account={account} quota={quota} compact limit={isApi ? 1 : 2} />}
          {!isApi && policy.resetCredits && <ResetCreditsMetrics quota={quota} compact disabled={disabled} />}
          {!policy.showQuota && !policy.showBalance && !policy.resetCredits && <p className="line-clamp-3 text-[11px] leading-relaxed text-jolu-muted">{hidden ? redactAccountMessage(emptyReason) : emptyReason}</p>}
        </>}
        {isApi && <div className="mt-auto flex min-w-0 shrink-0 flex-col gap-1.5">
          {account.model && <Stat label="模型" value={account.model} />}
          {account.masked && <SensitiveValue label="Key 标识" icon={KeyRound} value={account.masked} compact />}
        </div>}
      </div>
      <CardActions account={account} paused={paused} refresh={refresh} hasQuota={policy.showQuota} canPromote={canPromote} readOnly={selecting || disabled} onPromote={onPromote} onEdit={onEdit} onRemove={onRemove} onDetails={()=>setDetailsOpen(true)} />
      {detailsOpen && <ConfirmShell title="账号详情" subtitle={`${displayName(account,hidden)} · ${entry.name}`} onClose={()=>setDetailsOpen(false)} busy={false} footer={<JoluButton variant="neutral" onClick={()=>setDetailsOpen(false)}>关闭</JoluButton>}>
        <div className="flex max-h-[calc(100dvh-250px)] min-h-0 flex-col gap-4 overflow-y-auto pr-1" data-account-details>
          {email && <SensitiveValue label="邮箱 / 账号" value={email} />}
          {(!isApi || policy.expiresAt) && <SubscriptionExpiry account={account} quota={quota} />}
          {paused ? <p className="text-xs text-jolu-muted">已暂停查询，额度与余额不更新。</p> : <>
            {(policy.showQuota || account.kind === "auth") && <SubscriptionMetrics account={account} quota={quota} />}
            {queries.balance && <ApiMetrics account={account} balance={balance} />}
            {policy.resetCredits && <ResetCreditsMetrics quota={quota} disabled={disabled} />}
          </>}
          {account.baseUrl && <SensitiveValue label="Base URL" icon={Globe} value={account.baseUrl} />}
          {account.protocol && <Stat label="服务协议" value={protocolLabel(account.protocol)} />}
          {account.model && <Stat label="默认模型" value={account.model} />}
          {account.kind === "api" && account.masked && <SensitiveValue label="Key 标识" icon={KeyRound} value={account.masked} />}
        </div>
      </ConfirmShell>}
    </article>
  )
}

function Badge({ tone, children }: { tone: "on" | "muted" | "danger"; children: ReactNode }) {
  return (
    <span className={cn(
      "shrink-0 rounded-[5px] px-2 py-[5px] text-[11px] leading-none",
      tone === "on" ? "bg-jolu-selected text-jolu-on" : tone === "danger" ? "bg-jolu-danger-soft text-jolu-danger" : "bg-jolu-stage text-jolu-muted",
    )}>{children}</span>
  )
}

function Stat({ label, value, tone }: { label: string; value: string; tone?: "danger" | "muted" }) {
  return (
    <div className="flex min-w-0 items-center justify-between gap-2">
      <span className="text-[10px] text-jolu-muted">{label}</span>
      <span title={value} className={cn("truncate text-xs font-medium", tone === "danger" ? "text-jolu-danger" : tone === "muted" ? "text-jolu-muted" : "text-jolu-text")}>{value}</span>
    </div>
  )
}

/** Auth 固定显示订阅到期；ApiKey 仅有真实套餐日期时显示套餐到期。 */
function SubscriptionExpiry({ account, quota, compact=false }: { account: Connection; quota: QuotaResult; compact?:boolean }) {
  const policy=accountCardPolicy(account,quota.state)
  const subscription=subscriptionDisplay(quota.state?.state==="ok" ? quota.state.account : null)
  const state=(quota.stale ? quota.failure?.state : null) ?? quota.state?.state
  const value=account.status==="paused" ? "已暂停查询"
    : policy.expiresAt ? `${subscription.expires}${policy.expiryIsSnapshot ? " · 本机" : ""}${quota.stale ? " · 上次数据" : ""}`
    : state==="unauthorized" ? "登录失效"
    : state==="failed" ? "查询失败"
    : state==="forbidden" ? "无查询权限"
    : state==="rate_limited" ? "请求受限"
    : state==="ok" || state==="unsupported" ? "未提供"
    : quota.loading ? "查询中…" : "尚未查询"
  return <>
    <Stat label={account.kind === "api" ? "套餐到期" : "订阅到期"} value={value} tone={state==="unauthorized" || state==="failed" || state==="forbidden" || state==="rate_limited" || quota.stale ? "danger" : "muted"} />
    {!compact && subscription.note && <p className="text-[11px] leading-relaxed text-jolu-muted">{subscription.note}</p>}
  </>
}

/** 按真实响应显示套餐与额度；主卡限主要窗口，详情完整保留。 */
function SubscriptionMetrics({ account, quota, compact=false, limit=2 }: { account: Connection; quota: QuotaResult; compact?:boolean; limit?:number }) {
  const hidden = useContext(PrivacyContext)
  const state = quota.state
  const unauthorized = state?.state === "unauthorized"
  const policy=accountCardPolicy(account,state)
  const windows=compact ? policy.windows.slice(0,limit) : policy.windows
  return <>
    {policy.plan && (compact ? <span className="truncate text-[10px] font-medium leading-4 text-jolu-text" title={policy.plan}>{policy.plan}</span> : <Stat label="等级 / 套餐" value={policy.plan} />)}
    {windows.length > 0 && <div className="grid grid-cols-1 gap-2">{windows.map(window=><QuotaBar key={window.key} window={window} compact={compact} showReset={!compact} />)}</div>}
    {compact && policy.windows.length > windows.length && <span className="text-[10px] text-jolu-muted">共 {policy.windows.length} 项额度，完整内容见详情</span>}
    {state?.state !== "ok" && <div className={cn("rounded-lg bg-jolu-side px-2 py-1.5 text-[11px] text-jolu-muted",compact&&"line-clamp-2")}>
      {quota.loading && !state ? "正在读取额度…" : state ? `额度 · ${unauthorized ? "账号认证已失效，重新登录后同步" : hidden ? redactAccountMessage(state.reason) : state.reason}` : "额度尚未查询"}
    </div>}
    {!compact && quota.stale && quota.failure && <p role="status" className="flex items-center gap-1.5 text-[11px] text-jolu-danger"><AlertTriangle aria-hidden className="size-3.5" />刷新失败 · {formatSyncedAt(quota.fetchedAt).replace("同步", "数据")}</p>}
  </>
}

/** 没有真实重置卡对象就不挂载此区域，零值仍正常显示。 */
function ResetCreditsMetrics({ quota,compact=false,disabled=false }: { quota: QuotaResult; compact?:boolean;disabled?:boolean }) {
  const [checkingCards, setCheckingCards] = useState(false)
  const credits = quota.state?.state === "ok" ? quota.state.account?.reset_credits : null
  if(!credits) return null
  const text=resetCreditsText(credits,compact)
  return <div className="flex shrink-0 items-center gap-1.5 rounded-[7px] bg-jolu-side px-2 py-1 text-[10px] text-jolu-muted">
    <Ticket aria-hidden className="size-3.5 shrink-0" /><span className="shrink-0">重置卡</span><span className="min-w-0 flex-1 truncate" title={resetCreditsText(credits)}>{text}</span>
    <JoluButton size="sm" variant="soft" className={compact?"!size-6 !p-0":undefined} aria-label="查询重置卡" disabled={disabled || !isTauri || checkingCards || quota.loading} onClick={()=>{setCheckingCards(true);void quota.refresh().finally(()=>setCheckingCards(false))}}>{checkingCards ? <LoaderCircle aria-hidden className="animate-spin" /> : <RefreshCw aria-hidden />}{!compact&&"查询"}</JoluButton>
  </div>
}

function QuotaBar({ window,compact=false,showReset=true }: { window: QuotaWindowDto;compact?:boolean;showReset?:boolean }) {
  const used = window.used_percent
  const remaining = used == null || !Number.isFinite(used) || used < 0 || used > 100 ? null : remainingPercent(used)
  const reset = formatResetAt(window.resets_at)
  return (
    <div className="flex min-w-0 flex-col gap-[3px]">
      <div className="flex items-center justify-between text-[11px]">
        <span className={cn("min-w-0 text-jolu-muted",compact?"truncate":"break-all")} title={window.window_name}>{quotaTitle(window.key, window.window_name)}</span>
        <span className="text-right font-semibold text-jolu-text">{quotaValue(window)}</span>
      </div>
      {/* 未知额度不用空进度条表示（画板 23） */}
      {remaining !== null && (
        <div className="h-[5px] overflow-hidden rounded-[3px] bg-jolu-track">
          <div className="motion-quota-bar h-full rounded-[3px] bg-jolu-accent" style={{ width: `${remaining}%` }} />
        </div>
      )}
      {showReset && (reset || !compact) && <span className="text-[10px] text-jolu-muted">{reset ?? "重置时间未提供"}</span>}
    </div>
  )
}

/** API账号显示真实余额与消费；不占位显示来源没有提供的今日消费/充值。 */
function ApiMetrics({ account, balance,compact=false }: { account: Connection; balance: BalanceResult;compact?:boolean }) {
  const hidden = useContext(PrivacyContext)
  const state = balance.state
  const policy=accountCardPolicy(account,null,state)
  return <>
    {policy.showBalance ? <div className={cn("flex flex-col gap-1 rounded-lg bg-jolu-selected px-2.5 py-2",compact&&"py-1.5")}>
      <div className="flex items-center justify-between text-[10px]"><span className="text-jolu-on">可用余额</span>{balance.stale&&<span className="text-jolu-danger">刷新失败 · 上次金额</span>}</div>
      {policy.balanceAmount!==null ? <div className="flex items-baseline gap-1"><span className={cn("tnum font-semibold text-jolu-text",compact?"text-lg":"text-[22px]")}>{formatMoney(policy.balanceAmount,policy.balanceCurrency??"")}</span><span className="text-[10px] text-jolu-muted">{policy.balanceCurrency}</span></div>
        : <span className={cn("text-[11px] text-jolu-muted",compact&&"line-clamp-2")}>{balance.loading&&!state ? "正在获取余额…" : state&&state.state!=="ok" ? `${state.state==="unauthorized"?"凭证失效":state.state==="forbidden"?"查询权限不足":"余额查询失败"}：${hidden?redactAccountMessage(state.reason):state.reason}` : "余额尚未查询"}</span>}
    </div> : !compact && state?.state==="unsupported" && <p className="text-[11px] text-jolu-muted">余额 · {hidden?redactAccountMessage(state.reason):state.reason}</p>}
    {policy.used!==null&&<Stat label="累计消费" value={`${formatMoney(policy.used,policy.balanceCurrency??"")} ${policy.balanceCurrency??""}`} />}
  </>
}

function CardActions({ account, paused, refresh, hasQuota, canPromote, readOnly = false, onPromote, onEdit, onRemove,onDetails }: {
  account: Connection
  paused: boolean
  refresh: QuotaResult | BalanceResult | null
  hasQuota: boolean
  canPromote: boolean
  readOnly?: boolean
  onPromote: () => void
  onEdit: () => void
  onRemove: () => void
  onDetails: () => void
}) {
  return (
    <div className="flex shrink-0 items-center justify-end gap-1 border-t border-jolu-border pt-2">
      <span className={cn("min-w-0 flex-1 truncate text-[10px]",refresh?.stale?"text-jolu-danger":"text-jolu-muted")}>
        {paused ? "暂停查询" : refresh?.stale ? "刷新失败 · 上次结果" : refresh?.fetchedAt ? formatSyncedAt(refresh.fetchedAt, new Date(), hasQuota ? "" : "余额") : account.lastSyncText}
      </span>
      <JoluButton size="sm" variant="neutral" className="!px-2" aria-label="查看账号详情" onClick={onDetails}>详情</JoluButton>
      {!readOnly && <>
      {canPromote && <Tip label="设置为使用"><JoluButton size="sm" variant="soft" className="!w-8 !px-0" aria-label="设置为使用" disabled={!isTauri} onClick={onPromote}><Check aria-hidden /><span className="sr-only">设置为使用</span></JoluButton></Tip>}
      {refresh && <RefreshButton label={hasQuota ? "刷新当前账号" : "刷新余额"} result={refresh} />}
      {/* 编辑入口位置待确认：Jolu.pen 画板 27 有编辑弹窗，但账号卡片上未画入口 */}
      <Tip label="编辑连接">
        <button
          type="button"
          aria-label="编辑连接"
          onClick={onEdit}
          className="grid size-8 shrink-0 place-items-center rounded-lg bg-jolu-side text-jolu-muted outline-none hover:bg-jolu-stage active:bg-jolu-track focus-visible:outline-2 focus-visible:outline-jolu-accent"
        >
          <Pencil aria-hidden className="size-4" />
        </button>
      </Tip>
      <Tip label="删除账号">
        <button
          type="button"
          aria-label="删除账号"
          onClick={onRemove}
          className="grid size-8 shrink-0 place-items-center rounded-lg bg-jolu-danger-soft text-jolu-danger outline-none hover:brightness-[0.98] active:bg-jolu-danger-pressed focus-visible:outline-2 focus-visible:outline-jolu-accent"
        >
          <Trash2 aria-hidden className="size-4" />
        </button>
      </Tip>
      </>}
    </div>
  )
}

/** 刷新只影响当前卡片；请求中禁用防重复点击，结果直接体现在卡片上（画板 23「正在刷新」） */
function RefreshButton({ label, result }: { label: string; result: QuotaResult | BalanceResult }) {
  const [busy, setBusy] = useState(false)
  const loading = busy || result.loading
  return (
    <Tip label={label}><JoluButton
      size="sm"
      variant="neutral"
      className="!w-8 !px-0"
      aria-label={label}
      disabled={loading || !isTauri}
      onClick={() => {
        setBusy(true)
        void result.refresh().finally(() => setBusy(false))
      }}
    >
      {loading ? <LoaderCircle aria-hidden className="animate-spin" /> : <RefreshCw aria-hidden />}
      <span className="sr-only">{loading ? "正在刷新…" : label}</span>
    </JoluButton></Tip>
  )
}

function ConfirmShell({ title, subtitle, children, footer, onClose, busy }: {
  title: string
  subtitle: string
  children: ReactNode
  footer: ReactNode
  onClose: () => void
  busy: boolean
}) {
  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open && !busy) onClose() }}>
      <Dialog.Portal>
        <Dialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/40" />
        <Dialog.Content aria-describedby={undefined} className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex w-[480px] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 rounded-2xl border border-jolu-border bg-jolu-bg p-6 shadow-dialog outline-none">
          <div className="flex items-start justify-between gap-3">
            <div className="flex min-w-0 flex-col gap-1">
              <Dialog.Title className="text-base font-semibold text-jolu-text">{title}</Dialog.Title>
              <span className="truncate text-xs text-jolu-muted">{subtitle}</span>
            </div>
            <Dialog.Close asChild>
              <button type="button" aria-label="关闭" disabled={busy} className="grid size-7 shrink-0 place-items-center rounded-md text-jolu-muted hover:bg-jolu-stage">
                <X aria-hidden className="size-4" />
              </button>
            </Dialog.Close>
          </div>
          {children}
          <div className="flex justify-end gap-2.5">{footer}</div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

function useConfirm(onConfirm: () => Promise<void>) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const run = async () => {
    setBusy(true)
    setError(null)
    try { await onConfirm() } catch (reason) { setError(String(reason)) } finally { setBusy(false) }
  }
  return { busy, error, run }
}

const NONE = "__none__"

/** 删除账号：正在使用时需选择替代账号或暂不使用（画板 25） */
function RemoveDialog({ account, current, alternatives, islandUsers, onClose, onConfirm }: {
  account: Connection
  current: boolean
  alternatives: Connection[]
  islandUsers: string[]
  onClose: () => void
  onConfirm: (replacement: string | null) => Promise<void>
}) {
  const entry = connectionProviderEntry(account)
  const [replacement, setReplacement] = useState<string>(alternatives[0]?.id ?? "")
  const confirm = useConfirm(() => onConfirm(current && replacement ? replacement : null))
  return (
    <ConfirmShell
      title={current ? "删除正在使用的账号？" : account.kind === "api" ? "删除 API Key 账号？" : "删除订阅账号？"}
      subtitle={`${entry.name} · ${account.baseName}`}
      onClose={onClose}
      busy={confirm.busy}
      footer={<>
        <JoluButton variant="outline" disabled={confirm.busy} onClick={onClose}>取消</JoluButton>
        <JoluButton variant="danger" disabled={confirm.busy} onClick={() => void confirm.run()}>
          {confirm.busy && <LoaderCircle aria-hidden className="animate-spin" />}
          {current && replacement ? "删除并切换" : "删除账号"}
        </JoluButton>
      </>}
    >
      <p className="text-xs leading-relaxed text-jolu-text">
        {current
          ? `删除后，此账号将从 Jolu 移除。选择替代账号后，${entry.name} 会使用新的连接。`
          : account.kind === "api"
            ? "将移除本地连接配置及保存的凭据。远端 API Key 仍然有效，需要停用时请到供应商后台撤销。"
            : "将从 Jolu 移除此连接及保存的凭据，不会注销远端账号。历史统计保留。"}
      </p>
      {current && (
        <div className="flex flex-col gap-1.5">
          <SelectField
            label="替代账号"
            value={replacement || NONE}
            onValueChange={(value) => setReplacement(value === NONE ? "" : value)}
            className="h-10 w-full rounded-lg border-jolu-border bg-jolu-bg px-3 text-jolu-text hover:bg-jolu-stage"
            options={[
              ...alternatives.map((a) => ({ value: a.id, label: `${a.baseName} · ${a.kind === "api" ? "API Key" : "订阅"}` })),
              { value: NONE, label: `暂不使用 ${entry.name}` },
            ]}
          />
          <span className="text-[11px] text-jolu-muted">也可选择「暂不使用 {entry.name}」。不会注销远端账号。</span>
        </div>
      )}
      {islandUsers.length > 0 && (
        <p className="rounded-lg bg-jolu-side px-3 py-2 text-[11px] text-jolu-muted">受影响：{islandUsers.join("、")}正在显示此账号，删除后需重新选择连接。</p>
      )}
      {confirm.error && <p role="alert" className="text-xs text-jolu-danger">{confirm.error}</p>}
    </ConfirmShell>
  )
}

/**
 * 编辑已保存的连接（画板 27）：留空不覆盖已保存密钥；更换后重新获取模型；仅改名称无需重新验证。
 * 其他平台更换密钥由后端验证；自定义供应商的后端只看「有模型」（D-11），所以更换密钥后必须用新密钥
 * 重新获取模型，或在模型接口不可用时手填模型 ID 并「验证模型」，才能保存（editSaveState）。
 */
function EditDialog({ account, others, onClose, onSave }: {
  account: Connection
  others: Connection[]
  onClose: () => void
  onSave: (name: string, secret: string | null, model: string | null) => Promise<void>
}) {
  const entry = connectionProviderEntry(account)
  const [name, setName] = useState(account.baseName)
  const [replacing, setReplacing] = useState(false)
  const [secret, setSecret] = useState("")
  const [model, setModel] = useState(account.model ?? "")
  const [modelReady, setModelReady] = useState(false)
  const nameError = validateConnectionName(name, others, connectionProviderId(account) ?? account.platformId, account.kind)
  const newSecret = replacing ? secret.trim() : ""
  const save = editSaveState({
    platformId: account.platformId, kind: account.kind, nameChanged: name.trim() !== account.baseName,
    nameValid: !nameError, newSecret, model, modelReady,
  })
  const confirm = useConfirm(() => onSave(name.trim(), newSecret || null, save.needsModel ? model.trim() : null))
  return (
    <ConfirmShell
      title="编辑已保存的连接"
      subtitle={`${entry.name} · ${account.kind === "api" ? "API Key" : "订阅授权"}`}
      onClose={onClose}
      busy={confirm.busy}
      footer={<>
        <JoluButton variant="outline" disabled={confirm.busy} onClick={onClose}>取消</JoluButton>
        <JoluButton variant="primary" disabled={confirm.busy || !save.canSave} onClick={() => void confirm.run()}>
          {confirm.busy && <LoaderCircle aria-hidden className="animate-spin" />}
          保存
        </JoluButton>
      </>}
    >
      <label className="flex flex-col gap-[7px]">
        <span className="text-xs font-medium text-jolu-text">连接名称 *</span>
        <input value={name} onChange={(event) => setName(event.target.value)} spellCheck={false} autoComplete="off" className="h-10 rounded-lg border border-jolu-border bg-jolu-bg px-3 text-[13px] text-jolu-text outline-none focus:border-jolu-accent" />
        {nameError && name !== account.baseName && <span className="text-[11px] text-jolu-danger">{nameError}</span>}
      </label>
      {account.kind === "api" && (
        <div className="flex flex-col gap-[7px]">
          <span className="text-xs font-medium text-jolu-text">API Key</span>
          {replacing ? (
            <input type="password" value={secret} onChange={(event) => { setSecret(event.target.value); setModelReady(false) }} placeholder="粘贴新的 API Key" spellCheck={false} autoComplete="off" className="h-10 rounded-lg border border-jolu-border bg-jolu-bg px-3 font-mono text-[13px] text-jolu-text outline-none focus:border-jolu-accent" />
          ) : (
            <div className="flex h-10 items-center justify-between rounded-lg border border-jolu-border bg-jolu-side px-3 text-[13px] text-jolu-muted">
              <span>已保存 · {account.masked || "••••"}</span>
              <JoluButton size="sm" variant="neutral" className="h-7" onClick={() => setReplacing(true)}>更换密钥</JoluButton>
            </div>
          )}
          <span className="text-[11px] text-jolu-muted">{save.needsModel || account.platformId === "custom" ? "留空不覆盖已保存密钥；更换后重新获取模型。仅改名称无需重新验证。" : "留空不覆盖已保存密钥；仅改名称无需重新验证。"}</span>
        </div>
      )}
      {save.needsModel && account.baseUrl && (
        <CustomModelSection
          // 密钥变化后旧的获取 / 验证结果作废：用 key 重置整个区域
          key={newSecret}
          baseUrl={account.baseUrl}
          secret={newSecret}
          protocol={account.protocol ?? "openai"}
          model={model}
          onModel={(next, ready) => { setModel(next); setModelReady(ready) }}
        />
      )}
      {save.reason && <p className="text-[11px] text-jolu-muted">{save.reason}</p>}
      {confirm.error && <p role="alert" className="text-xs text-jolu-danger">{confirm.error}</p>}
    </ConfirmShell>
  )
}

/**
 * 自定义供应商更换密钥后的模型区（画板 27「地址或密钥已变更」「接口不支持模型列表」）：
 * 用新密钥重新获取模型并选择；模型接口不可用时手填模型 ID，确认后「验证模型」（可能产生少量 API 用量）。
 */
function CustomModelSection({ baseUrl, secret, protocol, model, onModel }: {
  baseUrl: string
  secret: string
  protocol: "openai" | "anthropic"
  model: string
  onModel: (model: string, ready: boolean) => void
}) {
  const [models, setModels] = useState<string[]>([])
  const [fetchError, setFetchError] = useState<string | null>(null)
  const [manual, setManual] = useState(false)
  const [askVerify, setAskVerify] = useState(false)
  const [verifyResult, setVerifyResult] = useState<{ ok: boolean; text: string } | null>(null)
  // 获取与验证共用一个闸门：Key 变化会让本组件以新 key 重新挂载，卸载时作废进行中的请求，
  // 旧 Key 的获取 / 验证结果回来后不会把新 Key 标记为已验证。
  // 忙碌状态只有一个（modelBusyAfter）：新请求开始即替换，过期请求结束不改状态，不会残留「获取中」
  const gate = useRef(requestGate()).current
  const [busy, setBusy] = useState<ModelBusy>("idle")
  const fetching = busy === "fetching"
  const verifying = busy === "verifying"
  const start = (kind: "fetching" | "verifying") => setBusy((b) => modelBusyAfter(b, { type: "start", kind }))
  const finish = (id: number) => {
    // 请求结束这一刻判断是否仍是最新请求，再交给状态更新
    const current = gate.isCurrent(id)
    setBusy((b) => modelBusyAfter(b, { type: "end", current }))
  }

  useEffect(() => () => gate.invalidate(), [gate])

  const fetchModels = async () => {
    const id = gate.begin()
    start("fetching")
    setFetchError(null)
    setVerifyResult(null)
    try {
      const list = await api.fetchRemoteModels("custom", baseUrl, secret, protocol)
      if (!gate.isCurrent(id)) return
      setModels(list)
      setManual(false)
      // 新列表包含原默认模型时保留选择，否则需要重新选择
      onModel(list.includes(model) ? model : "", list.includes(model))
    } catch (reason) {
      if (!gate.isCurrent(id)) return
      setFetchError(String(reason))
      if (fallsBackToManualModel(String(reason))) { setManual(true); onModel(model, false) }
    } finally {
      finish(id)
    }
  }

  const verify = async () => {
    if (!canStartVerify(busy)) return
    const id = gate.begin()
    const target = model.trim()
    setAskVerify(false)
    start("verifying")
    setVerifyResult(null)
    try {
      await api.verifyCustomModel(baseUrl, secret, protocol, target)
      if (!gate.isCurrent(id)) return
      setVerifyResult({ ok: true, text: "验证通过：服务已接受该模型的请求。" })
      onModel(target, true)
    } catch (reason) {
      if (!gate.isCurrent(id)) return
      setVerifyResult({ ok: false, text: String(reason) })
      onModel(target, false)
    } finally {
      finish(id)
    }
  }

  return (
    <div className="flex flex-col gap-2">
      {manual && fetchError && (
        <div role="alert" className="flex items-center gap-2.5 rounded-lg bg-jolu-danger-soft px-2.5 py-2 text-[11px] text-jolu-danger">
          <CircleAlert aria-hidden className="size-3.5 shrink-0" />
          <span className="min-w-0 flex-1 break-all">{modelFetchErrorText(fetchError)}</span>
        </div>
      )}
      <span className="text-xs font-medium text-jolu-text">{manual ? "模型 ID *" : "默认模型 *"}</span>
      <div className="flex gap-2">
        {manual ? (
          <input
            value={model}
            // 验证期间锁定模型 ID，避免验证结果对应的不是当前填写的模型
            disabled={verifying}
            onChange={(event) => { onModel(event.target.value, false); setVerifyResult(null) }}
            placeholder="输入服务支持的完整模型 ID"
            aria-label="模型 ID"
            spellCheck={false}
            autoComplete="off"
            className="h-10 min-w-0 flex-1 rounded-lg border border-jolu-border bg-jolu-bg px-3 font-mono text-[13px] text-jolu-text outline-none placeholder:text-jolu-muted focus:border-jolu-accent disabled:bg-jolu-side disabled:text-jolu-muted"
          />
        ) : (
          <ModelPicker models={models} value={models.length ? model : ""} onChange={(next) => onModel(next, true)} />
        )}
        {manual && (
          <JoluButton variant="outline" className="h-10 px-3" disabled={!model.trim() || !canStartVerify(busy) || !isTauri} onClick={() => setAskVerify(true)}>
            {verifying && <LoaderCircle aria-hidden className="animate-spin" />}
            验证模型
          </JoluButton>
        )}
        <JoluButton variant="soft" className="h-10 w-[116px] px-3" disabled={fetching || !secret || !isTauri} onClick={() => void fetchModels()}>
          {fetching ? <LoaderCircle aria-hidden className="animate-spin" /> : <RefreshCw aria-hidden />}
          {models.length ? "重新获取" : "获取模型"}
        </JoluButton>
      </div>
      {askVerify && (
        <div role="alertdialog" aria-label="验证模型" className="flex items-center gap-3 rounded-[10px] border border-jolu-border bg-jolu-side px-4 py-2.5 text-xs text-jolu-text">
          <span className="min-w-0 flex-1">验证会向服务发送一次最小请求，可能产生少量 API 用量。</span>
          <JoluButton size="sm" variant="outline" onClick={() => setAskVerify(false)}>取消</JoluButton>
          <JoluButton size="sm" variant="soft" disabled={!canStartVerify(busy)} onClick={() => void verify()}>继续验证</JoluButton>
        </div>
      )}
      {fetching ? (
        <span role="status" className="flex items-center gap-1.5 text-[11px] text-jolu-muted">
          <LoaderCircle aria-hidden className="size-3 animate-spin" />正在获取可用模型…
        </span>
      ) : verifyResult ? (
        <span role={verifyResult.ok ? "status" : "alert"} className={cn("text-[11px]", verifyResult.ok ? "text-jolu-on" : "text-jolu-danger")}>{verifyResult.text}</span>
      ) : fetchError && !manual ? (
        <span role="alert" className="text-[11px] text-jolu-danger">{modelFetchErrorText(fetchError)}</span>
      ) : (
        <span className="text-[11px] text-jolu-muted">
          {manual ? "模型接口不支持枚举时才展示。验证可能产生少量 API 用量，先提示再执行。" : "旧模型列表标记失效；重新获取后再选择默认模型。"}
        </span>
      )}
    </div>
  )
}

function PromoteDialog({ account, onClose, onConfirm }: { account: Connection; onClose: () => void; onConfirm: () => Promise<void> }) {
  const entry = connectionProviderEntry(account)
  const confirm = useConfirm(onConfirm)
  return (
    <ConfirmShell
      title={`将「${account.baseName}」设为当前账号`}
      subtitle={`仅更改 ${entry.name} 的当前账号`}
      onClose={onClose}
      busy={confirm.busy}
      footer={<>
        <JoluButton variant="outline" disabled={confirm.busy} onClick={onClose}>取消</JoluButton>
        <JoluButton variant="primary" disabled={confirm.busy} onClick={() => void confirm.run()}>
          {confirm.busy ? <LoaderCircle aria-hidden className="animate-spin" /> : <Check aria-hidden className="!text-[#071B10]" />}
          设为当前
        </JoluButton>
      </>}
    >
      <div className="flex items-center gap-2 rounded-lg bg-jolu-side px-3 py-2.5 text-xs text-jolu-text">
        <PlatformLogo platform={account.platformId} />
        {account.baseName} · {account.kind === "api" ? "API Key" : "订阅授权"}
      </div>
      <p className="text-xs leading-relaxed text-jolu-text">
        {account.kind === "api"
          ? "新的连接可能按 API 用量计费。会写入本机 CLI 配置（原配置先备份）；已有会话是否支持切换取决于对应工具。"
          : `切换后 ${entry.name} 使用此订阅账号，会写入本机 CLI 配置（原配置先备份）；已有会话是否支持切换取决于对应工具。`}
      </p>
      {confirm.error && <p role="alert" className="text-xs text-jolu-danger">{confirm.error}</p>}
    </ConfirmShell>
  )
}
