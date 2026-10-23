import { useEffect, useState } from "react"
import { ArrowLeft, ChevronRight, CircleAlert, CircleCheck, Clock3, Ellipsis, LoaderCircle, LogIn, Pause, Play, RefreshCw, RotateCcw, Square, X } from "lucide-react"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import { observeActiveRequest } from "@/lib/activeRequest"
import {
  accountText, blockBanner, clock, dayTime, launchLabel, projectOf, runningMs, shortId, taskCode, taskStatus, toolName, untilText, windowLabel,
} from "@/lib/automation"
import { automationApi, type QuotaVerdictDto, type TaskAction, type TaskDetailDto } from "@/lib/automationApi"
import { localZone, zoneName } from "@/lib/usageStats"
import { demoQuota } from "@/mock/automation"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { JoluButton } from "@/components/ui/JoluButton"
import { Card } from "@/components/usage/usageControls"
import { KeyValue, Notice, QuotaBar, SectionTitle, TONE, TaskMenu } from "./automationParts"
import type { AccountRef } from "./TaskList"
import type { PlatformId } from "@/types"

/**
 * 任务详情（画板 62「等待额度与续跑详情」）：恢复时间与检查状态、继续执行的位置、执行内容，
 * 右栏为账号额度、同账号队列与等待策略。其他状态沿用同一布局，主卡片按状态切换。
 */
export function TaskDetail({ detail, now, account, onBack, onAction, onEdit, onCancel, onDelete, onOpenRun, onOpenSession, onOpenProviders }: {
  detail: TaskDetailDto
  now: number
  account: AccountRef | undefined
  onBack: () => void
  onAction: (action: TaskAction) => Promise<void>
  onEdit: () => void
  onCancel: () => void
  onDelete: () => void
  onOpenRun: (runId: number) => void
  onOpenSession: (platform: string, sessionId: string) => void
  onOpenProviders: () => void
}) {
  const { item, queue, events } = detail
  const run = item.active
  const last = item.last
  const status = taskStatus(item, now, zoneName(localZone()))
  const [busy, setBusy] = useState<TaskAction | null>(null)
  const [quotaResult, setQuotaResult] = useState<{ connectionId: string; quota: QuotaVerdictDto | null; error: string | null } | null>(null)
  const quota = quotaResult?.connectionId === item.connection_id ? quotaResult.quota : null
  const quotaError = quotaResult?.connectionId === item.connection_id ? quotaResult.error : null
  const accountName = account?.name ?? run?.account ?? "所选账号"
  const sessionId = run?.session_id ?? last?.session_id ?? item.session_id

  useEffect(() => {
    if (!isTauri) {
      setQuotaResult({ connectionId: item.connection_id, quota: demoQuota(Date.now()), error: null })
      return
    }
    return observeActiveRequest(
      automationApi.quota(item.connection_id, false),
      (value) => setQuotaResult({ connectionId: item.connection_id, quota: value, error: null }),
      (reason) => setQuotaResult({ connectionId: item.connection_id, quota: null, error: String(reason) }),
    )
  }, [item.connection_id, run?.last_check_at])

  const act = async (action: TaskAction) => {
    setBusy(action)
    try {
      await onAction(action)
    } finally {
      setBusy(null)
    }
  }
  const banner = run && (run.status === "waiting" || run.status === "attention") ? blockBanner(run, now) : null
  const pauseEvent = [...events].reverse().find((e) => e.kind === "pause")

  return (
    <>
      <div className="flex min-h-12 items-center gap-2.5">
        <div className="flex min-w-0 flex-1 flex-col gap-[5px]">
          <h2 className="truncate text-[22px] font-semibold text-jolu-text">{item.name}</h2>
          <span className="truncate text-[11px] text-jolu-muted">任务 {taskCode(item.id)} · {projectOf(item.cwd)} · 创建于{dayTime(item.created_at, now)}</span>
        </div>
        <JoluButton size="sm" variant="neutral" onClick={onBack}><ArrowLeft aria-hidden />返回任务</JoluButton>
        <TaskMenu
          item={item}
          now={now}
          onAction={(a) => void act(a)}
          onEdit={onEdit}
          onQueue={() => document.getElementById("automation-queue")?.scrollIntoView({ behavior: "smooth", block: "center" })}
          onCancel={onCancel}
          onDelete={onDelete}
          trigger={
            <button type="button" aria-label="任务更多操作" className="grid size-[30px] place-items-center rounded-lg bg-jolu-side text-jolu-muted outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent">
              <Ellipsis aria-hidden className="size-4" />
            </button>
          }
        />
      </div>

      {banner && (
        <Notice icon={banner.danger ? CircleAlert : CircleCheck} tone={banner.danger ? "danger" : "selected"} title={banner.title}>{banner.body}</Notice>
      )}

      <div className="flex items-start gap-4">
        <div className="flex w-[640px] min-w-0 shrink flex-col gap-3.5">
          <Card className="flex flex-col gap-3.5 p-5">
            <StatusCard item={detail.item} now={now} busy={busy} onAction={act} onCancel={onCancel} onOpenRun={onOpenRun} onOpenProviders={onOpenProviders} onEdit={onEdit} statusLabel={status.label} statusTone={status.tone} statusNext={status.next} />
          </Card>

          <Card className="flex flex-col gap-3 p-4">
            <SectionTitle title="继续执行的位置" note={sessionId ? "已关联原会话" : "尚未开始"} />
            <KeyValue label="执行工具 / 账号" value={`${toolName(item.platform)} / ${accountName}`} />
            <KeyValue label="项目" value={item.cwd} title={item.cwd} />
            {run?.started_at || last?.started_at || item.session_id ? (
              <>
                <KeyValue label="原会话" value={sessionId ? shortId(sessionId) : "新建会话（开始执行后关联）"} title={sessionId ?? undefined} />
                {pauseEvent && <KeyValue label="中断原因" value={pauseEvent.detail ?? pauseEvent.title} title={pauseEvent.detail ?? undefined} />}
                <Notice title={item.auto_resume ? "恢复后继续当前会话" : "未开启自动续跑"}>
                  {item.auto_resume
                    ? "保留已经完成的文件修改。执行器确认可以续跑后，从原会话继续；不支持时提示手动处理。"
                    : "执行中额度耗尽时结束本次运行，保留记录；可在编辑任务中开启「等待恢复后自动继续」。"}
                </Notice>
              </>
            ) : (
              <span className="text-[11px] text-jolu-muted">尚未开始：首次执行后显示原会话与中断位置。</span>
            )}
          </Card>

          <Card className="flex flex-col gap-2 p-4">
            <SectionTitle title="执行内容" />
            <p className="whitespace-pre-wrap text-xs leading-[1.6] text-jolu-text">{item.prompt}</p>
            {sessionId && (
              <JoluButton size="sm" variant="soft" className="self-start" onClick={() => onOpenSession(item.platform, sessionId)}>查看关联会话<ChevronRight aria-hidden /></JoluButton>
            )}
          </Card>
        </div>

        <div className="flex min-w-0 flex-1 flex-col gap-3.5">
          <Card className="flex flex-col gap-4 p-4">
            <div className="flex items-center gap-2.5">
              <PlatformLogo platform={item.platform as PlatformId} size={22} />
              <div className="flex min-w-0 flex-col gap-1">
                <span className="truncate text-sm font-semibold text-jolu-text">{accountName}</span>
                <span className="truncate text-[10px] text-jolu-muted">{account ? accountText(account.name, account.kind) : "账号已移除"}</span>
              </div>
            </div>
            {quota === null ? (
              <span className="flex items-center gap-1.5 text-[11px] text-jolu-muted">
                {quotaError ? <>额度读取失败：{quotaError}</> : <><LoaderCircle aria-hidden className="size-3.5 animate-spin" />正在读取额度…</>}
              </span>
            ) : quota.windows.length ? quota.windows.slice(0, 3).map((w) => (
              <QuotaBar
                key={w.key}
                label={`${windowLabel(w)}已用`}
                value={`${w.used_percent === null ? "—" : `${Math.round(w.used_percent)}%`} · ${w.blocking ? "当前阻塞" : "可用"}`}
                note={w.resets_at ? `预计${dayTime(w.resets_at, now)} ${w.blocking ? "恢复" : "重置"}` : undefined}
                percent={w.used_percent}
                blocking={w.blocking}
              />
            )) : (
              <span className="text-[11px] text-jolu-muted">{quota.reason ?? "该账号没有可显示的额度窗口"}</span>
            )}
            <JoluButton size="sm" variant="soft" className="self-start" onClick={onOpenProviders}>查看账号额度<ChevronRight aria-hidden /></JoluButton>
          </Card>

          <Card className="flex flex-col gap-3 p-4">
            <div id="automation-queue" className="contents" />
            <SectionTitle title={`${accountName} 的队列`} note={`${queue.length} 个任务`} />
            {queue.length ? queue.map((q, index) => (
              <span key={`${q.task_id}-${index}`} className={cn("truncate text-xs", q.task_id === item.id ? "text-jolu-on" : "text-jolu-text")}>
                {String(index + 1).padStart(2, "0")}  {q.name} · {q.status === "running" ? "执行中" : q.status === "waiting" ? "等待额度" : "排队中"}
              </span>
            )) : <span className="text-xs text-jolu-muted">当前账号没有排队的任务</span>}
            <span className="text-[10px] text-jolu-muted">同一账号依次执行。额度恢复后，先继续当前任务。</span>
          </Card>

          <Card className="flex flex-col gap-3 p-4">
            <SectionTitle title="等待策略" />
            <KeyValue label="启动方式" value={launchLabel(item)} />
            <KeyValue label="等待截止" value={item.wait_deadline ? dayTime(item.wait_deadline, now) : "不限制"} />
            <KeyValue label="恢复方式" value={item.auto_resume ? "自动检查后续跑" : "不自动续跑"} />
            <KeyValue label="重复运行" value="禁止重叠执行" />
            <span className="text-[10px] text-jolu-muted">电脑休眠期间不检查；唤醒或重启 Jolu 后重新检查可用额度。</span>
          </Card>
        </div>
      </div>
      <p className="text-[10px] text-jolu-muted">新建且从未执行的任务显示「尚未开始」，不展示中断步骤与原会话；实际续跑能力按执行工具开放。</p>
    </>
  )
}

function StatusCard({ item, now, busy, onAction, onCancel, onOpenRun, onOpenProviders, onEdit, statusLabel, statusTone, statusNext }: {
  item: TaskDetailDto["item"]
  now: number
  busy: TaskAction | null
  onAction: (action: TaskAction) => void
  onCancel: () => void
  onOpenRun: (runId: number) => void
  onOpenProviders: () => void
  onEdit: () => void
  statusLabel: string
  statusTone: "on" | "text" | "danger" | "muted"
  statusNext: string
}) {
  const run = item.active
  const spin = (action: TaskAction) => busy === action
  const header = (icon: React.ReactNode, title: string) => (
    <div className="flex items-center gap-2.5">
      {icon}
      <span className="text-[13px] text-jolu-muted">{title}</span>
      <span className="flex-1" />
      <span className={cn("inline-flex h-[22px] items-center rounded-[5px] px-2 text-[10px]", statusTone === "danger" ? "bg-jolu-danger-soft text-jolu-danger" : "bg-jolu-selected text-jolu-on")}>{statusLabel}</span>
    </div>
  )
  const cancelButton = item.state !== "cancelled" && (
    <JoluButton size="sm" variant="danger-soft" disabled={!!busy} onClick={onCancel}><X aria-hidden />取消任务</JoluButton>
  )
  const pauseButton = item.state === "paused"
    ? <JoluButton size="sm" variant="neutral" disabled={!!busy} onClick={() => onAction("resume")}><Play aria-hidden />恢复任务</JoluButton>
    : item.state === "active" && <JoluButton size="sm" variant="neutral" disabled={!!busy} onClick={() => onAction("pause")}><Pause aria-hidden />{run?.status === "waiting" ? "暂停等待" : "暂停任务"}</JoluButton>

  if (run?.status === "waiting") {
    const checking = spin("check")
    return (
      <>
        {header(<Clock3 aria-hidden className="size-[22px] text-jolu-on" />, run.resets_at ? "预计恢复" : "恢复时间")}
        <span className="tnum text-[30px] font-semibold leading-none text-jolu-text">{run.resets_at ? dayTime(run.resets_at, now) : "未知"}</span>
        <span className="text-[11px] text-jolu-muted">{run.resets_at ? `${untilText(run.resets_at, now)} · 预计时间不代表已可执行` : "暂未获取重置时间，任务保持等待"}</span>
        <div className="flex gap-5 text-[11px] text-jolu-muted">
          <span>上次检查 {run.last_check_at ? dayTime(run.last_check_at, now).replace("今天 ", "") : "—"}</span>
          <span>下次检查 {run.next_check_at ? dayTime(run.next_check_at, now).replace("今天 ", "") : "—"}</span>
        </div>
        <div className="flex gap-2">
          <JoluButton size="sm" variant="primary" disabled={!!busy || item.state === "paused"} aria-busy={checking} onClick={() => onAction("check")}>
            <RefreshCw aria-hidden className={cn(checking && "animate-spin")} />{checking ? "检查中…" : "立即检查"}
          </JoluButton>
          {pauseButton}
          {cancelButton}
        </div>
      </>
    )
  }
  if (run?.status === "running") {
    return (
      <>
        {header(<LoaderCircle aria-hidden className="size-[22px] animate-spin text-jolu-on" />, "正在执行")}
        <span className="tnum text-[30px] font-semibold leading-none text-jolu-text">{clock(runningMs(run, now), false)}</span>
        <span className="text-[11px] text-jolu-muted">实际执行时长，不含等待额度的时间</span>
        <div className="flex gap-2">
          <JoluButton size="sm" variant="soft" onClick={() => onOpenRun(run.id)}>查看日志<ChevronRight aria-hidden /></JoluButton>
          <JoluButton size="sm" variant="danger-soft" disabled={!!busy} onClick={() => onAction("stop")}><Square aria-hidden />停止运行</JoluButton>
        </div>
      </>
    )
  }
  if (run?.status === "queued") {
    return (
      <>
        {header(<Clock3 aria-hidden className="size-[22px] text-jolu-on" />, "排队中")}
        <span className="text-[30px] font-semibold leading-none text-jolu-text">{item.queue_ahead ? `前方 ${item.queue_ahead} 个任务` : "即将开始"}</span>
        <span className="text-[11px] text-jolu-muted">同一账号按队列顺序执行，前一个任务结束后自动开始。</span>
        <div className="flex gap-2">{pauseButton}{cancelButton}</div>
      </>
    )
  }
  if (run?.status === "attention") {
    const reauth = run.block_kind === "reauth"
    const accountIssue = run.block_kind === "account_mismatch"
    return (
      <>
        {header(<CircleAlert aria-hidden className="size-[22px] text-jolu-danger" />, "需要处理")}
        <span className="text-xs leading-[1.6] text-jolu-text">{run.reason ?? "查看详情后处理"}</span>
        <div className="flex flex-wrap gap-2">
          {reauth || accountIssue ? (
            <JoluButton size="sm" variant="soft" onClick={onOpenProviders}>{reauth ? <><LogIn aria-hidden />重新登录</> : <>前往供应商<ChevronRight aria-hidden /></>}</JoluButton>
          ) : null}
          <JoluButton size="sm" variant="primary" disabled={!!busy || item.state !== "active"} onClick={() => onAction("retry")}><RotateCcw aria-hidden />重新排队</JoluButton>
          {cancelButton}
        </div>
      </>
    )
  }
  if (item.launch === "schedule" && item.state === "active" && item.next_run_at) {
    return (
      <>
        {header(<Clock3 aria-hidden className="size-[22px] text-jolu-on" />, "下次运行")}
        <span className="tnum text-[30px] font-semibold leading-none text-jolu-text">{dayTime(item.next_run_at, now)}</span>
        <span className="text-[11px] text-jolu-muted">{launchLabel(item)} · {zoneName(localZone())}时间 · 仍在执行时跳过本次触发</span>
        <div className="flex gap-2">
          <JoluButton size="sm" variant="primary" disabled={!!busy} onClick={() => onAction("run")}><Play aria-hidden />立即运行</JoluButton>
          {pauseButton}
          {cancelButton}
        </div>
      </>
    )
  }
  const last = item.last
  return (
    <>
      {header(<Clock3 aria-hidden className="size-[22px] text-jolu-muted" />, last ? "最近运行" : "尚未开始")}
      <span className={cn("text-[30px] font-semibold leading-none", TONE[statusTone])}>{statusLabel}</span>
      <span className="text-[11px] text-jolu-muted">{statusNext}</span>
      <div className="flex flex-wrap gap-2">
        {last && <JoluButton size="sm" variant="soft" onClick={() => onOpenRun(last.id)}>查看运行记录<ChevronRight aria-hidden /></JoluButton>}
        {item.state === "active" || item.state === "finished" ? (
          <JoluButton size="sm" variant="primary" disabled={!!busy} onClick={() => onAction(item.missed_at ? "run" : last ? "retry" : "run")}><Play aria-hidden />{item.missed_at ? "补跑" : last ? "再次运行" : "立即运行"}</JoluButton>
        ) : null}
        {item.state === "paused" && pauseButton}
        {item.state !== "cancelled" && <JoluButton size="sm" variant="neutral" onClick={onEdit}>编辑任务</JoluButton>}
        {cancelButton}
      </div>
    </>
  )
}
