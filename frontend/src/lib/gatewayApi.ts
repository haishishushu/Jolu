import { invoke } from "./api"

/** 网关接线状态（与后端 `WiringStatus` 一致）。 */
export interface WiringStatusDto {
  agent: string
  wired: boolean
  config_path: string | null
  notice: string | null
  reason: string | null
  /** 接线记录阶段：idle / pending / wired；旧版后端不返回 */
  state?: "idle" | "pending" | "wired" | null
  /** 漂移：pending / removed / changed / foreign */
  drift?: "pending" | "removed" | "changed" | "foreign" | null
  /** Codex 检测到的登录方式 */
  codex_mode?: "oauth" | "provider" | null
}

export interface GatewayGroupDto {
  id: string
  name: string
  sort: number
  members: string[]
}

export interface GatewayGroupsDto {
  sources: GatewayGroupDto[]
  agents: GatewayGroupDto[]
}

export type GroupKind = "source" | "agent"

export interface CatalogModelDto {
  id: string
  source: string
  slug: string
  model: string
  kind: "auth" | "api" | string
  status: string
}

export interface RouteLogDto {
  /** 自增流水 id：分页的稳定行标识（旧版后端不返回） */
  id?: number
  ts: number
  agent: string | null
  requested_model: string
  source: string | null
  upstream_model: string | null
  status_code: number | null
  result: "ok" | "rejected" | "failed" | string
  duration_ms: number | null
  connection_id?: string | null
  session_platform?: string | null
  session_id?: string | null
}

/** 路由流水的一页（与后端 `RouteLogPage` 一致）。 */
export interface RouteLogPageDto {
  rows: RouteLogDto[]
  page: number
  page_size: number
  /** 查询上界内、当前保留范围内的总行数 */
  total: number
  /** 没有数据时为 0 */
  page_count: number
  /** 本次浏览的查询上界；翻页时带回，刷新时传 null 重新建立 */
  upper_id: number | null
}

/** 在途请求（与后端 `LiveRequest` 一致）。 */
export interface LiveRequestDto {
  request_id: number
  ts: number
  agent: string | null
  requested_model: string
  source: string
  connection_id: string | null
  session_platform: string | null
  session_id: string | null
}

/** 会话经网关到某个连接的历史关联（保留范围内）。 */
export interface SessionLinkDto {
  session_platform: string
  session_id: string
  connection_id: string | null
  source: string | null
  requested_model: string
  upstream_model: string | null
  agent: string | null
  last_ts: number
  requests: number
}

export interface SessionTokensDto {
  fresh_input: number | null
  cache_read: number | null
  cache_write: number | null
  output: number | null
  /** 只有四个分项都可信时才有值 */
  total: number | null
  /** known：分项完整；partial：有分项缺失；missing：会话线程里还没有这条会话 */
  state: "known" | "partial" | "missing"
}

export interface GatewaySessionDto {
  platform: string
  session_id: string
  agent: string | null
  title: string | null
  cwd: string | null
  thread_last_ts: number | null
  gateway_last_ts: number | null
  /** 会话线程状态；还没采集到时为 null */
  thread_status: "running" | "done" | "failed" | "recent" | "usage_only" | string | null
  tokens: SessionTokensDto
  links: SessionLinkDto[]
}

export interface SessionsOverviewDto {
  /** 在途快照对应的事件序号：只应用序号更大的实时事件 */
  seq: number
  at: number
  in_flight: LiveRequestDto[]
  sessions: GatewaySessionDto[]
}

/** 网关总览（与后端 `GatewayStatus` 一致；不含任何凭证）。 */
export interface GatewayStatusDto {
  mode: "observe" | "relay" | string
  enabled: boolean
  port: number
  circuit_threshold: number
  auto_escape: boolean
  default_connection: string | null
  wiring: WiringStatusDto[]
}

export type GatewayMode = "observe" | "relay"

export interface SourceCapabilityDto {
  connection_id: string
  can_serve: boolean
  state: "ready" | "needs_auth" | "unsupported"
  reason: string
}

/** 批量加入 / 移出模型池的单项结果 */
export interface ServeBatchItemDto {
  connection_id: string
  ok: boolean
  error: string | null
}

/** 来源经网关的使用情况（路由流水保留范围内） */
export interface SourceUsageDto {
  connection_id: string
  last_ts: number
  requests: number
  failed: number
}

export const gatewayApi = {
  sourceCapabilities: () => invoke<SourceCapabilityDto[]>("gateway_source_capabilities"),
  accessKey: (refresh: boolean) => invoke<string>("gateway_access_key", { refresh }),
  copyKey: () => invoke<void>("gateway_copy_key"),
  status: () => invoke<GatewayStatusDto>("gateway_status"),
  /** 切换模式；切回 observe 时后端会先还原全部接线 */
  setMode: (mode: GatewayMode, defaultConnection: string | null) =>
    invoke<unknown>("gateway_set_mode", { mode, defaultConnection }),
  /** 连接进出模型池 */
  setServe: (connectionId: string, enabled: boolean) =>
    invoke<void>("gateway_set_serve", { connectionId, enabled }),
  /** 批量进出模型池：逐项返回结果，单个失败不影响其他来源 */
  setServeMany: (connectionIds: string[], enabled: boolean) =>
    invoke<ServeBatchItemDto[]>("gateway_set_serve_many", { connectionIds, enabled }),
  /** 各来源最近使用时间与请求数 */
  sourceUsage: () => invoke<SourceUsageDto[]>("gateway_source_usage"),
  /** 接入 / 断开单个工具 */
  setWiring: (agent: string, wired: boolean) => invoke<string>("gateway_wiring_set", { agent, wired }),
  /** 接受漂移：只清接线记录，不改用户的配置文件 */
  acceptWiring: (agent: string) => invoke<void>("gateway_wiring_accept", { agent }),
  /** 逃生门：还原全部接线并停网关 */
  escape: () => invoke<string>("gateway_escape"),
  /** 来源分组 / Agent 分组（只影响排序） */
  groups: () => invoke<GatewayGroupsDto>("gateway_groups"),
  createGroup: (kind: GroupKind, name: string) => invoke<GatewayGroupDto>("gateway_group_create", { kind, name }),
  renameGroup: (kind: GroupKind, id: string, name: string) => invoke<void>("gateway_group_rename", { kind, id, name }),
  deleteGroup: (kind: GroupKind, id: string) => invoke<void>("gateway_group_delete", { kind, id }),
  moveToGroup: (kind: GroupKind, member: string, groupId: string | null) =>
    invoke<void>("gateway_group_move", { kind, member, groupId }),
  reorderGroups: (kind: GroupKind, ids: string[]) => invoke<void>("gateway_group_reorder", { kind, ids }),
  /** 改菜单 id；旧 id 之后请求会得到 404 说明 */
  setSlug: (connectionId: string, slug: string) => invoke<string>("gateway_set_slug", { connectionId, slug }),
  /** 模型清单（只读缓存，不打上游） */
  catalog: () => invoke<CatalogModelDto[]>("gateway_catalog"),
  /** 最近 20 条路由流水（灵动岛用） */
  routeLog: () => invoke<RouteLogDto[]>("gateway_route_log"),
  /** 路由流水分页；upperId 为 null 时以最新一条建立查询范围 */
  routeLogPage: (page: number, pageSize: number, upperId: number | null) =>
    invoke<RouteLogPageDto>("gateway_route_log_page", { page, pageSize, upperId }),
  /** 模型来源 → 最近会话：在途快照 + 经网关会话 + 权威 Token */
  sessionsOverview: () => invoke<SessionsOverviewDto>("gateway_sessions_overview"),
  /** 今日（本地零点起）经网关的请求数 */
  todayCounts: () => invoke<{ since: number; at: number; ok: number; rejected: number; failed: number }>("gateway_today_counts"),
}
