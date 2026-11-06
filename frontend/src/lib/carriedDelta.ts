/**
 * 胶囊收缩态增量段（尾部绿色 `+xx.xK`）的 A + B 组合，2026-11-05 追加
 * （见 docs/plans/2026-11-04-capsule-island-implementation.md §5.7）。
 *
 * A · 本轮终值锁存：会话进行中（`working`）时记下 `running_tokens`（后端只把 running 会话求和）
 *     的最新非零读数；会话结束后不再变化，停在最后一个锁存值上，不跳变、不重新起跳。
 *     新一轮开始（`working` 重新为真、数值从 0 / 较小值往上走）时被新值覆盖，等于从新的一轮重新计。
 * B · 今日累计兜底：内存态锁存丢失（应用重启）或当天第一笔请求还没落库时，
 *     显示 `today_tokens`（今天已落库请求的新鲜 Token 合计）。
 *
 * 显示优先级：**锁存值 ?? 今日累计**。两者都不可显示（未知 / 0）时：
 * - 仍在写入（`writing`）→ 段照常渲染，数值位置显示 `+—`（未知，§5.8）；
 * - 否则整段不渲染。
 * 清空只由「今日累计归 0 / 未知」（跨天重置、切换平台、重启后的初始快照）与手动刷新触发，
 * 前端不自己判日期，也不在前端累加 Token。
 */

/** 增量段的数值来源：`session` = 本轮终值锁存，`today` = 今日累计兜底 */
export type CarriedDeltaSource = "session" | "today"

export interface CarriedDeltaView {
  value: number | null
  source: CarriedDeltaSource | null
}

/** 今日累计已归 0 / 未知：后端跨天重置、切换平台、重启后的初始快照都走这一条 */
export function todayReset(todayTokens: number | null): boolean {
  return todayTokens == null || todayTokens <= 0
}

/**
 * 本轮终值锁存的一步判定（纯函数，输入上一次锁存值，输出下一次）。
 *
 * - 正在写且拿得到本轮读数：记最新值。数值变小（新的一轮）也覆盖，不做「只涨不跌」的取最大。
 * - 本轮刚起跑还没有 Token（0 / 未知）：保持上一次读数，避免被 0 抹掉后闪一下。
 * - 没有可记的本轮值且今日累计已归 0 / 未知：清空——昨天的终值不许一直挂在胶囊上。
 * - 其余情况（会话已结束、今日累计还有值）：停在最后一个锁存值上。
 */
export function latchSessionDelta(args: {
  /** 上一次的锁存值；没有则是 null */
  previous: number | null
  /** 来源提供可信运行状态且确有 running 会话 */
  working: boolean
  /** 后端 `running_tokens`：只含 running 会话的本轮累计，未知为 null */
  runningDelta: number | null
  /** 后端 `today_tokens`：今天已落库的新鲜 Token 合计，未知为 null */
  todayTokens: number | null
}): number | null {
  const running = args.runningDelta
  if (args.working && running != null && running > 0) return running
  if (todayReset(args.todayTokens)) return null
  return args.previous
}

/**
 * 增量段的显示值与来源。
 * 采集源出错（`sourceFailed`）时状态不可判定，整段不渲染，也不拿锁存值冒充读数。
 */
export function carriedDeltaView(args: {
  latched: number | null
  todayTokens: number | null
  sourceFailed: boolean
}): CarriedDeltaView {
  if (args.sourceFailed) return { value: null, source: null }
  if (args.latched != null && args.latched > 0) return { value: args.latched, source: "session" }
  if (args.todayTokens != null && args.todayTokens > 0) return { value: args.todayTokens, source: "today" }
  return { value: null, source: null }
}

/** 增量段一次渲染的判定结果 */
export interface CarriedDeltaSegment {
  /** 是否渲染这一段（含分格线 + 绿色读数） */
  visible: boolean
  value: number | null
  source: CarriedDeltaSource | null
  /** 未知但仍在写入：显示 `+—`，不写假的 0、也不整段吞掉 */
  unknown: boolean
}

/**
 * 收缩胶囊尾部增量段的显示判定（§5.8 定的优先级，纯函数）：
 *
 *   1. 有锁存 / 今日累计（> 0）→ 显示这个数；
 *   2. 有预览文案（`deltaText`，只有浏览器预览的示例数据有）→ 显示文案；
 *   3. 否则仍在写入 → **照常渲染这一段**，数值未知显示 `+—`；
 *   4. 三者都没有 → 不渲染。
 *
 * 1 与 2 的先后是 §5.6 的既有行为（`carried != null ? carried !== 0 : Boolean(deltaText)`）：
 * 示例数据里 `carried` 恒为 null，文案分支永远走 2。
 */
export function carriedDeltaSegment(args: {
  /** `carriedDeltaView` 的结果值（锁存 ?? 今日累计） */
  value: number | null
  source: CarriedDeltaSource | null
  /** 浏览器预览的示例文案 */
  text: string | null
  /** 会话 / 请求正在写入（`islandWriting` 的 `working`） */
  writing: boolean
  /** 连接不可用 */
  unavailable: boolean
  /** 被网关转发路线占位 */
  relay: boolean
}): CarriedDeltaSegment {
  const hidden: CarriedDeltaSegment = { visible: false, value: null, source: null, unknown: false }
  if (args.unavailable || args.relay) return hidden
  if (args.value != null && args.value > 0) {
    return { visible: true, value: args.value, source: args.source, unknown: false }
  }
  if (args.text) return { visible: true, value: null, source: null, unknown: false }
  if (args.writing) return { visible: true, value: null, source: null, unknown: true }
  return hidden
}

/**
 * 增量段的追数键：只有进行中才给——结束后是静态读数，不订阅追数，不跑逐帧动画。
 * 锁存追本轮合计 `total`（`running_tokens` 的补间），兜底追今日累计 `today`。
 */
export function carriedDeltaCountKey(source: CarriedDeltaSource | null, working: boolean): string | undefined {
  if (!working) return undefined
  if (source === "session") return "total"
  if (source === "today") return "today"
  return undefined
}
