import { useEffect, useId, useRef, useState } from "react"
import { axisTicks, hitIndex, labelEvery, seriesMax, smoothPath, xAt, yAt, type Series } from "@/lib/chartGeometry"
import { formatTokens, formatUsd } from "@/lib/usageStats"
import type { StatsTrendBucketDto } from "@/lib/api"

export type TrendMode = "tokens" | "requests" | "cost"

/** 画板 30 的四条折线颜色；浅深色相同 */
const TOKEN_SERIES = [
  { key: "fresh_input", label: "输入", color: "#059669" },
  { key: "output", label: "输出", color: "#34C88A" },
  { key: "cache_read", label: "缓存读取", color: "#87BFA8" },
  { key: "cache_write", label: "缓存写入", color: "#748C82" },
] as const

const HEIGHT = 154
const AXIS_W = 40
const LABEL_H = 18

function seriesFor(points: StatsTrendBucketDto[], mode: TrendMode): (Series & { color: string })[] {
  if (mode === "tokens") return TOKEN_SERIES.map((s) => ({ key: s.key, label: s.label, colorVar: "", color: s.color, values: points.map((p) => p[s.key]) }))
  if (mode === "requests") return [{ key: "requests", label: "请求", colorVar: "", color: "#059669", values: points.map((p) => p.requests) }]
  return [{ key: "cost", label: "估算费用", colorVar: "", color: "#059669", values: points.map((p) => p.cost) }]
}

/** 轴刻度去掉多余小数：1 M、500 K、2.5 M */
function axisLabel(value: number, mode: TrendMode): string {
  if (mode === "cost") return value === 0 ? "0" : `$${Number(value.toFixed(2))}`
  if (mode === "requests") return String(Math.round(value))
  if (value >= 1_000_000) return `${Number((value / 1_000_000).toFixed(1))} M`
  if (value >= 1_000) return `${Number((value / 1_000).toFixed(1))} K`
  return String(value)
}

function tooltipTitle(point: StatsTrendBucketDto, bucket: "2h" | "day" | "week"): string {
  const d = new Date(point.start)
  const day = `${d.getMonth() + 1} 月 ${d.getDate()} 日`
  if (bucket === "2h") return `${day} · ${point.label}`
  return bucket === "week" ? `${day}起一周` : day
}

export function TrendLegend({ mode }: { mode: TrendMode }) {
  const items = mode === "tokens" ? TOKEN_SERIES : [{ label: mode === "requests" ? "请求" : "估算费用", color: "#059669" }]
  return (
    <div className="flex h-4 items-center gap-[18px]">
      {items.map((item) => (
        <span key={item.label} className="flex items-center gap-1.5 text-[10px] text-jolu-muted">
          <span aria-hidden className="h-[3px] w-3.5 rounded-sm" style={{ background: item.color }} />
          {item.label}
        </span>
      ))}
    </div>
  )
}

/** 平滑折线趋势：悬浮看分项，点击数据点下钻到对应时段的调用明细（画板 38） */
export function TrendChart({ points, bucket, mode, onDrill }: {
  points: StatsTrendBucketDto[]
  bucket: "2h" | "day" | "week"
  mode: TrendMode
  onDrill: (point: StatsTrendBucketDto) => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(560)
  const [hover, setHover] = useState(-1)
  const gradientId = useId()

  useEffect(() => {
    const el = ref.current
    if (!el) return
    const observer = new ResizeObserver(([entry]) => setWidth(Math.max(120, entry.contentRect.width - AXIS_W)))
    observer.observe(el)
    return () => observer.disconnect()
  }, [])

  const series = seriesFor(points, mode)
  const ticks = axisTicks(seriesMax(series), 3)
  const top = ticks[0] ?? 0
  const plot = { width, height: HEIGHT - LABEL_H - 6 }
  const every = labelEvery(points.length, 7)
  const latest = (values: Array<number | null>) => {
    for (let i = values.length - 1; i >= 0; i--) if (values[i] !== null) return i
    return -1
  }
  const point = hover >= 0 ? points[hover] : null

  return (
    <div ref={ref} className="relative h-[154px] w-full select-none">
      {ticks.map((tick) => {
        const y = yAt(tick, top, plot.height) + 3
        return (
          <div key={tick} className="pointer-events-none absolute inset-x-0" style={{ top: y }}>
            <span className="tnum absolute left-0 -translate-y-1/2 text-[9px] text-jolu-muted">{axisLabel(tick, mode)}</span>
            <span className="absolute right-0 h-px bg-jolu-border" style={{ left: AXIS_W }} />
          </div>
        )
      })}
      <svg
        role="img"
        aria-label="用量趋势"
        className="absolute top-[3px] cursor-pointer overflow-visible"
        style={{ left: AXIS_W }}
        width={plot.width}
        height={plot.height}
        onMouseMove={(event) => setHover(hitIndex(event.nativeEvent.offsetX, points.length, plot.width))}
        onMouseLeave={() => setHover(-1)}
        onClick={() => point && onDrill(point)}
      >
        <defs>
          <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#06C167" stopOpacity={0.08} />
            <stop offset="1" stopColor="#06C167" stopOpacity={0} />
          </linearGradient>
        </defs>
        {series[0] && (() => {
          const line = smoothPath(series[0].values, top, plot)
          const first = series[0].values.findIndex((v) => v !== null)
          const last = latest(series[0].values)
          if (!line || first < 0 || last === first) return null
          const base = plot.height.toFixed(2)
          return <path d={`${line} L${xAt(last, points.length, plot.width).toFixed(2)},${base} L${xAt(first, points.length, plot.width).toFixed(2)},${base} Z`} fill={`url(#${gradientId})`} />
        })()}
        {point && <line x1={xAt(hover, points.length, plot.width)} x2={xAt(hover, points.length, plot.width)} y1={0} y2={plot.height} stroke="var(--jolu-border)" strokeDasharray="3 3" />}
        {series.map((s) => (
          <path key={s.key} d={smoothPath(s.values, top, plot)} fill="none" stroke={s.color} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" />
        ))}
        {series.map((s) => {
          const index = hover >= 0 && s.values[hover] !== null ? hover : latest(s.values)
          const value = index >= 0 ? s.values[index] : null
          if (value === null) return null
          return <circle key={s.key} cx={xAt(index, points.length, plot.width)} cy={yAt(value, top, plot.height)} r={3.5} fill={s.color} stroke="var(--jolu-bg)" strokeWidth={1.5} />
        })}
      </svg>
      {points.map((p, index) => (index % every === 0 ? (
        <span key={p.start} className="tnum pointer-events-none absolute bottom-0 -translate-x-1/2 text-[9px] text-jolu-muted" style={{ left: AXIS_W + xAt(index, points.length, plot.width) }}>
          {p.label}
        </span>
      ) : null))}
      {point && (
        <div
          role="tooltip"
          className="pointer-events-none absolute top-0 z-10 flex w-[200px] flex-col gap-2 rounded-xl border border-jolu-border bg-jolu-bg p-3 shadow-popover"
          style={xAt(hover, points.length, plot.width) > plot.width / 2
            ? { right: plot.width - xAt(hover, points.length, plot.width) + 12 }
            : { left: AXIS_W + xAt(hover, points.length, plot.width) + 12 }}
        >
          <span className="text-[13px] font-semibold text-jolu-text">{tooltipTitle(point, bucket)}</span>
          {mode === "tokens" ? (
            <>
              {[["普通输入", point.fresh_input], ["输出", point.output], ["缓存读取", point.cache_read], ["缓存写入", point.cache_write]].map(([label, value]) => (
                <span key={label as string} className="flex justify-between text-xs"><span className="text-jolu-muted">{label}</span><span className="tnum text-jolu-text">{formatTokens(value as number | null)}</span></span>
              ))}
              <span className="flex items-center justify-between text-xs"><span className="text-jolu-text">总 Token</span><span className="tnum text-base font-semibold text-jolu-on">{formatTokens(point.total)}</span></span>
              {[point.fresh_input, point.output, point.cache_read, point.cache_write].some(value=>value===null) && <span className="text-[10px] text-jolu-muted">此时段部分数据未提供，折线跨过缺失点连接。</span>}
            </>
          ) : (
            <span className="flex items-center justify-between text-xs">
              <span className="text-jolu-text">{mode === "requests" ? "请求" : "估算费用"}</span>
              <span className="tnum text-base font-semibold text-jolu-on">{mode === "requests" ? `${point.requests} 次` : formatUsd(point.cost)}</span>
            </span>
          )}
          <span className="text-[10px] text-jolu-muted">点击查看这段时间的调用明细</span>
        </div>
      )}
    </div>
  )
}
