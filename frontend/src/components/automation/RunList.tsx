import { ChevronLeft, ChevronRight, Info, RefreshCw } from "lucide-react"
import { cn } from "@/lib/utils"
import { RUN_RANGES, RUN_STATUSES, clock, hhmmss, runCode, runStatusLabel, runTokens, runningMs, shortTool, sourceLabel, waitingMs } from "@/lib/automation"
import type { RunFilterDto, RunPageDto, RunRange, RunStatus } from "@/lib/automationApi"
import { formatTokens, localZone } from "@/lib/usageStats"
import { Card, MenuFilter, MetricCard } from "@/components/usage/usageControls"
import { JoluButton } from "@/components/ui/JoluButton"
import { Notice, TONE } from "./automationParts"
import type { AccountRef } from "./TaskList"

const ALL = "__all__"

/** 运行记录页签（画板 64）：概览、筛选、记录表与分页 */
export function RunList({ data, filter, page, now, loading, accounts, onFilter, onPage, onRefresh, onOpen }: {
  data: RunPageDto
  filter: RunFilterDto
  page: number
  now: number
  loading: boolean
  accounts: Map<string, AccountRef>
  onFilter: (next: RunFilterDto) => void
  onPage: (page: number) => void
  onRefresh: () => void
  onOpen: (runId: number) => void
}) {
  const today = filter.range === "today"
  const rangeLabel = RUN_RANGES.find((r) => r.value === filter.range)?.label ?? "今天"
  const accountIds = [...new Set(data.rows.map((r) => r.connection_id).concat(filter.connection ? [filter.connection] : []))]
  return (
    <>
      <div className="grid grid-cols-4 gap-3">
        <MetricCard label={today ? "今日完成" : "完成"} value={String(data.completed)} note={`其中 ${data.completed_waited} 次经历额度等待`} />
        <MetricCard label="执行中" value={String(data.running)} note={data.running ? "可查看步骤并停止" : "当前没有执行中的运行"} />
        <MetricCard label="失败" value={String(data.failed)} note={data.failed_note ?? "没有失败的运行"} />
        <MetricCard label={today ? "今日消耗 Token" : "消耗 Token"} value={formatTokens(data.tokens).replace(" ", "")} note="不含等待期间；以执行器返回为准" />
      </div>

      <div className="flex h-[34px] items-center gap-1.5">
        <MenuFilter label="时间范围" value={filter.range} onChange={(v) => onFilter({ ...filter, range: v as RunRange })} options={RUN_RANGES.map((r) => ({ value: r.value, label: r.label }))} />
        <MenuFilter label="状态筛选" value={filter.status ?? ALL} onChange={(v) => onFilter({ ...filter, status: v === ALL ? null : (v as RunStatus) })} options={[{ value: ALL, label: "全部状态" }, ...RUN_STATUSES]} />
        <MenuFilter
          label="账号筛选"
          value={filter.connection ?? ALL}
          onChange={(v) => onFilter({ ...filter, connection: v === ALL ? null : v })}
          options={[{ value: ALL, label: "全部账号" }, ...accountIds.map((id) => ({ value: id, label: accounts.get(id)?.name ?? "未知账号" }))]}
        />
        <MenuFilter
          label="任务筛选"
          value={filter.task === null ? ALL : String(filter.task)}
          onChange={(v) => onFilter({ ...filter, task: v === ALL ? null : Number(v) })}
          options={[{ value: ALL, label: "全部任务" }, ...data.tasks.map((t) => ({ value: String(t.task_id), label: t.name }))]}
        />
        <span className="flex-1" />
        <button type="button" aria-label="刷新运行记录" disabled={loading} onClick={onRefresh} className="grid size-[30px] place-items-center rounded-[7px] bg-jolu-side text-jolu-muted outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
          <RefreshCw aria-hidden className={cn("size-4", loading && "animate-spin")} />
        </button>
      </div>

      <Card className="overflow-hidden">
        <div className="flex h-11 items-center bg-jolu-side px-4 text-[11px] font-medium text-jolu-muted">
          <span className="w-[240px] shrink-0">任务 / 运行编号</span>
          <span className="w-[184px] shrink-0">账号 / 启动方式</span>
          <span className="min-w-0 flex-1">开始时间</span>
          <span className="w-[140px] shrink-0">执行 / 等待</span>
          <span className="w-[90px] shrink-0">Token</span>
          <span className="w-[172px] shrink-0">状态 / 操作</span>
        </div>
        {data.rows.length ? data.rows.map((run) => {
          const status = runStatusLabel(run.status)
          const account = accounts.get(run.connection_id)?.name ?? run.account ?? "未知账号"
          return (
            <div key={run.id} className="flex h-[72px] items-center border-t border-jolu-border px-4 transition-colors hover:bg-jolu-stage">
              <div className="flex w-[240px] shrink-0 flex-col gap-1.5 pr-3">
                <span className="truncate text-xs font-medium text-jolu-text">{run.task_name}</span>
                <span className="text-[10px] text-jolu-muted">{runCode(run.id)}</span>
              </div>
              <div className="flex w-[184px] shrink-0 flex-col gap-1.5 pr-3">
                <span className="truncate text-[11px] text-jolu-text">{shortTool(run.platform)} · {account}</span>
                <span className="text-[10px] text-jolu-muted">{sourceLabel(run.source)}</span>
              </div>
              <span className="tnum min-w-0 flex-1 text-[11px] text-jolu-text">{hhmmss(run.started_at ?? run.created_at)}</span>
              <span className="tnum w-[140px] shrink-0 text-[11px] text-jolu-text">{clock(runningMs(run, now))} / {clock(waitingMs(run, now))}</span>
              <span className="tnum w-[90px] shrink-0 text-[11px] text-jolu-text">{formatTokens(runTokens(run)).replace(" ", "")}</span>
              <div className="flex w-[172px] shrink-0 items-center gap-2.5">
                <span className={cn("min-w-[52px] text-[11px]", TONE[status.tone])}>{status.label}</span>
                <JoluButton size="sm" variant="soft" className="w-20" onClick={() => onOpen(run.id)}>详情<ChevronRight aria-hidden /></JoluButton>
              </div>
            </div>
          )
        }) : (
          <div className="border-t border-jolu-border px-4 py-5 text-[11px] text-jolu-muted">{rangeLabel}没有符合条件的运行记录</div>
        )}
        <div className="flex h-11 items-center gap-2.5 border-t border-jolu-border px-4 text-[11px] text-jolu-muted">
          <span>{rangeLabel}共 {data.total} 次运行 · 当前显示 {data.rows.length} 条</span>
          <span className="flex-1" />
          <JoluButton size="sm" variant="neutral" disabled={page <= 1} onClick={() => onPage(page - 1)}><ChevronLeft aria-hidden />上一页</JoluButton>
          <span className="tnum text-jolu-text">{data.page} / {data.pages}</span>
          <JoluButton size="sm" variant="neutral" disabled={page >= data.pages} onClick={() => onPage(page + 1)}>下一页<ChevronRight aria-hidden /></JoluButton>
        </div>
      </Card>

      <Notice icon={Info} title="执行耗时与等待时长分开记录">额度不足期间不计入执行耗时；同一次运行可多次等待和恢复，重试会生成新的运行记录。</Notice>
      <p className="text-[10px] text-jolu-muted">时间按 {localZone()} 展示；Token 缺失显示 —，不以 0 代替。</p>
    </>
  )
}
