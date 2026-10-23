import { ArrowDownLeft, ArrowUpRight, HardDriveDownload, HardDriveUpload } from "lucide-react"
import { formatPercent, formatSeconds, formatTokens, formatUsd, modelDisplayName, share } from "@/lib/usageStats"
import type { StatsActivityDto, StatsModelRowDto, StatsOverviewDto } from "@/lib/api"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import type { PlatformId } from "@/types"
import { ActivityCalendar } from "./ActivityCalendar"
import { Card, CardTitle, MetricCard, providerName } from "./usageControls"

/** 用量分析（画板 32）：Token 四分项、全年活跃日历、模型消耗与费用、供应商费用 */
export function AnalysisPanel({ data, activity, onDrillModel, onDrillProvider }: {
  data: StatsOverviewDto
  activity: StatsActivityDto | null
  onDrillModel: (model: StatsModelRowDto) => void
  onDrillProvider: (platform: string) => void
}) {
  const t = data.tokens
  const part = (value: number | null, note: string) => `${formatPercent(share(value, t.total))} · ${note}`
  // 画板 32「按费用降序」：未计价的排在最后
  const models = [...data.models].sort((a, b) => (b.cost ?? -1) - (a.cost ?? -1) || (b.tokens ?? 0) - (a.tokens ?? 0)).slice(0, 5)
  const priced = data.providers.filter((p) => p.cost.priced_requests > 0).sort((a, b) => b.cost.amount - a.cost.amount)
  const total = data.cost.amount

  return (
    <>
      <div className="grid grid-cols-4 gap-3">
        <MetricCard icon={<ArrowDownLeft />} label="未缓存输入" value={formatTokens(t.fresh_input)} note={part(t.fresh_input, "普通输入")} />
        <MetricCard icon={<ArrowUpRight />} label="输出 Token" value={formatTokens(t.output)} note={part(t.output, "模型生成")} />
        <MetricCard icon={<HardDriveDownload />} label="缓存读取" value={formatTokens(t.cache_read)} note={part(t.cache_read, "已复用输入")} />
        <MetricCard icon={<HardDriveUpload />} label="缓存写入" value={formatTokens(t.cache_write)} note={part(t.cache_write, "新建缓存")} />
      </div>

      <ActivityCalendar activity={activity} />

      <div className="grid min-h-[344px] grid-cols-[minmax(0,644fr)_minmax(0,348fr)] gap-4">
        <Card className="flex flex-col gap-2 p-4">
          <CardTitle title="模型消耗与估算费用" note="按费用降序 ↓" />
          <div className="flex h-6 items-center gap-2.5 rounded-[5px] bg-jolu-side px-2.5 text-[10px] text-jolu-muted">
            <span className="min-w-0 flex-1">模型</span>
            <span className="w-14 text-right">调用次数</span>
            <span className="w-16 text-right">Token</span>
            <span className="w-14 text-right">缓存命中</span>
            <span className="w-14 text-right">平均响应</span>
            <span className="w-20 text-right">估算费用 USD</span>
          </div>
          {models.map((m) => (
            <button
              key={`${m.platform}-${m.model}`}
              type="button"
              title={m.model ?? undefined}
              onClick={() => onDrillModel(m)}
              className="tnum flex h-10 items-center gap-2.5 rounded-[5px] px-2.5 text-left text-xs text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent"
            >
              <span className="flex min-w-0 flex-1 items-center gap-2.5">
                <span className="grid size-7 shrink-0 place-items-center rounded-[7px] bg-jolu-side"><PlatformLogo platform={m.platform as PlatformId} size={20} /></span>
                <span className="truncate font-medium">{modelDisplayName(m.model)}</span>
              </span>
              <span className="w-14 text-right">{m.requests}</span>
              <span className="w-16 text-right">{formatTokens(m.tokens)}</span>
              <span className="w-14 text-right">{formatPercent(m.cache_hit_rate)}</span>
              <span className="w-14 text-right">{formatSeconds(m.avg_duration_ms, true)}</span>
              <span className="w-20 text-right">{formatUsd(m.cost)}</span>
            </button>
          ))}
          {!models.length && <span className="py-2 text-xs text-jolu-muted">范围内没有模型记录</span>}
          <span className="flex-1" />
          <span className="text-[10px] text-jolu-muted">未计价显示 — · 平均响应只统计经本地代理的请求 · 点击模型查看明细</span>
        </Card>

        <Card className="flex flex-col gap-2.5 p-4">
          <CardTitle title="供应商费用" note="估算 · USD" />
          <div className="flex h-9 items-center justify-between rounded-lg bg-jolu-selected px-2.5">
            <span className="text-[11px] text-jolu-on">当前范围合计</span>
            <span className="tnum text-xl font-semibold text-jolu-on">{formatUsd(data.cost.priced_requests ? total : null)}</span>
          </div>
          <div className="flex flex-col gap-1">
            {priced.map((p) => {
              const ratio = share(p.cost.amount, total)
              return (
                <button
                  key={p.platform}
                  type="button"
                  onClick={() => onDrillProvider(p.platform)}
                  className="flex flex-col gap-[5px] rounded-md px-1.5 py-1 text-left outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent"
                >
                  <span className="flex w-full items-center gap-2">
                    <span className="grid size-5 place-items-center rounded-[5px] bg-jolu-side"><PlatformLogo platform={p.platform as PlatformId} size={16} /></span>
                    <span className="min-w-0 flex-1 truncate text-xs text-jolu-text">{providerName(p.platform)}</span>
                    <span className="tnum text-xs font-semibold text-jolu-text">{formatUsd(p.cost.amount)}</span>
                    <span className="tnum w-11 text-right text-[10px] text-jolu-muted">{formatPercent(ratio)}</span>
                  </span>
                  <span className="h-[3px] w-full rounded-sm bg-jolu-track">
                    <span className="block h-full rounded-sm bg-jolu-accent" style={{ width: `${(ratio ?? 0) * 100}%` }} />
                  </span>
                </button>
              )
            })}
            {!priced.length && <span className="py-2 text-xs text-jolu-muted">范围内没有可计价的记录</span>}
          </div>
          <span className="flex-1" />
          <span className="whitespace-pre-line text-[10px] text-jolu-muted">{"仅汇总已计价记录 · 不含订阅月费\n点击供应商查看对应调用明细"}</span>
        </Card>
      </div>
      <p className="text-[11px] text-jolu-muted">输入已扣除单独计量的缓存项，避免重复统计；若来源无法拆分，保留原口径并明确标注。</p>
    </>
  )
}
