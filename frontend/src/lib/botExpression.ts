/**
 * 胶囊灵动岛的 bot 表情（Jolu.pen 模块 A3-5 / A3-6，设计说明 §6.1）。
 * 表情只由真实状态推出，不随机出现；同一时刻只显示优先级最高的一个。
 */
export type BotExpr =
  | "idle" | "blinkShot" | "look" | "work" | "refresh" | "happy" | "surprise" | "greet" | "drag"
  | "peek" | "worry" | "sad" | "error" | "sleep" | "reset" | "wait" | "relay"

/** 一次性表情：触发后短暂停留，不回放已经错过的 */
export type BotShot = "happy" | "surprise" | "greet" | "reset"

/** 无运行状态、无告警时 30 分钟没有写入就睡着 */
export const SLEEP_AFTER_MS = 30 * 60_000
/** 单段新增达到这个数就惊讶 */
export const SURPRISE_TOKENS = 50_000

export const BOT_NAMES: Record<BotExpr, string> = {
  idle: "空闲", blinkShot: "眨眼", look: "看向指针", work: "工作中", refresh: "刷新中", happy: "完成",
  surprise: "惊讶", greet: "打招呼", drag: "被拖动", peek: "贴边偷看", worry: "担心", sad: "沮丧",
  error: "出错", sleep: "睡着", reset: "额度重置", wait: "等待额度", relay: "转发中",
}

export interface BotInput {
  /** 有可用连接；false 表示未连接，bot 睡着 */
  connected: boolean
  /** 额度查询失败或连接过期 */
  queryFailed: boolean
  /** 各额度窗口的已用百分比；未知为 null，不当作 0 */
  quotaUsed: (number | null)[]
  dragging: boolean
  /** 自动化任务在等额度恢复 */
  waiting?: boolean
  refreshing: boolean
  /** 来源有可信运行状态，且有会话正在写入 Token */
  working: boolean
  /** 正在显示的一次性表情 */
  shot?: BotShot | null
  /** 距离最近一次会话写入或网关请求的毫秒数；未知按刚活跃处理 */
  inactiveMs?: number
  /** 停靠在屏幕边缘 */
  docked?: boolean
  hover?: boolean
  /** 模型入口：请求转发中 / 刚转发成功 / 被网关拒绝 */
  relay?: "in" | "ok" | "rejected" | null
}

export interface BotResult {
  expr: BotExpr
  reason: string
}

export function botExpression(input: BotInput): BotResult {
  const known = input.quotaUsed.filter((value): value is number => value !== null)
  if (input.connected && input.queryFailed) return { expr: "error", reason: "额度查询失败" }
  if (input.relay === "rejected") return { expr: "worry", reason: "网关拒绝了请求" }
  if (known.some((value) => value >= 100)) return { expr: "sad", reason: "额度用尽（100%）" }
  if (input.dragging) return { expr: "drag", reason: "拖动窗口中" }
  if (known.some((value) => value >= 80)) return { expr: "worry", reason: "任一额度窗口 ≥ 80%" }
  if (input.waiting) return { expr: "wait", reason: "自动化任务在等额度恢复" }
  if (input.refreshing) return { expr: "refresh", reason: "正在刷新额度" }
  if (input.relay === "in") return { expr: "relay", reason: "请求正在经模型入口转发" }
  if (input.relay === "ok") return { expr: "happy", reason: "转发成功" }
  if (input.working) return { expr: "work", reason: "有会话正在写入 Token" }
  const shot = input.shot ?? null
  if (shot && shot !== "greet") return { expr: shot, reason: SHOT_REASON[shot] }
  if (!input.connected) return { expr: "sleep", reason: "未连接" }
  if ((input.inactiveMs ?? 0) >= SLEEP_AFTER_MS) return { expr: "sleep", reason: "30 分钟内没有会话写入与网关请求" }
  if (shot) return { expr: shot, reason: SHOT_REASON[shot] }
  if (input.docked) return { expr: "peek", reason: "贴边停靠" }
  if (input.hover) return { expr: "look", reason: "指针悬停在胶囊上" }
  return { expr: "idle", reason: "没有会话写入 Token" }
}

const SHOT_REASON: Record<BotShot, string> = {
  happy: "本段输出结束",
  surprise: "单段新增 ≥ 50k Token",
  greet: "双击展开",
  reset: "额度窗口刚重置",
}

/** 一次性表情的停留时长 */
export const SHOT_MS: Record<BotShot, number> = { happy: 1200, surprise: 900, greet: 900, reset: 1400 }

/**
 * 额度窗口刚重置：已用比例从高位回到低位。
 * 两次读数都要是已知值，且按窗口标识对齐，新增或缺失的窗口不判定。
 */
export function quotaJustReset(prev: Record<string, number | null>, next: Record<string, number | null>): boolean {
  return Object.keys(next).some((key) => {
    const before = prev[key], after = next[key]
    return before != null && after != null && before - after >= 40 && after <= 20
  })
}
