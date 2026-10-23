/** 平台、连接、来源、模型的区分见需求文档 §7.2 —— 四者是不同层级，不可混用 */
export type PlatformId = "claude" | "codex" | "gemini" | "grok" | "opencode" | "zcode" | "trae" | "qoder" | "workbuddy" | "custom"

/** 已实现并可连接 / 已实现但尚未连接 / 尚未接入（禁用） */
export type PlatformAvailability = "available" | "not-connected" | "not-integrated"

export type PlatformCapability =
  | "auth_connection"
  | "api_connection"
  | "local_sessions"
  | "subscription_quota"
  | "organization_usage"
  | "gateway_balance"
  | "realtime_delta"
  | "cost_estimate"
  | "credits"

export interface Platform {
  id: PlatformId
  name: string
  availability: PlatformAvailability
  limitation?: string | null
  capabilities: readonly PlatformCapability[]
}

/** 连接只有两种类型（§6.3）；本地会话记录是「统计来源」而非连接 */
export type ConnectionKind = "auth" | "api"

export type ConnectionStatus = "connected" | "expired" | "invalid" | "offline" | "paused"

export interface Connection {
  id: string
  platformId: PlatformId
  kind: ConnectionKind
  /** 用户填写的连接名称 */
  name: string
  /** 后端存储的原始名称；API 连接的 name 会并上脱敏标识用于展示，编辑名称时用这个 */
  baseName: string
  /** Auth：套餐名称原样透传；API：脱敏 Key 标识。不得由额度百分比推断 */
  label: string
  /** API 连接的脱敏标识，随 Key 自动生成；Auth 为空。仅供界面展示，不可编辑 */
  masked: string
  status: ConnectionStatus
  lastSyncText: string
  /** 自建网关地址；官方直连为 null */
  baseUrl: string | null
  /** 手动添加时保存的默认参数（画布 18）；本机读取与历史连接为 null */
  model: string | null
  effort: string | null
  context1m: boolean | null
  /** 自定义供应商的接口协议；其他平台为 null */
  protocol: "openai" | "anthropic" | null
}

/** 额度窗口是一组可变窗口，不是固定的两个字段（§7.4） */
export interface QuotaWindow {
  /** 窗口标识，如 5h / 7d / 1d */
  key: string
  /** 窗口说明，如「5 小时窗口」 */
  windowName: string
  /** 已用百分比；总量分母缺失时为 null，此时不生成百分比进度条 */
  usedPercent: number | null
  /** 已用 / 总量的文字描述，缺失时为 null */
  usedText: string | null
  /** 重置倒计时，未提供时为 null（显示「—」，不推测） */
  resetCountdown: string | null
  /** 标签配色：5h 淡紫、7d 淡绿，表示窗口身份，不随水位变化 */
  badgeTone: "purple" | "green" | "blue" | "neutral"
}

export type StatSourceId = string

export interface StatSource {
  id: StatSourceId
  name: string
  /** 采集范围说明 */
  scope: string
  /** 能否按当前连接区分 */
  perConnection: boolean
  /** 是否支持实时增量（不支持时不播放增量动效） */
  realtimeDelta: boolean
  lastUpdatedText: string
}

export interface SessionActivity {
  id: string
  title: string
  /** 当前会话的本轮累计或实时采集增量文本，如「+8.0K Token」 */
  deltaText: string
  /** 实时追数键：有值时逐帧数值从 LiveCountsContext 订阅，优先于 deltaText */
  countKey?: string
  state: "running" | "done" | "failed" | "unknown"
  updatedAtMs?: number
  startedAtMs?: number
  highlighted?: boolean
}

/** 灵动岛形态：自由态（收缩 / 展开）与停靠态是两个维度（§2.1.3） */
export type IslandMode = "collapsed" | "expanded" | "docked"
export type DockEdge = "top" | "bottom" | "left" | "right"
