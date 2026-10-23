import { useState } from "react"
import { ChevronRight, CircleDollarSign, Layers, Repeat2, Target } from "lucide-react"
import { cn } from "@/lib/utils"
import { bucketLabel, changeText, formatPercent, formatTokens, formatUsd, modelDisplayName, share } from "@/lib/usageStats"
import { projectName } from "@/lib/sessionThreads"
import type { StatsModelRowDto, StatsOverviewDto, StatsTrendBucketDto } from "@/lib/api"
import { Card, CardTitle, MetricCard, providerName } from "./usageControls"
import { TrendChart, TrendLegend, type TrendMode } from "./TrendChart"

const MODES: { value: TrendMode; label: string }[] = [
  { value: "tokens", label: "Token" },
  { value: "requests", label: "请求" },
  { value: "cost", label: "费用" },
]

/** 概览（画板 30）：四项指标、趋势与模型分布、供应商用量与高消耗会话 */
export function OverviewPanel({ data, isToday, onDrillBucket, onDrillModel, onOpenSession }: {
  data: StatsOverviewDto
  isToday: boolean
  onDrillBucket: (point: StatsTrendBucketDto) => void
  onDrillModel: (model: StatsModelRowDto) => void
  onOpenSession: (target?: { platform: string; sessionId: string }) => void
}) {
  const [mode, setMode] = useState<TrendMode>("tokens")
  const change = changeText(data.tokens.total, data.previous_total)
  const success = data.requests ? (data.requests - data.failed) / data.requests : null
  /** 画板 30：模型分布显示前 5 个，不足时有几个显示几个，不补「其他」行 */
  // 真实为 0 Token 的行（如 Claude Code 的 <synthetic> 报错消息）对 Token 分布没有意义；未知值保留
  const topModels = data.models.filter((m) => m.tokens !== 0).slice(0, 5)
  const unattributed = data.requests - data.attributed

  return (
    <>
      <div className="grid grid-cols-4 gap-3">
        <MetricCard
          icon={<Layers />}
          label="总 Token"
          value={formatTokens(data.tokens.total)}
          note={`本机记录${change ? ` · ${isToday ? "较昨天同时段" : "较上一周期"} ${change}` : ""}`}
        />
        <MetricCard icon={<Repeat2 />} label="调用次数" value={String(data.requests)} note={`成功率 ${formatPercent(success)} · ${data.failed} 次失败`} />
        <MetricCard
          icon={<CircleDollarSign />}
          label="估算费用 · USD"
          value={formatUsd(data.cost.priced_requests ? data.cost.amount : null)}
          note={`已计价 ${data.cost.priced_requests} / ${data.cost.requests} 次 · 不含月费`}
        />
        <MetricCard icon={<Target />} label="输入缓存命中率" value={formatPercent(data.cache_hit_rate)} note="缓存读取 / 全部输入 Token" />
      </div>

      <div className="grid h-[250px] grid-cols-[minmax(0,644fr)_minmax(0,348fr)] gap-4">
        <Card className="flex flex-col gap-2.5 p-4">
          <div className="flex h-7 items-center justify-between">
            <div className="flex items-baseline gap-2">
              <h3 className="text-sm font-semibold text-jolu-text">{isToday ? "今日用量" : "用量趋势"}</h3>
              <span className="text-[10px] text-jolu-muted">{bucketLabel(data.trend.bucket)}</span>
            </div>
            <div role="radiogroup" aria-label="趋势指标" className="flex h-7 gap-0.5 rounded-[7px] bg-jolu-side p-[3px]">
              {MODES.map((m) => (
                <button
                  key={m.value}
                  type="button"
                  role="radio"
                  aria-checked={mode === m.value}
                  onClick={() => setMode(m.value)}
                  className={cn(
                    "h-[22px] rounded-[5px] px-2.5 text-[10px] outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
                    mode === m.value ? "bg-jolu-selected font-semibold text-jolu-on" : "text-jolu-muted hover:text-jolu-text",
                  )}
                >
                  {m.label}
                </button>
              ))}
            </div>
          </div>
          <TrendLegend mode={mode} />
          <TrendChart points={data.trend.points} bucket={data.trend.bucket} mode={mode} onDrill={onDrillBucket} />
        </Card>

        <Card className="flex flex-col gap-2.5 p-4">
          <CardTitle title="模型分布" note="按 Token" />
          {topModels.length ? topModels.map((m) => {
            const ratio = share(m.tokens, data.tokens.total)
            return (
              <button
                key={`${m.platform}-${m.model}`}
                type="button"
                title={m.model ?? undefined}
                onClick={() => onDrillModel(m)}
                className="-mx-1 -my-0.5 flex flex-col gap-1.5 rounded-md px-1 py-0.5 text-left outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent"
              >
                <span className="flex w-full justify-between gap-2 text-[11px] leading-[13px]">
                  <span className="truncate text-jolu-text">{modelDisplayName(m.model)}</span>
                  <span className="tnum shrink-0 text-jolu-muted">{formatTokens(m.tokens)} · {formatPercent(ratio)}</span>
                </span>
                <span className="h-1.5 w-full rounded-[3px] bg-jolu-track">
                  <span className="block h-full rounded-[3px] bg-jolu-accent" style={{ width: `${Math.max(2, (ratio ?? 0) * 100)}%` }} />
                </span>
              </button>
            )
          }) : <span className="text-xs text-jolu-muted">范围内没有带模型名的记录</span>}
          <span className="mt-auto text-[10px] leading-3 text-jolu-muted">点击模型查看对应调用明细</span>
        </Card>
      </div>

      <div className="grid grid-cols-2 gap-4">
        <Card className="flex min-h-[268px] flex-col gap-2.5 p-4">
          <CardTitle title="供应商用量" />
          {data.providers.map((p) => (
            <div key={p.platform} className="flex h-7 items-center gap-2.5 text-xs">
              <span className="min-w-0 flex-1 truncate text-jolu-text">{providerName(p.platform)}</span>
              <span className="tnum text-jolu-text">{formatTokens(p.tokens)}</span>
              <span className="tnum w-16 text-right text-[11px] text-jolu-muted">{p.requests} 次</span>
            </div>
          ))}
          <span className="flex-1" />
          <span className="text-[10px] text-jolu-muted">{data.attributed} / {data.requests} 次可归属账号 · {unattributed} 次未归属</span>
        </Card>

        <Card className="flex min-h-[268px] flex-col gap-1.5 p-4">
          <div className="flex h-7 items-center justify-between">
            <h3 className="text-sm font-semibold text-jolu-text">高消耗会话</h3>
            <button type="button" onClick={() => onOpenSession()} className="inline-flex h-7 items-center gap-1.5 rounded-lg bg-jolu-selected px-2.5 text-xs font-medium text-jolu-on outline-none hover:bg-jolu-pressed focus-visible:outline-2 focus-visible:outline-jolu-accent">
              查看会话
              <ChevronRight aria-hidden className="size-3.5" />
            </button>
          </div>
          <span className="text-[10px] text-jolu-muted">当前筛选范围 · 按 Token 排序</span>
          <div className="flex flex-col gap-0.5">
            {data.sessions.length ? data.sessions.map((s, index) => (
              <button
                key={`${s.platform}-${s.session_id}`}
                type="button"
                onClick={() => onOpenSession({ platform: s.platform, sessionId: s.session_id })}
                className="flex h-[34px] items-center gap-2.5 rounded-lg px-1.5 text-left outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent"
              >
                <span className={cn("tnum w-4 text-[11px] font-semibold", index === 0 ? "text-jolu-on" : "text-jolu-muted")}>{String(index + 1).padStart(2, "0")}</span>
                <span className="flex min-w-0 flex-1 flex-col gap-[3px]">
                  <span className="truncate text-xs font-medium text-jolu-text">{s.title ?? `会话 ${s.session_id.slice(0, 8)}`}</span>
                  {/* 画板 30：「项目 · 供应商」；读不到项目目录时只显示供应商，不猜项目 */}
                  <span className="truncate text-[10px] text-jolu-muted">{s.cwd ? `${projectName(s.cwd)} · ${providerName(s.platform)}` : providerName(s.platform)}</span>
                </span>
                <span className="tnum text-xs font-semibold text-jolu-text">{formatTokens(s.tokens)}</span>
                <ChevronRight aria-hidden className="size-3.5 text-jolu-muted" />
              </button>
            )) : <span className="py-2 text-xs text-jolu-muted">范围内没有可归入会话的记录</span>}
          </div>
        </Card>
      </div>
      <p className="text-[11px] text-jolu-muted">本机统计不等于服务方账单；费用仅汇总已计价记录；缓存四项按互斥口径统计。</p>
    </>
  )
}
