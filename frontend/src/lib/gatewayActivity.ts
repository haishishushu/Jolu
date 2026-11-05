/**
 * 胶囊灵动岛的模型入口适配（设计说明 §8 / §9.1）：纯逻辑，测试见 gatewayActivity.test.mjs。
 *
 * 事件：
 *   gateway-route-start  选路成功、交给上游之前（被拒不发）
 *   gateway-route        每个请求结束（含被拒与失败），与开始事件带同一个 request_id
 */
import type { RouteLogDto } from "./gatewayApi"

export interface RouteStartEvent {
  request_id: number
  ts: number
  agent: string | null
  requested_model: string
  source: string
}

export interface RouteEndEvent {
  request_id: number
  ts: number
  agent: string | null
  requested_model: string
  source: string | null
  upstream_model: string | null
  status_code: number | null
  result: "ok" | "rejected" | "failed" | string
  duration_ms: number
}

/** 胶囊上正在显示的一次转发 */
export interface ActiveRelay {
  requestId: number
  agent: string | null
  /** 来源 slug；被拒时为 null */
  source: string | null
  /** 请求模型去掉 slug 前缀后的部分 */
  model: string
  phase: "in" | "ok" | "rejected" | "failed"
  startedAt: number
  /** 结果出来的时刻；转发中为 null */
  endedAt: number | null
  durationMs: number | null
  statusCode: number | null
}

export interface ActivityState {
  /** 进行中的请求（按开始时间排序），用于展开态每个 Agent 一行 */
  inFlight: ActiveRelay[]
  /** 胶囊上显示的那一条：最新开始的进行中请求，或刚结束停留中的结果 */
  shown: ActiveRelay | null
  /** 每个 Agent 最近一次去向 */
  lastByAgent: Record<string, { source: string | null; model: string; ts: number; ok: boolean; durationMs: number | null; statusCode: number | null }>
  /** 今日统计（本地时区零点起） */
  today: { day: string; ok: number; rejected: number; failed: number }
}

/** 成功停留 2.4s，被拒 / 失败停留 4.8s；只有开始没有结束的请求 120s 后收回 */
export const OK_HOLD_MS = 2400
export const REJECT_HOLD_MS = 4800
export const STALE_IN_FLIGHT_MS = 120_000

export const dayKey = (ts: number) => {
  const d = new Date(ts)
  return `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`
}

export const modelPart = (requested: string, source: string | null) =>
  source && requested.startsWith(`${source}/`) ? requested.slice(source.length + 1) : requested

export function emptyActivity(now: number): ActivityState {
  return { inFlight: [], shown: null, lastByAgent: {}, today: { day: dayKey(now), ok: 0, rejected: 0, failed: 0 } }
}

/** 从路由流水恢复「最近去向」与今日统计；流水是最新在前，最多 20 条，今日统计只是近似 */
export function activityFromLog(rows: RouteLogDto[], now: number): ActivityState {
  const state = emptyActivity(now)
  for (const row of [...rows].reverse()) {
    if (row.agent) {
      state.lastByAgent[row.agent] = {
        source: row.source, model: modelPart(row.requested_model, row.source), ts: row.ts,
        ok: row.result === "ok", durationMs: row.duration_ms, statusCode: row.status_code,
      }
    }
    if (dayKey(row.ts) === state.today.day) countResult(state.today, row.result)
  }
  return state
}

function countResult(today: ActivityState["today"], result: string) {
  if (result === "ok") today.ok += 1
  else if (result === "rejected") today.rejected += 1
  else today.failed += 1
}

const latest = (list: ActiveRelay[]) => list.reduce<ActiveRelay | null>((a, b) => (!a || b.startedAt >= a.startedAt ? b : a), null)

export function onStart(state: ActivityState, event: RouteStartEvent, now: number): ActivityState {
  if (state.inFlight.some((r) => r.requestId === event.request_id)) return state
  const relay: ActiveRelay = {
    requestId: event.request_id, agent: event.agent, source: event.source,
    model: modelPart(event.requested_model, event.source), phase: "in",
    startedAt: now, endedAt: null, durationMs: null, statusCode: null,
  }
  const inFlight = [...state.inFlight, relay]
  // 同时有多个请求时，胶囊只显示最新开始的一条
  return { ...state, inFlight, shown: relay }
}

export function onEnd(state: ActivityState, event: RouteEndEvent, now: number): ActivityState {
  const started = state.inFlight.find((r) => r.requestId === event.request_id)
  const inFlight = state.inFlight.filter((r) => r.requestId !== event.request_id)
  const phase: ActiveRelay["phase"] = event.result === "ok" ? "ok" : event.result === "rejected" ? "rejected" : "failed"
  // 沿用开始事件的显示；只收到结束事件时直接从结果开始
  const relay: ActiveRelay = {
    requestId: event.request_id, agent: event.agent, source: event.source ?? started?.source ?? null,
    model: modelPart(event.requested_model, event.source ?? started?.source ?? null), phase,
    startedAt: started?.startedAt ?? now, endedAt: now, durationMs: event.duration_ms, statusCode: event.status_code,
  }
  const today = dayKey(now) === state.today.day ? { ...state.today } : { day: dayKey(now), ok: 0, rejected: 0, failed: 0 }
  countResult(today, event.result)
  const lastByAgent = event.agent
    ? { ...state.lastByAgent, [event.agent]: { source: relay.source, model: relay.model, ts: now, ok: phase === "ok", durationMs: event.duration_ms, statusCode: event.status_code } }
    : state.lastByAgent
  // 结果停留期间若还有别的请求在转发，先显示这次结果；停留结束后回到进行中的那条
  return { inFlight, shown: relay, lastByAgent, today }
}

/**
 * 定时收尾：结果停留到期后切回仍在进行的最新一条或收起；
 * 只有开始没有结束的请求超过 120s 自动移除，不一直停在「转发中」。
 */
export function onTick(state: ActivityState, now: number): ActivityState {
  const inFlight = state.inFlight.filter((r) => now - r.startedAt < STALE_IN_FLIGHT_MS)
  let shown = state.shown
  if (shown && shown.endedAt !== null) {
    const hold = shown.phase === "ok" ? OK_HOLD_MS : REJECT_HOLD_MS
    if (now - shown.endedAt >= hold) shown = latest(inFlight)
  } else if (shown && !inFlight.some((r) => r.requestId === shown!.requestId)) {
    shown = latest(inFlight)
  }
  const today = dayKey(now) === state.today.day ? state.today : { day: dayKey(now), ok: 0, rejected: 0, failed: 0 }
  if (inFlight.length === state.inFlight.length && shown === state.shown && today === state.today) return state
  return { ...state, inFlight, shown, today }
}

/** 下一次需要 onTick 的时刻；没有待收尾的内容时为 null */
export function nextTickAt(state: ActivityState): number | null {
  const times: number[] = state.inFlight.map((r) => r.startedAt + STALE_IN_FLIGHT_MS)
  if (state.shown?.endedAt != null) times.push(state.shown.endedAt + (state.shown.phase === "ok" ? OK_HOLD_MS : REJECT_HOLD_MS))
  return times.length ? Math.min(...times) : null
}

/**
 * 合并启动快照与实时事件。快照（流水 + 今日计数）读取期间到达的实时事件不能丢：
 * 今日统计 = 快照计数 + 快照时刻之后才结束的实时请求；去向以较新的一方为准。
 */
export function mergeSnapshot(
  live: ActivityState,
  snapshot: { lastByAgent: ActivityState["lastByAgent"]; today: { day: string; ok: number; rejected: number; failed: number } },
  liveSince: { ok: number; rejected: number; failed: number },
): ActivityState {
  const lastByAgent = { ...snapshot.lastByAgent }
  for (const [agent, entry] of Object.entries(live.lastByAgent)) {
    if (!lastByAgent[agent] || lastByAgent[agent].ts <= entry.ts) lastByAgent[agent] = entry
  }
  const today = snapshot.today.day === live.today.day
    ? { day: live.today.day, ok: snapshot.today.ok + liveSince.ok, rejected: snapshot.today.rejected + liveSince.rejected, failed: snapshot.today.failed + liveSince.failed }
    : live.today
  return { ...live, lastByAgent, today }
}

/** 被拒 / 失败原因：以状态码为准，与 gateway/route.rs 的拒绝分支一一对应 */
export function rejectReason(status: number | null, phase: ActiveRelay["phase"]): string {
  if (phase === "failed") return status ? `上游返回 ${status}` : "上游不可达"
  switch (status) {
    case 429: return "冷却中"
    case 401: return "凭据已失效"
    case 402: return "额度已用尽"
    case 503: return "已暂停"
    case 404: return "来源不存在"
    case 400: return "请求无效"
    default: return status ? `被拒 ${status}` : "被拒"
  }
}

/** 耗时文案：到响应头的时间，不是首字时间 */
export const formatDuration = (ms: number | null) => (ms == null ? "—" : ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`)
