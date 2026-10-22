import type { PlatformId } from "@/types"

/**
 * 供应商页的平台目录（Jolu.pen 模块 B 画板 05 / 29）。
 *
 * 名称与「接入方式」文案取自画板 05；两种连接方式的状态按画板 29 与当前实现共同决定：
 *   ready   已实现，可以连接
 *   pending 设计已确认、程序尚未接入：入口显示「待接入」，不假装能连
 *   none    平台不提供该方式，不展示入口
 */
export type ModeState = "ready" | "pending" | "none"

export interface ProviderEntry {
  id: PlatformId
  name: string
  access: string
  subscription: ModeState
  apiKey: ModeState
  /** API Key 方式的 Base URL：预填值；locked 表示服务限定地址，不开放编辑 */
  baseUrl?: { value: string; locked: boolean }
}

export const PROVIDERS: readonly ProviderEntry[] = [
  { id: "claude", name: "Claude Code", access: "订阅账号 / API Key", subscription: "ready", apiKey: "ready", baseUrl: { value: "https://api.anthropic.com", locked: false } },
  { id: "codex", name: "Codex", access: "订阅账号 / API Key", subscription: "ready", apiKey: "ready", baseUrl: { value: "https://api.openai.com/v1", locked: false } },
  { id: "gemini", name: "Gemini", access: "账号授权 / API Key", subscription: "ready", apiKey: "ready", baseUrl: { value: "https://generativelanguage.googleapis.com/v1beta/openai", locked: true } },
  { id: "grok", name: "Grok", access: "按可用接入方式连接", subscription: "ready", apiKey: "ready", baseUrl: { value: "https://api.x.ai/v1", locked: true } },
  { id: "opencode", name: "OpenCode", access: "先选择模型服务", subscription: "pending", apiKey: "pending" },
  { id: "zcode", name: "ZCode", access: "订阅套餐 / API Key", subscription: "ready", apiKey: "pending" },
  { id: "qoder", name: "Qoder", access: "账号登录 / 可用模型服务", subscription: "ready", apiKey: "pending" },
  { id: "workbuddy", name: "WorkBuddy", access: "账号登录 / 自定义 API", subscription: "ready", apiKey: "pending" },
  // Trae 不在原画板里，按鼠鼠要求保留并已补进 Jolu.pen 画板 05 / 06 / 29
  { id: "trae", name: "Trae", access: "本机登录检测", subscription: "ready", apiKey: "none" },
]

export function providerEntry(id: PlatformId): ProviderEntry {
  return PROVIDERS.find((entry) => entry.id === id) ?? PROVIDERS[0]
}

/** 至少一种方式已实现，卡片上的「连接」按钮才可用 */
export function canConnect(entry: ProviderEntry): boolean {
  return entry.subscription === "ready" || entry.apiKey === "ready"
}

/** 两种方式都已实现才显示方式页签；只有一种时直接显示该方式（画板 29） */
export function showsModeTabs(entry: ProviderEntry): boolean {
  return entry.subscription === "ready" && entry.apiKey === "ready"
}

export type ConnectMode = "subscription" | "apiKey"

export function defaultMode(entry: ProviderEntry): ConnectMode {
  return entry.subscription === "ready" ? "subscription" : "apiKey"
}

/** 只提供一种方式时，向用户解释另一种为什么不在（画板 13「条件说明」） */
export function singleModeReason(entry: ProviderEntry): string | null {
  if (showsModeTabs(entry)) return null
  if (entry.apiKey === "pending") return "API Key 连接待接入；当前只提供订阅连接。"
  if (entry.apiKey === "none") return `${entry.name} 暂不支持 API Key；当前只提供订阅连接。`
  if (entry.subscription === "pending") return "订阅连接待接入；当前只提供 API Key 连接。"
  return null
}

/** 卡片右上角数量徽标 */
export function connectionBadge(count: number): string {
  return count > 0 ? `${count} 个账号` : "尚未连接"
}

/** 名称去首尾空白；同平台同方式重名时提示修改（画板 29「输入与模型」） */
export function validateConnectionName(
  name: string,
  existing: readonly { platformId: string; kind: string; baseName: string }[],
  platform: string,
  kind: "auth" | "api",
): string | null {
  const trimmed = name.trim()
  if (!trimmed) return "请填写连接名称"
  if (trimmed.length > 80) return "连接名称最多 80 个字符"
  if (existing.some((c) => c.platformId === platform && c.kind === kind && c.baseName.trim() === trimmed)) {
    return "同一平台已有同名连接，请修改名称"
  }
  return null
}

/** 模型搜索：不区分大小写的子串匹配，保留原顺序 */
export function filterModels(models: readonly string[], query: string): string[] {
  const q = query.trim().toLowerCase()
  return q ? models.filter((model) => model.toLowerCase().includes(q)) : [...models]
}
