import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { Cable, Settings2, ShieldAlert } from "lucide-react"
import { cn } from "@/lib/utils"
import { api, isTauri } from "@/lib/api"
import {
  gatewayApi, type CatalogModelDto, type GatewayGroupsDto, type GatewayMode,
  type GatewayStatusDto, type SourceCapabilityDto, type WiringStatusDto,
} from "@/lib/gatewayApi"
import { agentLabel, canEscape, circuitText, wiredCount } from "@/lib/gatewayPage"
import { sourceTone, type SessionView } from "@/lib/gatewaySessions"
import { useConnections } from "@/hooks/useConnections"
import { useGatewaySessions } from "@/hooks/useGatewaySessions"
import { useToast } from "@/components/ui/Toast"
import { ConfirmDialog as ActionConfirmDialog } from "@/components/ui/ConfirmDialog"
import { GatewayAccessBar } from "@/components/gateway/GatewayAccessBar"
import { SessionGraph, type GraphSource } from "@/components/gateway/SessionGraph"
import { RouteLogPanel } from "@/components/gateway/RouteLogPanel"
import {
  CatalogDialog, CodexDialog, GeminiDialog, MessageDialog, RenameDialog, WiringDialog, orderGroups,
} from "@/components/gateway/GatewayDialogs"
import { SourceManageDialog } from "@/components/gateway/SourceManageDialog"
import type { Connection } from "@/types"

/**
 * 统一网关（Jolu.pen J6-01 `VcNCX` / J6-02 `qGotz` / J6-03 `vRih5`；开发说明 `docs/specs/2026-11-06-unified-gateway-development.md`）。
 *
 * 页面：单行连接信息 → 视图工具栏 → 模型来源直连最近会话 → 路由流水（每页 10 条）→ 安全说明与操作。
 * 来源进出模型池、改名与菜单 id、分组管理收进「编辑布局」打开的管理弹窗；接线在「接入设置」。
 * 展示结构变化不改变实际选路逻辑。
 */

type Dialog =
  | { kind: "manage" }
  | { kind: "rename"; connection: Connection }
  | { kind: "serve-conflict"; message: string }
  | { kind: "wiring" }
  | { kind: "codex" }
  | { kind: "gemini" }
  | { kind: "restore-failed"; message: string; agent: string }
  | { kind: "catalog"; models?: CatalogModelDto[] }
  | { kind: "confirm"; title: string; description: string; label: string; danger?: boolean; work: () => Promise<boolean | void> }
  | null

const EMPTY_GROUPS: GatewayGroupsDto = { sources: [], agents: [] }
const EMPTY_WIRING: WiringStatusDto[] = []
const VIEW_KEY = "jolu:gateway:view:v6"
const PIN_KEY = "jolu:gateway:pinned"
/** 会话平台 → 接线里的 Agent id（「只看」按 Agent 分组筛选会话用） */
const SESSION_AGENT: Record<string, string> = { claude: "claude-code", codex: "codex", gemini: "gemini", opencode: "opencode", zcode: "zcode" }

type ViewState = { view?: "group" | "flat"; onlyServing?: boolean; filterGroup?: string }

function readJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key)
    return raw ? (JSON.parse(raw) as T) : fallback
  } catch {
    return fallback
  }
}

export function GatewayView() {
  const toast = useToast()
  const { connections, reload } = useConnections()
  const sessions = useGatewaySessions()
  const [status, setStatus] = useState<GatewayStatusDto | null>(null)
  const [groups, setGroups] = useState<GatewayGroupsDto>(EMPTY_GROUPS)
  const [capabilities, setCapabilities] = useState<SourceCapabilityDto[]>([])
  const [catalog, setCatalog] = useState<CatalogModelDto[]>([])
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [tick, setTick] = useState(0)
  const operationLock = useRef(false)
  const accessTrigger = useRef<HTMLButtonElement>(null)
  const initialView = useMemo(() => readJson<ViewState>(VIEW_KEY, {}), [])
  // J6 主稿默认「平铺」+「仅看已接入」
  const [view, setView] = useState<"group" | "flat">(initialView.view === "group" ? "group" : "flat")
  const [onlyServing, setOnlyServing] = useState(initialView.onlyServing !== false)
  const [filterGroup, setFilterGroup] = useState(initialView.filterGroup ?? "")
  const [pinned, setPinned] = useState<string[]>(() => readJson<string[]>(PIN_KEY, []))
  const [dialog, setDialog] = useState<Dialog>(null)

  const refresh = useCallback(async () => {
    if (!isTauri) {
      setLoading(false)
      return
    }
    try {
      const [nextStatus, nextGroups, nextCatalog, nextCapabilities] = await Promise.all([
        gatewayApi.status(), gatewayApi.groups(), gatewayApi.catalog(), gatewayApi.sourceCapabilities(),
      ])
      setCapabilities(nextCapabilities)
      setStatus(nextStatus)
      setGroups(nextGroups)
      setCatalog(nextCatalog)
    } catch (error) {
      toast.danger("读取统一网关失败", String(error))
    } finally {
      setLoading(false)
    }
  }, [toast])

  useEffect(() => {
    void refresh()
    const timer = window.setInterval(() => {
      if (!isTauri || document.hidden) return
      void refresh()
      setTick((value) => value + 1)
    }, 15_000)
    return () => window.clearInterval(timer)
  }, [refresh])

  useEffect(() => {
    try { localStorage.setItem(VIEW_KEY, JSON.stringify({ view, onlyServing, filterGroup })) } catch { /* 本地偏好存不进去只影响下次打开 */ }
  }, [view, onlyServing, filterGroup])

  const run = async (work: () => Promise<unknown>, ok?: string, throwOnFailure = false) => {
    if (operationLock.current) return false
    operationLock.current = true
    setBusy(true)
    try {
      await work()
      if (ok) toast.success(ok)
      await Promise.all([refresh(), reload()])
      return true
    } catch (error) {
      toast.danger("操作失败", String(error))
      if (throwOnFailure) throw error
      return false
    } finally {
      operationLock.current = false
      setBusy(false)
    }
  }

  const agents = status?.wiring ?? EMPTY_WIRING
  const serving = connections.filter((connection) => connection.serve_enabled)
  const wired = wiredCount(status)
  const sourceGroups = orderGroups(groups.sources, pinned, "source")
  const agentGroups = orderGroups(groups.agents, pinned, "agent")

  /* ── 左栏来源：分组 = 按来源分组顺序排列（只影响排列），平铺 = 连接顺序；仅看已接入 = 只列模型池中的来源 ── */
  const graphSources: GraphSource[] = useMemo(() => {
    const ordered = view === "group"
      ? [
        ...sourceGroups.flatMap((group) => group.members.map((id) => connections.find((c) => c.id === id)).filter((c): c is Connection => !!c)),
        ...connections.filter((c) => !sourceGroups.some((group) => group.members.includes(c.id))),
      ]
      : connections
    const list = onlyServing ? ordered.filter((connection) => connection.serve_enabled) : ordered
    return list.map((connection, index) => ({ connection, tone: sourceTone(index) }))
  }, [view, onlyServing, connections, sourceGroups])
  const shownSlugs = new Set(graphSources.map((source) => source.connection.slug).filter(Boolean))
  const shownModels = catalog.filter((model) => shownSlugs.has(model.slug)).length

  /* ── 「只看」：按 Agent 分组筛选会话 ── */
  const views: SessionView[] = useMemo(() => {
    if (!filterGroup) return sessions.views
    const members = agentGroups.find((group) => group.id === filterGroup)?.members ?? []
    return sessions.views.filter((item) => members.includes(item.session.agent ?? SESSION_AGENT[item.session.platform] ?? item.session.platform))
  }, [filterGroup, agentGroups, sessions.views])

  const togglePin = (key: string) => {
    setPinned((current) => {
      const next = current.includes(key) ? current.filter((item) => item !== key) : [...current, key]
      try { localStorage.setItem(PIN_KEY, JSON.stringify(next)) } catch { /* 本地偏好存不进去只影响下次 */ }
      return next
    })
  }

  const wire = (agent: string) => {
    if (agent === "codex") return setDialog({ kind: "codex" })
    if (agent === "gemini") return setDialog({ kind: "gemini" })
    setDialog({ kind: "confirm", title: `接入 ${agentLabel(agent)}？`, description: "将更新这个工具的本机配置，使其通过 Jolu 统一网关调用模型。原连接配置会被保留，断开时恢复。", label: "确认接入", work: () => run(() => gatewayApi.setWiring(agent, true), `已接入 ${agentLabel(agent)}`, true) })
  }
  const confirmUnwire = (agent: string) => setDialog({ kind: "confirm", title: `断开 ${agentLabel(agent)}？`, description: "将恢复这个工具接入前的连接配置，后续请求不再通过 Jolu 统一网关。", label: "确认断开", danger: true, work: () => run(() => gatewayApi.setWiring(agent, false), `已断开 ${agentLabel(agent)}`, true) })
  const confirmAccept = (agent: string) => setDialog({ kind: "confirm", title: `接受 ${agentLabel(agent)} 的现有配置？`, description: "将以工具当前的外部配置为准，结束 Jolu 对这份接线记录的管理。确认前请核对配置变动。", label: "接受现状", work: () => run(() => gatewayApi.acceptWiring(agent), "已接受现状", true) })

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

  /** 来源卡与管理弹窗共用：加入 / 移出模型池前确认；加入后的调用能力以实际请求为准 */
  const askServe = (connection: Connection) => {
    const enable = !connection.serve_enabled
    setDialog({
      kind: "confirm",
      title: enable ? `把「${connection.baseName}」加入模型池？` : `把「${connection.baseName}」移出模型池？`,
      description: enable
        ? `加入后，接入网关的工具可以按「${connection.slug ?? "来源"}/模型」调用这个来源；授权与转发能力以实际请求为准。`
        : `移出后，工具再请求「${connection.slug ?? "来源"}/…」会返回不可用；账号、Key 与历史记录不会删除。`,
      label: enable ? "加入模型池" : "移出模型池",
      danger: !enable,
      work: async () => {
        if (!enable) return run(() => gatewayApi.setServe(connection.id, false), `已移出 ${connection.baseName}`, true)
        const ok = await run(() => gatewayApi.setServe(connection.id, true), undefined, true)
        const capability = capabilities.find((item) => item.connection_id === connection.id)
        if (ok) toast.success(capability?.can_serve ? `已把 ${connection.baseName} 加入模型池` : `已选择 ${connection.baseName}，${capability?.state === "needs_auth" ? "请补全账号授权后调用" : "转发尚未就绪"}`)
        return ok
      },
    })
  }

  const toggleRelay = (on: boolean) => {
    const work = (throwOnFailure = false) => run(() => gatewayApi.setMode(on ? "relay" : "observe", status?.default_connection ?? null), on ? "已允许接入" : "已关闭接入，一键接入的工具已恢复原连接", throwOnFailure)
    if (!on && wired > 0) setDialog({ kind: "confirm", title: "关闭统一网关接入？", description: `将停止统一网关转发，并把 ${wired} 个已接入工具恢复到原连接。正在使用统一网关的调用可能中断。`, label: "确认关闭", danger: true, work: () => work(true) })
    else void work()
  }

  const segment = (value: "group" | "flat", label: string) => (
    <button type="button" aria-pressed={view === value} onClick={() => setView(value)}
      className={cn("h-8 rounded-lg px-2.5 text-[12px] outline-none transition-colors focus-visible:outline-2 focus-visible:outline-jolu-accent", view === value ? "bg-jolu-bg text-jolu-text shadow-sm" : "text-jolu-muted hover:text-jolu-text")}>
      {label}
    </button>
  )

  return (
    <div className="flex h-full flex-col gap-6 overflow-y-auto pb-6 pr-1">
      <header className="flex min-h-16 items-center justify-between gap-4 pt-1">
        <div className="flex min-w-0 flex-col gap-[5px]">
          <h1 className="text-[22px] font-semibold text-jolu-text">统一网关</h1>
          <p className="max-w-[660px] text-xs text-jolu-muted">模型来源直连最近会话，实时查看 AI 正在处理的任务。</p>
        </div>
        <button ref={accessTrigger} type="button" onClick={() => setDialog({ kind: "wiring" })} className="jolu-control motion-button flex h-8 shrink-0 items-center gap-1.5 rounded-lg border border-jolu-border bg-jolu-cc-surface px-[11px] text-[11.5px] text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <Cable aria-hidden className="h-3.5 w-3.5" />
          接入设置
        </button>
      </header>

      <GatewayAccessBar status={status} loading={loading} busy={busy} desktop={isTauri}
        sources={serving.length} models={catalog.length} agents={wired} onToggle={toggleRelay} />

      <div className="-mb-2 flex h-10 shrink-0 items-center gap-2.5">
        <div className="flex items-center gap-0.5 rounded-lg bg-jolu-track p-0.5" role="group" aria-label="来源排列">
          {segment("group", "分组")}
          {segment("flat", "平铺")}
        </div>
        <label className="flex h-9 items-center gap-1 rounded-lg px-2.5 text-[12px] text-jolu-text">
          <span className="text-jolu-text">只看：</span>
          <select value={filterGroup} onChange={(event) => setFilterGroup(event.target.value)} aria-label="按 Agent 分组筛选会话"
            className="bg-transparent text-[12px] text-jolu-text outline-none">
            <option value="">全部会话</option>
            {agentGroups.map((group) => <option key={group.id} value={group.id}>{group.name}</option>)}
          </select>
        </label>
        <button type="button" aria-pressed={onlyServing} onClick={() => setOnlyServing((value) => !value)}
          className={cn("flex h-9 items-center gap-1.5 rounded-lg px-2.5 text-[11px] outline-none transition-colors focus-visible:outline-2 focus-visible:outline-jolu-accent", onlyServing ? "bg-jolu-selected text-jolu-on" : "text-jolu-muted hover:text-jolu-text")}>
          <span className={cn("size-[5px] rounded-full", onlyServing ? "bg-jolu-accent" : "bg-jolu-border")} />
          仅看已接入
        </button>
        <div className="flex-1" />
        <button type="button" onClick={() => setDialog({ kind: "manage" })} title="加入 / 移出模型池、改名与菜单 id、分组与排序"
          className="jolu-control motion-button flex h-9 items-center gap-1.5 rounded-lg border border-jolu-border bg-jolu-bg px-3 text-[11.5px] text-jolu-text outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <Settings2 aria-hidden className="size-3.5 text-jolu-muted" />
          管理来源
        </button>
      </div>

      <SessionGraph sources={graphSources} views={views} sourceLive={sessions.sourceLive} modelCount={shownModels} desktop={isTauri}
        loading={sessions.loading} loaded={sessions.loaded} error={sessions.error} freshness={sessions.freshness} busy={busy}
        onRetry={sessions.refresh} onToggleServe={askServe} />

      <RouteLogPanel desktop={isTauri} connections={connections} tick={tick} />

      <section className="flex min-h-16 shrink-0 items-center gap-2.5 rounded-xl bg-jolu-cc-surface px-5">
        <ShieldAlert aria-hidden className="size-[15px] shrink-0 text-jolu-on" />
        <span className="text-[10.5px] text-jolu-muted">只监听本机回环 · {circuitText(status)} · 停止接入不会删除账号、Key 或历史记录。</span>
        <div className="flex-1" />
        <button type="button" disabled={busy || !canEscape(status) || !isTauri} onClick={() => void run(() => gatewayApi.escape(), "已切回直连")}
          className="motion-button h-[30px] shrink-0 rounded-lg bg-jolu-cc-surface px-3 text-[10.5px] text-jolu-danger outline-none hover:bg-jolu-side focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-40">
          紧急切回直连
        </button>
      </section>

      {dialog?.kind === "manage" && (
        <SourceManageDialog connections={connections} capabilities={capabilities} catalog={catalog} groups={groups} agents={agents}
          pinned={pinned} busy={busy} run={run} onClose={() => setDialog(null)}
          onRename={(connection) => setDialog({ kind: "rename", connection })}
          onShowModels={(models) => setDialog({ kind: "catalog", models })}
          onTogglePin={togglePin}
          onWire={wire} onUnwire={confirmUnwire} onAccept={confirmAccept} />
      )}
      {dialog?.kind === "rename" && (
        <RenameDialog connection={dialog.connection} busy={busy} onClose={() => setDialog({ kind: "manage" })} onSave={async (name, slug) => {
          const ok = await run(async () => {
            if (name !== dialog.connection.baseName) await api.renameConnection(dialog.connection.id, name)
            if (slug && slug !== dialog.connection.slug) await gatewayApi.setSlug(dialog.connection.id, slug)
          }, "已保存")
          if (ok) setDialog({ kind: "manage" })
        }} />
      )}
      {dialog?.kind === "serve-conflict" && <MessageDialog title="这个来源没有加入模型池" tone="danger" message={dialog.message} onClose={() => setDialog({ kind: "manage" })} />}
      {dialog?.kind === "wiring" && <WiringDialog agents={agents} busy={busy} onClose={() => setDialog(null)} onWire={(agent) => { setDialog(null); wire(agent) }} onUnwire={confirmUnwire} onAccept={confirmAccept} />}
      {dialog?.kind === "codex" && <CodexDialog returnFocusTo={accessTrigger.current} mode={agents.find((entry) => entry.agent === "codex")?.codex_mode ?? "provider"} busy={busy} onClose={() => setDialog(null)} onConfirm={() => run(() => gatewayApi.setWiring("codex", true), "已接入 Codex", true)} />}
      {dialog?.kind === "gemini" && <GeminiDialog returnFocusTo={accessTrigger.current} busy={busy} onClose={() => setDialog(null)} onConfirm={() => run(() => gatewayApi.setWiring("gemini", true), "已接入 Gemini CLI", true)} />}
      {dialog?.kind === "restore-failed" && <MessageDialog title="没能恢复工具配置" tone="danger" message={dialog.message} action={{ label: "重试断开", onClick: async () => { setDialog(null); await unwire(dialog.agent) } }} onClose={() => setDialog(null)} />}
      {dialog?.kind === "confirm" && <ActionConfirmDialog returnFocusTo={accessTrigger.current} title={dialog.title} description={dialog.description} confirmLabel={dialog.label} danger={dialog.danger} onClose={() => setDialog(null)} onConfirm={dialog.work} />}
      {dialog?.kind === "catalog" && <CatalogDialog models={dialog.models ?? catalog} onClose={() => setDialog({ kind: "manage" })} />}
    </div>
  )
}

export type { GatewayMode }
