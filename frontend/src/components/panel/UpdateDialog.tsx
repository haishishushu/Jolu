import { Check, Download, LoaderCircle } from "lucide-react"
import { useEffect, useId, useState } from "react"
import { isTauri, listenEvent } from "@/lib/api"
import { formatBytes, progressPercent } from "@/lib/updateState"
import { useUpdate } from "./UpdateContext"
import { AUTOMATION_CHANGED, automationApi } from "@/lib/automationApi"

/**
 * 更新进度对话框（画布 17 · B）。
 *
 * 点击任一版本入口后展示下载进度；校验完成后只需点击一次「安装」。
 */
export function UpdateDialog({ onViewTasks }: { onViewTasks: () => void }) {
  const { dialogOpen, closeDialog, cancelDownload, phase, info, progress, error, startDownload, installDownloaded } = useUpdate()
  const [running, setRunning] = useState<number | null>(null)
  const [taskCheckError, setTaskCheckError] = useState(false)
  useEffect(() => {
    if (!dialogOpen || phase !== "downloaded") return
    let stopped = false
    let off: (() => void) | undefined
    const check = () => {
      if (!isTauri) { setRunning(0); return }
      void automationApi.overview().then((value) => {
        if (!stopped) { setRunning(value.running); setTaskCheckError(false) }
      }).catch(() => { if (!stopped) { setRunning(null); setTaskCheckError(true) } })
    }
    setRunning(null)
    setTaskCheckError(false)
    check()
    void listenEvent(AUTOMATION_CHANGED, check).then((unsubscribe) => {
      if (stopped) unsubscribe()
      else off = unsubscribe
    })
    return () => { stopped = true; off?.() }
  }, [dialogOpen, phase])
  const titleId = useId()

  if (!info || !dialogOpen) return null
  if (phase !== "downloading" && phase !== "downloadError" && phase !== "downloaded" && phase !== "installing" && phase !== "ready") return null

  const percent = progressPercent(progress.downloaded, progress.total)
  const installing = phase === "installing"
  const downloaded = phase === "downloaded"
  const downloadError = phase === "downloadError"
  const ready = phase === "ready"
  const tasksRunning = downloaded && ((running ?? 0) > 0 || (running === null && !!error?.includes("自动化任务")))
  const bytes = progress.total === null
    ? formatBytes(progress.downloaded)
    : `${formatBytes(progress.downloaded)} / ${formatBytes(progress.total)}`

  return (
    <div role="dialog" aria-modal="true" aria-labelledby={titleId} className="motion-overlay absolute inset-0 z-20 grid place-items-center bg-black/25 p-8">
      <div className="motion-dialog flex w-[360px] max-w-full flex-col gap-3.5 rounded-xl border bg-surface p-5 shadow-dialog">
        <div className="flex items-center gap-3">
          <span className={`grid size-10 place-items-center rounded-full ${downloadError ? "bg-danger-soft" : "bg-success-soft"} ${installing ? "animate-pulse" : ""}`}>
            {ready || downloaded ? <Check className="size-5 text-success-text" aria-hidden />
              : downloadError ? <Download className="size-5 text-danger" aria-hidden />
                : <LoaderCircle className="size-5 animate-spin text-success-text" aria-hidden />}
          </span>
          <span className="flex flex-col gap-0.5">
            <span id={titleId} className="text-sm font-semibold text-text-primary">
              {ready ? "安装完成" : installing ? "正在安装" : downloaded ? `准备安装 v${info.availableVersion}` : downloadError ? "下载失败" : `正在下载 v${info.availableVersion}`}
            </span>
            <span className="text-[11px] text-text-secondary">
              {ready
                ? isTauri ? `v${info.availableVersion} 已安装，应用即将自动重启` : "预览模式：安装流程已模拟完成"
                : installing ? "安装器正在接管，完成后应用自动重启"
                  : downloaded ? "安装包已下载并通过签名校验，点击安装即可完成更新"
                    : downloadError ? "请重试下载更新包"
                      : "正在下载并校验更新包"}
            </span>
          </span>
        </div>

        <div className="tnum flex items-center gap-2 rounded-lg bg-surface-2 px-3 py-2.5 font-mono text-xs">
          <span className="text-text-tertiary">v{info.currentVersion}</span>
          <span aria-hidden className="text-text-tertiary">→</span>
          <span className="font-bold text-success-text">v{info.availableVersion}</span>
          <span className="flex-1" />
          {info.pubDate && <span className="font-sans text-[10px] text-text-tertiary">{info.pubDate.slice(0, 10)} 发布</span>}
        </div>

        {info.notes && (
          <div className="flex flex-col gap-1.5 rounded-lg bg-surface-2 p-3">
            <span className="text-[11px] font-semibold text-text-secondary">更新说明</span>
            <p className="text-[11px] leading-[1.5] text-text-secondary">{info.notes}</p>
          </div>
        )}

        {phase === "downloading" && (
          <div className="flex flex-col gap-1.5">
            <div
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={percent ?? undefined}
              className="h-1.5 w-full overflow-hidden rounded-full bg-track"
            >
              <div
                className="h-full rounded-full bg-success transition-[width] duration-200"
                style={{ width: `${percent === null ? 40 : percent}%` }}
              />
            </div>
            <div className="tnum flex justify-between font-mono text-[11px]">
              <span className="text-text-secondary">{bytes}</span>
              <span className="font-bold text-success-text">{percent === null ? "…" : `${percent}%`}</span>
            </div>
          </div>
        )}

        {downloaded && !error && <p className="rounded-lg bg-surface-2 p-3 text-[11px] text-text-secondary">
          {tasksRunning ? `还有 ${running} 个自动化任务正在运行，请先处理任务。`
            : taskCheckError ? "任务状态暂时无法读取，安装时将再次检查。"
              : running === null ? "正在检查运行中的任务…" : "没有运行中的自动化任务，可以安装更新。"}
        </p>}
        {error && <p role="alert" className="text-[11px] text-danger">{error}</p>}

        <p className="text-center text-[11px] text-text-tertiary">
          {ready
            ? "灵动岛与后台采集会一并恢复"
            : installing ? "请等待安装器完成，应用将自动重启"
              : downloaded ? "点击下方安装按钮后无需再次确认"
                : downloadError ? "网络恢复后可直接重试"
                  : "下载期间不会中断用量采集"}
        </p>

        {phase === "downloading" && <div className="flex justify-end">
          <button type="button" onClick={cancelDownload} className="rounded-lg border px-4 py-2 text-xs text-text-secondary hover:bg-surface-2">取消下载</button>
        </div>}
        {(downloaded || downloadError || ready) && (
          <div className="flex justify-end gap-2">
            <button type="button" onClick={closeDialog} className="rounded-lg border px-4 py-2 text-xs text-text-secondary hover:bg-surface-2">
              {downloaded ? "稍后安装" : "关闭"}
            </button>
            {!ready &&
            <button
              type="button"
              onClick={downloadError ? startDownload : tasksRunning ? () => { closeDialog(); onViewTasks() } : installDownloaded}
              disabled={downloaded && running === null && !taskCheckError && !error}
              className="rounded-[8px] bg-success-text px-4 py-2 text-xs font-semibold text-white transition-opacity hover:opacity-90 dark:bg-success dark:text-[#0f2a1a]"
            >
              {downloadError ? "重试下载" : tasksRunning ? "查看任务" : "重启并安装"}
            </button>}
          </div>
        )}

      </div>
    </div>
  )
}
