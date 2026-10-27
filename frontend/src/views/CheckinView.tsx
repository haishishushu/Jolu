import { useCallback, useEffect, useRef, useState } from "react"
import { CalendarCheck2, Check, Eye, EyeOff, Info, LoaderCircle, Plus, RefreshCw } from "lucide-react"
import { api, isTauri, listenEvent } from "@/lib/api"
import { checkinApi, type CheckinRecordDto, type CheckinUpdate } from "@/lib/checkinApi"
import { demoCheckinApi } from "@/mock/checkin"
import { CHECKIN_EDITIONS, STATE_LABELS, applyCheckinUpdate, checkinAccountLabel, checkinSummary, checkinTargets, currentOutcome, initialCheckinAccount, rewardLabel, uniqueCheckinAccounts, type CheckinAccount, type CheckinEdition } from "@/lib/dailyCheckin"
import { cn } from "@/lib/utils"
import { JoluButton } from "@/components/ui/JoluButton"
import { LoadingSurface } from "@/components/ui/LoadingSurface"
import { useToast } from "@/components/ui/Toast"
import { AccountConnectDialog } from "@/components/providers/AccountConnectDialog"
import { connectionProviderId, type ProviderId } from "@/lib/providerCatalog"
import { CheckinProviderDialog, CheckinRemoveAccountDialog } from "@/components/checkin/CheckinDialogs"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { CheckinAccountStats } from "@/components/checkin/CheckinAccountStats"
import { useAccountSelection, type AccountSelection } from "@/hooks/useAccountSelection"
import { AccountBatchToolbar, BatchAccountDeleteDialog, type BatchAccountTarget } from "@/components/providers/AccountBatchControls"
import { removeAccounts, type BatchAccountDeleteResult } from "@/lib/accountActions"

type Activity = { kind: "refresh" | "claim"; done: number; total: number; name: string } | null
const PRIVACY_KEY = "jolu:checkin:hideAccounts"
// 浏览器预览使用示例数据；桌面端读取失败时不回落到示例。
const checkinSource: typeof checkinApi = isTauri ? checkinApi : demoCheckinApi

/** Jolu.pen 模块 I（133–160）：四列摘要、右侧详情、统一操作区及批量删除/领取。 */
export function CheckinView({ active = true }: { active?: boolean }) {
  const [connecting, setConnecting] = useState<ProviderId | null>(null)
  const connectingRef = useRef<ProviderId | null>(null)
  const [choosingProvider, setChoosingProvider] = useState(false)
  const choosingProviderRef = useRef(false)
  const openConnection = (provider: ProviderId | null) => {
    if (!provider) return
    connectingRef.current = provider
    setConnecting(provider)
  }
  const [accounts, setAccounts] = useState<CheckinAccount[]>([])
  const [records, setRecords] = useState<CheckinRecordDto[]>([])
  const [loaded, setLoaded] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [activity, setActivity] = useState<Activity>(null)
  const [removingAccount, setRemovingAccount] = useState<CheckinAccount | null>(null)
  const removingAccountRef = useRef<CheckinAccount | null>(null)
  const [removing, setRemoving] = useState(false)
  const [removeError, setRemoveError] = useState<string | null>(null)
  const selection = useAccountSelection(accounts)
  const [batchTargets, setBatchTargets] = useState<BatchAccountTarget[] | null>(null)
  const batchTargetsRef = useRef<BatchAccountTarget[] | null>(null)
  const [batchDeleting, setBatchDeleting] = useState(false)
  const [hidden, setHidden] = useState(() => { try { return localStorage.getItem(PRIVACY_KEY) === "1" } catch { return false } })
  const [, tick] = useState(0)
  const alive = useRef(true)
  const operating = useRef(false)
  const activeRef = useRef(active)
  activeRef.current = active
  const pendingRefresh = useRef(false)
  const refreshRef = useRef<() => Promise<void>>(async () => {})
  const toast = useToast()

  const refresh = useCallback(async (notify = false) => {
    if (connectingRef.current || choosingProviderRef.current || removingAccountRef.current || batchTargetsRef.current) { pendingRefresh.current = true; return }
    if (operating.current) return
    operating.current = true
    pendingRefresh.current = false
    if (alive.current) { setError(null); setActivity({ kind: "refresh", done: 0, total: 0, name: "" }) }
    try {
      const data = await checkinSource.overview()
      if (!alive.current) return
      let rows = data.accounts.map(initialCheckinAccount)
      setAccounts(rows)
      setRecords(data.records)
      setLoaded(true)
      const targets = rows.filter(a => a.connection_status !== "paused" && CHECKIN_EDITIONS.some(e => e.manual && e.platform === a.platform && e.edition === a.edition))
      let cursor = 0, done = 0
      // 只读状态至多三个并行；任何账号失败都保留独立结果。
      await Promise.all(Array.from({ length: Math.min(3, targets.length) }, async () => {
        while (alive.current && cursor < targets.length) {
          const account = targets[cursor++]
          let update: CheckinUpdate
          try { update = await checkinSource.status(account.id) }
          catch { update = failedUpdate(account, "签到状态读取失败，请重新查询") }
          // 先刷新缓存再挂载该身份的额度区，末个完成的账号也能拿到本次手动查询结果。
          if (notify && alive.current && activeRef.current && update.account_key !== null) {
            try { await api.connectionQuota(account.id, true) } catch { /* 额度区独立展示查询失败。 */ }
          }
          if (!alive.current) return
          rows = applyCheckinUpdate(rows, update)
          setAccounts(rows)
          setActivity({ kind: "refresh", done: ++done, total: targets.length, name: "" })
        }
      }))
      if (notify && alive.current) {
        const summary = checkinSummary(rows)
        const detail = `${summary.ready} 个可签到 · ${summary.claimed} 个已签到${summary.errors ? ` · ${summary.errors} 个需处理，请查看对应账号状态` : ""}`
        if (summary.errors) toast.warn("签到状态已刷新", detail)
        else toast.success("签到状态已刷新", detail)
      }
    } catch {
      if (alive.current) {
        setError("签到账号或记录读取失败，请重新读取。")
        if (notify) toast.danger("签到状态刷新失败", "签到账号或记录读取失败，请重新读取。")
      }
    } finally {
      operating.current = false
      if (alive.current) {
        setActivity(null)
        if (pendingRefresh.current) void refreshRef.current()
      }
    }
  }, [toast])
  refreshRef.current = refresh

  useEffect(() => {
    alive.current = true
    void refresh()
    let off: (() => void) | undefined
    void listenEvent("connections-changed", () => {
      if (!alive.current) return
      if (operating.current) pendingRefresh.current = true
      else if (activeRef.current) void refresh()
    }).then(unlisten => { if (alive.current) off = unlisten; else unlisten() }).catch(() => {})
    return () => { alive.current = false; off?.() }
  }, [refresh])

  useEffect(() => {
    if (!active) return
    const onFocus = () => { if (document.visibilityState === "visible") void refresh() }
    const timer = window.setInterval(() => {
      tick(value => value + 1)
      if (document.visibilityState === "visible") void refresh()
    }, 60_000)
    window.addEventListener("focus", onFocus)
    document.addEventListener("visibilitychange", onFocus)
    if (loaded && !operating.current) void refresh()
    return () => { window.clearInterval(timer); window.removeEventListener("focus", onFocus); document.removeEventListener("visibilitychange", onFocus) }
    // 返回页面及每分钟只读刷新，避免跨日或原工具换账号后继续使用旧状态。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, refresh])

  const queryOne = async (account: CheckinAccount) => {
    if (operating.current) return
    operating.current = true
    setActivity({ kind: "refresh", done: 0, total: 1, name: account.name })
    try {
      const result = await checkinSource.status(account.id)
      if (alive.current) setAccounts(rows => applyCheckinUpdate(rows, result))
    } catch {
      if (alive.current) setAccounts(rows => applyCheckinUpdate(rows, failedUpdate(account, "签到状态读取失败，请重新查询")))
    } finally {
      operating.current = false
      if (alive.current) { setActivity(null); if (pendingRefresh.current) void refresh() }
    }
  }

  const run = async (selected?: CheckinAccount) => {
    if (operating.current) return
    const targets = checkinTargets(selected ? [selected] : accounts)
    if (!targets.length) return
    operating.current = true
    let rows = accounts, finished = 0, claimed = 0, failures = 0
    setActivity({ kind: "claim", done: 0, total: targets.length, name: "" })
    try {
      for (const account of targets) {
        if (!alive.current) break
        // 上一个结果已将重复连接同步为已领取时，不再提交相同身份。
        if (!checkinTargets(rows).some(a => a.id === account.id)) { finished++; continue }
        setActivity({ kind: "claim", done: finished, total: targets.length, name: hidden ? "账号" : account.name })
        let update: CheckinUpdate
        try { update = await checkinSource.claim(account.id, account.account_key!) }
        catch { update = failedUpdate(account, "领取结果尚未确认，请先刷新状态核实", "unknown") }
        rows = applyCheckinUpdate(rows, update)
        if (update.outcome.state === "claimed") claimed++
        else failures++
        finished++
        if (alive.current) { setAccounts(rows); setActivity({ kind: "claim", done: finished, total: targets.length, name: "" }) }
      }
      if (alive.current) {
        const title = failures ? `本次处理完成：${claimed} 个已签到，${failures} 个需处理` : `本次处理完成：${claimed} 个已签到`
        if (failures) toast.warn(title, "请查看对应账号状态；未自动重试领取。")
        else toast.success(title)
        try { setRecords((await checkinSource.overview()).records) }
        catch { setError("签到操作已结束，但最新记录读取失败，请刷新状态。") }
      }
    } finally {
      operating.current = false
      if (alive.current) { setActivity(null); if (pendingRefresh.current) void refresh() }
    }
  }

  const summary = checkinSummary(accounts)
  const unresolved = accounts.filter(a => !CHECKIN_EDITIONS.some(e => e.platform === a.platform && e.edition === a.edition))
  const operationBusy = activity !== null || removingAccount !== null || batchTargets !== null || batchDeleting
  const busy = operationBusy || selection.active
  const closeRemove = () => {
    if (operating.current) return
    removingAccountRef.current = null
    setRemovingAccount(null)
    setRemoveError(null)
    if (pendingRefresh.current) void refresh()
  }
  const removeAccount = async () => {
    const account = removingAccountRef.current
    if (!account || operating.current) return
    operating.current = true
    setRemoving(true)
    setRemoveError(null)
    try {
      await api.removeConnection(account.id)
      if (!alive.current) return
      setAccounts(rows => rows.filter(row => row.id !== account.id))
      removingAccountRef.current = null
      setRemovingAccount(null)
      toast.success("账号已删除", "已移除 Jolu 中的连接，签到记录保留。")
      pendingRefresh.current = true
    } catch (reason) {
      if (alive.current) setRemoveError(String(reason))
    } finally {
      operating.current = false
      if (alive.current) {
        setRemoving(false)
        if (!removingAccountRef.current && pendingRefresh.current) void refresh()
      }
    }
  }
  const closeBatch = () => {
    if (operating.current) return
    batchTargetsRef.current = null
    setBatchTargets(null)
    if (pendingRefresh.current) void refresh()
  }
  const removeSelected = async (ids: string[]): Promise<BatchAccountDeleteResult> => {
    if (operating.current) return { deletedIds: [], failed: ids.map(id => ({ id, message: "当前操作尚未结束，请稍后重试" })) }
    const confirmed = new Set(batchTargetsRef.current?.map(target => target.id))
    const accepted = ids.filter(id => confirmed.has(id))
    operating.current = true
    setBatchDeleting(true)
    try {
      const result = await removeAccounts(accepted, api.removeConnection)
      if (alive.current) {
        const deleted = new Set(result.deletedIds)
        setAccounts(rows => rows.filter(row => !deleted.has(row.id)))
        selection.discard(result.deletedIds)
        if (result.failed.length) toast.warn(`已删除 ${result.deletedIds.length} 个账号，${result.failed.length} 个未删除`, "失败账号已保留，可在确认窗口重试。")
        else toast.success(`已删除 ${result.deletedIds.length} 个账号`, "签到记录保留，原工具登录不受影响。")
        pendingRefresh.current = true
      }
      return result
    } finally {
      operating.current = false
      if (alive.current) setBatchDeleting(false)
    }
  }
  const privacy = () => setHidden(value => {
    try { localStorage.setItem(PRIVACY_KEY, value ? "0" : "1") } catch { /* 不影响本次隐私切换 */ }
    return !value
  })

  return <section aria-label="每日签到" className="@container flex min-w-0 flex-col gap-4">
    <div className="flex flex-wrap items-center justify-between gap-4 py-1">
      <div className="flex min-w-0 flex-col gap-2">
        <h1 className="text-2xl font-semibold text-jolu-text">每日福利，一次领取</h1>
        <p className="text-xs text-jolu-muted">{activity?.kind === "refresh" ? "正在查询账号签到状态…" : error ? "账号状态可能已过期，请刷新核实" : !loaded ? "尚未读取账号状态" : <>{summary.ready} 个可签到 · {summary.claimed} 个已签到{summary.errors > 0 && ` · ${summary.errors} 个需处理`} · 仅领取官方每日奖励</>}</p>
      </div>
      <div data-checkin-toolbar className="flex flex-wrap items-center gap-2.5">
        <JoluButton aria-label={hidden ? "显示账号" : "隐藏账号"} variant="outline" onClick={privacy}>{hidden ? <EyeOff aria-hidden /> : <Eye aria-hidden />}</JoluButton>
        <JoluButton disabled={busy} onClick={() => void refresh(true)}><RefreshCw aria-hidden className={cn(activity?.kind === "refresh" && "animate-spin motion-reduce:animate-none")} />刷新状态</JoluButton>
        <JoluButton disabled={busy} onClick={() => { choosingProviderRef.current = true; setChoosingProvider(true) }}><Plus aria-hidden />添加账号</JoluButton>
        <JoluButton variant="primary" disabled={busy || !summary.ready} onClick={() => void run()}>{activity?.kind === "claim" ? <LoaderCircle aria-hidden className="animate-spin motion-reduce:animate-none" /> : <CalendarCheck2 aria-hidden />}一键签到</JoluButton>
      </div>
    </div>
    <div data-checkin-batch-toolbar className="flex flex-wrap items-center justify-end gap-2">
      <AccountBatchToolbar selection={selection} total={accounts.length} disabled={operationBusy || !loaded} onDelete={() => {
        if (operating.current) return
        const targets = accounts.filter(account => selection.selected.has(account.id)).map(account => {
          const entry = CHECKIN_EDITIONS.find(item => item.platform === account.platform && item.edition === account.edition)
          return { id: account.id, label: checkinAccountLabel(account, hidden), provider: entry ? `${entry.name} ${entry.region}` : "版本未识别", current: account.id.startsWith("monitor:native:") }
        })
        if (!targets.length) return
        batchTargetsRef.current = targets
        setBatchTargets(targets)
      }} />
    </div>
    {error && <div role="alert" className="flex flex-wrap items-center justify-between gap-3 rounded-lg bg-jolu-danger-soft p-4 text-xs text-jolu-danger"><span>{error}</span><JoluButton size="sm" variant="danger-soft" disabled={busy} onClick={() => void refresh()}>重新读取</JoluButton></div>}
    {!loaded && busy ? <LoadingSurface layout="cards" label="正在读取签到账号" /> : <>
      {unresolved.length > 0 && <div role="alert" className="flex flex-wrap items-center justify-between gap-3 rounded-lg bg-jolu-side p-3 text-xs text-jolu-muted"><span>{unresolved.length} 个账号的版本无法核实，请重新授权对应版本。</span>{selection.active ? <div className="flex flex-wrap gap-3">{unresolved.map(account => <label key={account.id} className="flex items-center gap-2"><input type="checkbox" aria-label={`选择账号 ${checkinAccountLabel(account, hidden)}`} checked={selection.selected.has(account.id)} disabled={operationBusy} onChange={() => selection.toggle(account.id)} className="size-4 accent-jolu-accent" />{checkinAccountLabel(account, hidden)}</label>)}</div> : <JoluButton size="sm" disabled={busy} onClick={() => openConnection("claude")}>连接账号</JoluButton>}</div>}
      <div className="grid grid-cols-1 gap-4 @[560px]:grid-cols-2 @[780px]:grid-cols-3 @[1040px]:grid-cols-4">
        {CHECKIN_EDITIONS.map(entry => {
          const rows = accounts.filter(account => account.platform === entry.platform && account.edition === entry.edition)
          const displayed = selection.active ? rows : uniqueCheckinAccounts(rows.filter(account => account.connection_status !== "paused"))
          return <CheckinCard key={entry.id} entry={entry} accounts={displayed} paused={selection.active ? 0 : rows.filter(account => account.connection_status === "paused").length} busy={busy} selection={selection} selectionDisabled={operationBusy} hidden={hidden} active={active} onConnect={() => openConnection(connectionProviderId({ platformId: entry.platform, edition: entry.edition }))} onQuery={account => void queryOne(account)} onClaim={account => void run(account)} onRemove={account => { if (operating.current) return; removingAccountRef.current = account; setRemovingAccount(account); setRemoveError(null) }} />
        })}
      </div>
      <p className="flex items-start gap-2.5 rounded-[10px] bg-jolu-side px-4 py-3 text-xs leading-relaxed text-jolu-muted"><Info aria-hidden className="mt-0.5 size-[17px] shrink-0" />账号沿用供应商连接；相同账号只领取一次。奖励和开放状态以平台实时返回为准。</p>
      <div className="overflow-hidden rounded-xl border border-jolu-border bg-jolu-bg p-4">
        <h2 className="mb-3 text-sm font-semibold text-jolu-text">最近签到记录</h2>
        {!records.length ? <p className="py-2 text-xs text-jolu-muted">暂无签到记录，完成首次签到后显示结果。</p> : <ol className="flex max-h-64 flex-col divide-y divide-jolu-border overflow-auto">
          {records.map(record => {
            const entry = CHECKIN_EDITIONS.find(e => e.platform === record.platform && e.edition === record.edition)
            return <li key={record.id} className="flex flex-wrap items-start justify-between gap-x-4 gap-y-1 py-3 text-xs">
              <div className="flex min-w-0 flex-1 flex-col gap-1.5"><span className="break-words text-jolu-text">{entry ? `${entry.name} ${entry.region}` : record.platform} · {hidden ? "账号已隐藏" : record.name}</span><span className={cn("break-words leading-relaxed", record.outcome.state === "claimed" ? "text-jolu-on" : "text-jolu-muted")}>{record.outcome.message}{record.outcome.reward !== null && ` · ${rewardLabel(record.outcome)}`}</span></div>
              <time dateTime={new Date(record.created_ms).toISOString()} className="shrink-0 text-[11px] text-jolu-muted">{new Date(record.created_ms).toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" })}</time>
            </li>
          })}
        </ol>}
      </div>
    </>}
    {connecting && <div data-checkin-connect-dialog><AccountConnectDialog platform={connecting} onClose={() => { connectingRef.current = null; setConnecting(null); if (pendingRefresh.current) void refresh() }} onSaved={() => { pendingRefresh.current = true }} /></div>}
    {choosingProvider && <CheckinProviderDialog onClose={() => { choosingProviderRef.current = false; setChoosingProvider(false); if (pendingRefresh.current) void refresh() }} onPick={provider => { choosingProviderRef.current = false; setChoosingProvider(false); openConnection(provider) }} />}
    {removingAccount && <CheckinRemoveAccountDialog account={checkinAccountLabel(removingAccount, hidden)} busy={removing} error={removeError} onClose={closeRemove} onConfirm={() => void removeAccount()} />}
    {batchTargets && <BatchAccountDeleteDialog targets={batchTargets} hidden={hidden} onClose={closeBatch} onConfirm={removeSelected} />}
  </section>
}

function failedUpdate(account: CheckinAccount, message: string, state: "failed" | "unknown" = "failed"): CheckinUpdate {
  return { id: account.id, account_key: null, account: null, checked_at: Date.now(), statistics: null, outcome: { state, message, reward: null, unit: account.outcome.unit, period_key: null, reset_at: null } }
}

function CheckinCard({ entry, accounts, paused, busy, hidden, active, onConnect, onQuery, onClaim, onRemove, selection, selectionDisabled }: {
  entry: CheckinEdition; accounts: CheckinAccount[]; paused: number; busy: boolean; hidden: boolean; active: boolean
  onConnect: () => void; onQuery: (account: CheckinAccount) => void; onClaim: (account: CheckinAccount) => void
  onRemove: (account: CheckinAccount) => void
  selection: AccountSelection; selectionDisabled: boolean
}) {
  const [detailsAccountId, setDetailsAccountId] = useState<string | null>(null)
  useEffect(() => {
    if (detailsAccountId && (!active || !accounts.some(account => account.id === detailsAccountId))) setDetailsAccountId(null)
  }, [active, accounts, detailsAccountId])
  return <article aria-label={`${entry.name} ${entry.region}`} data-checkin-edition={entry.id} className="flex min-h-[210px] min-w-0 flex-col gap-3 rounded-xl border border-jolu-border bg-jolu-bg p-4">
    <header className="flex items-center justify-between gap-2"><h2 className="flex min-w-0 items-center gap-2.5 text-base font-semibold text-jolu-text"><PlatformLogo platform={entry.platform} size={22} /><span className="truncate">{entry.name}</span></h2><span className="shrink-0 rounded-[5px] bg-jolu-side px-2 py-1 text-[11px] text-jolu-muted">{entry.region}</span></header>
    {!entry.manual && !accounts.length ? <><p className="text-xs text-jolu-muted">{entry.platform === "workbuddy" ? "自动活跃奖励" : "暂未开放"}</p><p className="text-2xl font-medium text-jolu-text">{entry.platform === "workbuddy" ? "无需签到" : "—"}</p><p className="text-[11px] leading-relaxed text-jolu-muted">{entry.description}</p><JoluButton className="mt-auto w-full" disabled>{entry.platform === "workbuddy" ? "无需手动领取" : "暂未开放"}</JoluButton></>
    : !accounts.length ? <><p className="text-xs text-jolu-muted">{paused ? `${paused} 个账号已暂停连接` : "尚未连接账号"}</p><p className="text-2xl font-medium text-jolu-text">{entry.platform === "qoder" ? "每日 Credits" : "每日积分"}</p><p className="text-[11px] leading-relaxed text-jolu-muted">{entry.description}</p><JoluButton className="mt-auto w-full" variant="soft" disabled={busy} onClick={() => onConnect()}>{paused ? "管理连接" : "去连接"}</JoluButton></>
    : <div className="flex flex-1 flex-col gap-4">
      {accounts.map(account => {
        const outcome = currentOutcome(account)
        const ready = outcome.state === "ready", claimed = outcome.state === "claimed"
        const accountPaused = account.connection_status === "paused"
        const label = checkinAccountLabel(account, hidden)
        const actionLabel = accountPaused ? "已暂停" : !entry.manual ? entry.platform === "workbuddy" ? "无需手动领取" : "暂未开放" : ready ? "签到" : claimed ? "本轮已签到" : outcome.state === "unauthorized" ? "更新登录" : outcome.state === "inactive" ? "活动未开放" : STATE_LABELS[outcome.state]
        const action = <JoluButton size="sm" className="w-full min-w-0 gap-1 px-2 text-[11px] [&_svg]:size-3.5" variant={ready || claimed ? "soft" : "neutral"} disabled={busy || accountPaused || !entry.manual || claimed || outcome.state === "unsupported" || outcome.state === "inactive"} aria-label={`${entry.name} ${entry.region} ${label} ${ready ? "签到" : claimed ? "本轮已签到" : outcome.state === "unauthorized" ? "更新登录" : "查询状态"}`} onClick={() => ready ? onClaim(account) : outcome.state === "unauthorized" ? onConnect() : onQuery(account)}>{claimed ? <Check aria-hidden /> : null}{actionLabel}</JoluButton>
        const deleteAction = <JoluButton size="sm" variant="danger-soft" className="px-2" aria-label={`删除账号 ${label}`} disabled={busy} onClick={() => onRemove(account)}>删除</JoluButton>
        return <div key={account.id} data-checkin-account={account.id} className={cn("flex flex-1 flex-col gap-2", selection.active && "rounded-lg p-2 ring-1 ring-inset", selection.active && (selection.selected.has(account.id) ? "ring-jolu-accent" : "ring-jolu-border"))}>
          <div className="flex items-start gap-2 text-xs">
            {selection.active && <input type="checkbox" aria-label={`选择账号 ${label}`} checked={selection.selected.has(account.id)} disabled={selectionDisabled} onChange={() => selection.toggle(account.id)} className="mt-0.5 size-4 shrink-0 accent-jolu-accent" />}
            <span className="min-w-0 flex-1 break-all text-jolu-muted">账号：{label}</span>
            {accountPaused && <span className="shrink-0 text-[10px] text-jolu-muted">已暂停</span>}
          </div>
          <div className="flex flex-wrap items-baseline justify-between gap-2"><p className={cn("text-2xl font-medium tabular-nums", ready || claimed ? "text-jolu-on" : "text-jolu-text")}>{!entry.manual ? entry.platform === "workbuddy" ? "自动活跃奖励" : "暂未开放" : outcome.reward !== null ? rewardLabel(outcome) : claimed ? "已领取" : "—"}</p>{entry.manual && <span className="text-[11px] text-jolu-muted">今日签到奖励</span>}</div>
          <CheckinAccountStats key={`${account.id}:${account.account_key ?? account.account ?? "unknown"}`} accountId={account.id} statistics={account.statistics} official={outcome.official_statistics} identityKey={accountPaused ? null : entry.manual ? account.account_key : account.account ?? null} active={active} showHistory={entry.manual} accountLabel={label} providerLabel={`${entry.name} ${entry.region}`} statusMessage={accountPaused ? "该账号已暂停查询，配置与历史保留。" : account.checked_at === null ? entry.description : outcome.message} detailsOpen={detailsAccountId === account.id} onDetailsOpenChange={open => setDetailsAccountId(open ? account.id : null)} action={action} deleteAction={deleteAction} />
        </div>
      })}
      {paused > 0 && <button type="button" disabled={busy} onClick={() => onConnect()} className="text-left text-[11px] text-jolu-muted underline underline-offset-2 focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-40">另有 {paused} 个账号已暂停连接</button>}
    </div>}
  </article>
}
