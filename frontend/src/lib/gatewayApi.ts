import { invoke } from "./api"

/** 网关接线状态（与后端 `WiringStatus` 一致）。 */
export interface WiringStatusDto {
  agent: string
  wired: boolean
  config_path: string | null
  notice: string | null
  reason: string | null
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
  /** 逃生门：还原全部接线并停网关 */
  escape: () => invoke<string>("gateway_escape"),
}
