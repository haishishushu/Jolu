import { useEffect, useState } from "react"
import { CalendarDays, Download, Search } from "lucide-react"
import { cn } from "@/lib/utils"
import { api, isTauri, type StatsCallPageDto, type StatsFilterDto, type StatsModelRowDto, type StatsTrendBucketDto } from "@/lib/api"
import { dateKey, localZone, rangeLabel, rangeToMillis, todayRange, utcOffsetLabel, zoneName, type DateRange } from "@/lib/usageStats"
import { accountsFor, keepApplicable } from "@/lib/usageFilters"
import { useConnections } from "@/hooks/useConnections"
import { useStatsQuery } from "@/hooks/useStatsQuery"
import { demoActivity, demoCalls, demoOverview } from "@/mock/usageStats"
import { useToast } from "@/components/ui/Toast"
import { AnalysisPanel } from "@/components/usage/AnalysisPanel"
import { CallsPanel } from "@/components/usage/CallsPanel"
import { DateRangeDialog } from "@/components/usage/DateRangeDialog"
import { OverviewPanel } from "@/components/usage/OverviewPanel"
import {
  ALL, AccountFilter, EmptyCard, ErrorNotice, FilterTrigger, LoadingCard, MenuFilter, RefreshButton, ResetButton, ToolButton, providerOptions,
} from "@/components/usage/usageControls"
import type { PlatformId } from "@/types"

export type UsageTab = "overview" | "analysis" | "calls"

const TABS: { value: UsageTab; label: string }[] = [
  { value: "overview", label: "概览" },
  { value: "analysis", label: "用量分析" },
  { value: "calls", label: "调用明细" },
]

type CallStatus = "success" | "error" | null

/**
 * 用量统计（Jolu.pen 模块 C，画板 30–39）：概览、用量分析、调用明细三个页签。
 * 日期范围与供应商在三个页签间共用；账号筛选只作用于概览与分析（调用明细画板没有账号筛选）。
 */
/** 外部要求打开调用明细：会话页「查看用量明细」带入会话 ID 与日期范围 */
export interface UsageRequest {
  search: string
  range: DateRange
  nonce: number
}

export function UsageView({ active, request, onOpenSession }: {
  active: boolean
  request: UsageRequest | null
  /** 不带参数时只打开会话线程页 */
  onOpenSession: (target?: { platform: string; sessionId: string }) => void
}) {
  const toast = useToast()
  const conn = useConnections()
  const [tab, setTab] = useState<UsageTab>("overview")
  const [range, setRange] = useState<DateRange>(() => todayRange())
  const [platform, setPlatform] = useState<PlatformId | null>(null)
  const [accounts, setAccounts] = useState<string[]>([])
  const [model, setModel] = useState<string | null>(null)
  const [status, setStatus] = useState<CallStatus>(null)
  const [searchDraft, setSearchDraft] = useState("")
  const [search, setSearch] = useState("")
  const [page, setPage] = useState(1)
  const [selected, setSelected] = useState<number | null>(null)
  const [picking, setPicking] = useState(false)
  const [exporting, setExporting] = useState(false)

  useEffect(() => {
    if (!request) return
    setTab("calls")
    setRange(request.range)
    setPlatform(null)
    setModel(null)
    setStatus(null)
    setSearch(request.search)
    setSearchDraft(request.search)
    setPage(1)
  }, [request])

  const statsFilter = (): StatsFilterDto => ({ ...rangeToMillis(range), platform, connections: accounts })
  const callsFilter = (): StatsFilterDto => ({ ...rangeToMillis(range), platform, connections: [], model, status, search: search.trim() || null })

  const overview = useStatsQuery(
    JSON.stringify([range, platform, accounts]),
    () => api.statsOverview(statsFilter()),
    () => demoOverview(rangeToMillis(range).start),
    platform,
    active && tab !== "calls",
  )
  const activity = useStatsQuery(
    JSON.stringify([platform, accounts]),
    () => api.statsActivity(platform, accounts, Date.now()),
    () => demoActivity(new Date()),
    platform,
    active && tab === "analysis",
  )
  const calls = useStatsQuery<StatsCallPageDto>(
    JSON.stringify([range, platform, model, status, search, page]),
    () => api.statsCalls(callsFilter(), page),
    () => demoCalls(Date.now(), page),
    platform,
    active && tab === "calls",
  )

  const changePlatform = (next: PlatformId | null) => {
    setPlatform(next)
    setAccounts((value) => keepApplicable(value, conn.connections, next))
    setModel(null)
    setPage(1)
  }
  const changeRange = (next: DateRange) => {
    setRange(next)
    setPage(1)
  }
  const resetCalls = () => {
    setRange(todayRange())
    setPlatform(null)
    setModel(null)
    setStatus(null)
    setSearch("")
    setSearchDraft("")
    setPage(1)
  }
  const clearFilters = () => {
    setPlatform(null)
    setAccounts([])
  }

  /** 下钻到调用明细：图表数据点按所在日期，模型 / 供应商按名称（画板 38「交互」） */
  const drillBucket = (point: StatsTrendBucketDto) => {
    changeRange({ start: dateKey(new Date(point.start)), end: dateKey(new Date(point.end - 1)) })
    setTab("calls")
  }
  const drillModel = (row: StatsModelRowDto) => {
    setPlatform(row.platform as PlatformId)
    setModel(row.model)
    setPage(1)
    setTab("calls")
  }
  const drillProvider = (id: string) => {
    changePlatform(id as PlatformId)
    setTab("calls")
  }

  const exportCsv = async () => {
    if (!isTauri) return toast.info("浏览器预览不支持导出", "请在桌面应用中导出 CSV")
    setExporting(true)
    try {
      const result = await api.exportCallsCsv(callsFilter())
      if (result) toast.success(`导出完成 · ${result.count} 条`, result.path)
    } catch (error) {
      toast.danger("导出失败", error instanceof Error ? error.message : String(error))
    } finally {
      setExporting(false)
    }
  }

  const isToday = range.start === range.end && range.start === dateKey(new Date())
  const zone = zoneName(localZone())
  const current = tab === "calls" ? calls : overview
  const callRows = calls.data?.rows ?? []
  const selectedId = callRows.some((row) => row.id === selected) ? selected : callRows[0]?.id ?? null

  const dateButton = (calendar: boolean) => (
    <FilterTrigger
      aria-label="日期范围"
      onClick={() => setPicking(true)}
      leading={calendar ? <CalendarDays aria-hidden className="size-4 text-jolu-muted" /> : undefined}
      trailingChevron={calendar}
    >
      {rangeLabel(range)}
    </FilterTrigger>
  )

  return (
    <div className="flex w-full flex-col gap-3">
      <div role="tablist" aria-label="用量统计" className="flex h-[38px] gap-6 border-b border-jolu-border">
        {TABS.map((t) => (
          <button
            key={t.value}
            type="button"
            role="tab"
            aria-selected={tab === t.value}
            onClick={() => setTab(t.value)}
            className={cn(
              "-mb-px border-b-2 px-0.5 text-sm outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
              tab === t.value ? "border-jolu-accent font-semibold text-jolu-text" : "border-transparent text-jolu-muted hover:text-jolu-text",
            )}
          >
            {t.label}
          </button>
        ))}
      </div>

      {tab !== "calls" ? (
        <div className="flex h-[34px] items-center gap-2">
          {dateButton(false)}
          <MenuFilter label="供应商筛选" value={platform ?? ALL} options={providerOptions()} onChange={(v) => changePlatform(v === ALL ? null : (v as PlatformId))} />
          <AccountFilter accounts={accountsFor(conn.connections, platform)} value={accounts} onChange={setAccounts} />
          <span className="flex-1" />
          <span className="text-[11px] text-jolu-muted">{range.end.slice(5).replace("-", "/")} · {zone}时区</span>
          <RefreshButton loading={overview.loading} onClick={() => { overview.refresh(); if (tab === "analysis") activity.refresh() }} />
        </div>
      ) : (
        <>
          <div className="flex h-[34px] items-center gap-2">
            {dateButton(true)}
            <MenuFilter label="供应商筛选" trailingChevron value={platform ?? ALL} options={providerOptions()} onChange={(v) => changePlatform(v === ALL ? null : (v as PlatformId))} />
            <MenuFilter
              label="模型筛选"
              trailingChevron
              value={model ?? ALL}
              options={[{ value: ALL, label: "全部模型" }, ...(calls.data?.models ?? []).map((m) => ({ value: m, label: m }))]}
              onChange={(v) => { setModel(v === ALL ? null : v); setPage(1) }}
            />
            <MenuFilter
              label="状态筛选"
              trailingChevron
              value={status ?? ALL}
              options={[{ value: ALL, label: "全部状态" }, { value: "success", label: "成功" }, { value: "error", label: "失败" }]}
              onChange={(v) => { setStatus(v === ALL ? null : (v as CallStatus)); setPage(1) }}
            />
            <span className="flex-1" />
            <ResetButton onClick={resetCalls} />
            <RefreshButton loading={calls.loading} onClick={calls.refresh} />
          </div>
          <div className="flex h-[34px] items-center gap-2">
            <label className="flex h-[34px] w-[360px] items-center gap-2 rounded-lg border border-jolu-border bg-jolu-bg px-3 focus-within:border-jolu-accent">
              <Search aria-hidden className="size-4 shrink-0 text-jolu-muted" />
              <input
                value={searchDraft}
                onChange={(event) => setSearchDraft(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key !== "Enter") return
                  setSearch(searchDraft)
                  setPage(1)
                }}
                placeholder="搜索调用 ID / 会话 ID，按 Enter 查询"
                aria-label="搜索调用 ID 或会话 ID"
                className="min-w-0 flex-1 bg-transparent text-xs text-jolu-text outline-none placeholder:text-jolu-muted"
              />
            </label>
            <span className="flex-1" />
            <span className="text-[11px] text-jolu-muted">统计时区 · {zone} {utcOffsetLabel()}</span>
            <ToolButton icon={<Download />} busy={exporting} disabled={exporting || !calls.data?.total_count} onClick={() => void exportCsv()}>
              {exporting ? `正在导出 ${calls.data?.total_count ?? 0} 条…` : "导出 CSV"}
            </ToolButton>
          </div>
        </>
      )}

      {current.error && <ErrorNotice error={current.error} stale={current.data !== null} onRetry={current.refresh} />}

      {tab === "calls" ? (
        calls.data ? (
          <CallsPanel data={calls.data} selectedId={selectedId} onSelect={setSelected} onPage={setPage} onOpenSession={onOpenSession} />
        ) : !calls.error && <LoadingCard />
      ) : overview.data ? (
        overview.data.requests === 0 ? (
          <EmptyCard onClear={platform || accounts.length ? clearFilters : undefined} />
        ) : tab === "overview" ? (
          <OverviewPanel data={overview.data} isToday={isToday} onDrillBucket={drillBucket} onDrillModel={drillModel} onOpenSession={onOpenSession} />
        ) : (
          <AnalysisPanel data={overview.data} activity={activity.data} onDrillModel={drillModel} onDrillProvider={drillProvider} />
        )
      ) : !overview.error && <LoadingCard />}

      {picking && (
        <DateRangeDialog
          value={range}
          onClose={() => setPicking(false)}
          onConfirm={(next) => {
            changeRange(next)
            setPicking(false)
          }}
        />
      )}
    </div>
  )
}
