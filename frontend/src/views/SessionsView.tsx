import { useEffect, useMemo, useState } from "react"
import { ArrowDownLeft, ArrowDownWideNarrow, ArrowUpRight, Database, Files, Layers, MessagesSquare, RefreshCw, RotateCcw, Search, Target } from "lucide-react"
import { cn } from "@/lib/utils"
import { api, isTauri, type SessionItemDto } from "@/lib/api"
import {
  TIME_PRESETS, UNKNOWN_PROJECT, groupByProject, projectKey, projectName, sessionKey, sessionTotals, sinceFor,
  type SessionSort, type TimePreset,
} from "@/lib/sessionThreads"
import { formatPercent, formatTokens } from "@/lib/usageStats"
import { UNATTRIBUTED_ACCOUNT, accountLabel, accountsFor } from "@/lib/usageFilters"
import { useConnections } from "@/hooks/useConnections"
import { useStatsQuery } from "@/hooks/useStatsQuery"
import { demoSessions } from "@/mock/sessionThreads"
import { useToast } from "@/components/ui/Toast"
import { SessionDetail } from "@/components/sessions/SessionDetail"
import { SessionTree } from "@/components/sessions/SessionTree"
import { AutomationConfirmDialog, ContinueDialog } from "@/components/sessions/sessionDialogs"
import { ALL, ErrorNotice, MenuFilter, ToolButton, providerName, providerOptions } from "@/components/usage/usageControls"
import type { PlatformId } from "@/types"

export interface SessionTarget {
  platform: string
  sessionId: string
  nonce: number
}

/**
 * 会话线程（Jolu.pen 模块 D，画板 40–57）：顶部统计、搜索与筛选、项目 → 会话树与只读详情。
 * 压缩会话（画板 48–55）需要调用模型，暂标「待接入」。
 */
export function SessionsView({ active, target, onDrillUsage, onCreateAutomation }: {
  active: boolean
  target: SessionTarget | null
  onDrillUsage: (session: SessionItemDto) => void
  onCreateAutomation: (session: SessionItemDto) => void
}) {
  const toast = useToast()
  const conn = useConnections()
  const [platform, setPlatform] = useState<PlatformId | null>(null)
  const [account, setAccount] = useState<string | null>(null)
  const [project, setProject] = useState<string | null>(null)
  const [time, setTime] = useState<TimePreset>("all")
  const [sort, setSort] = useState<SessionSort>("recent")
  const [searchDraft, setSearchDraft] = useState("")
  const [search, setSearch] = useState("")
  const [selected, setSelected] = useState<string | null>(null)
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [continuing, setContinuing] = useState<SessionItemDto | null>(null)
  const [automating, setAutomating] = useState<SessionItemDto | null>(null)
  const [syncedAt, setSyncedAt] = useState(() => Date.now())
  const [scanning, setScanning] = useState(false)
  /** 外部跳转要定位的会话：列表刷新完成前不被「默认选中最近会话」覆盖 */
  const [pending, setPending] = useState<string | null>(null)

  const query = useStatsQuery(
    JSON.stringify([platform, account, time, search]),
    () => api.sessionsList({ platform, connection: account, project: null, since: sinceFor(time), search: search.trim() || null }),
    () => demoSessions(Date.now()),
    platform,
    active,
  )
  useEffect(() => { if (query.data) setSyncedAt(Date.now()) }, [query.data])

  const all = useMemo(() => query.data?.sessions ?? [], [query.data])
  const projects = useMemo(() => {
    const seen = new Map<string, string>()
    for (const s of all) if (!seen.has(projectKey(s.cwd))) seen.set(projectKey(s.cwd), projectName(s.cwd))
    return [...seen.entries()].sort((a, b) => (!a[0] !== !b[0] ? (a[0] ? -1 : 1) : a[1].localeCompare(b[1])))
  }, [all])
  const sessions = useMemo(() => (project === null ? all : all.filter((s) => projectKey(s.cwd) === project)), [all, project])
  const groups = useMemo(() => groupByProject(sessions, sort), [sessions, sort])
  const totals = sessionTotals(sessions)
  /** 列表尚未读到时统计未知，显示「—」而不是 0 */
  const known = query.data !== null
  const current = sessions.find((s) => sessionKey(s) === selected) ?? null
  const searching = !!search.trim()
  const filtered = !!(platform || account || project !== null || time !== "all" || searching)

  // 默认选中最近的会话并展开它所在的项目；搜索时展开全部匹配项目
  useEffect(() => {
    if (pending) {
      if (sessions.some((s) => sessionKey(s) === pending)) {
        setSelected(pending)
        setPending(null)
        return
      }
      if (query.loading) return
      setPending(null)
      toast.info("没有找到这条会话", "可能已被清理或不在本机记录中")
    }
    if (!groups.length) return
    if (searching) setExpanded(new Set(groups.map((g) => g.key)))
    if (!current) {
      const first = groups[0].sessions[0]
      setSelected(sessionKey(first))
      setExpanded((value) => new Set([...value, groups[0].key]))
    }
  }, [groups, searching, current, pending, query.loading])

  // 从用量统计跳转过来时定位到指定会话
  useEffect(() => {
    if (!target) return
    setPlatform(null)
    setAccount(null)
    setProject(null)
    setTime("all")
    setSearch("")
    setSearchDraft("")
    setPending(`${target.platform}:${target.sessionId}`)
  }, [target])
  useEffect(() => {
    const found = current ?? null
    if (found) setExpanded((value) => (value.has(projectKey(found.cwd)) ? value : new Set([...value, projectKey(found.cwd)])))
  }, [current])

  const toggle = (key: string, open?: boolean) => setExpanded((value) => {
    const next = new Set(value)
    if (open ?? !next.has(key)) next.add(key)
    else next.delete(key)
    return next
  })

  const clearAll = () => {
    setPlatform(null)
    setAccount(null)
    setProject(null)
    setTime("all")
    setSearch("")
    setSearchDraft("")
  }

  const rescan = async () => {
    if (!isTauri) return query.refresh()
    setScanning(true)
    try {
      await api.scanLocalSessions()
    } catch (reason) {
      toast.danger("重新扫描失败", reason instanceof Error ? reason.message : String(reason))
    } finally {
      setScanning(false)
      query.refresh()
    }
  }

  const accountOptions = [
    { value: ALL, label: "全部账号" },
    ...accountsFor(conn.connections, platform).map((c) => ({ value: c.id, label: accountLabel(c) })),
    { value: UNATTRIBUTED_ACCOUNT, label: "未归属账号" },
  ]
  const minutes = Math.floor((Date.now() - syncedAt) / 60_000)

  return (
    <div className="flex w-full flex-col gap-3">
      <div className="flex h-[92px] gap-2">
        <Stat selected icon={<MessagesSquare />} label="线程" value={known ? String(totals.threads) : "—"} note="筛选内去重会话" />
        <Stat icon={<Layers />} label="消耗 Token" value={known ? formatTokens(totals.consumption) : "—"} note={totals.unknown ? `输入 + 输出 · ${totals.unknown} 条未知` : "输入 + 输出"} />
        <Stat icon={<Target />} label="缓存命中" value={known ? formatPercent(totals.hitRate) : "—"} note="缓存读取占输入" />
        <Stat icon={<Files />} label="记录文件" value={query.data ? String(query.data.files) : "—"} note="去重来源文件" />
        <div className="flex min-w-0 flex-1 items-center rounded-[10px] border border-jolu-border bg-jolu-bg">
          <Stat bare icon={<ArrowDownLeft />} label="输入 Token" value={known ? formatTokens(totals.input) : "—"} note="含缓存输入" />
          <span className="h-14 w-px bg-jolu-border" />
          <Stat bare icon={<Database />} label="缓存输入" value={known ? formatTokens(totals.cache) : "—"} note="已包含于输入" />
          <span className="h-14 w-px bg-jolu-border" />
          <Stat bare icon={<ArrowUpRight />} label="输出 Token" value={known ? formatTokens(totals.output) : "—"} note="生成 Token" />
        </div>
      </div>

      <div className="flex h-[38px] items-center gap-2.5">
        <label className="flex h-[38px] w-[360px] items-center gap-2 rounded-lg border border-jolu-border bg-jolu-bg px-3 focus-within:border-jolu-accent">
          <Search aria-hidden className="size-4 shrink-0 text-jolu-muted" />
          <input
            value={searchDraft}
            onChange={(event) => {
              setSearchDraft(event.target.value)
              if (!event.target.value) setSearch("")
            }}
            onKeyDown={(event) => event.key === "Enter" && setSearch(searchDraft)}
            placeholder="搜索标题、会话 ID 或可用正文"
            aria-label="搜索会话"
            className="min-w-0 flex-1 bg-transparent text-xs text-jolu-text outline-none placeholder:text-jolu-muted"
          />
        </label>
        <span className="flex-1" />
        <span className="text-[11px] text-jolu-muted">{minutes < 1 ? "刚刚同步" : `${minutes} 分钟前同步`}</span>
        <ToolButton icon={<RefreshCw />} busy={scanning || query.loading} disabled={scanning || query.loading} onClick={() => void rescan()}>刷新</ToolButton>
      </div>

      <div className="flex h-[34px] items-center gap-2">
        <MenuFilter label="供应商筛选" value={platform ?? ALL} options={providerOptions()} onChange={(v) => {
          const next = v === ALL ? null : (v as PlatformId)
          setPlatform(next)
          if (account && account !== UNATTRIBUTED_ACCOUNT && !accountsFor(conn.connections, next).some((c) => c.id === account)) setAccount(null)
        }} />
        <MenuFilter label="账号筛选" value={account ?? ALL} options={accountOptions} onChange={(v) => setAccount(v === ALL ? null : v)} />
        <MenuFilter
          label="项目筛选"
          value={project === null ? ALL : project || "__unknown__"}
          options={[{ value: ALL, label: "全部项目" }, ...projects.map(([key, name]) => ({ value: key || "__unknown__", label: key ? name : UNKNOWN_PROJECT }))]}
          onChange={(v) => setProject(v === ALL ? null : v === "__unknown__" ? "" : v)}
        />
        <MenuFilter label="时间筛选" value={time} options={TIME_PRESETS} onChange={(v) => setTime(v as TimePreset)} />
        <span className="flex-1" />
        <MenuFilter
          label="排序"
          value={sort}
          options={[{ value: "recent", label: "最近更新", icon: <ArrowDownWideNarrow className="size-4 text-jolu-muted" /> }, { value: "tokens", label: "消耗最多", icon: <ArrowDownWideNarrow className="size-4 text-jolu-muted" /> }]}
          onChange={(v) => setSort(v as SessionSort)}
        />
      </div>

      {query.error && <ErrorNotice error={query.error} stale={query.data !== null} onRetry={query.refresh} />}

      <div className="flex h-[630px] gap-4">
        {!query.data ? (
          !query.error && <Placeholder title="正在读取本机会话" note="刷新保留项目展开状态、列表位置及选中会话。" loading />
        ) : !all.length && !filtered ? (
          <Placeholder icon={<MessagesSquare />} title="还没有本机会话" note="先在支持的工具中开始会话，再回来重新扫描。" action={<ToolButton icon={<RefreshCw />} busy={scanning} onClick={() => void rescan()}>重新扫描</ToolButton>} />
        ) : !sessions.length ? (
          <Placeholder icon={<Search />} title="没有找到匹配的会话" note="尝试缩短关键词，或清除供应商、项目与日期筛选。" action={<ToolButton icon={<RotateCcw />} onClick={clearAll}>清除筛选</ToolButton>} />
        ) : (
          <>
            <SessionTree
              groups={groups}
              expanded={expanded}
              selected={current ? sessionKey(current) : null}
              searching={searching}
              total={sessions.length}
              onToggle={toggle}
              onSelect={setSelected}
              onClear={clearAll}
            />
            {current && (
              <SessionDetail
                key={sessionKey(current)}
                session={current}
                connections={conn.connections}
                onDrillUsage={() => onDrillUsage(current)}
                onContinue={() => setContinuing(current)}
                onCreateAutomation={() => setAutomating(current)}
              />
            )}
          </>
        )}
      </div>
      <p className="text-[10px] text-jolu-muted">顶部统计跟随当前筛选；消耗 = 输入 + 输出，缓存输入已计入输入；无法确认的账号仍标为“未归属账号”。</p>

      {continuing && (
        <ContinueDialog
          session={continuing}
          accountKnown={!!continuing.connection_id}
          onClose={() => setContinuing(null)}
          onDone={() => {
            toast.success(`已在 ${providerName(continuing.platform)} 中打开`, "后续消息由原工具处理")
            setContinuing(null)
          }}
        />
      )}
      {automating && (
        <AutomationConfirmDialog
          session={automating}
          onClose={() => setAutomating(null)}
          onConfirm={() => {
            onCreateAutomation(automating)
            setAutomating(null)
          }}
        />
      )}
    </div>
  )
}

function Stat({ icon, label, value, note, selected, bare }: { icon: React.ReactNode; label: string; value: string; note: string; selected?: boolean; bare?: boolean }) {
  return (
    <div className={cn(
      "flex min-w-0 flex-col justify-center gap-2 px-3",
      bare ? "flex-1" : "w-[136px] shrink-0 rounded-[10px] border",
      !bare && (selected ? "border-transparent bg-jolu-selected" : "border-jolu-border bg-jolu-bg"),
    )}>
      <span className={cn("flex h-3.5 items-center gap-[5px] text-[11px] [&_svg]:size-3.5", selected ? "text-jolu-on" : "text-jolu-muted")}>{icon}{label}</span>
      <span className={cn("tnum truncate text-[22px] font-semibold leading-none", selected ? "text-jolu-on" : "text-jolu-text")}>{value}</span>
      <span className="truncate text-[9px] text-jolu-muted">{note}</span>
    </div>
  )
}

function Placeholder({ icon, title, note, action, loading }: { icon?: React.ReactNode; title: string; note: string; action?: React.ReactNode; loading?: boolean }) {
  return (
    <div className="flex flex-1 flex-col items-start gap-3 rounded-xl border border-jolu-border bg-jolu-bg p-4">
      {icon && <span className="text-jolu-muted [&_svg]:size-7">{icon}</span>}
      <span className={cn("font-semibold text-jolu-text", loading ? "text-sm" : "text-base")}>{title}</span>
      {loading && [230, 285, 190].map((w) => <span key={w} className="h-6 animate-pulse rounded-[5px] bg-jolu-track" style={{ width: w }} />)}
      <span className="text-xs text-jolu-muted">{note}</span>
      {action}
    </div>
  )
}
