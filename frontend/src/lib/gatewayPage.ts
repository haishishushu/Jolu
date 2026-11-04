import type { GatewayMode, GatewayStatusDto, WiringStatusDto } from "./gatewayApi"

/**
 * 网关页面的纯逻辑（画板 161–166；测试见 `gatewayPage.test.mjs`）。
 *
 * 纪律：未知 / 不支持 / 失败 / 空 分别表达，不合并成一句「无数据」——
 * 与 AGENTS.md「未知值、真实的零、能力不支持、查询失败、数据过期要分别表达」一致。
 */

/** 接入工具的展示名（与 providerCatalog 的平台文案对齐）。 */
export const AGENT_LABELS: Readonly<Record<string, string>> = {
  "claude-code": "Claude Code",
  codex: "Codex",
  zcode: "ZCode",
  gemini: "Gemini CLI",
  opencode: "OpenCode",
}

export function agentLabel(agent: string): string {
  return AGENT_LABELS[agent] ?? agent
}

export type Tone = "on" | "muted" | "danger"

export interface StatusLine {
  text: string
  tone: Tone
}

/** 网关运行状态行：启用 / 关闭 / 模式未知 三种情况各自成句。 */
export function gatewayRunLine(status: GatewayStatusDto | null): StatusLine {
  if (!status) return { text: "读取中…", tone: "muted" }
  if (status.mode === "relay") {
    return status.enabled
      ? { text: "模型转发已开启", tone: "on" }
      : { text: "模型转发未启动，请重新开启", tone: "danger" }
  }
  if (status.mode === "observe") return { text: "未开启模型转发", tone: "muted" }
  return { text: "暂时无法确认运行状态，请刷新", tone: "danger" }
}

/** 某个工具的接线摘要：已接入 / 未接入 / 无法接入（附原因）。 */
export function wiringLine(entry: WiringStatusDto): StatusLine {
  if (entry.wired) {
    return { text: "已接入网关", tone: "on" }
  }
  if (entry.reason) {
    return { text: entry.reason, tone: "muted" }
  }
  return { text: "未接入", tone: "muted" }
}

/** 已接入的工具数（页签上的计数用）。 */
export function wiredCount(status: GatewayStatusDto | null): number {
  if (!status) return 0
  return status.wiring.filter((entry) => entry.wired).length
}

/** 模式切换目标：已启用 relay 时可以切回 observe，其余情况切到 relay。 */
export function nextMode(status: GatewayStatusDto | null): GatewayMode {
  return status?.mode === "relay" ? "observe" : "relay"
}

/** 逃生门是否可用：只有 relay 模式下才有意义。 */
export function canEscape(status: GatewayStatusDto | null): boolean {
  return status?.mode === "relay"
}

/** 熔断说明：关闭自动逃生时必须明确告诉用户「不会自动切回」。 */
export function circuitText(status: GatewayStatusDto | null): string {
  if (!status) return ""
  const base = `连续失败 ${status.circuit_threshold} 次后自动切回直连`
  return status.auto_escape ? base : `${base}（当前已关闭自动逃生，只能手动切回）`
}

/** 来源选择不等于调用就绪；授权与适配状态单独展示，不能把开关永久锁死。 */
export function canToggleSource(_serving: boolean, busy: boolean, _capability?: { can_serve: boolean }): boolean {
  return !busy
}
