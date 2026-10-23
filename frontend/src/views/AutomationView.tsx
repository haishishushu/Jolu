import { ContentTransition } from "@/components/ui/LoadingSurface"
import { useEffect, useMemo, useState } from "react"
import { Plus } from "lucide-react"
import { isTauri, type SessionItemDto } from "@/lib/api"
import { automationApi, type RunFilterDto, type TaskAction, type TaskInputDto, type TaskItemDto } from "@/lib/automationApi"
import { useConnections } from "@/hooks/useConnections"
import { useAutomationQuery, useNow } from "@/hooks/useAutomationQuery"
import { demoAccounts, demoOverview, demoRunDetail, demoRuns, demoTaskDetail } from "@/mock/automation"
import { useToast } from "@/components/ui/Toast"
import { JoluButton } from "@/components/ui/JoluButton"
import { ErrorNotice, LoadingCard } from "@/components/usage/usageControls"
import { AutomationTabs } from "@/components/automation/automationParts"
import { TaskList, type AccountRef } from "@/components/automation/TaskList"
import { TaskDetail } from "@/components/automation/TaskDetail"
import { RunList } from "@/components/automation/RunList"
import { RunDetail } from "@/components/automation/RunDetail"
import { TaskDialog, type TaskDialogRequest } from "@/components/automation/TaskDialog"
import { CancelTaskDialog } from "@/components/automation/CancelTaskDialog"
import { DeleteTaskDialog } from "@/components/automation/DeleteTaskDialog"

/** 会话线程「创建自动化」带入的会话（画板 72） */
export interface AutomationPrefill {
  session: SessionItemDto
  nonce: number
}

/** 点击任务通知（任务 2.5）：打开该任务的详情。当前插件版本不支持 Windows 点击回调，暂由后续升级接上 */
export interface AutomationTarget {
  taskId: number
  nonce: number
}

type Route = { kind: "list" } | { kind: "task"; id: number } | { kind: "run"; id: number; back: Route }

const ACTION_DONE: Partial<Record<TaskAction, string>> = {
  pause: "已暂停任务",
  resume: "已恢复任务",
  stop: "已发送停止请求",
  run: "已加入执行队列",
  retry: "已重新排队",
  check: "已检查额度",
}

/**
 * 自动化（Jolu.pen 模块 E，画板 58–73）：任务列表、新建 / 编辑任务、等待额度详情、运行记录与运行详情。
 * 任务在本机用 Claude Code / Codex 的非交互模式执行；所选 Skills 在执行前检查并写入提示词前缀（D-08）。
 */
export function AutomationView({ active, prefill, target, onTargetHandled, onOpenSession, onOpenProviders }: {
  active: boolean
  prefill: AutomationPrefill | null
  target: AutomationTarget | null
  /** 已按通知打开任务详情；页面切走再回来时不再重复打开 */
  onTargetHandled: () => void
  onOpenSession: (platform: string, sessionId: string) => void
  onOpenProviders: () => void
}) {
  const toast = useToast()
  const conn = useConnections()
  const [tab, setTab] = useState<"tasks" | "runs">("tasks")
  const [route, setRoute] = useState<Route>({ kind: "list" })
  const [dialog, setDialog] = useState<TaskDialogRequest | null>(null)
  const [cancelling, setCancelling] = useState<TaskItemDto | null>(null)
  const [deleting, setDeleting] = useState<TaskItemDto | null>(null)
  const [runFilter, setRunFilter] = useState<RunFilterDto>({ range: "today", status: null, connection: null, task: null })
  const [runPage, setRunPage] = useState(1)
  const [stopping, setStopping] = useState(false)

  useEffect(() => {
    if (!prefill) return
    setRoute({ kind: "list" })
    setTab("tasks")
    setDialog({ mode: "create", session: prefill.session, nonce: prefill.nonce })
  }, [prefill])

  useEffect(() => {
    if (!target) return
    setTab("tasks")
    setRoute({ kind: "task", id: target.taskId })
    onTargetHandled()
  }, [target, onTargetHandled])

  const overview = useAutomationQuery("overview", automationApi.overview, () => demoOverview(Date.now()), active)
  const taskId = route.kind === "task" ? route.id : null
  const task = useAutomationQuery(`task-${taskId}`, () => automationApi.task(taskId ?? 0), () => demoTaskDetail(taskId ?? 24, Date.now()), active && taskId !== null, 15_000)
  const runs = useAutomationQuery(JSON.stringify([runFilter, runPage]), () => automationApi.runs(runFilter, runPage), () => demoRuns(runPage, Date.now()), active && tab === "runs" && route.kind === "list")
  const runId = route.kind === "run" ? route.id : null
  // 执行中的运行定时刷新日志；状态变化另由 automation-changed 事件触发
  const [runPoll, setRunPoll] = useState(0)
  const run = useAutomationQuery(`run-${runId}`, () => automationApi.run(runId ?? 0), () => demoRunDetail(runId ?? 91, Date.now()), active && runId !== null, runPoll)
  const runStatus = run.data?.run.status
  useEffect(() => setRunPoll(runStatus === "running" ? 3_000 : 0), [runStatus])
  const ticking = !!overview.data?.running || !!overview.data?.waiting || run.data?.run.status === "running"
  const now = useNow(1000, active && ticking)

  const accounts = useMemo(() => {
    const map = new Map<string, AccountRef>()
    for (const c of conn.connections) map.set(c.id, { name: c.baseName, kind: c.kind })
    if (!isTauri) for (const p of ["claude", "codex"] as const) for (const a of demoAccounts(p)) map.set(a.id, { name: a.name, kind: a.kind })
    return map
  }, [conn.connections])

  const refreshAll = () => {
    overview.revalidate()
    if (taskId !== null) task.revalidate()
    if (runId !== null) run.revalidate()
    if (tab === "runs") runs.revalidate()
  }

  const doAction = async (id: number, action: TaskAction) => {
    if (!isTauri) {
      toast.info(ACTION_DONE[action] ?? "已完成", "浏览器预览不执行实际操作")
      return
    }
    try {
      await automationApi.action(id, action)
      if (ACTION_DONE[action]) toast.success(ACTION_DONE[action]!)
      refreshAll()
    } catch (reason) {
      toast.danger("操作失败", reason instanceof Error ? reason.message : String(reason))
    }
  }

  const openTask = (item: TaskItemDto, target: "task" | "run") => {
    if (target === "run" && item.active) setRoute({ kind: "run", id: item.active.id, back: { kind: "list" } })
    else setRoute({ kind: "task", id: item.id })
  }

  const editTask = (item: TaskItemDto) => setDialog({ mode: "edit", task: item, nonce: Date.now() })

  const onSaved = (_id: number, launch: TaskInputDto["launch"]) => {
    const editing = dialog?.mode === "edit"
    setDialog(null)
    toast.success(editing ? "已保存修改" : launch === "quota" ? "已创建任务，等待额度恢复" : launch === "schedule" ? "已创建定时任务" : "已创建任务，加入执行队列", editing ? "编辑仅影响后续执行" : undefined)
    refreshAll()
  }

  const header = (title: string, note: string) => (
    <div className="flex min-h-12 items-center gap-2.5">
      <div className="flex min-w-0 flex-1 flex-col gap-[5px]">
        <h2 className="truncate text-[22px] font-semibold text-jolu-text">{title}</h2>
        <span className="truncate text-[11px] text-jolu-muted">{note}</span>
      </div>
      <JoluButton size="sm" variant="primary" onClick={() => setDialog({ mode: "create", nonce: Date.now() })}><Plus aria-hidden />新建任务</JoluButton>
    </div>
  )

  let body: React.ReactNode
  if (route.kind === "task") {
    body = task.error && !task.data ? <ErrorNotice error={task.error} stale={false} onRetry={task.refresh} />
      : !task.data ? <LoadingCard title="正在读取任务" layout="cards" />
      : (
        <TaskDetail
          detail={task.data}
          now={now}
          account={accounts.get(task.data.item.connection_id)}
          onBack={() => setRoute({ kind: "list" })}
          onAction={(action) => doAction(task.data!.item.id, action)}
          onEdit={() => editTask(task.data!.item)}
          onCancel={() => setCancelling(task.data!.item)}
          onDelete={() => setDeleting(task.data!.item)}
          onOpenRun={(id) => setRoute({ kind: "run", id, back: route })}
          onOpenSession={onOpenSession}
          onOpenProviders={onOpenProviders}
        />
      )
  } else if (route.kind === "run") {
    const back = route.back
    body = run.error && !run.data ? <ErrorNotice error={run.error} stale={false} onRetry={run.refresh} />
      : !run.data ? <LoadingCard title="正在读取运行记录" />
      : (
        <RunDetail
          detail={run.data}
          now={now}
          account={accounts.get(run.data.run.connection_id)}
          onBack={() => setRoute(back)}
          onOpenSession={onOpenSession}
          stopping={stopping}
          onStop={async () => {
            setStopping(true)
            await doAction(run.data!.run.task_id, "stop")
            setStopping(false)
          }}
        />
      )
  } else {
    body = (
      <>
        {tab === "tasks"
          ? header("把重复工作交给 AI", "任务按所选账号执行，额度不足时保留进度并等待。")
          : header("每一次运行，都有记录", "等待额度和恢复执行归属同一次运行，展开详情查看全过程。")}
        <AutomationTabs value={tab} onChange={setTab} />
        {tab === "tasks" ? (
          <>
            {overview.error && <ErrorNotice error={overview.error} stale={!!overview.data} onRetry={overview.refresh} />}
            {overview.data ? (
              <TaskList
                data={overview.data}
                now={now}
                accounts={accounts}
                onOpen={openTask}
                onCreate={() => setDialog({ mode: "create", nonce: Date.now() })}
                onEdit={editTask}
                onAction={(item, action) => void doAction(item.id, action)}
                onCancel={setCancelling}
                onDelete={setDeleting}
              />
            ) : !overview.error && <LoadingCard title="正在读取任务" layout="tasks" />}
            <p className="text-[10px] text-jolu-muted">{isTauri ? "" : "示例数据 · "}本地任务需要 Jolu 后台运行；休眠与关机期间停止调度，重新打开后检查待执行任务。</p>
          </>
        ) : (
          <>
            {runs.error && <ErrorNotice error={runs.error} stale={!!runs.data} onRetry={runs.refresh} />}
            {runs.data ? (
              <RunList
                data={runs.data}
                filter={runFilter}
                page={runPage}
                now={now}
                loading={runs.refreshing}
                accounts={accounts}
                onFilter={(f) => { setRunFilter(f); setRunPage(1) }}
                onPage={setRunPage}
                onRefresh={runs.refresh}
                onOpen={(id) => setRoute({ kind: "run", id, back: { kind: "list" } })}
              />
            ) : !runs.error && <LoadingCard title="正在读取运行记录" />}
          </>
        )}
      </>
    )
  }

  return (
    <div className="flex flex-col gap-3.5">
      <ContentTransition identity={`${tab}-${route.kind}-${route.kind === "task" ? taskId : route.kind === "run" ? runId : "list"}-${route.kind === "task" ? !!task.data : route.kind === "run" ? !!run.data : tab === "tasks" ? !!overview.data : !!runs.data}`} className="flex flex-col gap-3.5">{body}</ContentTransition>
      {dialog && (
        <TaskDialog
          key={dialog.nonce}
          request={dialog}
          onClose={() => setDialog(null)}
          onSaved={onSaved}
          onBackToSession={(session) => {
            setDialog(null)
            onOpenSession(session.platform, session.session_id)
          }}
        />
      )}
      {cancelling && (
        <CancelTaskDialog
          name={cancelling.name}
          running={cancelling.active?.status === "running"}
          onClose={() => setCancelling(null)}
          onConfirm={async () => {
            if (isTauri) await automationApi.action(cancelling.id, "cancel")
            toast.success("已取消任务", "保留运行记录和已经完成的文件修改")
            refreshAll()
          }}
        />
      )}
      {deleting && (
        <DeleteTaskDialog
          name={deleting.name}
          onClose={() => setDeleting(null)}
          onConfirm={async () => {
            if (!isTauri) {
              toast.info("已删除任务", "浏览器预览不执行实际操作")
              return
            }
            // 失败时抛出，由弹窗显示原因；成功后关闭弹窗和任务详情并刷新列表（画板 68 第 10 格），不另弹提示
            await automationApi.deleteTask(deleting.id)
            if (taskId === deleting.id) setRoute({ kind: "list" })
            overview.revalidate()
            if (tab === "runs") runs.revalidate()
          }}
        />
      )}
    </div>
  )
}
