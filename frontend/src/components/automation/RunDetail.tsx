import { ArrowLeft, ArrowUpRight, Check, ChevronRight, CircleAlert, CircleCheck, CircleX, Clock3, FileCode, LoaderCircle, Pause, Play, RefreshCw, Square, type LucideIcon } from "lucide-react"
import { cn } from "@/lib/utils"
import {
  dayTime, durationText, hhmm, hhmmss, projectOf, runCode, runStatusLabel, runTokens, runningMs, toolName, waitingMs,
} from "@/lib/automation"
import type { RunDetailDto } from "@/lib/automationApi"
import { formatTokens } from "@/lib/usageStats"
import { JoluButton } from "@/components/ui/JoluButton"
import { Card, MetricCard } from "@/components/usage/usageControls"
import { KeyValue, SectionTitle } from "./automationParts"
import type { AccountRef } from "./TaskList"

const EVENT_ICONS: Record<string, LucideIcon> = {
  start: Play,
  pause: Pause,
  wait: Clock3,
  queue: Clock3,
  check: RefreshCw,
  resume: Play,
  done: CircleCheck,
  fail: CircleX,
  stop: Square,
  attention: CircleAlert,
}

/** 结果正文：以「- 」开头的行作为完成事项 */
function splitResult(text: string): { intro: string[]; items: string[] } {
  const intro: string[] = []
  const items: string[] = []
  for (const line of text.split(/\r?\n/)) {
    const t = line.trim()
    if (!t) continue
    const m = /^[-*•]\s+(.*)$/.exec(t)
    if (m) items.push(m[1])
    else intro.push(t)
  }
  return { intro: intro.slice(0, 8), items: items.slice(0, 12) }
}

/** 运行详情（画板 66）：本次运行统计、结果、变更与日志，右栏为运行过程与执行上下文 */
export function RunDetail({ detail, now, account, onBack, onOpenSession, onStop, stopping }: {
  detail: RunDetailDto
  now: number
  account: AccountRef | undefined
  onBack: () => void
  onOpenSession: (platform: string, sessionId: string) => void
  onStop: () => void
  stopping: boolean
}) {
  const { run, task, events, logs } = detail
  const status = runStatusLabel(run.status)
  const exec = runningMs(run, now)
  const wait = waitingMs(run, now)
  const tokens = runTokens(run)
  const accountName = account?.name ?? run.account ?? "未知账号"
  const span = run.started_at ? `${dayTime(run.started_at, now)}${run.ended_at ? `–${hhmm(run.ended_at)}` : " 起"}` : `创建于${dayTime(run.created_at, now)}`
  const result = run.result ? splitResult(run.result) : null
  const ResultIcon = run.status === "completed" ? CircleCheck : run.status === "running" ? LoaderCircle : run.status === "failed" ? CircleX : CircleAlert
  const resultTitle = { completed: "任务完成", failed: "运行失败", cancelled: "已停止运行", running: "正在执行", waiting: "等待额度", queued: "排队中", attention: "需要处理" }[run.status]
  const resultNote = run.status === "completed"
    ? (run.waits ? `经历 ${run.waits} 次额度等待` : "未经历额度等待")
    : run.status === "failed" ? "查看错误信息" : run.reason ?? (run.status === "running" ? "可查看日志并停止" : "—")

  return (
    <>
      <div className="flex min-h-12 items-center gap-2.5">
        <div className="flex min-w-0 flex-1 flex-col gap-[5px]">
          <h2 className="truncate text-[22px] font-semibold text-jolu-text">{run.task_name}</h2>
          <span className="truncate text-[11px] text-jolu-muted">运行 {runCode(run.id)} · {span} · {toolName(run.platform)} / {accountName}</span>
        </div>
        <JoluButton size="sm" variant="neutral" onClick={onBack}><ArrowLeft aria-hidden />返回记录</JoluButton>
        {run.status === "running" && (
          <JoluButton size="sm" variant="danger-soft" disabled={stopping} onClick={onStop}><Square aria-hidden />{stopping ? "正在停止…" : "停止运行"}</JoluButton>
        )}
        {run.session_id && (
          <JoluButton size="sm" variant="primary" onClick={() => onOpenSession(run.platform, run.session_id!)}><ArrowUpRight aria-hidden />查看会话</JoluButton>
        )}
      </div>

      <div className="grid grid-cols-4 gap-3">
        <MetricCard label="运行结果" value={status.label} note={resultNote} />
        <MetricCard label="实际执行" value={durationText(exec)} note="不含等待时长" />
        <MetricCard label="等待额度" value={wait ? durationText(wait) : "—"} note={run.waits ? (run.status === "completed" ? "已从原会话继续" : `${run.waits} 次等待`) : "未经历额度等待"} />
        <MetricCard
          label="消耗 Token"
          value={formatTokens(tokens).replace(" ", "")}
          note={tokens === null ? "执行器未返回用量" : `输入 ${formatTokens(run.input_tokens).replace(" ", "")} / 输出 ${formatTokens(run.output_tokens).replace(" ", "")}`}
        />
      </div>

      <div className="flex items-start gap-4">
        <div className="flex w-[640px] min-w-0 shrink flex-col gap-3.5">
          <Card className="flex flex-col gap-3 p-5">
            <div className="flex items-center gap-2.5">
              <ResultIcon aria-hidden className={cn("size-5", run.status === "failed" || run.status === "attention" ? "text-jolu-danger" : "text-jolu-on", run.status === "running" && "animate-spin")} />
              <span className="text-base font-semibold text-jolu-text">{resultTitle}</span>
            </div>
            {result ? (
              <>
                {result.intro.map((line, i) => <p key={i} className="text-[13px] leading-[1.6] text-jolu-text">{line}</p>)}
                {result.items.map((line, i) => (
                  <span key={i} className="flex items-start gap-2 text-xs text-jolu-text"><Check aria-hidden className="mt-0.5 size-3.5 shrink-0 text-jolu-on" />{line}</span>
                ))}
              </>
            ) : run.error ? (
              <p className="whitespace-pre-wrap text-xs leading-[1.6] text-jolu-danger">{run.error}</p>
            ) : (
              <p className="text-xs text-jolu-muted">{run.reason ?? (run.status === "running" ? "执行器尚未返回结果。" : "执行器没有返回结果。")}</p>
            )}
          </Card>

          <Card className="flex flex-col gap-3 p-4">
            <SectionTitle title="变更与验证" note="执行器返回" />
            {run.files.length ? run.files.slice(0, 12).map((file) => (
              <div key={file} className="flex items-center gap-2.5">
                <FileCode aria-hidden className="size-4 shrink-0 text-jolu-muted" />
                <span className="min-w-0 flex-1 truncate text-xs text-jolu-text" title={file}>{file.split(/[\\/]/).pop()}</span>
                <span className="truncate text-[11px] text-jolu-muted" title={file}>{file.includes("/") || file.includes("\\") ? file.replace(/[\\/][^\\/]*$/, "") : "已修改"}</span>
              </div>
            )) : <span className="text-xs text-jolu-muted">执行器没有返回文件修改</span>}
            <div className="flex items-center gap-2.5">
              <FileCode aria-hidden className="size-4 shrink-0 text-jolu-muted" />
              <span className="flex-1 text-xs text-jolu-text">相关测试</span>
              <span className="text-[11px] text-jolu-muted">未验证</span>
            </div>
            <span className="text-[10px] text-jolu-muted">仅展示实际执行器返回的结果；未返回验证信息时标记「未验证」。</span>
          </Card>

          <Card className="flex flex-col gap-2.5 p-4">
            <SectionTitle title="执行日志" note="完整内容见关联会话" />
            {logs.length ? (
              <pre className="max-h-[220px] overflow-auto whitespace-pre-wrap font-mono text-[11px] leading-[1.7] text-jolu-muted">
                {logs.map((l) => `${hhmmss(l.ts)}  ${l.title}`).join("\n")}
              </pre>
            ) : <span className="text-[11px] text-jolu-muted">暂无执行日志</span>}
            {run.session_id && (
              <JoluButton size="sm" variant="soft" className="self-start" onClick={() => onOpenSession(run.platform, run.session_id!)}>打开完整会话<ChevronRight aria-hidden /></JoluButton>
            )}
          </Card>
        </div>

        <div className="flex min-w-0 flex-1 flex-col gap-3.5">
          <Card className="flex flex-col gap-3.5 p-4">
            <SectionTitle title="运行过程" note="同一次运行" />
            {events.length ? events.map((e, i) => {
              const Icon = EVENT_ICONS[e.kind] ?? Clock3
              return (
                <div key={i} className="flex gap-2.5">
                  <Icon aria-hidden className={cn("mt-0.5 size-4 shrink-0", e.kind === "fail" || e.kind === "attention" ? "text-jolu-danger" : "text-jolu-on")} />
                  <div className="flex min-w-0 flex-col gap-1">
                    <span className="text-xs font-medium text-jolu-text">{e.title}</span>
                    <span className="tnum text-[10px] text-jolu-muted">{hhmmss(e.ts)}</span>
                    {e.detail && <span className="break-words text-[10px] text-jolu-muted">{e.detail}</span>}
                  </div>
                </div>
              )
            }) : <span className="text-[11px] text-jolu-muted">暂无记录</span>}
          </Card>

          <Card className="flex flex-col gap-3 p-4">
            <SectionTitle title="执行上下文" />
            <KeyValue label="项目" value={projectOf(task?.cwd)} title={task?.cwd} />
            <KeyValue label="账号" value={accountName} />
            <KeyValue label="模型" value={run.model ?? "跟随工具默认"} />
            <KeyValue label="Skills" value={task?.skills.length ? task.skills.join(" / ") : "未选择"} />
            <span className="text-[10px] text-jolu-muted">执行器、模型和账号以本次运行快照为准。</span>
          </Card>
        </div>
      </div>
      <p className="text-[10px] text-jolu-muted">自动化负责安排与记录运行；文件修改权限沿用任务执行器的明确授权。</p>
    </>
  )
}
