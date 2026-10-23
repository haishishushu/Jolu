/**
 * 设置 · 网络与代理（画板 90 / 102，D-24 / D-25）：应用联网方式与连接诊断的纯逻辑。
 *
 * 联网方式只作用于 Jolu 自己的额度查询、模型获取与检查更新；CLI 本地代理的上游不跟随。
 * 地址校验以后端 `net::validate_proxy_url` 为准，这里只做同口径的即时提示。
 */

export type NetworkMode = "system" | "direct" | "manual"

export const NETWORK_MODES: { value: NetworkMode; label: string }[] = [
  { value: "system", label: "跟随系统" },
  { value: "direct", label: "直连" },
  { value: "manual", label: "手动代理" },
]

/** 与后端 `net::PROXY_URL_HINT` 一致 */
export const PROXY_URL_HINT = "请输入 http:// 或 https:// 开头、带端口的代理地址"

export interface ProbeDto {
  ok: boolean
  latency_ms: number | null
  error: string | null
  /** 未实际检测（没有可检测的地址，或代理不可达而跳过），原因在 error */
  skipped: boolean
}

export interface EndpointProbeDto {
  connection_name: string
  platform: string
  probe: ProbeDto
}

export interface NetworkDiagnosisDto {
  /** 只有手动代理才检测代理本身 */
  proxy: ProbeDto | null
  endpoints: EndpointProbeDto[]
}

export type Tone = "ok" | "danger" | "muted"
export interface ToneText { tone: Tone; text: string }

type SavedNetwork = { network_mode: string; network_proxy_url: string | null }

export function normalizeNetworkMode(value: string): NetworkMode {
  return value === "direct" || value === "manual" ? value : "system"
}

/** 代理地址即时校验：合法返回 null，否则返回 D-24 的提示 */
export function proxyUrlError(draft: string): string | null {
  const match = /^(https?):\/\/([^/?#@]+?)\/?$/i.exec(draft.trim())
  if (!match) return PROXY_URL_HINT
  const authority = match[2]
  const split = authority.lastIndexOf(":")
  if (split <= 0) return PROXY_URL_HINT
  const host = authority.slice(0, split)
  const port = authority.slice(split + 1)
  if (host.endsWith(":") || !/^\d+$/.test(port)) return PROXY_URL_HINT
  // IPv6 必须写在方括号里，`[::1]` 这种没有端口的写法会落在这里
  if (host.includes(":") && !/^\[[0-9a-f:.]+\]$/i.test(host)) return PROXY_URL_HINT
  const value = Number(port)
  return value >= 1 && value <= 65535 ? null : PROXY_URL_HINT
}

/** 手动代理的地址草稿与已保存的不同（或尚未保存为手动代理）时，需要点「保存配置」 */
export function isNetworkDraft(mode: NetworkMode, draftUrl: string, saved: SavedNetwork): boolean {
  if (mode !== "manual") return false
  return saved.network_mode !== "manual" || draftUrl.trim() !== (saved.network_proxy_url ?? "")
}

/** 连接方式下方的状态行 */
export function networkStatusLine(mode: NetworkMode, saved: SavedNetwork, draftUrl: string): string {
  if (mode === "system") return "使用系统当前网络设置"
  if (mode === "direct") return "不使用代理，直接连接"
  if (isNetworkDraft(mode, draftUrl, saved)) return "代理地址草稿 · 点击保存后生效"
  // 待确认：画板 102 只画了草稿态，已保存的手动代理沿用同一位置显示当前地址
  return `已通过 ${saved.network_proxy_url} 连接`
}

/** 缺少延迟（失败、未检测）显示「—」 */
export function latencyText(probe: ProbeDto): string {
  return probe.latency_ms == null ? "—" : `${probe.latency_ms} ms`
}

export function probeStatus(probe: ProbeDto): ToneText {
  if (probe.ok) return { tone: "ok", text: "可达" }
  if (probe.skipped) return { tone: "muted", text: probe.error ?? "未检测" }
  return { tone: "danger", text: probe.error ?? "不可达" }
}

/** 诊断摘要（画板 102「代理可达 · 额度接口可达」）：代理一项、额度接口一项 */
export function diagnosisSummary(result: NetworkDiagnosisDto): ToneText[] {
  const items: ToneText[] = []
  if (result.proxy) items.push(result.proxy.ok ? { tone: "ok", text: "代理可达" } : { tone: "danger", text: "代理不可达" })
  const checked = result.endpoints.filter((row) => !row.probe.skipped)
  const failed = checked.filter((row) => !row.probe.ok).length
  if (result.endpoints.length === 0) items.push({ tone: "muted", text: "没有已保存的连接" })
  else if (failed > 0) items.push({ tone: "danger", text: `${failed} 个连接不可达` })
  else if (checked.length === 0) items.push({ tone: "muted", text: "额度接口未检测" })
  else items.push({ tone: "ok", text: "额度接口可达" })
  return items
}
