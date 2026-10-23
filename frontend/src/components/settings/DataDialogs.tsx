import * as Dialog from "@radix-ui/react-dialog"
import { useEffect, useState, type ReactNode } from "react"
import { CalendarDays, CircleAlert, Download, Square, SquareCheck, X } from "lucide-react"
import { api, isTauri, type CleanupPreviewDto } from "@/lib/api"
import { PLATFORMS } from "@/lib/platforms"
import { RETENTION_OPTIONS, cleanupCutoffDate, formatCount } from "@/lib/settingsPage"
import { rangeLabel, rangeToMillis, todayRange, type DateRange } from "@/lib/usageStats"
import { useToast } from "@/components/ui/Toast"
import { DateRangeDialog } from "@/components/usage/DateRangeDialog"
import { ActionButton, SelectButton } from "./settingsUi"

function Shell({ width, title, onClose, busy, children }: { width: number; title: string; onClose: () => void; busy?: boolean; children: ReactNode }) {
  return (
    <Dialog.Root open onOpenChange={(open) => !open && !busy && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/35" />
        <Dialog.Content
          aria-describedby={undefined}
          className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex max-h-[calc(100dvh-70px)] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-[18px] overflow-y-auto rounded-2xl border border-jolu-border bg-jolu-bg p-6 shadow-dialog outline-none"
          style={{ width }}
        >
          <div className="flex items-center gap-2.5">
            <Dialog.Title className="flex-1 text-xl font-semibold text-jolu-text">{title}</Dialog.Title>
            <Dialog.Close aria-label="关闭" disabled={busy} className="grid size-7 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
              <X aria-hidden className="size-4" />
            </Dialog.Close>
          </div>
          {children}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

/** 导出本地数据：按供应商与日期范围导出 JSON（D-27）；不选日期时导出全部 */
export function ExportDialog({ onClose }: { onClose: () => void }) {
  const toast = useToast()
  const [platform, setPlatform] = useState("all")
  // null = 全部日期；选定后按 [开始日 0 点, 结束日次日 0 点) 导出，结束日是今天时到当前时刻
  const [range, setRange] = useState<DateRange | null>(null)
  const [picking, setPicking] = useState(false)
  const [busy, setBusy] = useState(false)
  const run = () => {
    setBusy(true)
    const millis = range ? rangeToMillis(range) : null
    api.exportDataFile(platform === "all" ? null : platform, millis?.start ?? null, millis?.end ?? null)
      .then((path) => {
        // 用户在保存对话框里取消时什么都没发生，不弹提示
        if (!path) return
        toast.success("导出完成", path)
        onClose()
      })
      .catch((reason) => toast.danger("导出失败", String(reason)))
      .finally(() => setBusy(false))
  }
  return (
    <Shell width={460} title="导出本地数据" onClose={onClose} busy={busy}>
      <p className="text-xs text-jolu-muted">导出统计记录与会话标题为 JSON，可在另一台设备导入。</p>
      <div className="flex items-center gap-4">
        <span className="flex-1 text-[13px] font-medium text-jolu-text">供应商</span>
        <SelectButton label="导出范围" value={platform} disabled={busy} onChange={setPlatform}
          options={[{ value: "all", label: "全部供应商" }, ...PLATFORMS.map((p) => ({ value: p.id, label: p.name }))]} />
      </div>
      <div className="flex items-center gap-4">
        <span className="flex-1 text-[13px] font-medium text-jolu-text">日期范围</span>
        {/* 「全部日期」恢复入口画板未画，待确认 */}
        {range && <button type="button" disabled={busy} onClick={() => setRange(null)}
          className="rounded text-xs text-jolu-muted outline-none hover:text-jolu-on focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">全部日期</button>}
        <ActionButton title="选择导出日期范围" icon={<CalendarDays aria-hidden className="size-3.5 text-jolu-muted" />} disabled={busy} onClick={() => setPicking(true)}
          className="w-40 justify-start">
          {range ? rangeLabel(range) : "全部日期"}
        </ActionButton>
      </div>
      <p className="rounded-[9px] bg-jolu-side p-3.5 text-[11px] text-jolu-muted">导出文件不包含 API Key、登录凭证和原始会话正文。</p>
      <div className="flex justify-end gap-2">
        <ActionButton disabled={busy} onClick={onClose}>取消</ActionButton>
        <ActionButton tone="accent" icon={<Download aria-hidden className="size-3.5" />} busy={busy} disabled={!isTauri} onClick={run}>导出数据</ActionButton>
      </div>
      {picking && (
        <DateRangeDialog
          value={range ?? todayRange()}
          currentLabel={range ? undefined : "全部日期"}
          onConfirm={(next) => { setRange(next); setPicking(false) }}
          onClose={() => setPicking(false)}
        />
      )}
    </Shell>
  )
}

/** 清理历史数据确认（画板 98）：先预览范围，勾选确认后才允许删除 */
export function CleanupDialog({ initialDays, onClose, onExport, onDone }: {
  initialDays: number
  onClose: () => void
  onExport: () => void
  onDone: () => void
}) {
  const toast = useToast()
  const [days, setDays] = useState(initialDays)
  const [preview, setPreview] = useState<CleanupPreviewDto | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [confirmed, setConfirmed] = useState(false)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    setPreview(null)
    setError(null)
    setConfirmed(false)
    if (!isTauri) return
    let stopped = false
    api.cleanupPreview(days)
      .then((next) => { if (!stopped) setPreview(next) })
      .catch((reason) => { if (!stopped) setError(String(reason)) })
    return () => { stopped = true }
  }, [days])

  const count = preview?.requests ?? null
  const run = () => {
    setBusy(true)
    api.cleanupHistory(days)
      .then((result) => {
        toast.success(`已清理 ${result.requests.toLocaleString("zh-CN")} 条请求记录`, `保留最近 ${days} 天；采集位置保留，旧日志不会重新导入`)
        onDone()
      })
      .catch((reason) => toast.danger("历史清理失败", String(reason)))
      .finally(() => setBusy(false))
  }

  return (
    <Shell width={580} title="清理历史数据" onClose={onClose} busy={busy}>
      <p className="-mt-2 text-xs text-jolu-muted">先确认范围，再删除本地统计记录。</p>
      <section className="flex flex-col rounded-xl border border-jolu-border px-5">
        <h3 className="pb-3.5 pt-[18px] text-sm font-semibold text-jolu-text">清理范围</h3>
        <div className="flex min-h-[72px] items-center gap-5 border-t border-jolu-border">
          <span className="flex flex-1 flex-col gap-1.5">
            <span className="text-[13px] font-medium text-jolu-text">保留最近</span>
            <span className="text-[11px] text-jolu-muted">此日期之前的数据将被清理。</span>
          </span>
          <SelectButton label="保留最近" value={String(days)} disabled={busy} onChange={(v) => setDays(Number(v))}
            options={RETENTION_OPTIONS.map((n) => ({ value: String(n), label: `${n} 天` }))} />
        </div>
        <div className="flex min-h-[72px] items-center gap-5 border-t border-jolu-border">
          <span className="flex flex-1 flex-col gap-1.5">
            <span className="text-[13px] font-medium text-jolu-text">截止日期</span>
            <span className="text-[11px] text-jolu-muted">根据当前日期与时区计算。</span>
          </span>
          <span className="tnum text-xs text-jolu-muted">{cleanupCutoffDate(days, new Date(), preview?.cutoff_ms)}</span>
        </div>
      </section>
      <div className="grid grid-cols-2 gap-3 rounded-[10px] bg-jolu-side p-4">
        <span className="flex flex-col gap-1.5">
          <span className="text-[11px] text-jolu-muted">请求记录</span>
          <span className="tnum text-[22px] font-semibold text-jolu-text">{count === null ? (error ? "—" : "读取中…") : `${formatCount(count)} 条`}</span>
        </span>
        <span className="flex flex-col gap-1.5">
          <span className="text-[11px] text-jolu-muted">预计释放</span>
          <span className="tnum text-[22px] font-semibold text-jolu-text">—</span>
          <span className="text-[10px] text-jolu-muted">数据库文件不会立即缩小，释放量待确认</span>
        </span>
      </div>
      {error && <p role="alert" className="text-[11px] text-jolu-danger">清理预览失败：{error}</p>}
      {preview && <p className="-mt-2 text-[11px] text-jolu-muted">同时删除 {formatCount(preview.usage_events)} 条增量事件、{formatCount(preview.sessions)} 条已结束会话信息。</p>}
      <div className="flex items-start gap-2.5 rounded-[9px] bg-jolu-danger-soft p-3.5 text-[11px] text-jolu-danger">
        <CircleAlert aria-hidden className="size-4 shrink-0" />
        删除后无法撤销；原始会话文件、账号与 Skills 均会保留。
      </div>
      <p className="-mt-1 text-[11px] text-jolu-muted">采集位置会保留，旧日志不会在下一轮重新导入。</p>
      <button type="button" role="checkbox" aria-checked={confirmed} disabled={busy || !preview} onClick={() => setConfirmed((v) => !v)}
        className="flex items-center gap-2 self-start rounded-md text-xs text-jolu-text outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
        {confirmed ? <SquareCheck aria-hidden className="size-4 text-jolu-on" /> : <Square aria-hidden className="size-4 text-jolu-muted" />}
        我已了解本次清理范围
      </button>
      <div className="flex items-center justify-between gap-2">
        <ActionButton icon={<Download aria-hidden className="size-3.5" />} disabled={busy} onClick={onExport}>先导出数据</ActionButton>
        <div className="flex gap-2">
          <ActionButton disabled={busy} onClick={onClose}>取消</ActionButton>
          <ActionButton tone="danger" busy={busy} disabled={!confirmed || !count} onClick={run}>
            {count === null ? "清理" : count > 0 ? `清理 ${formatCount(count)} 条` : "没有可清理的记录"}
          </ActionButton>
        </div>
      </div>
    </Shell>
  )
}
