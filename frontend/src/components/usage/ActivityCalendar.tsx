import { useMemo } from "react"
import { activityLevel, calendarWeeks, formatMinutes, formatTokens, monthLabels } from "@/lib/usageStats"
import type { StatsActivityDto } from "@/lib/api"
import { Tip } from "@/components/ui/Tip"
import { Card } from "./usageControls"

/** 画板 32 的五档活跃色；0 档用侧栏底色，浅深色相同 */
const LEVEL_COLORS = ["var(--jolu-side)", "#D8F2E5", "#99DAB9", "#40C88A", "#099A56"] as const

/** 本机活跃趋势 · 全年日历（画板 32）：按周分列，周一在上 */
export function ActivityCalendar({ activity }: { activity: StatsActivityDto | null }) {
  const view = useMemo(() => {
    if (!activity) return null
    const weeks = calendarWeeks(activity.first_date, activity.last_date)
    const byDate = new Map(activity.days.map((d) => [d.date, d]))
    const sorted = activity.days.map((d) => d.tokens).filter((t) => t > 0).sort((a, b) => a - b)
    return { weeks, labels: monthLabels(weeks), byDate, sorted }
  }, [activity])

  return (
    <Card className="flex flex-col gap-2.5 p-4">
      <div className="flex items-center justify-between">
        <h3 className="text-[15px] font-semibold text-jolu-text">本机活跃趋势</h3>
        <span className="text-[10px] text-jolu-muted">近一年 · 本机记录</span>
      </div>
      {view ? (
        <div className="flex flex-col gap-2">
          <div className="relative h-4">
            {view.labels.map((l) => (
              <span key={`${l.column}-${l.label}`} className="absolute text-xs text-jolu-muted" style={{ left: `${(l.column / view.weeks.length) * 100}%` }}>{l.label}</span>
            ))}
          </div>
          <div className="grid gap-x-[1.4px]" style={{ gridTemplateColumns: `repeat(${view.weeks.length}, minmax(0, 1fr))` }}>
            {view.weeks.map((week, column) => (
              <div key={column} className="flex flex-col gap-[3px]">
                {week.map((key, row) => {
                  if (!key) return <span key={row} className="h-[19px]" />
                  const day = view.byDate.get(key)
                  const level = activityLevel(day?.tokens ?? 0, view.sorted)
                  return (
                    <Tip key={key} side="top" className="min-w-[190px] rounded-xl px-3.5 py-3" label={
                      <span className="flex flex-col gap-2">
                        <span className="tnum border-b border-jolu-border pb-2 text-xs font-semibold">{key}</span>
                        <span className="flex items-center justify-between gap-6"><span className="text-jolu-muted">Token 用量</span><span className="tnum font-medium">{day ? formatTokens(day.tokens) : "无记录"}</span></span>
                        <span className="flex items-center justify-between gap-6"><span className="text-jolu-muted">调用次数</span><span className="tnum font-medium">{day ? `${day.requests} 次` : "无记录"}</span></span>
                      </span>
                    }>
                      <span
                        tabIndex={0}
                        aria-label={day ? `${key}，${formatTokens(day.tokens)} Token，${day.requests} 次调用` : `${key}，无记录`}
                        className="h-[19px] rounded-[3px] outline-none transition-[box-shadow] hover:ring-2 hover:ring-jolu-on focus-visible:ring-2 focus-visible:ring-jolu-on"
                        style={{ background: LEVEL_COLORS[level] }}
                      />
                    </Tip>
                  )
                })}
              </div>
            ))}
          </div>
        </div>
      ) : (
        <span className="h-[170px] animate-pulse rounded-md bg-jolu-side" />
      )}
      <div className="flex h-[18px] items-center justify-end gap-[5px] text-[10px] text-jolu-muted">
        少
        {LEVEL_COLORS.map((color) => <span key={color} aria-hidden className="size-2.5 rounded-sm" style={{ background: color }} />)}
        多
      </div>
      <span className="h-px bg-jolu-border" />
      <div className="flex h-[49px] items-center gap-6">
        <Summary width={192} label="今日活跃时长（估算）" value={activity ? formatMinutes(activity.today_active_minutes) : "—"} />
        <span className="h-9 w-px bg-jolu-border" />
        <Summary width={148} label="本周活跃天数" value={activity ? `${activity.week_active_days} 天` : "—"} />
        <span className="h-9 w-px bg-jolu-border" />
        <Summary width={148} label="本月活跃天数" value={activity ? `${activity.month_active_days} 天` : "—"} />
      </div>
    </Card>
  )
}

function Summary({ width, label, value }: { width: number; label: string; value: string }) {
  return (
    <div className="flex flex-col gap-1.5" style={{ width }}>
      <span className="text-[11px] text-jolu-muted">{label}</span>
      <span className="tnum text-[17px] font-semibold text-jolu-text">{value}</span>
    </div>
  )
}

