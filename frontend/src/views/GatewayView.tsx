import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react"
import { flushSync } from "react-dom"
import {
  Cable, ChevronDown, ChevronRight, Folder, GripVertical, Link2, Pin, PinOff, Plus, Settings2, ShieldAlert, TriangleAlert, Users, X,
} from "lucide-react"
import { cn } from "@/lib/utils"
import { api, isTauri } from "@/lib/api"
import {
  gatewayApi, type CatalogModelDto, type GatewayGroupDto, type GatewayGroupsDto, type GatewayMode,
  type GatewayStatusDto, type SourceCapabilityDto, type GroupKind, type RouteLogDto, type WiringStatusDto,
} from "@/lib/gatewayApi"
import { agentLabel, canEscape, canToggleSource, circuitText, wiredCount } from "@/lib/gatewayPage"
import { useConnections } from "@/hooks/useConnections"
import { useToast } from "@/components/ui/Toast"
import { GatewayAccessBar } from "@/components/gateway/GatewayAccessBar"
import { JoluSwitch } from "@/components/ui/JoluSwitch"
import { AppIcon } from "@/components/brand/AppIcon"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import type { Connection, PlatformId } from "@/types"

/**
 * 模型入口（Jolu.pen 模块 J · V3 分组总览）。
 *
 * 三栏：模型来源 → 模型池 → Agent；分组只影响排列。动效按画板 OX4b7：
 * 链路描线 400ms、悬停高亮 180ms、流光点 2.4s、模型池呼吸 3.2s、弹窗 200ms。
 */

type Dialog =
  | { kind: "new-group"; groupKind: GroupKind }
  | { kind: "delete-group"; groupKind: GroupKind; group: GatewayGroupDto }
  | { kind: "rename"; connection: Connection }
  | { kind: "serve-conflict"; message: string }
  | { kind: "wiring" }
  | { kind: "codex" }
  | { kind: "gemini" }
  | { kind: "restore-failed"; message: string; agent: string }
  | { kind: "catalog" }
  | null

type LinkPath = { key: string; d: string; connected: boolean }

const STRIPES = ["#2563EB", "#7C3AED", "#0E9384", "#D97757", "#727277"]
const EMPTY_GROUPS: GatewayGroupsDto = { sources: [], agents: [] }
const EMPTY_WIRING: WiringStatusDto[] = []
const VIEW_KEY = "jolu:gateway:view"

type ViewState = {
  view?: "group" | "flat"
  onlyServing?: boolean
  filterGroup?: string
  expanded?: string[]
}

function readViewState(): ViewState {
  try {
    return JSON.parse(localStorage.getItem(VIEW_KEY) ?? "{}") as ViewState
  } catch {
    return {}
  }
}

function withViewTransition(work: () => void) {
  const doc = document as Document & { startViewTransition?: (callback: () => void) => void }
  const reduced = document.documentElement.classList.contains("reduce-motion")
    || window.matchMedia("(prefers-reduced-motion: reduce)").matches
  if (!doc.startViewTransition || reduced) {
    work()
    return
  }
  doc.startViewTransition(work)
}

function pathNumbers(path: string): number[] {
  return (path.match(/-?\d+(?:\.\d+)?/g) ?? []).map(Number)
}

function lerpPath(from: string, to: string, t: number): string {
  const a = pathNumbers(from)
  const b = pathNumbers(to)
  if (a.length !== b.length || a.length === 0) return to
  let index = 0
  return to.replace(/-?\d+(?:\.\d+)?/g, () => {
    const value = a[index] + (b[index] - a[index]) * t
    index += 1
    return value.toFixed(1)
  })
}
const PIN_KEY = "jolu:gateway:pinned"
const AGENT_PLATFORM: Record<string, PlatformId> = {
  "claude-code": "claude", codex: "codex", gemini: "gemini", zcode: "zcode", opencode: "opencode",
}

function readPinned(): string[] {
  try {
    return JSON.parse(localStorage.getItem(PIN_KEY) ?? "[]") as string[]
  } catch {
    return []
  }
}

export function GatewayView() {
  const toast = useToast()
  const { connections, reload } = useConnections()
  const [status, setStatus] = useState<GatewayStatusDto | null>(null)
  const [groups, setGroups] = useState<GatewayGroupsDto>(EMPTY_GROUPS)
  const [capabilities, setCapabilities] = useState<SourceCapabilityDto[]>([])
  const [catalog, setCatalog] = useState<CatalogModelDto[]>([])
  const [routeLog, setRouteLog] = useState<RouteLogDto[]>([])
  const initialView = readViewState()
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [view, setView] = useState<"group" | "flat">(initialView.view === "flat" ? "flat" : "group")
  const [onlyServing, setOnlyServing] = useState(initialView.onlyServing === true)
  const [editing, setEditing] = useState(false)
  const [filterGroup, setFilterGroup] = useState(initialView.filterGroup ?? "")
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set(initialView.expanded ?? ["ungrouped-source", "ungrouped-agent"]))
  const [pinned, setPinned] = useState<string[]>(readPinned)
  const [dialog, setDialog] = useState<Dialog>(null)
  const [hovered, setHovered] = useState<string | null>(null)
  const [closing, setClosing] = useState<string | null>(null)
  const [newGroup, setNewGroup] = useState<string | null>(null)
  const [links, setLinks] = useState<LinkPath[]>([])
  const [drawn, setDrawn] = useState<Set<string>>(new Set())
  const [draftOrder, setDraftOrder] = useState<{ source: string[]; agent: string[] } | null>(null)
  const [draggingGroup, setDraggingGroup] = useState<string | null>(null)
  const [dropTarget, setDropTarget] = useState<string | null>(null)
  const [visible, setVisible] = useState(() => !document.hidden)
  const stageRef = useRef<HTMLDivElement | null>(null)
  const hubRef = useRef<HTMLDivElement | null>(null)
  const cardRefs = useRef(new Map<string, HTMLDivElement>())
  const linksRef = useRef<LinkPath[]>([])
  const linkFrame = useRef<number | null>(null)
  const drawnRef = useRef(new Map<string, boolean>())
  const drawTimer = useRef<number | null>(null)

  const refresh = useCallback(async () => {
    if (!isTauri) {
      setLoading(false)
      return
    }
    try {
      const [nextStatus, nextGroups, nextCatalog, nextLog, nextCapabilities] = await Promise.all([
        gatewayApi.status(), gatewayApi.groups(), gatewayApi.catalog(), gatewayApi.routeLog(), gatewayApi.sourceCapabilities(),
      ])
      setCapabilities(nextCapabilities)
      setStatus(nextStatus)
      setGroups(nextGroups)
      setCatalog(nextCatalog)
      setRouteLog(nextLog)
    } catch (error) {
      toast.danger("读取模型入口失败", String(error))
    } finally {
      setLoading(false)
    }
  }, [toast])

  useEffect(() => {
    void refresh()
    const timer = window.setInterval(() => { if (isTauri) void refresh() }, 15_000)
    return () => window.clearInterval(timer)
  }, [refresh])

  const run = async (work: () => Promise<unknown>, ok?: string) => {
    if (busy) return false
    setBusy(true)
    try {
      await work()
      if (ok) toast.success(ok)
      await Promise.all([refresh(), reload()])
      return true
    } catch (error) {
      toast.danger("操作失败", String(error))
      return false
    } finally {
      setBusy(false)
    }
  }

  const mode: GatewayMode = status?.mode === "relay" ? "relay" : "observe"
  const running = mode === "relay" && !!status?.enabled
  const agents = status?.wiring ?? EMPTY_WIRING
  const serving = connections.filter((connection) => connection.serve_enabled)
  const wired = wiredCount(status)
  const agentGroups = orderedGroups(groups.agents, draftOrder?.agent, pinned, "agent")
  const sourceGroups = orderedGroups(groups.sources, draftOrder?.source, pinned, "source")
  const groupedSources = new Set(sourceGroups.flatMap((group) => group.members))
  const groupedAgents = new Set(agentGroups.flatMap((group) => group.members))
  const ungroupedSources = connections.filter((connection) => !groupedSources.has(connection.id))
  const ungroupedAgents = agents.filter((entry) => !groupedAgents.has(entry.agent))
  const visibleSources = (list: Connection[]) => (onlyServing ? list.filter((connection) => connection.serve_enabled) : list)
  const visibleAgents = (list: WiringStatusDto[]) => {
    const filtered = filterGroup ? list.filter((entry) => agentGroups.find((group) => group.id === filterGroup)?.members.includes(entry.agent)) : list
    return onlyServing ? filtered.filter((entry) => entry.wired) : filtered
  }

  useLayoutEffect(() => {
    const compute = () => {
      const stage = stageRef.current
      const hub = hubRef.current
      if (!stage || !hub) return
      const base = stage.getBoundingClientRect()
      const hubRect = hub.getBoundingClientRect()
      const next: LinkPath[] = []
      for (const group of sourceGroups) {
        const card = cardRefs.current.get(`source:${group.id}`)
        if (!card) continue
        const rect = card.getBoundingClientRect()
        const fromX = rect.right - base.left
        const fromY = rect.top - base.top + 34
        const toX = hubRect.left - base.left
        const toY = hubRect.top - base.top + hubRect.height / 2
        next.push({
          key: `source:${group.id}`,
          d: `M ${fromX} ${fromY} C ${fromX + 48} ${fromY}, ${toX - 48} ${toY}, ${toX} ${toY}`,
          connected: group.members.some((id) => connections.find((connection) => connection.id === id)?.serve_enabled),
        })
      }
      for (const group of agentGroups) {
        const card = cardRefs.current.get(`agent:${group.id}`)
        if (!card) continue
        const rect = card.getBoundingClientRect()
        const fromX = hubRect.right - base.left
        const fromY = hubRect.top - base.top + hubRect.height / 2
        const toX = rect.left - base.left
        const toY = rect.top - base.top + 34
        next.push({
          key: `agent:${group.id}`,
          d: `M ${fromX} ${fromY} C ${fromX + 48} ${fromY}, ${toX - 48} ${toY}, ${toX} ${toY}`,
          connected: group.members.some((agent) => agents.find((entry) => entry.agent === agent)?.wired),
        })
      }
      const reduced = document.documentElement.classList.contains("reduce-motion")
        || window.matchMedia("(prefers-reduced-motion: reduce)").matches
      if (reduced) {
        linksRef.current = next
        setLinks(next)
        return
      }
      const from = linksRef.current
      if (linkFrame.current) cancelAnimationFrame(linkFrame.current)
      const start = performance.now()
      const step = (now: number) => {
        const t = Math.min(1, (now - start) / 200)
        const eased = 1 - Math.pow(1 - t, 3)
        const merged = next.map((target) => {
          const old = from.find((link) => link.key === target.key)
          return old ? { ...target, d: lerpPath(old.d, target.d, eased) } : target
        })
        linksRef.current = merged
        setLinks(merged)
        if (t < 1) linkFrame.current = requestAnimationFrame(step)
        else {
          linksRef.current = next
          setLinks(next)
        }
      }
      linkFrame.current = requestAnimationFrame(step)
    }
    compute()
    const observer = new ResizeObserver(compute)
    if (stageRef.current) observer.observe(stageRef.current)
    return () => {
      observer.disconnect()
      if (linkFrame.current) cancelAnimationFrame(linkFrame.current)
    }
  }, [groups, connections, agents, expanded, view, filterGroup, onlyServing, pinned, draftOrder])

  useEffect(() => {
    const onVisibility = () => setVisible(!document.hidden)
    document.addEventListener("visibilitychange", onVisibility)
    return () => document.removeEventListener("visibilitychange", onVisibility)
  }, [])

  useEffect(() => {
    const newly: string[] = []
    for (const link of links) {
      const previous = drawnRef.current.get(link.key)
      if (link.connected && previous === false) newly.push(link.key)
      drawnRef.current.set(link.key, link.connected)
    }
    if (newly.length > 0) {
      setDrawn((current) => new Set([...current, ...newly]))
      if (drawTimer.current) window.clearTimeout(drawTimer.current)
      drawTimer.current = window.setTimeout(() => setDrawn(new Set()), 520)
    }
  }, [links])

  useEffect(() => () => { if (drawTimer.current) window.clearTimeout(drawTimer.current) }, [])

  useEffect(() => {
    try {
      localStorage.setItem(VIEW_KEY, JSON.stringify({
        view, onlyServing, filterGroup, expanded: [...expanded],
      }))
    } catch {
      // 本地偏好存不进去只影响下次打开
    }
  }, [view, onlyServing, filterGroup, expanded])

  const togglePin = (key: string) => {
    setPinned((current) => {
      const next = current.includes(key) ? current.filter((item) => item !== key) : [...current, key]
      try { localStorage.setItem(PIN_KEY, JSON.stringify(next)) } catch { /* 本地偏好存不进去只影响下次 */ }
      return next
    })
  }

  const toggleServe = async (connection: Connection) => {
    if (connection.serve_enabled) {
      await run(() => gatewayApi.setServe(connection.id, false), `已移出 ${connection.baseName}`)
      return
    }
    if (!isTauri) return
    setBusy(true)
    try {
      await gatewayApi.setServe(connection.id, true)
      const capability = capabilities.find(item => item.connection_id === connection.id)
      toast.success(capability?.can_serve ? `已把 ${connection.baseName} 加入模型池` : `已选择 ${connection.baseName}，${capability?.state === "needs_auth" ? "请补全账号授权后调用" : "转发尚未就绪"}`)
      await Promise.all([refresh(), reload()])
    } catch (error) {
      setDialog({ kind: "serve-conflict", message: String(error) })
    } finally {
      setBusy(false)
    }
  }

  const wire = (agent: string) => {
    if (agent === "codex") return setDialog({ kind: "codex" })
    if (agent === "gemini") return setDialog({ kind: "gemini" })
    void run(() => gatewayApi.setWiring(agent, true), `已接入 ${agentLabel(agent)}`)
  }

  const unwire = async (agent: string) => {
    if (!isTauri) return
    setBusy(true)
    try {
      await gatewayApi.setWiring(agent, false)
      toast.success(`已断开 ${agentLabel(agent)}`)
      await refresh()
    } catch (error) {
      setDialog({ kind: "restore-failed", message: String(error), agent })
    } finally {
      setBusy(false)
    }
  }

  const move = async (kind: GroupKind, member: string, groupId: string | null) => {
    const name = groupId ? groups[kind === "source" ? "sources" : "agents"].find((group) => group.id === groupId)?.name : "未分组"
    await run(() => gatewayApi.moveToGroup(kind, member, groupId), `已移到「${name ?? "未分组"}」`)
  }

  const toggleExpanded = (id: string) => {
    setExpanded((current) => {
      const next = new Set(current)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const enterEdit = () => {
    setDraftOrder({ source: groups.sources.map((group) => group.id), agent: groups.agents.map((group) => group.id) })
    setEditing(true)
  }

  const saveEdit = async () => {
    const ok = !draftOrder || await run(async () => {
      await gatewayApi.reorderGroups("source", draftOrder.source)
      await gatewayApi.reorderGroups("agent", draftOrder.agent)
    }, "布局已保存")
    if (!ok) return
    setEditing(false)
    setDraftOrder(null)
  }

  const revertEdit = () => {
    setEditing(false)
    setDraftOrder(null)
  }

  const reorderTo = (kind: GroupKind, fromKey: string, toKey: string, after: boolean) => {
    const fromId = fromKey.split(":")[1]
    const toId = toKey.split(":")[1]
    withViewTransition(() => {
      flushSync(() => setDraftOrder((current) => {
        if (!current) return current
        const list = [...current[kind]]
        const from = list.indexOf(fromId)
        const to = list.indexOf(toId)
        if (from < 0 || to < 0 || from === to) return current
        list.splice(from, 1)
        list.splice(after ? (from < to ? to : to + 1) : (from < to ? to - 1 : to), 0, fromId)
        return { ...current, [kind]: list }
      }))
    })
  }

  const nudgeGroup = (kind: GroupKind, key: string, direction: -1 | 1) => {
    const id = key.split(":")[1]
    withViewTransition(() => {
      flushSync(() => setDraftOrder((current) => {
        if (!current) return current
        const list = [...current[kind]]
        const index = list.indexOf(id)
        const target = index + direction
        if (index < 0 || target < 0 || target >= list.length) return current
        ;[list[index], list[target]] = [list[target], list[index]]
        return { ...current, [kind]: list }
      }))
    })
  }

  const groupDragProps = (kind: GroupKind, group: GatewayGroupDto) => ({
    editing,
    dragging: draggingGroup === `${kind}:${group.id}`,
    dropTarget: dropTarget === `${kind}:${group.id}`,
    onGroupDragStart: (event: React.DragEvent) => {
      event.dataTransfer.setData(`text/jolu-group-${kind}`, group.id)
      event.dataTransfer.effectAllowed = "move"
      setDraggingGroup(`${kind}:${group.id}`)
    },
    onGroupDragOver: (event: React.DragEvent) => {
      if (!editing || !draggingGroup?.startsWith(`${kind}:`)) return
      event.preventDefault()
      setDropTarget(`${kind}:${group.id}`)
    },
    onGroupDrop: (event: React.DragEvent) => {
      const id = event.dataTransfer.getData(`text/jolu-group-${kind}`)
      setDraggingGroup(null)
      setDropTarget(null)
      if (!id || !editing) return
      const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
      reorderTo(kind, `${kind}:${id}`, `${kind}:${group.id}`, event.clientY > rect.top + rect.height / 2)
    },
    onGroupDragEnd: () => {
      setDraggingGroup(null)
      setDropTarget(null)
    },
    onKeyReorder: (direction: -1 | 1) => nudgeGroup(kind, `${kind}:${group.id}`, direction),
  })

  const createGroup = async (kind: GroupKind, name: string) => {
    setBusy(true)
    try {
      const group = await gatewayApi.createGroup(kind, name)
      setNewGroup(`${kind}:${group.id}`)
      toast.success(`已新建分组「${name}」`)
      await Promise.all([refresh(), reload()])
      window.setTimeout(() => setNewGroup(null), 320)
      setDialog(null)
    } catch (error) {
      toast.danger("新建分组失败", String(error))
    } finally {
      setBusy(false)
    }
  }

  const deleteGroup = async (kind: GroupKind, group: GatewayGroupDto) => {
    setClosing(`${kind}:${group.id}`)
    await new Promise((resolve) => window.setTimeout(resolve, 200))
    const ok = await run(() => gatewayApi.deleteGroup(kind, group.id), "已删除分组")
    setClosing(null)
    if (ok) setDialog(null)
  }

  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto pb-6 pr-1">
      <header className="flex items-end justify-between gap-4 pt-1">
        <div className="flex flex-col gap-1">
          <h1 className="text-[22px] font-semibold text-jolu-text">模型入口</h1>
          <p className="max-w-[660px] text-xs text-jolu-muted">
            把官方订阅、API Key 和第三方 Key 接进同一个本机入口，各工具按「来源/模型」取用。
          </p>
        </div>
        <button type="button" onClick={() => setDialog({ kind: "wiring" })} className="motion-button flex h-8 items-center gap-2 rounded-lg border border-jolu-border px-3 text-[11.5px] text-jolu-text hover:bg-jolu-side">
          <Cable className="h-3.5 w-3.5" />
          接入设置
        </button>
      </header>

      {!isTauri && <Notice tone="muted">浏览器预览不读取网关状态；请在桌面端查看真实数据。</Notice>}

      <GatewayAccessBar status={status} loading={loading} busy={busy} desktop={isTauri}
        sources={serving.length} models={catalog.length} agents={wired}
        onToggle={(on) => { void run(() => gatewayApi.setMode(on ? "relay" : "observe", status?.default_connection ?? null), on ? "模型转发已开启" : "已关闭模型转发，一键接入的工具已恢复原连接") }} />

      <div className="flex items-center gap-2">
        <div className="flex items-center gap-1 rounded-lg bg-jolu-track p-0.5">
          {(["group", "flat"] as const).map((value) => (
            <button key={value} type="button" onClick={() => setView(value)} className={cn("h-7 rounded-md px-3 text-[10.5px] transition-colors", view === value ? "bg-jolu-bg text-jolu-text shadow-sm" : "text-jolu-muted")}>
              {value === "group" ? "分组" : "平铺"}
            </button>
          ))}
        </div>
        <label className="flex h-7 items-center gap-1.5 rounded-lg border border-jolu-border px-2 text-[10px] text-jolu-muted">
          只看
          <select value={filterGroup} onChange={(event) => setFilterGroup(event.target.value)} className="bg-transparent text-[10px] text-jolu-text outline-none">
            <option value="">全部 Agent</option>
            {agentGroups.map((group) => <option key={group.id} value={group.id}>{group.name}</option>)}
          </select>
        </label>
        <button type="button" onClick={() => setOnlyServing((value) => !value)} className={cn("flex h-7 items-center gap-1.5 rounded-lg px-2.5 text-[10px] transition-colors", onlyServing ? "bg-jolu-selected text-jolu-on" : "text-jolu-muted")}>
          <span className={cn("h-1.5 w-1.5 rounded-full", onlyServing ? "bg-jolu-accent" : "bg-jolu-border")} />
          {onlyServing ? `仅看已接入 · ${wired}` : "仅看已接入"}
        </button>
        <div className="flex-1" />
        <button type="button" onClick={() => { if (editing) void saveEdit(); else enterEdit() }} className={cn("flex h-7 items-center gap-1.5 rounded-lg px-2.5 text-[10.5px] transition-colors", editing ? "bg-[#17261E] text-white" : "text-jolu-muted hover:bg-jolu-side")}>
          <Settings2 className="h-3 w-3" />
          {editing ? "完成" : "编辑布局"}
        </button>
      </div>

      {editing && (
        <div className="gw-dialog-enter flex items-center gap-2 rounded-lg bg-[#17261E] px-3 py-2 text-[10px] text-white">
          <GripVertical className="h-3.5 w-3.5 text-[#8EE3B2]" />
          布局编辑中 · 拖动分组卡调整顺序，拖动成员换分组；Alt + ↑/↓ 也能排序。只影响排列，不改接线和菜单。
          <div className="flex-1" />
          <button type="button" onClick={revertEdit} className="rounded px-2 py-1 text-[9.5px] text-white hover:bg-white/10">还原</button>
          <button type="button" onClick={() => void saveEdit()} className="rounded bg-jolu-accent px-2 py-1 text-[9.5px] text-[#071B10]">完成</button>
        </div>
      )}

      <section ref={stageRef} className="relative grid grid-cols-[minmax(240px,272px)_240px_minmax(240px,272px)] justify-center gap-x-14 gap-y-3 rounded-xl bg-jolu-side px-5 py-4">
        <svg className="pointer-events-none absolute inset-0 h-full w-full" aria-hidden>
          {links.map((link) => {
            const active = hovered === link.key
            const dim = hovered !== null && !active
            return (
              <g key={link.key}>
                <path
                  d={link.d}
                  pathLength={1}
                  fill="none"
                  strokeWidth={active ? 2.2 : 1.5}
                  className={cn("gw-link", link.connected ? "stroke-jolu-accent" : "stroke-jolu-border", drawn.has(link.key) && "gw-link-draw")}
                  style={{ opacity: dim ? 0.35 : 1 }}
                />
                {link.connected && running && visible && (
                  <circle r="3" className="gw-flow-dot fill-jolu-accent" opacity={dim ? 0.35 : 1}>
                    <animateMotion dur="2.4s" repeatCount="indefinite" path={link.d} />
                  </circle>
                )}
              </g>
            )
          })}
        </svg>

        <div className="relative z-10 flex min-w-0 flex-col gap-2.5">
          <ColumnHeader step="1" title="模型来源" count={`${connections.length} · ${serving.length} 接入`} onAdd={() => setDialog({ kind: "new-group", groupKind: "source" })} addLabel="分组" />
          {view === "group" ? (
            <>
              {sourceGroups.map((group, index) => {
                const members = membersOf(group, connections)
                const key = `source:${group.id}`
                return (
                  <GroupCard
                    key={group.id}
                    innerRef={(element) => { if (element) cardRefs.current.set(key, element); else cardRefs.current.delete(key) }}
                    name={group.name}
                    stripe={STRIPES[index % STRIPES.length]}
                    icon="folder"
                    subtitle={`${members.filter((connection) => connection.serve_enabled).length}/${members.length} 已接入`}
                    models={members.reduce((sum, connection) => sum + catalog.filter((model) => model.slug === connection.slug).length, 0)}
                    avatars={members.map((connection) => connection.platformId)}
                    expanded={expanded.has(group.id)}
                    onToggle={() => toggleExpanded(group.id)}
                    pinned={pinned.includes(key)}
                    onPin={() => togglePin(key)}
                    onHover={(on) => setHovered(on ? key : null)}
                    onDelete={() => setDialog({ kind: "delete-group", groupKind: "source", group })}
                    closing={closing === key}
                    entering={newGroup === key}
                    dropType="source"
                    onDropMember={editing ? (id) => void move("source", id, group.id) : undefined}
                    {...groupDragProps("source", group)}
                  >
                    {visibleSources(members).map((connection) => (
                      <SourceRow key={connection.id} connection={connection} capability={capabilities.find(c => c.connection_id === connection.id)} busy={busy || !isTauri} editing={editing} groups={sourceGroups} onServe={() => void toggleServe(connection)} onRename={() => setDialog({ kind: "rename", connection })} onMove={(groupId) => void move("source", connection.id, groupId)} />
                    ))}
                  </GroupCard>
                )
              })}
              <GroupCard
                innerRef={(element) => { if (element) cardRefs.current.set("source:ungrouped", element); else cardRefs.current.delete("source:ungrouped") }}
                name="未分组"
                stripe="#727277"
                icon="folder"
                subtitle={`${ungroupedSources.filter((connection) => connection.serve_enabled).length}/${ungroupedSources.length} 已接入`}
                models={ungroupedSources.reduce((sum, connection) => sum + catalog.filter((model) => model.slug === connection.slug).length, 0)}
                avatars={ungroupedSources.map((connection) => connection.platformId)}
                expanded={expanded.has("ungrouped-source")}
                onToggle={() => toggleExpanded("ungrouped-source")}
                onHover={(on) => setHovered(on ? "source:ungrouped" : null)}
                closing={false}
                entering={false}
                dropType="source"
                onDropMember={editing ? (id) => void move("source", id, null) : undefined}
                editing={editing}
                dragging={false}
                dropTarget={false}
              >
                {visibleSources(ungroupedSources).map((connection) => (
                  <SourceRow key={connection.id} connection={connection} capability={capabilities.find(c => c.connection_id === connection.id)} busy={busy || !isTauri} editing={editing} groups={sourceGroups} onServe={() => void toggleServe(connection)} onRename={() => setDialog({ kind: "rename", connection })} onMove={(groupId) => void move("source", connection.id, groupId)} />
                ))}
              </GroupCard>
              <AddCard label="新建来源分组" hint="或拖入来源" onClick={() => setDialog({ kind: "new-group", groupKind: "source" })} />
            </>
          ) : (
            <FlatList>
              {visibleSources(connections).map((connection) => (
                <SourceRow key={connection.id} connection={connection} capability={capabilities.find(c => c.connection_id === connection.id)} busy={busy || !isTauri} editing={editing} groups={sourceGroups} onServe={() => void toggleServe(connection)} onRename={() => setDialog({ kind: "rename", connection })} onMove={(groupId) => void move("source", connection.id, groupId)} />
              ))}
            </FlatList>
          )}
        </div>

        <div className="relative z-10 flex flex-col gap-2.5">
          <ColumnHeader step="2" title="模型池" count={`${serving.length} 在池`} />
          <div ref={hubRef} className={cn("flex flex-col gap-3 rounded-xl border border-jolu-border bg-jolu-bg p-3", running && "gw-hub-running")}>
            <div className="flex items-center gap-2.5">
              <AppIcon size={32} className="rounded-lg" />
              <div className="flex min-w-0 flex-col gap-0.5">
                <span className="text-[12px] font-semibold text-jolu-text">本机模型池</span>
                <span className="flex items-center gap-1 text-[9px] text-jolu-muted">
                  <span className={cn("h-1.5 w-1.5 rounded-full", running ? "bg-jolu-accent" : "bg-jolu-border")} />
                  {running ? `运行中 · :${status?.port}` : "未启动"}
                </span>
              </div>
            </div>
            <div className="h-px bg-jolu-table-divider" />
            <div className="grid grid-cols-3 gap-2 text-center">
              <Metric value={serving.length} label="来源" small />
              <Metric value={catalog.length} label="模型" small />
              <Metric value={wired} label="Agent" small />
            </div>
            <div className="flex flex-col gap-1">
              {catalog.slice(0, 5).map((model) => (
                <span key={model.id} className={cn("truncate rounded-md px-2 py-1 text-[9.5px]", model.status === "connected" ? "bg-jolu-side text-jolu-text" : "bg-jolu-status-error-bg text-jolu-danger")}>{model.id}</span>
              ))}
              {catalog.length === 0 && <span className="py-3 text-center text-[9.5px] text-jolu-muted">还没有来源加入模型池</span>}
              {catalog.length > 5 && (
                <button type="button" onClick={() => setDialog({ kind: "catalog" })} className="text-center text-[9px] text-jolu-on">+ {catalog.length - 5} 个模型 · 查看全部</button>
              )}
            </div>
            <button type="button" onClick={() => setDialog({ kind: "catalog" })} className="flex h-8 items-center justify-center gap-1.5 rounded-lg bg-jolu-side text-[10px] text-jolu-on">
              管理模型池
              <ChevronRight className="h-3 w-3" />
            </button>
          </div>
        </div>

        <div className="relative z-10 flex min-w-0 flex-col gap-2.5">
          <ColumnHeader step="3" title="Agent" count={`${agents.length} · ${wired} 接入`} onAdd={() => setDialog({ kind: "new-group", groupKind: "agent" })} addLabel="分组" />
          {view === "group" ? (
            <>
              {agentGroups.map((group, index) => {
                const members = membersOf(group, agents)
                const key = `agent:${group.id}`
                return (
                  <GroupCard
                    key={group.id}
                    innerRef={(element) => { if (element) cardRefs.current.set(key, element); else cardRefs.current.delete(key) }}
                    name={group.name}
                    stripe={STRIPES[index % STRIPES.length]}
                    icon="users"
                    subtitle={members.map((entry) => agentLabel(entry.agent)).join(" · ") || "空分组"}
                    models={members.some((entry) => entry.wired) ? catalog.length : 0}
                    avatars={members.map((entry) => AGENT_PLATFORM[entry.agent] ?? "custom")}
                    expanded={expanded.has(group.id)}
                    onToggle={() => toggleExpanded(group.id)}
                    pinned={pinned.includes(key)}
                    onPin={() => togglePin(key)}
                    onHover={(on) => setHovered(on ? key : null)}
                    onDelete={() => setDialog({ kind: "delete-group", groupKind: "agent", group })}
                    closing={closing === key}
                    entering={newGroup === key}
                    dropType="agent"
                    onDropMember={editing ? (id) => void move("agent", id, group.id) : undefined}
                    {...groupDragProps("agent", group)}
                  >
                    {visibleAgents(members).map((entry) => (
                      <AgentRow key={entry.agent} entry={entry} busy={busy || !isTauri} editing={editing} groups={agentGroups} onWire={() => wire(entry.agent)} onUnwire={() => void unwire(entry.agent)} onAccept={() => run(() => gatewayApi.acceptWiring(entry.agent), "已接受现状")} onMove={(groupId) => void move("agent", entry.agent, groupId)} />
                    ))}
                  </GroupCard>
                )
              })}
              <GroupCard
                innerRef={(element) => { if (element) cardRefs.current.set("agent:ungrouped", element); else cardRefs.current.delete("agent:ungrouped") }}
                name="未分组"
                stripe="#727277"
                icon="users"
                subtitle={ungroupedAgents.map((entry) => agentLabel(entry.agent)).join(" · ") || "空分组"}
                models={ungroupedAgents.some((entry) => entry.wired) ? catalog.length : 0}
                avatars={ungroupedAgents.map((entry) => AGENT_PLATFORM[entry.agent] ?? "custom")}
                expanded={expanded.has("ungrouped-agent")}
                onToggle={() => toggleExpanded("ungrouped-agent")}
                onHover={(on) => setHovered(on ? "agent:ungrouped" : null)}
                closing={false}
                entering={false}
                dropType="agent"
                onDropMember={editing ? (id) => void move("agent", id, null) : undefined}
                editing={editing}
                dragging={false}
                dropTarget={false}
              >
                {visibleAgents(ungroupedAgents).map((entry) => (
                  <AgentRow key={entry.agent} entry={entry} busy={busy || !isTauri} editing={editing} groups={agentGroups} onWire={() => wire(entry.agent)} onUnwire={() => void unwire(entry.agent)} onAccept={() => run(() => gatewayApi.acceptWiring(entry.agent), "已接受现状")} onMove={(groupId) => void move("agent", entry.agent, groupId)} />
                ))}
              </GroupCard>
              <AddCard label="新建 Agent 分组" hint="或拖入 Agent" onClick={() => setDialog({ kind: "new-group", groupKind: "agent" })} />
            </>
          ) : (
            <FlatList>
              {visibleAgents(agents).map((entry) => (
                <AgentRow key={entry.agent} entry={entry} busy={busy || !isTauri} editing={editing} groups={agentGroups} onWire={() => wire(entry.agent)} onUnwire={() => void unwire(entry.agent)} onAccept={() => run(() => gatewayApi.acceptWiring(entry.agent), "已接受现状")} onMove={(groupId) => void move("agent", entry.agent, groupId)} />
              ))}
            </FlatList>
          )}
        </div>
      </section>

      <RouteLog rows={routeLog} />

      <section className="flex items-center gap-2.5 rounded-xl bg-jolu-side px-4 py-3">
        <ShieldAlert className="h-4 w-4 text-jolu-on" />
        <span className="text-[10.5px] text-jolu-muted">只监听本机回环 · {circuitText(status)} · 停止接入不会删除账号、Key 或历史记录。</span>
        <div className="flex-1" />
        <button type="button" disabled={busy || !canEscape(status) || !isTauri} onClick={() => run(() => gatewayApi.escape(), "已切回直连")} className="motion-button h-8 rounded-lg px-3 text-[10.5px] text-jolu-danger disabled:opacity-40">紧急切回直连</button>
      </section>

      {dialog?.kind === "new-group" && <NewGroupDialog kind={dialog.groupKind} busy={busy} onClose={() => setDialog(null)} onCreate={(name) => void createGroup(dialog.groupKind, name)} />}
      {dialog?.kind === "delete-group" && <ConfirmDialog title={`删除「${dialog.group.name}」？`} warning="组里的成员会回到「未分组」；来源、连接和模型都不会被删除。" confirm="删除分组" danger busy={busy} onClose={() => setDialog(null)} onConfirm={() => void deleteGroup(dialog.groupKind, dialog.group)} />}
      {dialog?.kind === "rename" && (
        <RenameDialog connection={dialog.connection} busy={busy} onClose={() => setDialog(null)} onSave={async (name, slug) => {
          const ok = await run(async () => {
            if (name !== dialog.connection.baseName) await api.renameConnection(dialog.connection.id, name)
            if (slug && slug !== dialog.connection.slug) await gatewayApi.setSlug(dialog.connection.id, slug)
          }, "已保存")
          if (ok) setDialog(null)
        }} />
      )}
      {dialog?.kind === "serve-conflict" && <MessageDialog title="这个来源没有加入模型池" tone="danger" message={dialog.message} onClose={() => setDialog(null)} />}
      {dialog?.kind === "wiring" && <WiringDialog agents={agents} busy={busy} onClose={() => setDialog(null)} onWire={(agent) => { setDialog(null); wire(agent) }} onUnwire={(agent) => { setDialog(null); void unwire(agent) }} onAccept={(agent) => run(() => gatewayApi.acceptWiring(agent), "已接受现状")} />}
      {dialog?.kind === "codex" && <CodexDialog mode={agents.find((entry) => entry.agent === "codex")?.codex_mode ?? "provider"} busy={busy} onClose={() => setDialog(null)} onConfirm={async () => { setDialog(null); await run(() => gatewayApi.setWiring("codex", true), "已接入 Codex") }} />}
      {dialog?.kind === "gemini" && <GeminiDialog busy={busy} onClose={() => setDialog(null)} onConfirm={async () => { setDialog(null); await run(() => gatewayApi.setWiring("gemini", true), "已接入 Gemini CLI") }} />}
      {dialog?.kind === "restore-failed" && <MessageDialog title="没能恢复工具配置" tone="danger" message={dialog.message} action={{ label: "重试断开", onClick: async () => { setDialog(null); await unwire(dialog.agent) } }} onClose={() => setDialog(null)} />}
      {dialog?.kind === "catalog" && <CatalogDialog models={catalog} onClose={() => setDialog(null)} />}
    </div>
  )
}

function orderedGroups(groups: GatewayGroupDto[], order: string[] | undefined, pinned: string[], kind: string): GatewayGroupDto[] {
  if (order && order.length > 0) {
    return [...groups].sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id))
  }
  return orderGroups(groups, pinned, kind)
}

function orderGroups(groups: GatewayGroupDto[], pinned: string[], kind: string): GatewayGroupDto[] {
  return [...groups].sort((a, b) => {
    const aPin = pinned.includes(`${kind}:${a.id}`) ? 0 : 1
    const bPin = pinned.includes(`${kind}:${b.id}`) ? 0 : 1
    return aPin - bPin || a.sort - b.sort
  })
}

function membersOf<T>(group: GatewayGroupDto, all: T[]): T[] {
  return group.members.map((id) => all.find((item) => (item as { id?: string }).id === id || (item as { agent?: string }).agent === id)).filter((item): item is T => !!item)
}

function Metric({ value, label, small }: { value: number; label: string; small?: boolean }) {
  return (
    <div className="flex flex-col items-center gap-0.5">
      <span className={cn("font-semibold tabular-nums text-jolu-text", small ? "text-[14px]" : "text-[17px]")}>{value}</span>
      <span className="text-[9.5px] text-jolu-muted">{label}</span>
    </div>
  )
}

function ColumnHeader({ step, title, count, onAdd, addLabel }: { step: string; title: string; count: string; onAdd?: () => void; addLabel?: string }) {
  return (
    <div className="flex h-7 items-center gap-2 px-0.5">
      <span className="flex h-[18px] w-[18px] items-center justify-center rounded-full bg-jolu-selected text-[9.5px] text-jolu-on">{step}</span>
      <span className="text-[12.5px] text-jolu-text">{title}</span>
      <span className="text-[10px] text-jolu-muted">{count}</span>
      <div className="flex-1" />
      {onAdd && <button type="button" onClick={onAdd} className="motion-button flex items-center gap-1 rounded-full bg-jolu-selected px-2 py-0.5 text-[9.5px] text-jolu-on"><Plus className="h-2.5 w-2.5" />{addLabel}</button>}
    </div>
  )
}

function GroupCard({
  innerRef, name, stripe, icon, subtitle, models, avatars, expanded, onToggle, pinned, onPin, onHover, onDelete, closing, entering, dropType, onDropMember, editing, dragging, dropTarget, onGroupDragStart, onGroupDragOver, onGroupDrop, onGroupDragEnd, onKeyReorder, children,
}: {
  innerRef: (element: HTMLDivElement | null) => void
  name: string
  stripe: string
  icon: "folder" | "users"
  subtitle: string
  models: number
  avatars: PlatformId[]
  expanded: boolean
  onToggle: () => void
  pinned?: boolean
  onPin?: () => void
  onHover: (on: boolean) => void
  onDelete?: () => void
  closing: boolean
  entering: boolean
  dropType: "source" | "agent"
  onDropMember?: (id: string) => void
  editing?: boolean
  dragging?: boolean
  dropTarget?: boolean
  onGroupDragStart?: (event: React.DragEvent) => void
  onGroupDragOver?: (event: React.DragEvent) => void
  onGroupDrop?: (event: React.DragEvent) => void
  onGroupDragEnd?: () => void
  onKeyReorder?: (direction: -1 | 1) => void
  children?: React.ReactNode
}) {
  return (
    <div
      ref={innerRef}
      tabIndex={editing ? 0 : undefined}
      onKeyDown={(event) => {
        if (!editing || !onKeyReorder) return
        if (event.altKey && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
          event.preventDefault()
          onKeyReorder(event.key === "ArrowUp" ? -1 : 1)
        }
      }}
      onMouseEnter={() => onHover(true)}
      onMouseLeave={() => onHover(false)}
      onDragOver={(event) => {
        onGroupDragOver?.(event)
        if (onDropMember && event.dataTransfer.types.includes(`text/jolu-${dropType}`)) event.preventDefault()
      }}
      onDrop={(event) => {
        onGroupDrop?.(event)
        if (!onDropMember) return
        const id = event.dataTransfer.getData(`text/jolu-${dropType}`)
        if (!id) return
        event.preventDefault()
        onDropMember(id)
      }}
      className={cn(
        "gw-card relative overflow-hidden rounded-lg border border-jolu-border bg-jolu-bg outline-none",
        closing && "gw-card-closing",
        entering && "gw-group-enter",
        dragging && "gw-card-lifting",
        dropTarget && "border-jolu-accent ring-2 ring-jolu-accent/40",
      )}
    >
      <span className="absolute inset-y-0 left-0 w-[3px]" style={{ backgroundColor: stripe }} />
      <div className="flex items-center gap-2 px-2.5 pt-2">
        {editing ? (
          <span draggable onDragStart={onGroupDragStart} onDragEnd={onGroupDragEnd} className="cursor-grab active:cursor-grabbing">
            <GripVertical className="h-3 w-3 shrink-0 text-jolu-muted" />
          </span>
        ) : (
          <GripVertical className="h-3 w-3 shrink-0 text-jolu-border" />
        )}
        <span className="flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-lg bg-jolu-selected text-jolu-on">
          {icon === "folder" ? <Folder className="h-3.5 w-3.5" /> : <Users className="h-3.5 w-3.5" />}
        </span>
        <button type="button" onClick={onToggle} className="flex min-w-0 flex-1 flex-col items-start gap-0.5 text-left">
          <span className="truncate text-[11.5px] font-semibold text-jolu-text">{name}</span>
          <span className="truncate text-[8.5px] text-jolu-muted">{subtitle}</span>
        </button>
        {onPin && (
          <button type="button" onClick={onPin} className="text-jolu-border hover:text-jolu-muted">
            {pinned ? <Pin className="h-3 w-3 text-jolu-on" /> : <PinOff className="h-3 w-3" />}
          </button>
        )}
        <button type="button" onClick={onToggle} className="text-jolu-muted">
          <ChevronDown className={cn("h-3.5 w-3.5 transition-transform duration-[180ms]", expanded ? "rotate-180" : "")} />
        </button>
        {onDelete && <button type="button" onClick={onDelete} className="text-[8.5px] text-jolu-muted hover:text-jolu-danger">删除</button>}
      </div>
      <div className="flex items-center gap-1 px-2.5 py-1.5">
        <div className="flex -space-x-1.5">
          {avatars.slice(0, 4).map((platform, index) => (
            <span key={`${platform}-${index}`} className="flex h-5 w-5 items-center justify-center rounded-full border border-jolu-bg bg-jolu-side">
              <PlatformLogo platform={platform} size={12} />
            </span>
          ))}
        </div>
        {avatars.length > 4 && <span className="text-[8.5px] text-jolu-muted">+{avatars.length - 4}</span>}
        <div className="flex-1" />
        <span className="text-[9.5px] text-jolu-muted">{models > 0 ? `${models} 个模型` : "未接入 · 菜单不变"}</span>
      </div>
      <div className={cn("gw-expand", expanded && "open")}>
        <div>
          <div className="flex flex-col border-t border-jolu-table-divider">{children}</div>
        </div>
      </div>
    </div>
  )
}

function SourceRow({ connection, capability, busy, editing, groups, onServe, onRename, onMove }: {
  connection: Connection; capability?: SourceCapabilityDto; busy: boolean; editing: boolean; groups: GatewayGroupDto[]; onServe: () => void; onRename: () => void; onMove: (groupId: string | null) => void
}) {
  return (
    <div className="flex items-center gap-2 border-b border-jolu-table-divider px-2.5 py-1.5 last:border-0" draggable={editing} onDragStart={(event) => { event.dataTransfer.setData("text/jolu-source", connection.id); event.dataTransfer.effectAllowed = "move" }}>
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="truncate text-[10.5px] text-jolu-text">{connection.baseName}</span>
        <span className="truncate text-[8.5px] text-jolu-muted">{connection.kind === "api" ? "API Key" : "官方订阅"} · {connection.slug ?? "无 id"}</span>
        <span title={capability?.reason} className={cn("truncate text-[8.5px]", capability?.state === "needs_auth" ? "text-jolu-danger" : "text-jolu-muted")}>{capability?.state === "needs_auth" ? "需要重新登录 · 到账号管理补全授权" : capability?.state === "unsupported" ? (connection.serve_enabled ? "已选择 · 转发适配尚未就绪" : "可选择接入 · 转发适配尚未就绪") : capability?.reason ?? "正在检查调用能力"}</span>
      </div>
      {editing && (
        <>
          <select value={groups.find((group) => group.members.includes(connection.id))?.id ?? ""} onChange={(event) => onMove(event.target.value || null)} className="h-6 rounded border border-jolu-border bg-jolu-bg px-1 text-[8.5px] text-jolu-text">
            <option value="">未分组</option>
            {groups.map((group) => <option key={group.id} value={group.id}>{group.name}</option>)}
          </select>
          <button type="button" onClick={onRename} className="text-[8.5px] text-jolu-muted hover:text-jolu-text">改 id</button>
        </>
      )}
      <JoluSwitch on={connection.serve_enabled} label={`${connection.baseName} 对外服务`} disabled={!canToggleSource(connection.serve_enabled, busy, capability)} onChange={onServe} />
    </div>
  )
}

function AgentRow({ entry, busy, editing, groups, onWire, onUnwire, onAccept, onMove }: {
  entry: WiringStatusDto; busy: boolean; editing: boolean; groups: GatewayGroupDto[]; onWire: () => void; onUnwire: () => void; onAccept: () => void; onMove: (groupId: string | null) => void
}) {
  const drift = entry.drift
  const label = entry.wired ? "已接入" : drift === "pending" ? "待修复" : drift ? "有改动" : entry.reason ?? "未接入"
  return (
    <div className="flex flex-col gap-1 border-b border-jolu-table-divider px-2.5 py-1.5 last:border-0" draggable={editing} onDragStart={(event) => { event.dataTransfer.setData("text/jolu-agent", entry.agent); event.dataTransfer.effectAllowed = "move" }}>
      <div className="flex items-center gap-2">
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="truncate text-[10.5px] text-jolu-text">{agentLabel(entry.agent)}</span>
          <span className={cn("truncate text-[8.5px]", entry.wired ? "text-jolu-on" : drift ? "text-jolu-danger" : "text-jolu-muted")}>{label}</span>
        </div>
        {editing && (
          <select value={groups.find((group) => group.members.includes(entry.agent))?.id ?? ""} onChange={(event) => onMove(event.target.value || null)} className="h-6 rounded border border-jolu-border bg-jolu-bg px-1 text-[8.5px] text-jolu-text">
            <option value="">未分组</option>
            {groups.map((group) => <option key={group.id} value={group.id}>{group.name}</option>)}
          </select>
        )}
        {entry.wired ? (
          <button type="button" disabled={busy} onClick={onUnwire} className="text-[8.5px] text-jolu-muted hover:text-jolu-danger">断开</button>
        ) : drift ? (
          <>
            <button type="button" disabled={busy} onClick={onWire} className="rounded bg-jolu-selected px-1.5 py-0.5 text-[8.5px] text-jolu-on">重新应用</button>
            <button type="button" disabled={busy} onClick={onAccept} className="text-[8.5px] text-jolu-muted">接受现状</button>
          </>
        ) : (
          <button type="button" disabled={busy} onClick={onWire} className="rounded bg-jolu-selected px-1.5 py-0.5 text-[8.5px] text-jolu-on">接线</button>
        )}
      </div>
      {entry.notice && entry.wired && <span className="text-[8px] text-jolu-muted">{entry.notice}</span>}
    </div>
  )
}

function AddCard({ label, hint, onClick }: { label: string; hint: string; onClick: () => void }) {
  return (
    <button type="button" onClick={onClick} className="motion-button flex h-11 items-center gap-2 rounded-lg border border-dashed border-jolu-border px-3 text-left">
      <span className="flex h-5 w-5 items-center justify-center rounded-full bg-jolu-selected"><Plus className="h-3 w-3 text-jolu-on" /></span>
      <span className="text-[10.5px] text-jolu-muted">{label}</span>
      <span className="flex-1" />
      <span className="text-[8.5px] text-jolu-muted">{hint}</span>
    </button>
  )
}

function FlatList({ children }: { children: React.ReactNode }) {
  return <div className="flex flex-col overflow-hidden rounded-lg border border-jolu-border bg-jolu-bg">{children}</div>
}

function RouteLog({ rows }: { rows: RouteLogDto[] }) {
  return (
    <section className="flex flex-col overflow-hidden rounded-xl border border-jolu-border bg-jolu-bg">
      <header className="flex h-11 items-center gap-2 px-4">
        <Link2 className="h-3.5 w-3.5 text-jolu-on" />
        <span className="text-[12.5px] text-jolu-text">路由流水</span>
        <span className="text-[10px] text-jolu-muted">最近 {rows.length} 条</span>
      </header>
      {rows.length === 0 ? (
        <div className="border-t border-jolu-border px-4 py-8 text-center text-[10.5px] text-jolu-muted">网关收到请求后，这里会显示时间、Agent、请求模型、路由到的来源、耗时和结果。</div>
      ) : (
        <div className="flex flex-col">
          <div className="grid grid-cols-[70px_92px_1fr_1fr_70px_70px] gap-2 border-t border-jolu-border bg-jolu-side px-4 py-2 text-[9px] text-jolu-muted">
            <span>时间</span><span>Agent</span><span>请求模型</span><span>路由到</span><span>耗时</span><span>结果</span>
          </div>
          {rows.map((row, index) => (
            <div key={`${row.ts}-${index}`} className={cn("gw-row-in grid grid-cols-[70px_92px_1fr_1fr_70px_70px] items-center gap-2 border-t border-jolu-table-divider px-4 py-2 text-[10px]", row.result === "failed" && "gw-row-fail")}>
              <span className="text-jolu-muted tabular-nums">{new Date(row.ts).toLocaleTimeString("zh-CN", { hour12: false })}</span>
              <span className="truncate text-jolu-text">{row.agent ? agentLabel(row.agent) : "—"}</span>
              <span className="truncate text-jolu-muted">{row.requested_model}</span>
              <span className="truncate text-jolu-text">{row.source ? `${row.source} · ${row.upstream_model ?? "—"}` : "—"}</span>
              <span className="text-jolu-muted tabular-nums">{row.duration_ms != null ? `${row.duration_ms}ms` : "—"}</span>
              <span className={cn(row.result === "ok" ? "text-jolu-on" : row.result === "rejected" ? "text-jolu-muted" : "text-jolu-danger")}>
                {row.result === "ok" ? "成功" : row.result === "rejected" ? "拒绝" : "失败"}
              </span>
            </div>
          ))}
        </div>
      )}
    </section>
  )
}

function Notice({ tone, children }: { tone: "muted" | "danger"; children: React.ReactNode }) {
  return <div className={cn("rounded-lg px-3 py-2 text-[11px]", tone === "danger" ? "bg-jolu-status-error-bg text-jolu-danger" : "bg-jolu-side text-jolu-muted")}>{children}</div>
}

/* ─────────────────────────── 弹窗 ─────────────────────────── */

function Modal({ title, icon, tone, children, footer, onClose }: { title: string; icon?: React.ReactNode; tone?: "danger"; children: React.ReactNode; footer: React.ReactNode; onClose: () => void }) {
  return (
    <div className="motion-overlay fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-6" onClick={onClose}>
      <div className="motion-dialog gw-dialog-enter flex max-h-[86vh] w-full max-w-[540px] flex-col gap-3 overflow-y-auto rounded-[14px] border border-jolu-border bg-jolu-bg p-5 shadow-2xl" onClick={(event) => event.stopPropagation()}>
        <div className="flex items-center gap-2">
          {icon}
          <span className={cn("text-[16px] font-semibold", tone === "danger" ? "text-jolu-danger" : "text-jolu-text")}>{title}</span>
          <div className="flex-1" />
          <button type="button" onClick={onClose} className="text-jolu-muted"><X className="h-3.5 w-3.5" /></button>
        </div>
        {children}
        <div className="flex justify-end gap-2">{footer}</div>
      </div>
    </div>
  )
}

function Button({ label, primary, danger, disabled, onClick }: { label: string; primary?: boolean; danger?: boolean; disabled?: boolean; onClick: () => void }) {
  return (
    <button type="button" disabled={disabled} onClick={onClick} className={cn("motion-button h-9 rounded-lg px-3.5 text-[10.5px] disabled:opacity-40", primary ? (danger ? "bg-jolu-danger text-white" : "bg-jolu-accent text-jolu-on") : "border border-jolu-border text-jolu-text")}>{label}</button>
  )
}

function Warning({ children, tone = "danger" }: { children: React.ReactNode; tone?: "danger" | "muted" }) {
  return <div className={cn("flex items-start gap-2 rounded-lg px-3 py-2.5 text-[10px]", tone === "danger" ? "bg-jolu-danger-soft text-jolu-danger" : "bg-jolu-side text-jolu-muted")}><TriangleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" /><span>{children}</span></div>
}

function NewGroupDialog({ kind, busy, onClose, onCreate }: { kind: GroupKind; busy: boolean; onClose: () => void; onCreate: (name: string) => void }) {
  const [name, setName] = useState("")
  return (
    <Modal title={kind === "source" ? "新建来源分组" : "新建 Agent 分组"} onClose={onClose} footer={<><Button label="取消" onClick={onClose} /><Button label="创建" primary disabled={busy || !name.trim()} onClick={() => onCreate(name.trim())} /></>}>
      <p className="text-[10px] text-jolu-muted">只影响排列，不改变路由、接线和 Agent 菜单。成员之后可以随时移动。</p>
      <input autoFocus value={name} onChange={(event) => setName(event.target.value)} placeholder="分组名称" className="h-9 rounded-lg border border-jolu-border bg-jolu-side px-3 text-[11px] text-jolu-text outline-none focus:border-jolu-accent" />
    </Modal>
  )
}

function ConfirmDialog({ title, warning, confirm, danger, busy, onClose, onConfirm }: { title: string; warning: string; confirm: string; danger?: boolean; busy: boolean; onClose: () => void; onConfirm: () => void }) {
  return <Modal title={title} tone={danger ? "danger" : undefined} onClose={onClose} footer={<><Button label="取消" onClick={onClose} /><Button label={confirm} primary danger={danger} disabled={busy} onClick={onConfirm} /></>}><Warning>{warning}</Warning></Modal>
}

function MessageDialog({ title, tone, message, action, onClose }: { title: string; tone?: "danger"; message: string; action?: { label: string; onClick: () => void }; onClose: () => void }) {
  return (
    <Modal title={title} tone={tone} icon={<TriangleAlert className={cn("h-4 w-4", tone === "danger" ? "text-jolu-danger" : "text-jolu-muted")} />} onClose={onClose} footer={<><Button label="关闭" onClick={onClose} />{action && <Button label={action.label} primary onClick={action.onClick} />}</>}>
      <Warning>{message}</Warning>
    </Modal>
  )
}

function RenameDialog({ connection, busy, onClose, onSave }: { connection: Connection; busy: boolean; onClose: () => void; onSave: (name: string, slug: string) => void }) {
  const [name, setName] = useState(connection.baseName)
  const [slug, setSlug] = useState(connection.slug ?? "")
  const reserved = ["route", "auto", "jolu", "group", "default"].includes(slug.trim())
  const invalid = !!slug && (!/^[a-z0-9-]+$/.test(slug) || slug.startsWith("-") || reserved)
  return (
    <Modal title="重命名来源" onClose={onClose} footer={<><Button label="取消" onClick={onClose} /><Button label="保存" primary disabled={busy || invalid || !name.trim()} onClick={() => onSave(name.trim(), slug.trim())} /></>}>
      <div className="flex flex-col gap-1">
        <span className="text-[9.5px] text-jolu-muted">显示名称</span>
        <input value={name} onChange={(event) => setName(event.target.value)} className="h-9 rounded-lg border border-jolu-border bg-jolu-side px-3 text-[11px] text-jolu-text outline-none focus:border-jolu-accent" />
        <span className="text-[8.5px] text-jolu-muted">只改显示名；已接入的工具不受影响。</span>
      </div>
      <div className="flex flex-col gap-1">
        <span className="text-[9.5px] text-jolu-muted">菜单 id</span>
        <input value={slug} onChange={(event) => setSlug(event.target.value)} className={cn("h-9 rounded-lg border bg-jolu-side px-3 text-[11px] text-jolu-text outline-none", invalid ? "border-jolu-danger" : "border-jolu-border focus:border-jolu-accent")} />
        {invalid ? <span className="text-[8.5px] text-jolu-danger">{reserved ? `「${slug}」是保留字，建议改用 ${slug}-src` : "只能使用小写字母、数字和连字符，不能以连字符开头。"}</span> : <span className="text-[8.5px] text-jolu-muted">Agent 配置里存的是这个 id；改名不会改它，改 id 后旧 id 会返回 404 并说明新名字。</span>}
      </div>
    </Modal>
  )
}

function WiringDialog({ agents, busy, onClose, onWire, onUnwire, onAccept }: { agents: WiringStatusDto[]; busy: boolean; onClose: () => void; onWire: (agent: string) => void; onUnwire: (agent: string) => void; onAccept: (agent: string) => void }) {
  return (
    <Modal title="Agent 接线" onClose={onClose} footer={<Button label="关闭" onClick={onClose} />}>
      <p className="text-[9.5px] text-jolu-muted">状态按配置文件里实际写入的值判断；重新应用不会重新拍快照。</p>
      <div className="flex flex-col gap-1.5">
        {agents.map((entry) => (
          <div key={entry.agent} className="flex items-center gap-2 rounded-lg border border-jolu-border bg-jolu-bg px-2.5 py-2">
            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className="text-[10.5px] text-jolu-text">{agentLabel(entry.agent)}</span>
              <span className={cn("truncate text-[8.5px]", entry.wired ? "text-jolu-on" : entry.drift ? "text-jolu-danger" : "text-jolu-muted")}>
                {entry.wired ? "已接入" : entry.drift === "foreign" ? "已被另一个 Jolu 接管" : entry.drift === "removed" ? "配置里已移除" : entry.drift === "changed" ? "配置被改写" : entry.drift === "pending" ? "上次接线未完成" : entry.reason ?? "未接入"}
              </span>
            </div>
            {entry.wired ? <Button label="断开" disabled={busy} onClick={() => onUnwire(entry.agent)} /> : entry.drift ? (<><Button label="重新应用" primary disabled={busy} onClick={() => onWire(entry.agent)} /><Button label="接受现状" disabled={busy} onClick={() => onAccept(entry.agent)} /></>) : (<Button label="接线" primary disabled={busy || (!entry.config_path && !!entry.reason)} onClick={() => onWire(entry.agent)} />)}
          </div>
        ))}
      </div>
      <span className="text-[8.5px] text-jolu-muted">同一工具的第二份安装（WSL、便携版）暂不支持，只在主安装上接线。</span>
    </Modal>
  )
}

function CodexDialog({ mode, busy, onClose, onConfirm }: { mode: "oauth" | "provider"; busy: boolean; onClose: () => void; onConfirm: () => void }) {
  const oauth = mode === "oauth"
  return (
    <Modal title="接入 Codex" onClose={onClose} footer={<><Button label="取消" onClick={onClose} /><Button label="接入" primary disabled={busy} onClick={onConfirm} /></>}>
      <p className="text-[10px] text-jolu-muted">Jolu 会把 Codex 的请求转到本机模型入口；只写入 config.toml。</p>
      <div className={cn("flex flex-col gap-1.5 rounded-lg border p-3", oauth ? "border-jolu-on bg-jolu-status-success-bg" : "border-jolu-border bg-jolu-side")}>
        <span className="text-[11px] font-semibold text-jolu-text">{oauth ? "保留 ChatGPT 登录" : "使用网关 Key"}</span>
        <span className="text-[9px] text-jolu-muted">{oauth ? "检测到 ChatGPT 登录：只改请求地址（openai_base_url），不写 Key、不动 auth.json。" : "没有检测到 ChatGPT 登录：写自定义 provider 和 jolu-codex 令牌，不改其他 provider。"}</span>
      </div>
      {!oauth && <Warning tone="muted">如果以后登录 ChatGPT，重新接线即可切回保留登录的接法。</Warning>}
      <Warning tone="muted">Codex 发 Responses 请求；只支持 Chat Completions 的上游会在请求时返回 404。</Warning>
    </Modal>
  )
}

function GeminiDialog({ busy, onClose, onConfirm }: { busy: boolean; onClose: () => void; onConfirm: () => void }) {
  return (
    <Modal title="接入 Gemini CLI" tone="danger" icon={<TriangleAlert className="h-4 w-4 text-jolu-danger" />} onClose={onClose} footer={<><Button label="取消" onClick={onClose} /><Button label="确认接入" primary disabled={busy} onClick={onConfirm} /></>}>
      <Warning>接线期间 Gemini 会改用 Jolu 网关 Key，不再使用 Google 账号登录。OAuth 凭据文件不会被删除。</Warning>
      <div className="flex flex-col rounded-lg bg-jolu-side px-3">
        <Line label="当前登录方式" value="Google 账号 · oauth-personal" />
        <Line label="接线期间" value="网关 Key · gemini-api-key" danger />
        <Line label="断开时" value="恢复原登录方式与原 Key" />
      </div>
      <span className="text-[8.5px] text-jolu-muted">原值会存进接线记录；如果断开时还原失败，会在「Agent 接线」里提示并提供重试。</span>
    </Modal>
  )
}

function CatalogDialog({ models, onClose }: { models: CatalogModelDto[]; onClose: () => void }) {
  const toast = useToast()
  return (
    <Modal title="模型池里的模型" onClose={onClose} footer={<Button label="关闭" onClick={onClose} />}>
      <p className="text-[9.5px] text-jolu-muted">复制「来源/模型」填入调用工具。订阅模型菜单不代表已完成真实调用验收。</p>
      <div className="flex max-h-[50vh] flex-col gap-1 overflow-y-auto">
        {models.map((model) => (
          <div key={model.id} className="flex items-center gap-2 rounded-lg bg-jolu-side px-2.5 py-1.5">
            <span className="truncate text-[10px] text-jolu-text">{model.id}</span>
            <button type="button" className="shrink-0 text-[9px] text-jolu-on" onClick={() => { void navigator.clipboard.writeText(model.id).then(() => toast.success("模型名已复制"), () => toast.danger("复制失败，请重试")) }}>复制模型名</button>
            <div className="flex-1" />
            <span className={cn("text-[8.5px]", model.status === "connected" ? "text-jolu-muted" : "text-jolu-danger")}>{model.status === "connected" ? model.source : "未连接"}</span>
          </div>
        ))}
      </div>
    </Modal>
  )
}

function Line({ label, value, danger }: { label: string; value: string; danger?: boolean }) {
  return (
    <div className="flex h-8 items-center gap-2 border-b border-jolu-table-divider text-[9.5px] last:border-0">
      <span className="text-jolu-muted">{label}</span>
      <div className="flex-1" />
      <span className={cn("font-semibold", danger ? "text-jolu-danger" : "text-jolu-text")}>{value}</span>
    </div>
  )
}

export type { GatewayMode }
