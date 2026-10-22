import * as Dialog from "@radix-ui/react-dialog"
import { useState } from "react"
import { CalendarDays, ChevronLeft, ChevronRight, X } from "lucide-react"
import { cn } from "@/lib/utils"
import { canConfirmRange, dateKey, monthGrid, parseDateKey, pickDate, rangeDays, rangeLabel, zoneName, localZone, type DateRange } from "@/lib/usageStats"

const WEEKDAYS = ["一", "二", "三", "四", "五", "六", "日"]

/**
 * 选择日期范围（画板 36）：双月日历点选开始与结束，确认后才生效；取消保持原筛选。
 * 允许同一天；缺日期或开始晚于结束时确认不可用；今天之后的日期不可选（画板 38）。
 */
export function DateRangeDialog({ value, onConfirm, onClose }: {
  value: DateRange
  onConfirm: (range: DateRange) => void
  onClose: () => void
}) {
  const today = dateKey(new Date())
  const [draft, setDraft] = useState<{ start: string | null; end: string | null }>({ start: value.start, end: value.end })
  // 右侧显示结束日所在月份，左侧是前一个月
  const [anchor, setAnchor] = useState(() => {
    const end = parseDateKey(value.end)
    return { year: end.getFullYear(), month: end.getMonth() }
  })
  const left = anchor.month === 0 ? { year: anchor.year - 1, month: 11 } : { year: anchor.year, month: anchor.month - 1 }
  const shift = (delta: number) => setAnchor(({ year, month }) => {
    const next = new Date(year, month + delta, 1)
    return { year: next.getFullYear(), month: next.getMonth() }
  })
  const nowMonth = new Date()
  const atLatest = anchor.year === nowMonth.getFullYear() && anchor.month === nowMonth.getMonth()
  const valid = canConfirmRange(draft.start, draft.end)
  const editing = draft.start === null ? "start" : "end"

  return (
    <Dialog.Root open onOpenChange={(open) => !open && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/40" />
        <Dialog.Content
          aria-describedby={undefined}
          className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex max-h-[calc(100dvh-70px)] w-[620px] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-[18px] overflow-y-auto rounded-2xl border border-jolu-border bg-jolu-bg p-6 shadow-dialog outline-none"
        >
          <div className="flex items-center justify-between">
            <Dialog.Title className="text-lg font-semibold text-jolu-text">选择日期范围</Dialog.Title>
            <Dialog.Close asChild>
              <button type="button" className="inline-flex h-[34px] items-center gap-1.5 rounded-lg border border-jolu-border px-3 text-xs font-medium text-jolu-text outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent">
                <X aria-hidden className="size-4 text-jolu-muted" />
                关闭
              </button>
            </Dialog.Close>
          </div>
          <p className="text-xs text-jolu-muted">当前筛选：{rangeLabel(value)} · 以下日期为待确认范围</p>
          <div className="flex gap-4">
            <DateBox label="开始日期" value={draft.start} active={editing === "start"} />
            <DateBox label="结束日期" value={draft.end} active={editing === "end"} />
          </div>
          <div className="flex gap-6">
            <Month year={left.year} month={left.month} draft={draft} today={today} onPrev={() => shift(-1)} onPick={(key) => setDraft((d) => pickDate(d, key))} />
            <Month year={anchor.year} month={anchor.month} draft={draft} today={today} onNext={atLatest ? undefined : () => shift(1)} onPick={(key) => setDraft((d) => pickDate(d, key))} />
          </div>
          <p className="text-[11px] text-jolu-muted">按{zoneName(localZone())}时区统计，包含开始与结束当天；今天统计至当前时刻。</p>
          <div className="flex h-9 items-center justify-between">
            <span className="text-xs text-jolu-muted">{valid ? `已选择 ${rangeDays({ start: draft.start!, end: draft.end! })} 天` : "请选择结束日期"}</span>
            <div className="flex gap-2">
              <button type="button" onClick={onClose} className="inline-flex h-[34px] items-center rounded-lg border border-jolu-border px-3 text-xs font-medium text-jolu-text outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent">取消</button>
              <button
                type="button"
                disabled={!valid}
                onClick={() => valid && onConfirm({ start: draft.start!, end: draft.end! })}
                className="inline-flex h-[34px] items-center rounded-lg bg-jolu-accent px-3 text-xs font-medium text-[#071B10] outline-none hover:brightness-95 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-45"
              >
                确认
              </button>
            </div>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

function DateBox({ label, value, active }: { label: string; value: string | null; active: boolean }) {
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-2">
      <span className="text-xs text-jolu-muted">{label}</span>
      <div className={cn("flex h-10 items-center gap-2 rounded-lg border px-3", active ? "border-jolu-accent" : "border-jolu-border")}>
        <CalendarDays aria-hidden className="size-4 text-jolu-muted" />
        <span className={cn("tnum text-[13px]", value ? "text-jolu-text" : "text-jolu-muted")}>{value ? value.replaceAll("-", "/") : "请选择"}</span>
      </div>
    </div>
  )
}

function Month({ year, month, draft, today, onPrev, onNext, onPick }: {
  year: number
  month: number
  draft: { start: string | null; end: string | null }
  today: string
  onPrev?: () => void
  onNext?: () => void
  onPick: (key: string) => void
}) {
  const nav = (label: string, onClick: (() => void) | undefined, icon: React.ReactNode) => (
    <button type="button" aria-label={label} disabled={!onClick} onClick={onClick}
      className={cn("grid size-6 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:invisible")}>
      {icon}
    </button>
  )
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-2">
      <div className="flex h-7 items-center justify-between">
        {nav("上一月", onPrev, <ChevronLeft aria-hidden className="size-4" />)}
        <span className="text-[13px] font-semibold text-jolu-text">{year} 年 {month + 1} 月</span>
        {nav("下一月", onNext, <ChevronRight aria-hidden className="size-4" />)}
      </div>
      <div className="grid grid-cols-7 gap-[3px]">
        {WEEKDAYS.map((d) => <span key={d} className="grid h-[22px] place-items-center text-[10px] text-jolu-muted">{d}</span>)}
        {monthGrid(year, month).flat().map((key, index) => {
          if (!key) return <span key={`empty-${index}`} className="h-[29px]" />
          const edge = key === draft.start || key === draft.end
          const inside = !!draft.start && !!draft.end && key > draft.start && key < draft.end
          const future = key > today
          return (
            <button
              key={key}
              type="button"
              disabled={future}
              aria-pressed={edge || inside}
              aria-label={key}
              onClick={() => onPick(key)}
              className={cn(
                "tnum h-[29px] rounded-md text-xs outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-35",
                edge ? "bg-jolu-accent font-semibold text-[#071B10]" : inside ? "bg-jolu-selected text-jolu-on" : "text-jolu-text hover:bg-jolu-stage",
              )}
            >
              {Number(key.slice(8))}
            </button>
          )
        })}
      </div>
    </div>
  )
}
