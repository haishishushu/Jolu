import { useState } from "react"
import { ChevronLeft, ChevronRight } from "lucide-react"
import { cn } from "@/lib/utils"
import { parsePageInput } from "@/lib/paginationState"
import { DASH, callStatus, effortLabel, formatSeconds, formatTokens, formatUsd, modelDisplayName, outputTps, pageWindow } from "@/lib/usageStats"
import type { StatsCallPageDto, StatsCallRowDto } from "@/lib/api"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import type { PlatformId } from "@/types"
import { Card } from "./usageControls"

/** 画板 34 表头列宽 */
const COLUMNS = [
  ["时间 ↓", "w-[76px]"], ["模型", "w-[196px]"], ["输入", "w-[72px]"], ["缓存", "w-[72px]"], ["输出", "w-[72px]"],
  ["总 Token", "w-[80px]"], ["总成本", "w-[84px]"], ["延迟", "w-[124px]"], ["思考强度", "w-[72px]"], ["状态", "w-[74px]"],
] as const

const sumKnown = (a: number | null, b: number | null) => (a === null && b === null ? null : (a ?? 0) + (b ?? 0))

function time(ts: number): string {
  const d = new Date(ts)
  return [d.getHours(), d.getMinutes(), d.getSeconds()].map((n) => String(n).padStart(2, "0")).join(":")
}

function monthDay(ts: number): string {
  const d = new Date(ts)
  return `${String(d.getMonth() + 1).padStart(2, "0")}/${String(d.getDate()).padStart(2, "0")}`
}

/** 调用明细表格与选中调用的详情（画板 34）。「—」表示未取得或未计价，失败请求不默认免费 */
export function CallsPanel({ data, selectedId, onSelect, onPage, onOpenSession }: {
  data: StatsCallPageDto
  selectedId: number | null
  onSelect: (id: number) => void
  onPage: (page: number) => void
  onOpenSession: (target: { platform: string; sessionId: string }) => void
}) {
  const selected = data.rows.find((r) => r.id === selectedId) ?? null
  return (
    <>
      <Card className="flex flex-col overflow-hidden">
        <div className="flex h-[42px] items-center justify-between px-4">
          <span className="text-[13px] font-semibold text-jolu-text">{data.total_count} 条调用记录</span>
          <span className="text-[11px] text-jolu-muted">总成本为估算 · USD · {data.unpriced_count} 条未计价</span>
        </div>
        <div className="flex h-9 items-center gap-1.5 border-y border-jolu-border bg-jolu-side px-4">
          {COLUMNS.map(([label, width]) => <span key={label} className={cn("shrink-0 text-[11px] font-medium text-jolu-muted", width)}>{label}</span>)}
        </div>
        {data.rows.map((row) => <CallRow key={row.id} row={row} active={row.id === selectedId} onSelect={() => onSelect(row.id)} />)}
        {!data.rows.length && <div className="px-4 py-8 text-center text-xs text-jolu-muted">没有符合筛选条件的调用</div>}
        <Pagination page={data.page} pageCount={data.page_count} pageSize={data.page_size} onPage={onPage} />
      </Card>
      {selected && <CallDetail row={selected} onOpenSession={onOpenSession} />}
      <p className="text-[10px] text-jolu-muted">— 表示未取得或未计价，失败请求不默认免费。导出遵循当前筛选，不包含 API Key 或对话正文。</p>
    </>
  )
}

function Num({ value, className }: { value: number | null; className?: string }) {
  return <span className={cn("tnum text-[11px]", value === null ? "text-jolu-muted" : "text-jolu-text", className)}>{formatTokens(value)}</span>
}

function CallRow({ row, active, onSelect }: { row: StatsCallRowDto; active: boolean; onSelect: () => void }) {
  const status = callStatus(row)
  const tps = outputTps(row)
  const metric = (label: string, value: string, known: boolean) => (
    <span className="flex h-3.5 items-center gap-1.5">
      <span className="text-[10px] text-jolu-muted">{label}</span>
      <span className={cn("tnum text-[10.5px] font-medium", known ? "text-jolu-on" : "text-jolu-muted")}>{value}</span>
    </span>
  )
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onSelect}
      className={cn(
        "flex h-[54px] w-full items-center gap-1.5 border-b border-jolu-table-divider px-4 text-left outline-none focus-visible:bg-jolu-table-active",
        active ? "bg-jolu-table-active" : "bg-jolu-bg hover:bg-jolu-side",
      )}
    >
      <span className="tnum w-[76px] shrink-0 text-[10px] text-jolu-muted">{time(row.ts)}</span>
      <span className="flex w-[196px] shrink-0 items-center gap-2" title={row.model ?? undefined}>
        <span className="grid size-5 place-items-center"><PlatformLogo platform={row.platform as PlatformId} size={16} /></span>
        <span className="truncate text-xs font-medium text-jolu-text">{modelDisplayName(row.model)}</span>
      </span>
      <Num value={row.fresh_input} className="w-[72px] shrink-0" />
      <Num value={sumKnown(row.cache_read, row.cache_write)} className="w-[72px] shrink-0" />
      <Num value={row.output} className="w-[72px] shrink-0" />
      <Num value={row.total} className="w-[80px] shrink-0 font-semibold" />
      <span className={cn("tnum w-[84px] shrink-0 text-xs", row.cost === null ? "text-jolu-muted" : "font-semibold text-jolu-on")}>{formatUsd(row.cost)}</span>
      <span className="flex w-[124px] shrink-0 items-center gap-[7px]">
        <span className="h-11 w-[3px] rounded-sm bg-jolu-accent" />
        <span className="flex flex-col gap-px">
          {metric("首字", formatSeconds(row.first_token_ms), row.first_token_ms !== null)}
          {metric("总耗时", formatSeconds(row.duration_ms), row.duration_ms !== null)}
          {metric("输出 TPS", tps === null ? DASH : `${tps.toFixed(1)} tok/s`, false)}
        </span>
      </span>
      <span className={cn("w-[72px] shrink-0 text-[11px]", row.effort ? "text-jolu-text" : "text-jolu-muted")}>{effortLabel(row.effort)}</span>
      <span className="w-[74px] shrink-0">
        <span className={cn("inline-flex h-6 items-center rounded-full px-3 text-[11px] font-medium", status.ok ? "bg-jolu-status-success-bg text-jolu-on" : "bg-jolu-status-error-bg text-jolu-danger")}>
          {status.label}
        </span>
      </span>
    </button>
  )
}

function Pagination({ page, pageCount, pageSize, onPage }: { page: number; pageCount: number; pageSize: number; onPage: (page: number) => void }) {
  const [draft, setDraft] = useState("")
  const [error, setError] = useState<string | null>(null)
  const box = "grid size-8 place-items-center rounded-lg border text-xs outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-45"
  const jump = () => {
    const parsed = parsePageInput(draft, pageCount)
    if (parsed.error !== null) return setError(parsed.error)
    setError(null)
    setDraft("")
    onPage(parsed.page)
  }
  return (
    <div className="flex h-[60px] items-center gap-2 px-4">
      {/* 页码输入有误时，这里临时换成错误提示 */}
      <span role={error ? "alert" : undefined} className={cn("text-[11px]", error ? "text-jolu-danger" : "text-jolu-muted")}>{error ?? `每页 ${pageSize} 条 · 共 ${pageCount} 页`}</span>
      <span className="flex-1" />
      <button type="button" aria-label="上一页" disabled={page <= 1} onClick={() => onPage(page - 1)} className={cn(box, "border-jolu-border bg-jolu-bg text-jolu-muted hover:bg-jolu-stage")}><ChevronLeft aria-hidden className="size-3.5" /></button>
      {pageWindow(page, pageCount).map((p, index) => p === "…"
        ? <span key={`gap-${index}`} className="grid h-8 w-6 place-items-center text-xs text-jolu-muted">…</span>
        : (
          <button key={p} type="button" aria-current={p === page ? "page" : undefined} onClick={() => onPage(p)}
            className={cn(box, p === page ? "border-jolu-accent bg-jolu-accent font-semibold text-[#071B10]" : "border-jolu-border bg-jolu-bg font-medium text-jolu-text hover:bg-jolu-stage")}>
            {p}
          </button>
        ))}
      <button type="button" aria-label="下一页" disabled={page >= pageCount} onClick={() => onPage(page + 1)} className={cn(box, "border-jolu-border bg-jolu-bg text-jolu-muted hover:bg-jolu-stage")}><ChevronRight aria-hidden className="size-3.5" /></button>
      <input
          value={draft}
          inputMode="numeric"
          placeholder="页码"
          aria-label="跳转页码"
          aria-invalid={!!error}
          onChange={(event) => { setDraft(event.target.value); setError(null) }}
          onKeyDown={(event) => event.key === "Enter" && jump()}
          className={cn("h-8 w-[76px] rounded-lg border bg-jolu-bg px-3 text-xs text-jolu-text outline-none placeholder:text-jolu-muted focus-visible:border-jolu-accent", error ? "border-jolu-danger" : "border-jolu-border")}
        />
      <button type="button" onClick={jump} className="h-8 w-[54px] rounded-lg border border-jolu-border bg-jolu-bg text-xs font-medium text-jolu-text outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent">跳转</button>
    </div>
  )
}

function CallDetail({ row, onOpenSession }: { row: StatsCallRowDto; onOpenSession: (target: { platform: string; sessionId: string }) => void }) {
  const items: [string, number | null][] = [
    ["普通输入", row.fresh_input], ["输出", row.output], ["缓存读取", row.cache_read], ["缓存写入", row.cache_write], ["合计", row.total],
  ]
  return (
    <Card className="flex flex-col gap-2 p-3">
      <div className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold text-jolu-text">调用详情 · {time(row.ts)}</h3>
        <span className="truncate text-[10px] text-jolu-muted" title={row.call_id}>{monthDay(row.ts)} · {row.call_id}</span>
      </div>
      <div className="flex gap-6">
        {items.map(([label, value]) => (
          <div key={label} className="flex min-w-0 flex-1 flex-col gap-[5px]">
            <span className="text-[10px] text-jolu-muted">{label}</span>
            <span className={cn("tnum text-lg font-semibold", value === null ? "text-jolu-muted" : "text-jolu-text")}>{formatTokens(value)}</span>
          </div>
        ))}
      </div>
      <div className="flex items-center gap-4">
        <span className="min-w-0 truncate text-[11px] text-jolu-muted">
          来源：本机日志 · {row.cost === null ? "未计价" : `估算 ${formatUsd(row.cost)}（按内置价目表）`}
        </span>
        <span className="flex-1" />
        <button
          type="button"
          disabled={!row.session_id}
          onClick={() => row.session_id && onOpenSession({ platform: row.platform, sessionId: row.session_id })}
          className="inline-flex h-[34px] shrink-0 items-center gap-1.5 rounded-lg bg-jolu-selected px-3 text-xs font-medium text-jolu-on outline-none hover:bg-jolu-pressed focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-45"
        >
          打开会话
          <ChevronRight aria-hidden className="size-3.5" />
        </button>
      </div>
    </Card>
  )
}
