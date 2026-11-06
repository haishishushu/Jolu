import type { GatewayGroupDto, SourceCapabilityDto } from "./gatewayApi"

/**
 * 「管理来源」弹窗的纯逻辑（测试见 `gatewaySources.test.mjs`）：搜索、已接入 / 未接入筛选、分组范围与计数、调用能力文案。
 * 「已接入」指来源在模型池里（serve_enabled），与连接栏的网关接入状态是两回事。
 */

export interface ManagedSource {
  id: string
  baseName: string
  platformId: string
  kind: string | null
  slug?: string | null
  serve_enabled: boolean
}

export type ServeFilter = "all" | "on" | "off"
/** 分组范围：全部、未分组或某个来源分组 id */
export type GroupScope = { kind: "all" } | { kind: "ungrouped" } | { kind: "group"; id: string }

export function inScope<T extends ManagedSource>(sources: T[], groups: GatewayGroupDto[], scope: GroupScope): T[] {
  if (scope.kind === "all") return sources
  if (scope.kind === "ungrouped") {
    const grouped = new Set(groups.flatMap((group) => group.members))
    return sources.filter((source) => !grouped.has(source.id))
  }
  const members = groups.find((group) => group.id === scope.id)?.members ?? []
  // 按分组里保存的顺序排列
  return members.map((id) => sources.find((source) => source.id === id)).filter((source): source is T => !!source)
}

export function matchesQuery(source: ManagedSource, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  return [source.baseName, source.slug ?? "", source.platformId].some((text) => text.toLowerCase().includes(q))
}

export function filterSources<T extends ManagedSource>(sources: T[], query: string, serve: ServeFilter): T[] {
  return sources.filter((source) => matchesQuery(source, query) && (serve === "all" || (serve === "on") === source.serve_enabled))
}

export function serveCounts(sources: ManagedSource[]): { all: number; on: number; off: number } {
  const on = sources.filter((source) => source.serve_enabled).length
  return { all: sources.length, on, off: sources.length - on }
}

/** 选择集只保留当前仍存在的来源（筛选或删除后裁剪） */
export function pruneSelection(selected: ReadonlySet<string>, visible: ManagedSource[]): Set<string> {
  const ids = new Set(visible.map((source) => source.id))
  return new Set([...selected].filter((id) => ids.has(id)))
}

/** 批量操作真正需要改动的来源：已经是目标状态的跳过 */
export function batchTargets(sources: ManagedSource[], selected: ReadonlySet<string>, enabled: boolean): string[] {
  return sources.filter((source) => selected.has(source.id) && source.serve_enabled !== enabled).map((source) => source.id)
}

export type CapabilityTone = "ok" | "muted" | "danger"

/** 调用能力：选择来源与调用就绪分开表达，不能把开关锁死 */
export function capabilityLine(capability: SourceCapabilityDto | undefined, serving: boolean): { text: string; tone: CapabilityTone } {
  if (!capability) return { text: "正在检查调用能力", tone: "muted" }
  if (capability.state === "needs_auth") return { text: "需要重新登录", tone: "danger" }
  if (capability.state === "unsupported") return { text: serving ? "已选择 · 转发适配尚未就绪" : "转发适配尚未就绪", tone: "muted" }
  return { text: "可调用", tone: "ok" }
}

/** 最近使用（路由流水保留范围内）；没有记录时如实说「暂无经网关的请求」 */
export function usageLine(usage: { last_ts: number; requests: number; failed: number } | undefined, now: number): string {
  if (!usage) return "暂无经网关的请求"
  const diff = Math.max(0, now - usage.last_ts)
  const ago = diff < 60_000 ? "刚刚" : diff < 3_600_000 ? `${Math.floor(diff / 60_000)} 分钟前`
    : diff < 86_400_000 ? `${Math.floor(diff / 3_600_000)} 小时前` : `${Math.floor(diff / 86_400_000)} 天前`
  return `最近使用 ${ago} · ${usage.requests} 次${usage.failed > 0 ? `（失败 ${usage.failed}）` : ""}`
}
