import { useEffect, type ReactNode } from "react"
import * as Dialog from "@radix-ui/react-dialog"
import { X } from "lucide-react"
import { useQuota } from "@/hooks/useQuota"
import { listenEvent } from "@/lib/api"
import type { CheckinStatistics, OfficialCheckinStatistics } from "@/lib/checkinApi"
import { checkinPeriodDisplay, checkinQuotaDisplay, checkinQuotaSummary } from "@/lib/checkinStats"
import { cn } from "@/lib/utils"
import { JoluButton } from "@/components/ui/JoluButton"

export interface CheckinAccountStatsProps {
  accountId: string
  statistics: CheckinStatistics | null
  active: boolean
  identityKey: string | null
  official?: OfficialCheckinStatistics | null
  showHistory?: boolean
  accountLabel: string
  providerLabel: string
  statusMessage: string
  detailsOpen: boolean
  onDetailsOpenChange: (open: boolean) => void
  action: ReactNode
  deleteAction: ReactNode
}

export function CheckinAccountStats({ accountId, statistics, active, identityKey, official, showHistory = true, accountLabel, providerLabel, statusMessage, detailsOpen, onDetailsOpenChange, action, deleteAction }: CheckinAccountStatsProps) {
  const enabled = active && identityKey !== null
  const quota = useQuota(accountId, false, enabled)
  const { refresh } = quota

  useEffect(() => {
    if (!enabled) return
    let stopped = false
    let off: (() => void) | undefined
    void listenEvent<string>("daily-checkin-changed", id => {
      if (!stopped && id === accountId) void refresh()
    }).then(unlisten => {
      if (stopped) unlisten()
      else off = unlisten
    }).catch(() => {})
    return () => { stopped = true; off?.() }
  }, [accountId, enabled, refresh])

  const periods = ([["week", "本周签到"], ["month", "本月签到"]] as const)
    .map(([period, title]) => ({ title, source: official?.[period] ? "官方返回日期" : "Jolu 记录", ...checkinPeriodDisplay(statistics, period, official) }))
  const all = checkinPeriodDisplay(statistics, "all", official)
  const display = checkinQuotaDisplay(quota)
  const summary = checkinQuotaSummary(quota)

  const details = <section aria-label="签到统计与账户额度" data-checkin-details={accountId} className="flex min-w-0 flex-col gap-4">
    <div className="flex flex-col gap-2"><h3 className="text-xs font-medium text-jolu-text">签到状态</h3><p className="break-words text-xs leading-relaxed text-jolu-muted">{statusMessage}</p></div>
    {showHistory && <><div className="grid grid-cols-2 gap-3 rounded-lg bg-jolu-side p-3">
      {periods.map(period => <div key={period.title} className="flex min-w-0 flex-col gap-1">
        <p className="text-[11px] text-jolu-muted">{period.title}</p>
        <p className="text-[22px] font-semibold leading-tight tabular-nums text-jolu-text">{period.days}</p>
        <p className={cn("break-words text-[11px] tabular-nums", period.reward === "—" ? "text-jolu-muted" : "text-jolu-on")}>获得 {period.reward}</p>
        <p className="text-[10px] text-jolu-muted">{period.source}</p>
      </div>)}
    </div>
    <div className="flex min-w-0 items-baseline justify-between gap-2 text-[11px]">
      <span className="shrink-0 text-jolu-muted">{official?.all ? `${official.scope}签到获得` : "累计签到获得"}</span>
      <span className={cn("min-w-0 break-words text-right font-medium tabular-nums", all.reward === "—" ? "text-jolu-muted" : "text-jolu-on")}>{all.reward}</span>
    </div>
    {statistics?.has_records && !statistics.unit.trim() && <p className="text-[10px] text-jolu-muted">奖励单位未获取或不一致</p>}</>}
    <div aria-label="账户额度" aria-busy={quota.loading} data-checkin-quota={accountId} className="flex min-w-0 flex-col gap-2 rounded-lg border border-jolu-border p-3">
      <h3 className="text-[11px] font-medium text-jolu-text">账户额度</h3>
      {display.rows.map(row => <div key={row.key} className="flex min-w-0 flex-col gap-1.5">
        {row.label !== "账户额度" && <p className="break-words text-[11px] font-medium text-jolu-muted">{row.label}</p>}
        <dl className="grid min-w-0 grid-cols-2 gap-3">
          <div className="min-w-0">
            <dt className="text-[11px] text-jolu-muted">总额度</dt>
            <dd className="mt-1 break-words text-[13px] font-medium leading-snug tabular-nums text-jolu-text">{row.total}</dd>
          </div>
          <div className="min-w-0">
            <dt className="text-[11px] text-jolu-muted">{row.remainingLabel ?? "可用额度"}</dt>
            <dd className="mt-1 break-words text-[13px] font-medium leading-snug tabular-nums text-jolu-text">{row.remaining}</dd>
          </div>
        </dl>
        {row.used && <p className="text-[10px] tabular-nums text-jolu-muted">已用 {row.used}</p>}
        {row.reason && <p className="break-words text-[10px] leading-relaxed text-jolu-muted">{row.reason}</p>}
      </div>)}
      {display.message && <p role="status" className={cn("break-words text-[10px] leading-relaxed", display.stale ? "font-medium text-jolu-text" : "text-jolu-muted")}>{display.message}</p>}
      {display.stale && quota.fetchedAt && <p className="text-[10px] text-jolu-muted">上次更新 {quota.fetchedAt.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}</p>}
    </div>
    {showHistory && <>{official?.note && <p className="text-[10px] leading-relaxed text-jolu-muted">{official.note}</p>}
    <p className="text-[10px] leading-relaxed text-jolu-muted">{official?.all ? `${official.scope}累计来自官方；周/月来源见各项标注` : "签到统计仅含 Jolu 已记录数据"}</p></>}
  </section>

  return <Dialog.Root open={detailsOpen} onOpenChange={onDetailsOpenChange}>
    <section aria-label="账号数据摘要" data-checkin-stats={accountId} className="flex min-w-0 flex-1 flex-col gap-2.5">
      <dl className="grid grid-cols-2 gap-x-4 gap-y-4 rounded-lg bg-jolu-side p-3">
        {periods.map(period => <div key={period.title} className="min-w-0">
          <dt className="text-[11px] text-jolu-muted">{period.title}</dt>
          <dd className="mt-1 text-[20px] font-semibold leading-tight tabular-nums text-jolu-text">{showHistory ? period.days : "不适用"}</dd>
        </div>)}
        <div className="min-w-0"><dt className="text-[11px] text-jolu-muted">总额度</dt><dd className="mt-1 break-words text-[15px] font-medium leading-snug tabular-nums text-jolu-text">{summary.total}</dd></div>
        <div className="min-w-0"><dt className="text-[11px] text-jolu-muted">{summary.remainingLabel}</dt><dd className="mt-1 break-words text-[15px] font-medium leading-snug tabular-nums text-jolu-text">{summary.remaining}</dd></div>
      </dl>
      <div className="flex min-w-0 flex-wrap items-center justify-between gap-x-2 gap-y-1 text-[10px] text-jolu-muted">
        <span>{summary.scope === "账户额度" ? "统计范围见详情" : summary.scope}</span>
        {summary.status && <span role="status" className={cn(quota.stale && "text-jolu-danger")}>{summary.status}</span>}
      </div>
      <div data-checkin-actions={accountId} className="mt-auto flex items-center gap-1.5 pt-1.5">
        <Dialog.Trigger asChild><JoluButton size="sm" variant="outline" aria-label={`查看账号详情 ${accountLabel}`} className="shrink-0 px-2">详情</JoluButton></Dialog.Trigger>
        <div className="min-w-0 flex-1">{action}</div>
        {deleteAction}
      </div>
    </section>
    <Dialog.Portal>
      <Dialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/40" />
      <Dialog.Content aria-describedby={undefined} className="fixed bottom-0 right-0 top-[30px] z-50 flex w-[480px] max-w-full flex-col border-l border-jolu-border bg-jolu-bg shadow-dialog outline-none">
        <header className="flex shrink-0 items-start justify-between gap-4 border-b border-jolu-border p-6">
          <div className="min-w-0"><Dialog.Title className="text-lg font-semibold text-jolu-text">账号详情</Dialog.Title><p className="mt-2 text-xs text-jolu-muted">{providerLabel}</p><p className="mt-1 break-all text-xs text-jolu-text">{accountLabel}</p></div>
          <Dialog.Close aria-label="关闭账号详情" className="grid size-8 shrink-0 place-items-center rounded-lg text-jolu-muted outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent"><X aria-hidden className="size-4" /></Dialog.Close>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto p-6">{details}</div>
        <footer className="flex shrink-0 justify-end border-t border-jolu-border px-6 py-4"><Dialog.Close asChild><JoluButton variant="outline">关闭</JoluButton></Dialog.Close></footer>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>
}
