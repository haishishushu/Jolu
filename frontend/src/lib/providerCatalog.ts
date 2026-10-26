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
export type ProviderId = PlatformId | "workbuddy-ai" | "trae-cn" | "qoder-cn"

export interface ProviderEntry {
  id: ProviderId
  platformId: PlatformId
  edition?: string
  name: string
  access: string
  subscription: ModeState
  apiKey: ModeState
  /** API Key 方式的 Base URL：预填值；locked 表示服务限定地址，不开放编辑 */
  baseUrl?: { value: string; locked: boolean }
  baseUrlChoices?: readonly {label: string; value: string; protocol: ApiProtocol}[]
}

export const PROVIDERS: readonly ProviderEntry[] = [
  // 自定义供应商（D-11）：只有 API Key 方式，协议与地址由用户填写，不预填 Base URL
  { id: "custom", platformId: "custom", name: "自定义供应商", access: "名称 / Base URL / API Key", subscription: "none", apiKey: "ready" },
  { id: "claude", platformId: "claude", name: "Claude Code", access: "订阅账号 / API Key", subscription: "ready", apiKey: "ready", baseUrl: { value: "https://api.anthropic.com", locked: false } },
  { id: "codex", platformId: "codex", name: "Codex", access: "订阅账号 / API Key", subscription: "ready", apiKey: "ready", baseUrl: { value: "https://api.openai.com/v1", locked: false } },
  { id: "gemini", platformId: "gemini", name: "Gemini", access: "账号授权 / API Key", subscription: "ready", apiKey: "ready", baseUrl: { value: "https://generativelanguage.googleapis.com/v1beta/openai", locked: true } },
  { id: "grok", platformId: "grok", name: "Grok", access: "按可用接入方式连接", subscription: "ready", apiKey: "ready", baseUrl: { value: "https://api.x.ai/v1", locked: true } },
  { id: "opencode", platformId: "opencode", name: "OpenCode", access: "先选择模型服务", subscription: "ready", apiKey: "ready", baseUrl:{value:"https://opencode.ai/zen/go/v1",locked:true} },
  { id: "zcode", platformId: "zcode", name: "ZCode", access: "订阅套餐 / API Key", subscription: "ready", apiKey: "ready", baseUrl:{value:"https://open.bigmodel.cn/api/anthropic",locked:true}, baseUrlChoices:[
    {label:"国内 · Anthropic",value:"https://open.bigmodel.cn/api/anthropic",protocol:"anthropic"},
    {label:"国内 · OpenAI",value:"https://open.bigmodel.cn/api/coding/paas/v4",protocol:"openai"},
    {label:"国际 · Anthropic",value:"https://api.z.ai/api/anthropic",protocol:"anthropic"},
    {label:"国际 · OpenAI",value:"https://api.z.ai/api/coding/paas/v4",protocol:"openai"},
  ] },
  { id: "qoder-cn", platformId: "qoder", edition: "cn", name: "Qoder CN", access: "账号登录 / 可用模型服务", subscription: "ready", apiKey: "none" },
  { id: "qoder", platformId: "qoder", edition: "global", name: "Qoder", access: "账号登录 / 可用模型服务", subscription: "ready", apiKey: "none" },
  { id: "workbuddy", platformId: "workbuddy", edition: "cn", name: "WorkBuddy", access: "账号登录 / 原工具配置模型", subscription: "ready", apiKey: "none" },
  { id: "workbuddy-ai", platformId: "workbuddy", edition: "ai", name: "WorkBuddy AI", access: "账号登录 / 原工具配置模型", subscription: "ready", apiKey: "none" },
  // Trae 不在原画板里，按鼠鼠要求保留并已补进 Jolu.pen 画板 05 / 06 / 29
  { id: "trae-cn", platformId: "trae", edition: "cn", name: "Trae CN", access: "账号登录 / 本机登录", subscription: "ready", apiKey: "none" },
  { id: "trae", platformId: "trae", edition: "global", name: "Trae", access: "账号登录 / 本机登录", subscription: "ready", apiKey: "none" },
]

export function providerEntry(id: ProviderId): ProviderEntry {
  return PROVIDERS.find((entry) => entry.id === id) ?? PROVIDERS[0]
}

interface ProviderConnection {
  id?: string
  platformId: string
  edition?: string | null
}

const NATIVE_EDITIONS: Readonly<Record<string, string>> = {
  "monitor:native:qoder:cn": "cn", "monitor:native:qoder:global": "global",
  "monitor:native:workbuddy:main": "cn", "monitor:native:workbuddy:ai": "ai",
  "monitor:native:trae:cn": "cn", "monitor:native:trae:ide": "global",
}

/** 平台族用于统计，供应商版本仅用于连接展示；未知区域不能默认归到任一版本。 */
export function connectionProviderId(connection: ProviderConnection): ProviderId | null {
  const { platformId, id } = connection
  if (!["qoder", "workbuddy", "trae"].includes(platformId)) {
    return PROVIDERS.find(entry => entry.platformId === platformId)?.id ?? null
  }
  const edition = connection.edition ?? (id?.startsWith(`monitor:native:${platformId}:`) ? NATIVE_EDITIONS[id] : undefined)
  return PROVIDERS.find(entry => entry.platformId === platformId && entry.edition === edition)?.id ?? null
}

export function connectionProviderEntry(connection: ProviderConnection): ProviderEntry {
  const id = connectionProviderId(connection)
  if (id) return providerEntry(id)
  const entry = providerEntry(connection.platformId as PlatformId)
  return { ...entry, name: `${entry.name} · 区域未知` }
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
  if (entry.platformId === "qoder" || entry.platformId === "workbuddy") return `${entry.name} 的自定义模型在 ${entry.name} 客户端内配置，Jolu 不代管密钥。`
  if (showsModeTabs(entry)) return null
  if (entry.apiKey === "pending") return "API Key 连接待接入；当前只提供订阅连接。"
  if (entry.apiKey === "none") return `${entry.name} 暂不支持 API Key；当前只提供订阅连接。`
  if (entry.subscription === "pending") return "订阅连接待接入；当前只提供 API Key 连接。"
  return null
}

/** 自定义供应商的接口协议（D-11）：决定获取模型时的鉴权头 */
export type ApiProtocol = "openai" | "anthropic"

export const API_PROTOCOLS: readonly ApiProtocol[] = ["openai", "anthropic"]

export function protocolLabel(protocol: ApiProtocol | null): string {
  if (protocol === "openai") return "OpenAI 兼容"
  if (protocol === "anthropic") return "Anthropic 兼容"
  return "未记录协议"
}

/** Base URL 输入框示例：OpenAI 兼容服务通常带 /v1，Anthropic 兼容服务填根地址 */
export function protocolBaseUrlHint(protocol: ApiProtocol): string {
  return protocol === "anthropic" ? "https://api.example.com" : "https://api.example.com/v1"
}

/**
 * 自定义供应商获取模型失败后是否改为手填模型 ID（画板 29A 状态 A「模型接口不可用」）。
 * 401 / 403 说明密钥或权限被拒，手填模型也无法使用，保留错误让用户检查 Key（画板 27「获取失败 · 401」）。
 */
export function fallsBackToManualModel(error: string): boolean {
  return !/HTTP 40[13]\b/.test(error)
}

/** 获取模型失败后是否提供「手动填写模型」入口：自定义供应商的密钥被拒（401 / 403）时不提供 */
export function canEnterManualModel(platformId: PlatformId, error: string): boolean {
  return platformId !== "custom" || fallsBackToManualModel(error)
}

/**
 * 请求闸门：每次发起请求取一个序号，返回时只有最新序号的结果有效。
 * 地址、密钥变化或组件卸载时调用 invalidate，让进行中的获取 / 验证结果全部作废。
 */
export function requestGate() {
  let current = 0
  return {
    begin: () => ++current,
    isCurrent: (token: number) => token === current,
    invalidate: () => { current++ },
  }
}

/**
 * 编辑弹窗模型区的忙碌状态：获取与验证共用一个请求闸门，所以同一时刻只有一种进行中状态。
 * 新请求开始即替换旧状态；结束时只有闸门里仍是最新的请求才回到空闲，过期请求结束不改状态。
 */
export type ModelBusy = "idle" | "fetching" | "verifying"

export function modelBusyAfter(
  busy: ModelBusy,
  event: { type: "start"; kind: "fetching" | "verifying" } | { type: "end"; current: boolean },
): ModelBusy {
  if (event.type === "start") return event.kind
  return event.current ? "idle" : busy
}

/** 获取或验证进行中时不能开始验证（「验证模型」「继续验证」都禁用） */
export function canStartVerify(busy: ModelBusy): boolean {
  return busy === "idle"
}

/** 获取模型失败的提示文案：后端原因已带前缀时不重复 */
export function modelFetchErrorText(error: string): string {
  return error.startsWith("获取模型失败") ? error : `获取模型失败：${error}`
}

/** 编辑已保存连接的草稿（画板 27「编辑已保存的连接」） */
export interface EditDraft {
  platformId: PlatformId
  kind: "auth" | "api"
  nameChanged: boolean
  nameValid: boolean
  /** 新填写的 API Key；空串表示不更换 */
  newSecret: string
  model: string
  /** 模型是用新密钥获取到的，或手填后已「验证模型」通过 */
  modelReady: boolean
}

/**
 * 编辑保存规则（画板 27）：留空不覆盖已保存密钥；更换后重新获取模型；仅改名称无需重新验证。
 * 自定义供应商的后端校验只看「有模型」（D-11），所以「更换密钥后必须重新获取或验证模型」由前端把关；
 * 其他平台更换密钥时由后端照常验证。
 */
export function editSaveState(draft: EditDraft): { canSave: boolean; needsModel: boolean; reason: string | null } {
  const replacing = draft.newSecret.trim() !== ""
  const needsModel = draft.platformId === "custom" && draft.kind === "api" && replacing
  if (!draft.nameValid) return { canSave: false, needsModel, reason: null }
  if (!draft.nameChanged && !replacing) return { canSave: false, needsModel, reason: null }
  if (needsModel && !(draft.modelReady && draft.model.trim())) {
    return { canSave: false, needsModel, reason: "更换密钥后请重新获取模型；模型接口不可用时手填模型 ID 并验证模型。" }
  }
  return { canSave: true, needsModel, reason: null }
}

/** 卡片右上角数量徽标 */
export function connectionBadge(count: number): string {
  return count > 0 ? `${count} 个账号` : "尚未连接"
}

/** 名称去首尾空白；同平台同方式重名时提示修改（画板 29「输入与模型」） */
export function validateConnectionName(
  name: string,
  existing: readonly (ProviderConnection & { kind: string; baseName: string })[],
  platform: string,
  kind: "auth" | "api",
): string | null {
  const trimmed = name.trim()
  if (!trimmed) return "请填写连接名称"
  if (trimmed.length > 80) return "连接名称最多 80 个字符"
  if (existing.some((c) => connectionProviderId(c) === platform && c.kind === kind && c.baseName.trim() === trimmed)) {
    return "同一平台已有同名连接，请修改名称"
  }
  return null
}

/** 模型搜索：不区分大小写的子串匹配，保留原顺序 */
export function filterModels(models: readonly string[], query: string): string[] {
  const q = query.trim().toLowerCase()
  return q ? models.filter((model) => model.toLowerCase().includes(q)) : [...models]
}

export const OPENCODE_SERVICES = [
  {id:"opencode-go",label:"OpenCode Go",mode:"apiKey",baseUrl:"https://opencode.ai/zen/go/v1",locked:true},
  {id:"openai",label:"OpenAI 订阅",mode:"subscription",baseUrl:"",locked:true},
  {id:"openai-compatible",label:"自定义 OpenAI 兼容服务",mode:"apiKey",baseUrl:"",locked:false},
] as const
