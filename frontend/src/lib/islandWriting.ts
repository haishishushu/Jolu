/**
 * 胶囊「正在写入」的判定（2026-11-05 追加，见
 * docs/plans/2026-11-04-capsule-island-implementation.md §5.8）。
 *
 * §5.7 把增量段改成 A + B 之后，`working` 只剩一个信号源：`live.sessions` 里
 * 有 `state === "running"` 的会话。真机回归证明这条对 ZCode 永远不成立：
 *
 * - ZCode 的来源库 `model_usage` 只在请求**结束时**落行（实测 7904 completed /
 *   110 error / 29 cancelled，没有任何 running / pending 行），`zcode_session_state`
 *   因此永远算不出 running；`sessions.activity_state` 只会是 done / failed。
 * - 同时它也不是生命周期来源（`db::live_usage` 里 `lifecycle` 只含 codex / claude），
 *   所以 `running_tokens` 恒为 null，`active_sessions` 只是「最近活跃窗口」里的会话。
 *
 * 于是这里给 `working` 三个并列信号，任一为真即算在写：
 *   1. 有 `state === "running"` 的会话（codex / claude 的生命周期来源）；
 *   2. 网关转发有请求在途（`useGatewayActivity` 的 `inFlight`；只有 relay 模式才有事件，
 *      直连连接恒为 0，不会误报）；
 *   3. 来源只给「最近活跃」窗口时（zcode 等原生来源），窗口内还有写入就是运行证据——
 *      没有生命周期可读时，「刚刚写进去了 Token」是唯一可信的运行痕迹。
 *
 * 采集源出错（`sourceFailed`）不再参与这里的判定：它说明状态不可判定，
 * 但不能把「明明有请求在途 / 刚刚还在写」压成 false。
 */

/** 与后端 `db::ACTIVE_WINDOW_SECONDS` 同值；推送里 `active_window_seconds` 为 0 时兜底 */
export const DEFAULT_ACTIVE_WINDOW_SECONDS = 90
/** 未来时间戳的容差：超过这个量的时间戳不可信，不当成「刚刚写过」 */
export const FUTURE_TOLERANCE_MS = 60_000

export interface WritingSession {
  /** running / done / failed / unknown */
  state: string
  /** 最近一次写入时刻（后端 `last_seen_ms`） */
  updatedAtMs?: number | null
}

export interface IslandWritingInput {
  /** 本平台的会话列表（`live.sessions`） */
  sessions: ReadonlyArray<WritingSession>
  /** 来源只给「最近活跃」窗口（没有 running / done 生命周期） */
  recentWindow: boolean
  /** 最近活跃窗口长度（秒） */
  activeWindowSeconds: number
  /** 网关请求在途数量 */
  inFlight: number
  /** 当前时刻 */
  now: number
}

export interface IslandWriting {
  /** 三个信号任一为真 */
  working: boolean
  /** 运行中的会话数（只有生命周期来源会非 0） */
  running: number
  /** 最近活跃窗口内仍有写入 */
  recentWrite: boolean
  /** 网关有请求在途 */
  gatewayInFlight: boolean
  /** 最近一次写入时刻；没有可读的会话时刻时为 null */
  lastWriteMs: number | null
}

/** 会话列表里最新一次写入的时刻（`updatedAtMs` 未知的会话不参与） */
export function lastWriteAt(sessions: ReadonlyArray<WritingSession>): number | null {
  let latest: number | null = null
  for (const session of sessions) {
    const at = session.updatedAtMs
    if (at == null || !Number.isFinite(at)) continue
    if (latest == null || at > latest) latest = at
  }
  return latest
}

export function islandWriting(input: IslandWritingInput): IslandWriting {
  const running = input.sessions.filter((session) => session.state === "running").length
  const lastWriteMs = lastWriteAt(input.sessions)
  const windowSeconds = input.activeWindowSeconds > 0 ? input.activeWindowSeconds : DEFAULT_ACTIVE_WINDOW_SECONDS
  const age = lastWriteMs == null ? null : input.now - lastWriteMs
  // 未来时间戳只在容差内算数：来源时钟跑飞时不能一直显示「正在写入」
  const recentWrite = input.recentWindow && age != null && age <= windowSeconds * 1_000 && age >= -FUTURE_TOLERANCE_MS
  const gatewayInFlight = input.inFlight > 0
  return {
    working: running > 0 || gatewayInFlight || recentWrite,
    running,
    recentWrite,
    gatewayInFlight,
    lastWriteMs,
  }
}
