import { useEffect, useMemo, useState } from "react"
import * as Menu from "@radix-ui/react-dropdown-menu"
import { ArrowDown, ArrowUp, BadgeCheck, FolderInput, FolderMinus, KeyRound, List, MoreHorizontal, Pencil, Pin, PinOff, Plus, Search, Trash2 } from "lucide-react"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import {
  gatewayApi, type CatalogModelDto, type GatewayGroupDto, type GatewayGroupsDto, type GroupKind,
  type SourceCapabilityDto, type SourceUsageDto, type WiringStatusDto,
} from "@/lib/gatewayApi"
import { agentLabel, wiringLine } from "@/lib/gatewayPage"
import { credentialLabel } from "@/lib/gatewaySessions"
import {
  batchTargets, capabilityLine, filterSources, inScope, pruneSelection, serveCounts, usageLine, type GroupScope, type ServeFilter,
} from "@/lib/gatewaySources"
import { ConfirmDialog } from "@/components/ui/ConfirmDialog"
import { useToast } from "@/components/ui/Toast"
import { Button, Modal, orderGroups } from "./GatewayDialogs"
import { StatusCapsule } from "./StatusCapsule"
import type { Connection } from "@/types"

/**
 * 管理模型来源（鼠鼠 2026-11-06 确认「左右两栏大弹窗」；Jolu.pen 暂无画板，外观待确认）。
 *
 * 左栏：来源分组导航（全部 / 各分组 / 未分组 / 新建）与 Agent 分组导航。
 * 右栏：搜索 + 已接入 / 未接入筛选 + 多选批量加入 / 移出 / 移动；每行显示凭据类型、菜单 id、模型数、
 * 调用能力与最近使用，「已接入 / 未接入」胶囊可直接切换，「⋯」里改名与菜单 id、查看模型、移动分组。
 * 分组与顺序只影响排列，不改变路由、接线和 Agent 菜单；切换与批量操作都先确认。
 */

type Scope = GroupScope | { kind: "agents"; id: string | null }
type Confirm = { title: string; description: string; label: string; danger?: boolean; work: () => Promise<boolean | void> }
type NameDialog = { kind: GroupKind; group?: GatewayGroupDto }

const MENU_CLASS = "motion-popover z-[100] min-w-[176px] rounded-xl border border-jolu-border bg-jolu-cc-surface p-1.5 shadow-popover"
const ITEM_CLASS = "flex h-8 cursor-default select-none items-center gap-2 rounded-md px-2 text-[11px] text-jolu-text outline-none data-[highlighted]:bg-jolu-side data-[disabled]:opacity-40 [&_svg]:size-3.5 [&_svg]:text-jolu-muted"
const ICON_BUTTON = "grid size-7 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-stage hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-35"

export function SourceManageDialog({
  connections, capabilities, catalog, groups, agents, pinned, busy, run, onClose, onRename, onShowModels, onTogglePin, onWire, onUnwire, onAccept,
}: {
  connections: Connection[]
  capabilities: SourceCapabilityDto[]
  catalog: CatalogModelDto[]
  groups: GatewayGroupsDto
  agents: WiringStatusDto[]
  pinned: string[]
  busy: boolean
  /** 页面的统一操作入口：加锁、成功提示、刷新数据；失败时提示并返回 false */
  run: (work: () => Promise<unknown>, ok?: string, throwOnFailure?: boolean) => Promise<boolean>
  onClose: () => void
  onRename: (connection: Connection) => void
  onShowModels: (models: CatalogModelDto[]) => void
  onTogglePin: (key: string) => void
  onWire: (agent: string) => void
  onUnwire: (agent: string) => void
  onAccept: (agent: string) => void
}) {
  const toast = useToast()
  const [scope, setScope] = useState<Scope>({ kind: "all" })
  const [query, setQuery] = useState("")
  const [serve, setServe] = useState<ServeFilter>("all")
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [usage, setUsage] = useState<SourceUsageDto[] | null>(null)
  const [usageError, setUsageError] = useState(false)
  const [confirm, setConfirm] = useState<Confirm | null>(null)
  const [nameDialog, setNameDialog] = useState<NameDialog | null>(null)
  const now = Date.now()

  const sourceGroups = orderGroups(groups.sources, pinned, "source")
  const agentGroups = orderGroups(groups.agents, pinned, "agent")
  const groupedAgents = new Set(agentGroups.flatMap((group) => group.members))

  useEffect(() => {
    if (!isTauri) return
    let active = true
    gatewayApi.sourceUsage().then((rows) => { if (active) setUsage(rows) }).catch(() => { if (active) setUsageError(true) })
    return () => { active = false }
  }, [])

  // 分组被删除后回到「全部」
  useEffect(() => {
    if (scope.kind === "group" && !groups.sources.some((group) => group.id === scope.id)) setScope({ kind: "all" })
    if (scope.kind === "agents" && scope.id && !groups.agents.some((group) => group.id === scope.id)) setScope({ kind: "agents", id: null })
  }, [groups, scope])

  const scoped = scope.kind === "agents" ? [] : inScope(connections, sourceGroups, scope)
  const visible = filterSources(scoped, query, serve)
  const counts = serveCounts(scoped)
  const visibleKey = visible.map((source) => source.id).join("|")
  useEffect(() => { setSelected((current) => pruneSelection(current, visible)) }, [visibleKey])
  const allChecked = visible.length > 0 && visible.every((source) => selected.has(source.id))
  const usageOf = useMemo(() => new Map((usage ?? []).map((row) => [row.connection_id, row])), [usage])
  const groupOf = (id: string) => sourceGroups.find((group) => group.members.includes(id))
  const currentGroup = scope.kind === "group" ? sourceGroups.find((group) => group.id === scope.id) : scope.kind === "agents" && scope.id ? agentGroups.find((group) => group.id === scope.id) : undefined
  const groupKind: GroupKind = scope.kind === "agents" ? "agent" : "source"

  /* ── 操作 ── */
  const askServe = (connection: Connection) => {
    const enable = !connection.serve_enabled
    setConfirm({
      title: enable ? `把「${connection.baseName}」加入模型池？` : `把「${connection.baseName}」移出模型池？`,
      description: enable
        ? `加入后，接入网关的工具可以按「${connection.slug ?? "来源"}/模型」调用这个来源；授权与转发能力以实际请求为准。`
        : `移出后，工具再请求「${connection.slug ?? "来源"}/…」会返回不可用；账号、Key 与历史记录不会删除。`,
      label: enable ? "加入模型池" : "移出模型池",
      danger: !enable,
      work: () => run(() => gatewayApi.setServe(connection.id, enable), enable ? `已把 ${connection.baseName} 加入模型池` : `已移出 ${connection.baseName}`, true),
    })
  }

  const askBatch = (enable: boolean) => {
    const ids = batchTargets(visible, selected, enable)
    if (ids.length === 0) {
      toast.info(enable ? "选中的来源都已在模型池中" : "选中的来源都不在模型池中")
      return
    }
    setConfirm({
      title: enable ? `把 ${ids.length} 个来源加入模型池？` : `把 ${ids.length} 个来源移出模型池？`,
      description: enable ? "逐个加入；某个来源失败不影响其他来源，失败的会保持选中以便重试。" : "移出后，工具再请求这些来源会返回不可用；账号、Key 与历史记录不会删除。",
      label: enable ? "批量加入" : "批量移出",
      danger: !enable,
      work: () => run(async () => {
        const results = await gatewayApi.setServeMany(ids, enable)
        const failed = results.filter((item) => !item.ok)
        const okCount = results.length - failed.length
        if (failed.length === 0) {
          toast.success(enable ? `已加入 ${okCount} 个来源` : `已移出 ${okCount} 个来源`)
          setSelected(new Set())
          return
        }
        const names = failed.map((item) => connections.find((c) => c.id === item.connection_id)?.baseName ?? item.connection_id)
        setSelected(new Set(failed.map((item) => item.connection_id)))
        toast.danger(`${okCount} 个成功，${failed.length} 个失败`, `${names.join("、")}：${failed[0].error ?? "未知原因"}`)
      }, undefined, true),
    })
  }

  const moveTo = (ids: string[], groupId: string | null) => {
    const name = groupId ? sourceGroups.find((group) => group.id === groupId)?.name ?? "分组" : "未分组"
    void run(async () => {
      for (const id of ids) await gatewayApi.moveToGroup("source", id, groupId)
      setSelected(new Set())
    }, `已移到「${name}」`)
  }

  const reorder = (kind: GroupKind, list: GatewayGroupDto[], id: string, direction: -1 | 1) => {
    const ids = list.map((group) => group.id)
    const index = ids.indexOf(id)
    const target = index + direction
    if (index < 0 || target < 0 || target >= ids.length) return
    ;[ids[index], ids[target]] = [ids[target], ids[index]]
    void run(() => gatewayApi.reorderGroups(kind, ids), "顺序已保存")
  }

  const askDelete = (kind: GroupKind, group: GatewayGroupDto) => setConfirm({
    title: `删除「${group.name}」？`,
    description: "组里的成员会回到「未分组」；来源、连接和模型都不会被删除。",
    label: "删除分组",
    danger: true,
    work: () => run(() => gatewayApi.deleteGroup(kind, group.id), "已删除分组", true),
  })

  /* ── 左栏 ── */
  const nav = (key: string, active: boolean, label: string, count: number | null, onClick: () => void, extra?: React.ReactNode) => (
    <button key={key} type="button" aria-current={active ? "true" : undefined} onClick={onClick}
      className={cn("flex h-8 w-full items-center gap-2 rounded-lg px-2.5 text-left text-[11.5px] outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
        active ? "bg-jolu-selected font-semibold text-jolu-on" : "text-jolu-text hover:bg-jolu-stage")}>
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {extra}
      {count !== null && <span className={cn("text-[10px] tabular-nums", active ? "text-jolu-on" : "text-jolu-muted")}>{count}</span>}
    </button>
  )
  const addGroupButton = (kind: GroupKind) => (
    <button type="button" disabled={busy} onClick={() => setNameDialog({ kind })}
      className="mt-1 flex h-8 w-full items-center gap-1.5 rounded-lg border border-dashed border-jolu-border px-2.5 text-[11px] text-jolu-muted outline-none hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
      <Plus aria-hidden className="size-3.5" />新建分组
    </button>
  )

  return (
    <>
      <Modal title="管理模型来源" busy={busy} width={880} onClose={onClose}
        footer={<>
          <span className="mr-auto self-center text-[10.5px] text-jolu-muted">「已接入」= 在模型池中；分组与顺序只影响排列，不改变路由、接线和 Agent 菜单。</span>
          <Button label="完成" disabled={busy} onClick={onClose} />
        </>}>
        <div className="-mx-6 flex h-[min(540px,calc(100dvh-240px))] min-h-[320px] border-y border-jolu-border">
          <nav aria-label="分组" className="flex w-[208px] shrink-0 flex-col gap-0.5 overflow-y-auto border-r border-jolu-border bg-jolu-side p-3">
            <span className="px-2.5 pb-1 text-[10px] text-jolu-muted">来源分组</span>
            {nav("all", scope.kind === "all", "全部来源", connections.length, () => setScope({ kind: "all" }))}
            {sourceGroups.map((group) => nav(`s:${group.id}`, scope.kind === "group" && scope.id === group.id, group.name, group.members.length, () => setScope({ kind: "group", id: group.id }),
              pinned.includes(`source:${group.id}`) ? <Pin aria-label="已置顶" className="size-3 text-jolu-muted" /> : null))}
            {nav("ungrouped", scope.kind === "ungrouped", "未分组", inScope(connections, sourceGroups, { kind: "ungrouped" }).length, () => setScope({ kind: "ungrouped" }))}
            {addGroupButton("source")}
            <span className="px-2.5 pb-1 pt-4 text-[10px] text-jolu-muted">Agent 分组</span>
            {nav("agents", scope.kind === "agents" && scope.id === null, "全部 Agent", agents.length, () => setScope({ kind: "agents", id: null }))}
            {agentGroups.map((group) => nav(`a:${group.id}`, scope.kind === "agents" && scope.id === group.id, group.name, group.members.length, () => setScope({ kind: "agents", id: group.id }),
              pinned.includes(`agent:${group.id}`) ? <Pin aria-label="已置顶" className="size-3 text-jolu-muted" /> : null))}
            {addGroupButton("agent")}
          </nav>

          <div className="flex min-w-0 flex-1 flex-col">
            {currentGroup && (
              <div className="flex h-11 shrink-0 items-center gap-1 border-b border-jolu-border px-4">
                <span className="truncate text-[12.5px] font-semibold text-jolu-text">{currentGroup.name}</span>
                <span className="ml-1 text-[10.5px] text-jolu-muted">{currentGroup.members.length} 个成员</span>
                <span className="flex-1" />
                <button type="button" className={ICON_BUTTON} title={pinned.includes(`${groupKind}:${currentGroup.id}`) ? "取消置顶" : "置顶"} aria-label="切换置顶"
                  onClick={() => onTogglePin(`${groupKind}:${currentGroup.id}`)}>
                  {pinned.includes(`${groupKind}:${currentGroup.id}`) ? <PinOff aria-hidden className="size-3.5" /> : <Pin aria-hidden className="size-3.5" />}
                </button>
                {(() => {
                  const list = groupKind === "source" ? sourceGroups : agentGroups
                  const index = list.findIndex((group) => group.id === currentGroup.id)
                  return <>
                    <button type="button" className={ICON_BUTTON} title="上移" aria-label="上移分组" disabled={busy || index <= 0} onClick={() => reorder(groupKind, list, currentGroup.id, -1)}><ArrowUp aria-hidden className="size-3.5" /></button>
                    <button type="button" className={ICON_BUTTON} title="下移" aria-label="下移分组" disabled={busy || index >= list.length - 1} onClick={() => reorder(groupKind, list, currentGroup.id, 1)}><ArrowDown aria-hidden className="size-3.5" /></button>
                  </>
                })()}
                <button type="button" className={ICON_BUTTON} title="重命名分组" aria-label="重命名分组" disabled={busy} onClick={() => setNameDialog({ kind: groupKind, group: currentGroup })}><Pencil aria-hidden className="size-3.5" /></button>
                <button type="button" className={cn(ICON_BUTTON, "hover:text-jolu-danger")} title="删除分组" aria-label="删除分组" disabled={busy} onClick={() => askDelete(groupKind, currentGroup)}><Trash2 aria-hidden className="size-3.5" /></button>
              </div>
            )}

            {scope.kind === "agents" ? (
              <div className="flex-1 overflow-y-auto">
                {(scope.id ? agentGroups.find((group) => group.id === scope.id)?.members.map((id) => agents.find((entry) => entry.agent === id)).filter((e): e is WiringStatusDto => !!e) ?? [] : agents).map((entry) => (
                  <AgentManageRow key={entry.agent} entry={entry} groups={agentGroups} busy={busy}
                    groupId={agentGroups.find((group) => group.members.includes(entry.agent))?.id ?? null}
                    onMove={(groupId) => void run(() => gatewayApi.moveToGroup("agent", entry.agent, groupId), "已移动")}
                    onWire={() => onWire(entry.agent)} onUnwire={() => onUnwire(entry.agent)} onAccept={() => onAccept(entry.agent)} />
                ))}
                {scope.id && agents.some((entry) => !groupedAgents.has(entry.agent)) && (
                  <div className="flex flex-wrap items-center gap-1.5 px-4 py-3 text-[10.5px] text-jolu-muted">
                    加入此分组：
                    {agents.filter((entry) => !groupedAgents.has(entry.agent)).map((entry) => (
                      <button key={entry.agent} type="button" disabled={busy} onClick={() => void run(() => gatewayApi.moveToGroup("agent", entry.agent, scope.id), "已移动")}
                        className="rounded-full border border-jolu-border bg-jolu-bg px-2 py-0.5 text-[10.5px] text-jolu-text hover:bg-jolu-side disabled:opacity-45">+ {agentLabel(entry.agent)}</button>
                    ))}
                  </div>
                )}
                {agents.length === 0 && <Empty>读取接线状态后，这里会列出可接入的工具。</Empty>}
              </div>
            ) : (
              <>
                <div className="flex h-12 shrink-0 items-center gap-2.5 px-4">
                  <label className="flex h-8 w-[220px] items-center gap-1.5 rounded-lg border border-jolu-border bg-jolu-bg px-2.5 focus-within:border-jolu-accent">
                    <Search aria-hidden className="size-3.5 text-jolu-muted" />
                    <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索名称、菜单 id 或平台" aria-label="搜索来源"
                      className="min-w-0 flex-1 bg-transparent text-[11px] text-jolu-text outline-none placeholder:text-jolu-muted" />
                  </label>
                  <div className="flex items-center gap-0.5 rounded-lg bg-jolu-track p-0.5" role="group" aria-label="按接入状态筛选">
                    {([["all", "全部", counts.all], ["on", "已接入", counts.on], ["off", "未接入", counts.off]] as const).map(([value, label, count]) => (
                      <button key={value} type="button" aria-pressed={serve === value} onClick={() => setServe(value)}
                        className={cn("h-7 rounded-md px-2.5 text-[11px] outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent", serve === value ? "bg-jolu-bg text-jolu-text shadow-sm" : "text-jolu-muted hover:text-jolu-text")}>
                        {label} <span className="tabular-nums">{count}</span>
                      </button>
                    ))}
                  </div>
                </div>

                <div className={cn("flex h-10 shrink-0 items-center gap-2 border-y border-jolu-border px-4", selected.size > 0 ? "bg-jolu-selected" : "bg-jolu-side")}>
                  <input type="checkbox" checked={allChecked} disabled={visible.length === 0} aria-label="全选当前列表"
                    onChange={() => setSelected(allChecked ? new Set() : new Set(visible.map((source) => source.id)))} className="size-3.5 accent-[var(--jolu-accent)]" />
                  <span className="text-[10.5px] text-jolu-muted">{selected.size > 0 ? `已选 ${selected.size} 个` : `${visible.length} 个来源`}</span>
                  <span className="flex-1" />
                  {selected.size > 0 && <>
                    <button type="button" disabled={busy} onClick={() => askBatch(true)} className="h-7 rounded-md bg-jolu-accent px-2.5 text-[11px] font-medium text-[#071B10] outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jolu-accent disabled:opacity-45">加入模型池</button>
                    <button type="button" disabled={busy} onClick={() => askBatch(false)} className="h-7 rounded-md border border-jolu-border bg-jolu-bg px-2.5 text-[11px] text-jolu-danger outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">移出模型池</button>
                    <Menu.Root>
                      <Menu.Trigger asChild>
                        <button type="button" disabled={busy} className="flex h-7 items-center gap-1 rounded-md border border-jolu-border bg-jolu-bg px-2.5 text-[11px] text-jolu-text outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
                          <FolderInput aria-hidden className="size-3.5 text-jolu-muted" />移动到
                        </button>
                      </Menu.Trigger>
                      <Menu.Portal>
                        <Menu.Content align="end" sideOffset={6} collisionPadding={12} className={MENU_CLASS}>
                          {sourceGroups.map((group) => <Menu.Item key={group.id} className={ITEM_CLASS} onSelect={() => moveTo([...selected], group.id)}>{group.name}</Menu.Item>)}
                          <Menu.Item className={ITEM_CLASS} onSelect={() => moveTo([...selected], null)}><FolderMinus aria-hidden />移出分组</Menu.Item>
                        </Menu.Content>
                      </Menu.Portal>
                    </Menu.Root>
                    <button type="button" onClick={() => setSelected(new Set())} className="h-7 rounded-md px-2 text-[11px] text-jolu-muted hover:text-jolu-text">取消</button>
                  </>}
                </div>

                <div className="flex-1 overflow-y-auto">
                  {visible.map((connection) => {
                    const capability = capabilityLine(capabilities.find((item) => item.connection_id === connection.id), connection.serve_enabled)
                    const models = catalog.filter((model) => !!connection.slug && model.slug === connection.slug)
                    const group = groupOf(connection.id)
                    const Icon = connection.kind === "auth" ? BadgeCheck : KeyRound
                    return (
                      <div key={connection.id} className={cn("flex min-h-16 items-center gap-3 border-b border-jolu-table-divider px-4 py-2.5", selected.has(connection.id) ? "bg-jolu-table-active" : "hover:bg-jolu-side")}>
                        <input type="checkbox" checked={selected.has(connection.id)} aria-label={`选择 ${connection.baseName}`} className="size-3.5 accent-[var(--jolu-accent)]"
                          onChange={() => setSelected((current) => {
                            const next = new Set(current)
                            if (next.has(connection.id)) next.delete(connection.id)
                            else next.add(connection.id)
                            return next
                          })} />
                        <span className={cn("grid size-8 shrink-0 place-items-center rounded-lg", connection.serve_enabled ? "bg-jolu-selected" : "bg-jolu-stage")}>
                          <Icon aria-hidden className={cn("size-4", connection.serve_enabled ? "text-jolu-accent" : "text-jolu-muted")} />
                        </span>
                        <div className="flex min-w-0 flex-1 flex-col gap-1">
                          <div className="flex min-w-0 items-baseline gap-2">
                            <span className="truncate text-[12.5px] font-semibold text-jolu-text" title={connection.baseName}>{connection.baseName}</span>
                            <span className="shrink-0 text-[10px] text-jolu-muted">{credentialLabel(connection.kind, connection.platformId)}</span>
                            {group && scope.kind === "all" && <span className="shrink-0 rounded bg-jolu-track px-1.5 text-[9.5px] text-jolu-muted">{group.name}</span>}
                          </div>
                          <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 text-[10px] text-jolu-muted">
                            <code className="font-mono">id: {connection.slug ?? "—"}</code>
                            <span>·</span>
                            <span>{models.length} 个模型</span>
                            <span>·</span>
                            <span className={cn(capability.tone === "danger" ? "text-jolu-danger" : capability.tone === "ok" ? "text-jolu-on" : "")}>{capability.text}</span>
                            <span>·</span>
                            <span>{usageError ? "使用情况读取失败" : usage === null ? (isTauri ? "读取使用情况…" : "桌面端可用") : usageLine(usageOf.get(connection.id), now)}</span>
                          </div>
                        </div>
                        <StatusCapsule size="sm" tone={connection.serve_enabled ? "on" : "off"} label={connection.serve_enabled ? "已接入" : "未接入"}
                          title={connection.serve_enabled ? "在模型池中，点击移出" : "不在模型池中，点击加入"} disabled={busy || !isTauri} onClick={() => askServe(connection)} />
                        <Menu.Root>
                          <Menu.Trigger asChild>
                            <button type="button" aria-label={`${connection.baseName} 的更多操作`} className={cn(ICON_BUTTON, "size-8 data-[state=open]:bg-jolu-stage")}><MoreHorizontal aria-hidden className="size-4" /></button>
                          </Menu.Trigger>
                          <Menu.Portal>
                            <Menu.Content align="end" sideOffset={6} collisionPadding={12} className={MENU_CLASS}>
                              <Menu.Item className={ITEM_CLASS} disabled={busy || !isTauri} onSelect={() => onRename(connection)}><Pencil aria-hidden />改名与菜单 id</Menu.Item>
                              <Menu.Item className={ITEM_CLASS} disabled={models.length === 0} onSelect={() => onShowModels(models)}><List aria-hidden />查看模型 · {models.length}</Menu.Item>
                              {sourceGroups.length > 0 && <Menu.Separator className="my-1 h-px bg-jolu-border" />}
                              {sourceGroups.filter((item) => item.id !== group?.id).map((item) => (
                                <Menu.Item key={item.id} className={ITEM_CLASS} disabled={busy} onSelect={() => moveTo([connection.id], item.id)}><FolderInput aria-hidden />移到「{item.name}」</Menu.Item>
                              ))}
                              {group && <Menu.Item className={ITEM_CLASS} disabled={busy} onSelect={() => moveTo([connection.id], null)}><FolderMinus aria-hidden />移出「{group.name}」</Menu.Item>}
                            </Menu.Content>
                          </Menu.Portal>
                        </Menu.Root>
                      </div>
                    )
                  })}
                  {visible.length === 0 && (
                    <Empty>{connections.length === 0 ? "还没有账号或 API Key。先到「供应商」页连接账号，再回到这里加入模型池。" : scoped.length === 0 ? "这个分组还没有来源。可在「全部来源」里多选后「移动到」这里。" : "没有符合条件的来源。"}</Empty>
                  )}
                </div>
              </>
            )}
          </div>
        </div>
      </Modal>

      {confirm && <ConfirmDialog title={confirm.title} description={confirm.description} confirmLabel={confirm.label} danger={confirm.danger}
        onClose={() => setConfirm(null)} onConfirm={confirm.work} />}
      {nameDialog && <GroupNameDialog kind={nameDialog.kind} initial={nameDialog.group?.name ?? ""} busy={busy} onClose={() => setNameDialog(null)}
        onSave={async (name) => {
          const ok = nameDialog.group
            ? await run(() => gatewayApi.renameGroup(nameDialog.kind, nameDialog.group!.id, name), "已重命名分组")
            : await run(() => gatewayApi.createGroup(nameDialog.kind, name), `已新建分组「${name}」`)
          if (ok) setNameDialog(null)
        }} />}
    </>
  )
}

function Empty({ children }: { children: React.ReactNode }) {
  return <p className="px-6 py-10 text-center text-[11px] text-jolu-muted">{children}</p>
}

function AgentManageRow({ entry, groups, groupId, busy, onMove, onWire, onUnwire, onAccept }: {
  entry: WiringStatusDto; groups: GatewayGroupDto[]; groupId: string | null; busy: boolean
  onMove: (groupId: string | null) => void; onWire: () => void; onUnwire: () => void; onAccept: () => void
}) {
  const line = wiringLine(entry)
  const drift = entry.drift
  const action = "h-7 rounded-md px-2.5 text-[11px] outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45"
  return (
    <div className="flex min-h-14 items-center gap-3 border-b border-jolu-table-divider px-4 py-2">
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="truncate text-[12.5px] font-semibold text-jolu-text">{agentLabel(entry.agent)}</span>
        <span className={cn("truncate text-[10px]", entry.wired ? "text-jolu-on" : drift ? "text-jolu-danger" : "text-jolu-muted")}>
          {drift && !entry.wired ? (drift === "pending" ? "上次接线未完成" : drift === "removed" ? "配置里已移除" : drift === "foreign" ? "已被另一个 Jolu 接管" : "配置被改写") : line.text}
        </span>
      </div>
      <select value={groupId ?? ""} disabled={busy} onChange={(event) => onMove(event.target.value || null)} aria-label={`${agentLabel(entry.agent)} 所在分组`}
        className="h-7 rounded-md border border-jolu-border bg-jolu-bg px-1.5 text-[10.5px] text-jolu-text outline-none focus-visible:border-jolu-accent">
        <option value="">未分组</option>
        {groups.map((group) => <option key={group.id} value={group.id}>{group.name}</option>)}
      </select>
      {entry.wired ? (
        <button type="button" disabled={busy} onClick={onUnwire} className={cn(action, "border border-jolu-border bg-jolu-bg text-jolu-danger")}>断开</button>
      ) : drift ? (
        <>
          <button type="button" disabled={busy} onClick={onWire} className={cn(action, "bg-jolu-selected text-jolu-on")}>重新应用</button>
          <button type="button" disabled={busy} onClick={onAccept} className={cn(action, "text-jolu-muted")}>接受现状</button>
        </>
      ) : (
        <button type="button" disabled={busy || (!entry.config_path && !!entry.reason)} onClick={onWire} className={cn(action, "bg-jolu-selected text-jolu-on")}>接线</button>
      )}
    </div>
  )
}

function GroupNameDialog({ kind, initial, busy, onClose, onSave }: { kind: GroupKind; initial: string; busy: boolean; onClose: () => void; onSave: (name: string) => void }) {
  const [name, setName] = useState(initial)
  const title = initial ? "重命名分组" : kind === "source" ? "新建来源分组" : "新建 Agent 分组"
  return (
    <Modal busy={busy} title={title} onClose={onClose} width={420}
      footer={<><Button label="取消" disabled={busy} onClick={onClose} /><Button label={initial ? "保存" : "创建"} primary disabled={busy || !name.trim()} onClick={() => onSave(name.trim())} /></>}>
      <p className="text-[10.5px] text-jolu-muted">只影响排列，不改变路由、接线和 Agent 菜单。</p>
      <input autoFocus value={name} onChange={(event) => setName(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && name.trim() && !busy) onSave(name.trim()) }}
        placeholder="分组名称" aria-label="分组名称"
        className="jolu-control h-9 rounded-lg border border-jolu-border bg-jolu-side px-3 text-[11.5px] text-jolu-text outline-none focus:border-jolu-accent" />
    </Modal>
  )
}
