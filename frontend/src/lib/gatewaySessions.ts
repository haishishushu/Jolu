import type { GatewaySessionDto, LiveRequestDto, SessionTokensDto } from "./gatewayApi"
import type { RouteEndEvent, RouteStartEvent } from "./gatewayActivity"

/**
 * 统一网关「模型来源 → 最近会话」的纯逻辑（统一网关开发说明 §3.2–3.4；测试见 `gatewaySessions.test.mjs`）。
 *
 * 纪律：
 * - 会话身份 = 平台 + 原始会话 ID；不用标题、项目或列表序号；
 * - 运行状态只来自在途请求集合（开始 / 结束事件配对，按快照序号去重），不从耗时或文件时间推测；
 * - Token 是会话线程的权威累计值；未知、真实的零、失败、过期分别表达，不把空值当零。
 */

/** 默认窗口展示的会话数（J6-01）；展开后显示全部 */
export const DEFAULT_VISIBLE = 5
/** 会话结束运行后，排序上仍保留在前部的时长：Agent 两次请求之间的工具执行间隙不让卡片来回跳 */
export const PRIORITY_HOLD_MS = 20_000
const TOMBSTONE_LIMIT = 256

export const sessionKey = (platform: string, sessionId: string) => `${platform}\u001f${sessionId}`

/* ───────────────────────── 在途请求 ───────────────────────── */

export type LiveEvent =
  | { kind: "start"; event: RouteStartEvent }
  | { kind: "end"; event: RouteEndEvent }

export interface LiveState {
  /** 快照序号：序号不大于它的事件已经反映在快照里 */
  baseSeq: number
  /** request_id → 在途请求 */
  requests: Record<number, LiveRequestDto>
  /** 已结束的请求 id：结束事件先于开始事件到达时，迟到的开始事件不再登记（避免永久运行） */
  ended: number[]
}

export const emptyLive = (): LiveState => ({ baseSeq: 0, requests: {}, ended: [] })

export function applyLiveEvent(state: LiveState, live: LiveEvent): LiveState {
  const { event } = live
  if (typeof event.seq === "number" && event.seq <= state.baseSeq) return state
  const id = event.request_id
  if (live.kind === "start") {
    if (state.requests[id] || state.ended.includes(id)) return state
    const start = live.event
    return {
      ...state,
      requests: {
        ...state.requests,
        [id]: {
          request_id: id, ts: start.ts, agent: start.agent, requested_model: start.requested_model, source: start.source,
          connection_id: start.connection_id ?? null, session_platform: start.session_platform ?? null, session_id: start.session_id ?? null,
        },
      },
    }
  }
  const requests = { ...state.requests }
  delete requests[id]
  const ended = state.ended.includes(id) ? state.ended : [...state.ended, id].slice(-TOMBSTONE_LIMIT)
  return { ...state, requests, ended }
}

/** 以快照为准重建在途集合，再补上快照之后到达的事件（序号更大才生效）。 */
export function liveFromSnapshot(seq: number, inFlight: LiveRequestDto[], buffered: LiveEvent[]): LiveState {
  let state: LiveState = { baseSeq: seq, requests: Object.fromEntries(inFlight.map((r) => [r.request_id, r])), ended: [] }
  for (const event of buffered) state = applyLiveEvent(state, event)
  return state
}

export interface SessionLive {
  /** 在途请求数（并发请求逐个结束，全部结束才算停止） */
  count: number
  /** 正在使用的连接 id */
  connections: string[]
  /** 最近一个在途请求的开始时间 */
  latestTs: number
}

/** 按会话汇总在途请求；没有已验证会话标识的请求不归到任何会话。 */
export function liveBySession(state: LiveState): Map<string, SessionLive> {
  const out = new Map<string, SessionLive>()
  for (const request of Object.values(state.requests)) {
    if (!request.session_platform || !request.session_id) continue
    const key = sessionKey(request.session_platform, request.session_id)
    const entry = out.get(key) ?? { count: 0, connections: [], latestTs: 0 }
    entry.count += 1
    if (request.connection_id && !entry.connections.includes(request.connection_id)) entry.connections.push(request.connection_id)
    entry.latestTs = Math.max(entry.latestTs, request.ts)
    out.set(key, entry)
  }
  return out
}

export interface SourceLive {
  /** 这个来源上没有会话标识的在途请求数 */
  count: number
  /** 发起这些请求的 Agent（去重，识别不出时不列） */
  agents: string[]
}

/**
 * 没有已验证会话标识的在途请求（如 Gemini CLI 的 API Key 请求）按来源汇总：
 * 只在来源卡上显示「运行中」，不连到任何会话、不按时间猜会话（鼠鼠 2026-11-06 选定方案 A）。
 */
export function liveUnattributed(state: LiveState): Map<string, SourceLive> {
  const out = new Map<string, SourceLive>()
  for (const request of Object.values(state.requests)) {
    if ((request.session_platform && request.session_id) || !request.connection_id) continue
    const entry = out.get(request.connection_id) ?? { count: 0, agents: [] }
    entry.count += 1
    if (request.agent && !entry.agents.includes(request.agent)) entry.agents.push(request.agent)
    out.set(request.connection_id, entry)
  }
  return out
}

/* ───────────────────────── 排序与窗口 ───────────────────────── */

/** 排序优先：since = 进入运行的时刻；until = 运行结束后保留优先到何时（运行中为 null） */
export type PriorityState = Record<string, { since: number; until: number | null }>

export function updatePriority(prev: PriorityState, running: Iterable<string>, now: number): PriorityState {
  const next: PriorityState = {}
  const active = new Set(running)
  for (const key of active) {
    const old = prev[key]
    next[key] = { since: old && (old.until === null || old.until > now) ? old.since : now, until: null }
  }
  for (const [key, value] of Object.entries(prev)) {
    if (active.has(key)) continue
    const until = value.until ?? now + PRIORITY_HOLD_MS
    if (until > now) next[key] = { since: value.since, until }
  }
  return next
}

/** 下一次优先保留到期的时刻；没有则为 null */
export function nextPriorityExpiry(state: PriorityState): number | null {
  const times = Object.values(state).map((v) => v.until).filter((v): v is number => v !== null)
  return times.length ? Math.min(...times) : null
}

export interface SessionView {
  key: string
  session: GatewaySessionDto
  running: boolean
  /** 运行中正在使用的连接 */
  liveConnections: string[]
  /** 最近活动：会话线程、经网关流水与在途请求三者取最新 */
  activity: number
  /** 运行中或刚结束仍在保留期内，排在前部 */
  prioritized: boolean
}

export function buildSessionViews(sessions: GatewaySessionDto[], live: Map<string, SessionLive>, priority: PriorityState): SessionView[] {
  const views = sessions.map((session) => {
    const key = sessionKey(session.platform, session.session_id)
    const state = live.get(key)
    return {
      key,
      session,
      running: !!state && state.count > 0,
      liveConnections: state?.connections ?? [],
      activity: Math.max(session.thread_last_ts ?? 0, session.gateway_last_ts ?? 0, state?.latestTs ?? 0),
      prioritized: !!priority[key],
    }
  })
  return views.sort((a, b) => {
    const pa = priority[a.key]
    const pb = priority[b.key]
    // 运行中（含刚结束的保留期）在前，组内按进入运行的时刻从新到旧，不随每次请求互换位置
    if (pa && pb) return pb.since - pa.since || b.activity - a.activity || a.key.localeCompare(b.key)
    if (pa) return -1
    if (pb) return 1
    return b.activity - a.activity || a.key.localeCompare(b.key)
  })
}

/**
 * 默认窗口条数：至少 5 条；运行中（含保留期）的会话超过 5 条时全部放进默认窗口，
 * 在共用滚动区里滚动查看，不折进「展开更多」（鼠鼠 2026-11-06 确认）。
 */
export function defaultWindowSize(views: Pick<SessionView, "prioritized">[]): number {
  return Math.max(DEFAULT_VISIBLE, views.filter((view) => view.prioritized).length)
}

/** 默认窗口与展开：没有更多内容时 hidden 为 0（不显示无效入口） */
export function sessionWindow<T>(ordered: T[], expanded: boolean, size = DEFAULT_VISIBLE): { shown: T[]; hidden: number } {
  if (expanded || ordered.length <= size) return { shown: ordered, hidden: expanded ? Math.max(0, ordered.length - size) : 0 }
  return { shown: ordered.slice(0, size), hidden: ordered.length - size }
}

/* ───────────────────────── 关联与来源 ───────────────────────── */

export const modelPart = (requested: string, source: string | null) =>
  source && requested.startsWith(`${source}/`) ? requested.slice(source.length + 1) : requested

export interface SessionLine {
  connectionId: string
  /** 正在经这个连接运行 */
  active: boolean
}

/**
 * 一张会话卡画哪些线：正在运行的连接 + 最近一次关联的连接。
 * 同一会话的更早历史关联保留在数据里，展示形式待确认（开发说明 §8），这里不全部画出。
 */
export function sessionLines(view: SessionView): SessionLine[] {
  const lines: SessionLine[] = view.liveConnections.map((connectionId) => ({ connectionId, active: true }))
  const latest = view.session.links.find((link) => link.connection_id)?.connection_id
  if (latest && !view.liveConnections.includes(latest)) lines.push({ connectionId: latest, active: false })
  return lines
}

export interface SourceSummary {
  /** 关联到这个来源的最近会话数（按会话去重） */
  sessions: number
  /** 最近一次经这个来源的模型；还没有经网关的请求时为 null */
  model: string | null
}

export function sourceSummary(views: SessionView[], connectionId: string): SourceSummary {
  let sessions = 0
  let latest: { ts: number; model: string } | null = null
  for (const view of views) {
    const link = view.session.links.find((item) => item.connection_id === connectionId)
    if (link || view.liveConnections.includes(connectionId)) sessions += 1
    if (link && (!latest || link.last_ts > latest.ts)) {
      latest = { ts: link.last_ts, model: link.upstream_model ?? modelPart(link.requested_model, link.source) }
    }
  }
  return { sessions, model: latest?.model ?? null }
}

/** 来源配色：按来源顺序循环（Jolu.pen `jolu-accent` / `jolu-gateway-*`） */
export const SOURCE_TONES = ["green", "blue", "purple", "amber"] as const
export type SourceTone = (typeof SOURCE_TONES)[number]
export const sourceTone = (index: number): SourceTone => SOURCE_TONES[((index % 4) + 4) % 4]

/** 来源卡上的凭据类型 */
export function credentialLabel(kind: string | null | undefined, platformId: string): string {
  if (kind === "auth") return "官方订阅"
  return platformId === "custom" ? "第三方 API Key" : "官方 API Key"
}

/** 路由流水「路由到」后的类型标签 */
export function routeTypeTag(kind: string | null | undefined, platformId: string): string {
  if (kind === "auth") return "订阅"
  return platformId === "custom" ? "第三方" : "官方 Key"
}

/* ───────────────────────── 显示文案 ───────────────────────── */

const TOOL_LABELS: Readonly<Record<string, string>> = {
  claude: "Claude Code", codex: "Codex", gemini: "Gemini CLI", opencode: "OpenCode", zcode: "ZCode",
  qoder: "Qoder", trae: "Trae", workbuddy: "WorkBuddy", grok: "Grok",
}

export const toolLabel = (platform: string) => TOOL_LABELS[platform] ?? platform

/** 项目名取 cwd 最后一段；没有 cwd 时为 null（界面显示「未识别项目」） */
export function projectName(cwd: string | null): string | null {
  if (!cwd) return null
  const parts = cwd.split(/[\\/]+/).filter(Boolean)
  return parts.length ? parts[parts.length - 1] : cwd
}

/** 会话标题：没有标题时用原始会话 ID 前 8 位（去掉 `opencode:` 这类统计前缀），不编造标题 */
export function sessionTitle(session: Pick<GatewaySessionDto, "title" | "session_id">): string {
  const raw = session.session_id.split(":").pop() || session.session_id
  return session.title?.trim() || `会话 ${raw.slice(0, 8)}`
}

export function relativeTime(ts: number, now: number): string {
  if (!ts) return "—"
  const diff = Math.max(0, now - ts)
  if (diff < 60_000) return "刚刚"
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`
  if (diff < 7 * 86_400_000) return `${Math.floor(diff / 86_400_000)} 天前`
  const d = new Date(ts)
  return `${d.getMonth() + 1}月${d.getDate()}日`
}

export type SessionTone = "running" | "done" | "failed" | "recent"

/** 会话状态：只有在途请求算「运行中」；没有阶段数据，不细分「思考 / 生成」；没有完成证据时保持「最近活跃」 */
export function sessionStatus(running: boolean, threadStatus: string | null): { label: string; tone: SessionTone } {
  if (running) return { label: "运行中", tone: "running" }
  if (threadStatus === "done") return { label: "已完成", tone: "done" }
  if (threadStatus === "failed") return { label: "失败", tone: "failed" }
  return { label: "最近活跃", tone: "recent" }
}

/* ───────────────────────── Token ───────────────────────── */

const trimZero = (text: string) => text.replace(/\.0$/, "")

/** 小于 1,000 显示整数；其余 K / M，最多一位小数 */
export function compactTokens(n: number): string {
  const abs = Math.abs(n)
  if (abs < 1000) return String(Math.round(n))
  if (abs < 999_950) return `${trimZero((n / 1000).toFixed(1))}K`
  return `${trimZero((n / 1_000_000).toFixed(1))}M`
}

/** 数据新鲜度：fresh 最新；stale 刷新超时未更新；failed 最近一次刷新失败 */
export type TokenFreshness = "fresh" | "stale" | "failed"

export interface TokenDisplay {
  text: string
  /** 悬停提示：未缩写数值与口径说明 */
  title: string
  known: boolean
  /** 失败 / 过期时的说明；最新数据为 null */
  flag: string | null
  /** 角标：failed「刷新失败」/ stale「已过期」；最新数据为 null */
  badge: { kind: "failed" | "stale"; label: string } | null
}

export function tokenDisplay(tokens: SessionTokensDto | undefined, freshness: TokenFreshness): TokenDisplay {
  const flag = freshness === "failed" ? "刷新失败，显示上次结果" : freshness === "stale" ? "数据可能已过期" : null
  const badge = freshness === "failed" ? { kind: "failed" as const, label: "刷新失败" } : freshness === "stale" ? { kind: "stale" as const, label: "已过期" } : null
  const scope = "会话累计（会话线程统计口径，可能包含经其他入口的消耗）"
  if (!tokens || tokens.state === "missing") {
    return { text: "—", title: `${scope}：会话统计尚未采集到这条会话`, known: false, flag, badge }
  }
  if (tokens.state !== "known" || tokens.total === null) {
    return { text: "—", title: `${scope}：部分 Token 分项缺失，无法给出可信合计`, known: false, flag, badge }
  }
  const full = `${tokens.total.toLocaleString("zh-CN")} Tokens`
  return { text: compactTokens(tokens.total), title: flag ? `${full} · ${flag}\n${scope}` : `${full}\n${scope}`, known: true, flag, badge }
}
