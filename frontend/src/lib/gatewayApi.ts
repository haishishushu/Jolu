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
  ts: number
  agent: string | null
  requested_model: string
  source: string | null
  upstream_model: string | null
  status_code: number | null
  result: "ok" | "rejected" | "failed" | string
  duration_ms: number | null
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

export const gatewayApi = {
  status: () => invoke<GatewayStatusDto>("gateway_status"),
  /** 切换模式；切回 observe 时后端会先还原全部接线 */
  setMode: (mode: GatewayMode, defaultConnection: string | null) =>
    invoke<unknown>("gateway_set_mode", { mode, defaultConnection }),
  /** 连接进出模型池 */
  setServe: (connectionId: string, enabled: boolean) =>
    invoke<void>("gateway_set_serve", { connectionId, enabled }),
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
  /** 最近 20 条路由流水 */
  routeLog: () => invoke<RouteLogDto[]>("gateway_route_log"),
}
