import { useState } from "react"
import { Bot, ChevronRight, Plus, RotateCcw, Search, X } from "lucide-react"
import { cn } from "@/lib/utils"
import { accountText, countByFilter, dayTime, filterTasks, launchLabel, projectOf, taskStatus, toolName, type TaskFilter } from "@/lib/automation"
import type { TaskAction, TaskItemDto } from "@/lib/automationApi"
import { zoneName, localZone } from "@/lib/usageStats"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { JoluButton } from "@/components/ui/JoluButton"
import { Card, MenuFilter, MetricCard, ToolButton } from "@/components/usage/usageControls"
import { TONE, TaskMenu } from "./automationParts"
import type { PlatformId } from "@/types"

const FILTERS: { value: TaskFilter; label: string }[] = [
  { value: "running", label: "执行中" },
  { value: "waiting", label: "等待额度" },
  { value: "paused", label: "已暂停" },
  { value: "done", label: "已完成" },
]

export interface AccountRef {
  name: string
  kind: string
}

/** 任务页签（画板 58）：四项指标、状态 / 账号筛选与搜索、任务表格 */
export function TaskList({ data, now, accounts, onOpen, onCreate, onEdit, onAction, onCancel }: {
  data: { tasks: TaskItemDto[]; enabled: number; running: number; running_note: string | null; waiting: number; next_reset: number | null; done_today: number; failed_today: number }
  now: number
  accounts: Map<string, AccountRef>
  onOpen: (item: TaskItemDto, target: "task" | "run") => void
  onCreate: () => void
  onEdit: (item: TaskItemDto) => void
  onAction: (item: TaskItemDto, action: TaskAction) => void
  onCancel: (item: TaskItemDto) => void
}) {
  const [filter, setFilter] = useState<TaskFilter>("all")
  const [account, setAccount] = useState<string>("__all__")
  const [searchOpen, setSearchOpen] = useState(false)
  const [search, setSearch] = useState("")
  const zone = zoneName(localZone())
  const counts = countByFilter(data.tasks, now)
  const rows = filterTasks(data.tasks, filter, account === "__all__" ? null : account, search, now)
  const accountIds = [...new Set(data.tasks.map((t) => t.connection_id))]
  const filtered = filter !== "all" || account !== "__all__" || !!search.trim()
  const clear = () => {
    setFilter("all")
    setAccount("__all__")
    setSearch("")
    setSearchOpen(false)
  }

  return (
    <>
      <div className="grid grid-cols-4 gap-3">
        <MetricCard label="已启用任务" value={String(data.enabled)} note="包含定时与额度恢复任务" />
        <MetricCard label="执行中" value={String(data.running)} note={data.running_note ?? "当前没有执行中的任务"} />
        <MetricCard label="等待额度" value={String(data.waiting)} note={data.next_reset ? `最近预计恢复 ${dayTime(data.next_reset, now).replace("今天 ", "")}` : data.waiting ? "恢复时间未知" : "没有等待中的任务"} />
        <MetricCard label="今日完成" value={String(data.done_today)} note={data.failed_today ? `另有 ${data.failed_today} 次运行失败` : "今天没有失败的运行"} />
      </div>

      {data.tasks.length === 0 ? (
        <Card className="flex flex-col items-start gap-3 p-5">
          <Bot aria-hidden className="size-[26px] text-jolu-muted" />
          <span className="text-[13px] font-semibold text-jolu-text">还没有自动化任务</span>
          <span className="text-[11px] text-jolu-muted">创建一个任务，让 AI 在指定时间或额度恢复后开始工作。</span>
          <JoluButton variant="primary" size="sm" onClick={onCreate}><Plus aria-hidden />新建任务</JoluButton>
        </Card>
      ) : (
        <>
          <div className="flex h-[34px] items-center gap-1.5">
            {[{ value: "all" as const, label: `全部 ${counts.all}` }, ...FILTERS].map((f) => (
              <button
                key={f.value}
                type="button"
                aria-pressed={filter === f.value}
                onClick={() => setFilter(f.value)}
                className={cn(
                  "inline-flex h-[34px] items-center rounded-lg border px-3 text-xs text-jolu-text outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
                  filter === f.value ? "border-jolu-selected bg-jolu-selected" : "border-jolu-border bg-jolu-bg hover:bg-jolu-stage",
                )}
              >
                {f.label}
              </button>
            ))}
            <span className="flex-1" />
            <MenuFilter
              label="账号筛选"
              value={account}
              onChange={setAccount}
              options={[{ value: "__all__", label: "全部账号" }, ...accountIds.map((id) => ({ value: id, label: accounts.get(id)?.name ?? "未知账号" }))]}
            />
            {searchOpen ? (
              <label className="flex h-[34px] w-[220px] items-center gap-2 rounded-lg border border-jolu-accent bg-jolu-bg px-2.5">
                <Search aria-hidden className="size-4 text-jolu-muted" />
                <input
                  autoFocus
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  onKeyDown={(e) => { if (e.key === "Escape") { setSearch(""); setSearchOpen(false) } }}
                  placeholder="搜索任务或项目"
                  aria-label="搜索任务"
                  className="min-w-0 flex-1 bg-transparent text-xs text-jolu-text outline-none placeholder:text-jolu-muted"
                />
                <button type="button" aria-label="关闭搜索" onClick={() => { setSearch(""); setSearchOpen(false) }} className="text-jolu-muted hover:text-jolu-text"><X aria-hidden className="size-3.5" /></button>
              </label>
            ) : (
              <button type="button" aria-label="搜索任务" onClick={() => setSearchOpen(true)} className="grid size-[30px] place-items-center rounded-[7px] text-jolu-muted outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
                <Search aria-hidden className="size-4" />
              </button>
            )}
          </div>

          <Card className="overflow-hidden">
            <div className="flex h-11 items-center bg-jolu-side px-4 text-[11px] font-medium text-jolu-muted">
              <span className="w-[280px] shrink-0">任务 / 项目</span>
              <span className="w-[180px] shrink-0">执行工具 / 账号</span>
              <span className="min-w-0 flex-1">启动方式</span>
              <span className="w-[180px] shrink-0">状态 / 下一步</span>
              <span className="w-[176px] shrink-0">操作</span>
            </div>
            {rows.length ? rows.map((item) => {
              const status = taskStatus(item, now, zone)
              const acc = accounts.get(item.connection_id)
              return (
                <div key={item.id} className="flex h-[76px] items-center border-t border-jolu-border px-4 transition-colors hover:bg-jolu-stage">
                  <div className="flex w-[280px] shrink-0 flex-col gap-[7px] pr-4">
                    <span className="truncate text-[13px] font-medium text-jolu-text">{item.name}</span>
                    <span className="truncate text-[10px] text-jolu-muted" title={item.cwd}>{projectOf(item.cwd)}</span>
                  </div>
                  <div className="flex w-[180px] shrink-0 flex-col gap-[7px] pr-3">
                    <span className="flex items-center gap-[7px] text-[11px] text-jolu-text"><PlatformLogo platform={item.platform as PlatformId} size={16} />{toolName(item.platform)}</span>
                    <span className="truncate text-[10px] text-jolu-muted">{acc ? accountText(acc.name, acc.kind) : "账号已移除"}</span>
                  </div>
                  <span className="min-w-0 flex-1 truncate pr-3 text-[11px] text-jolu-text">{launchLabel(item)}</span>
                  <div className="flex w-[180px] shrink-0 flex-col gap-[7px] pr-3">
                    <span className={cn("text-xs font-medium", TONE[status.tone])}>{status.label}</span>
                    <span className="truncate text-[10px] text-jolu-muted" title={status.next}>{status.next}</span>
                  </div>
                  <div className="flex w-[176px] shrink-0 items-center gap-2">
                    <button
                      type="button"
                      onClick={() => onOpen(item, status.action === "查看运行" ? "run" : "task")}
                      className="inline-flex h-8 w-[104px] items-center justify-center gap-1.5 rounded-lg bg-jolu-selected px-2.5 text-xs font-medium text-jolu-on outline-none hover:bg-jolu-pressed focus-visible:outline-2 focus-visible:outline-jolu-accent"
                    >
                      {status.action}
                      <ChevronRight aria-hidden className="size-3.5" />
                    </button>
                    <TaskMenu
                      item={item}
                      now={now}
                      onAction={(action) => onAction(item, action)}
                      onEdit={() => onEdit(item)}
                      onQueue={() => onOpen(item, "task")}
                      onCancel={() => onCancel(item)}
                    />
                  </div>
                </div>
              )
            }) : (
              <div className="flex items-center gap-3 border-t border-jolu-border px-4 py-5 text-[11px] text-jolu-muted">
                筛选无结果：没有匹配的任务
                {filtered && <ToolButton icon={<RotateCcw />} onClick={clear}>清除筛选</ToolButton>}
              </div>
            )}
            <div className="flex h-11 items-center gap-2.5 border-t border-jolu-border px-4 text-[11px] text-jolu-muted">
              <span>{filtered ? `匹配 ${rows.length} / 共 ${data.tasks.length} 个任务` : `共 ${data.tasks.length} 个任务`}</span>
              <span className="flex-1" />
              <span>按最近更新排序</span>
            </div>
          </Card>
        </>
      )}
    </>
  )
}
