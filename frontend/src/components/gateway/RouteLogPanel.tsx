import { useCallback, useEffect, useRef, useState } from "react"
import { Link2 } from "lucide-react"
import { cn } from "@/lib/utils"
import { listenEvent } from "@/lib/api"
import { gatewayApi, type RouteLogDto, type RouteLogPageDto } from "@/lib/gatewayApi"
import { agentLabel } from "@/lib/gatewayPage"
import { modelPart, routeTypeTag } from "@/lib/gatewaySessions"
import { useToast } from "@/components/ui/Toast"
import { Pagination } from "@/components/ui/Pagination"
import type { Connection } from "@/types"

const PAGE_SIZE = 10
const COLUMNS = "grid grid-cols-[76px_100px_160px_minmax(0,1fr)_56px_76px] gap-3"

/**
 * 路由流水 · 每页 10 条（Jolu.pen `nCkHi`，统一网关开发说明 §5）。
 *
 * - 总数、页数与范围都来自后端；一次浏览固定查询上界，翻页期间新增请求不挤动旧页；
 * - 在第 1 页时，定时刷新与新请求会重新建立范围；在其他页时保持当前范围，不打断阅读；
 * - 目标页读取失败：保留原页内容与页码，可重试；
 * - 「耗时」是等待上游响应头的时间，不是首字耗时；结果只有成功 / 拒绝 / 失败，不画多次切换。
 */
export function RouteLogPanel({ desktop, connections, tick }: { desktop: boolean; connections: Connection[]; tick: number }) {
  const toast = useToast()
  const [data, setData] = useState<RouteLogPageDto | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const lock = useRef(false)
  const current = useRef<RouteLogPageDto | null>(null)

  const load = useCallback(async (page: number, upperId: number | null, quiet = false) => {
    if (!desktop || lock.current) return
    lock.current = true
    setBusy(true)
    try {
      const next = await gatewayApi.routeLogPage(page, PAGE_SIZE, upperId)
      current.current = next
      setData(next)
      setError(null)
    } catch (reason) {
      if (!current.current) setError(String(reason))
      else if (!quiet) toast.danger("读取路由流水失败", `${String(reason)}；仍显示第 ${current.current.page} 页，可稍后重试。`)
    } finally {
      lock.current = false
      setBusy(false)
    }
  }, [desktop, toast])

  // 进入页面与定时刷新：只在第 1 页时重新建立查询范围
  useEffect(() => {
    const shown = current.current
    if (!shown || shown.page === 1) void load(1, null, shown !== null)
  }, [tick, load])

  // 新请求结束后：正在看第 1 页时刷新（合并 1.5 秒内的多次结束）
  useEffect(() => {
    if (!desktop) return
    let stopped = false
    let timer: number | null = null
    let off: (() => void) | null = null
    void listenEvent("gateway-route", () => {
      if (timer) window.clearTimeout(timer)
      timer = window.setTimeout(() => {
        if (!current.current || current.current.page === 1) void load(1, null, true)
      }, 1500)
    }).then((unlisten) => { if (stopped) unlisten(); else off = unlisten }).catch(() => {})
    return () => { stopped = true; off?.(); if (timer) window.clearTimeout(timer) }
  }, [desktop, load])

  const rows = data?.rows ?? []
  const from = data && data.total > 0 ? (data.page - 1) * data.page_size + 1 : 0
  const to = data ? from + rows.length - 1 : 0
  const byId = new Map(connections.map((c) => [c.id, c]))
  const bySlug = new Map(connections.filter((c) => c.slug).map((c) => [c.slug as string, c]))

  return (
    <section aria-label="路由流水" className="flex shrink-0 flex-col overflow-hidden rounded-xl border border-jolu-border bg-jolu-cc-surface">
      <header className="flex h-[50px] items-center gap-2 px-4">
        <Link2 aria-hidden className="size-3.5 text-jolu-on" />
        <span className="text-[12.5px] text-jolu-text">路由流水</span>
        <span className="text-[10px] text-jolu-muted" title="只保留最近约 1,000 条；统计只代表当前保留范围">
          {!desktop ? "桌面端可用" : data && data.total > 0 ? `第 ${from}–${to} 条 · 共 ${data.total} 条` : data ? "暂无记录" : error ? "读取失败" : "读取中…"}
        </span>
      </header>
      {error && !data ? (
        <div className="flex items-center justify-center gap-3 border-t border-jolu-border px-4 py-8 text-[10.5px] text-jolu-danger">
          读取路由流水失败：{error}
          <button type="button" disabled={busy} onClick={() => void load(1, null)} className="rounded-md border border-jolu-border bg-jolu-bg px-2.5 py-1 text-[10.5px] text-jolu-text outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">重试</button>
        </div>
      ) : rows.length === 0 ? (
        <div className="border-t border-jolu-border px-4 py-8 text-center text-[10.5px] text-jolu-muted">
          {desktop ? "网关收到请求后，这里会显示时间、Agent、请求模型、路由到的来源、耗时和结果。" : "浏览器预览不读取路由流水；请在桌面端查看真实数据。"}
        </div>
      ) : (
        <div className={cn("flex flex-col transition-opacity", busy && "opacity-70")}>
          <div className={cn(COLUMNS, "h-[30px] items-center bg-jolu-side px-4 text-[9px] text-jolu-muted")}>
            <span>时间</span><span>Agent</span><span>请求模型</span><span>路由到</span><span title="等待上游响应头的耗时；流式请求不是首字耗时">耗时</span><span>结果</span>
          </div>
          {rows.map((row, index) => (
            <RouteRow key={row.id ?? `${row.ts}-${index}`} row={row} connection={(row.connection_id && byId.get(row.connection_id)) || (row.source ? bySlug.get(row.source) : undefined)} />
          ))}
        </div>
      )}
      {desktop && data && (
        <div className="border-t border-jolu-table-divider">
          <Pagination page={data.page} pageCount={data.page_count} pageSize={PAGE_SIZE} busy={busy}
            summary={data.page_count > 0 ? undefined : "暂无可翻页的记录"}
            onPage={(page) => void load(page, data.upper_id)} />
        </div>
      )}
    </section>
  )
}

function duration(ms: number | null): string {
  if (ms === null) return "—"
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`
}

function RouteRow({ row, connection }: { row: RouteLogDto; connection: Connection | undefined }) {
  const model = row.upstream_model ?? (row.source ? modelPart(row.requested_model, row.source) : null)
  const name = connection?.baseName ?? row.source
  return (
    <div className={cn(COLUMNS, "h-9 items-center border-t border-jolu-table-divider px-4")}>
      <span className="tabular-nums text-[10px] text-jolu-muted">{new Date(row.ts).toLocaleTimeString("zh-CN", { hour12: false })}</span>
      <span className="truncate text-[10.5px] text-jolu-text">{row.agent ? agentLabel(row.agent) : "—"}</span>
      <span className="truncate text-[10px] text-jolu-muted" title={row.requested_model}>{row.requested_model}</span>
      <span className="flex min-w-0 items-center gap-1.5">
        <span className="truncate text-[10.5px] text-jolu-text">{name ? `${name}${model ? ` · ${model}` : ""}` : "—"}</span>
        {connection && <span className="shrink-0 rounded bg-jolu-track px-[5px] text-[8.5px] leading-3 text-jolu-muted">{routeTypeTag(connection.kind, connection.platformId)}</span>}
      </span>
      <span className="tabular-nums text-[10px] text-jolu-muted">{duration(row.duration_ms)}</span>
      <span className={cn("text-[9.5px]", row.result === "ok" ? "text-jolu-on" : row.result === "rejected" ? "text-jolu-muted" : "text-jolu-danger")}>
        {row.result === "ok" ? "成功" : row.result === "rejected" ? "拒绝" : "失败"}
      </span>
    </div>
  )
}
