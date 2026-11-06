import type { PlatformId } from "@/types"

/**
 * 供应商额度描述符（`docs/specs/2026-11-05-provider-quota-descriptor.md` §4.2 的窄版）。
 *
 * 第 1 步只做搬移：字段只保留展示层真正读到的那些，取值的唯一来源是 `registry.ts`
 * 的数据对象。`components/island/` 里不允许再出现平台字面量，一律来这里查表。
 */

/**
 * 额度窗口形态。既是模板与几何的第一等字段，也是「哪些窗口算订阅窗口」的依据。
 *
 * `none` 表示来源自定义的窗口（key 由来源决定，如 `gemini:<model>`、`qoder:<key>`），
 * 归不进其余五种形态；它不等于「没有额度」，也不参与套餐判定。
 */
export type QuotaWindowKind = "rolling-5h" | "weekly-7d" | "daily" | "monthly" | "credit" | "none"

/** 计量单位。与后端返回一致；这一步只做声明，不参与换算 */
export type QuotaUnit = "token" | "percent" | "credit" | "currency" | "request"

/**
 * 额度环几何。几何按**窗口**声明（`ProviderWindow.ringGeometry`），不再按平台一刀切：
 *
 * - `arc`：弧段环（订阅窗口 5h / 7d，Jolu.pen A3-5 / A3-6）；
 * - `ticks`：24 段刻度环（日额度与积分桶，Jolu.pen A3-13 / A3-14）；
 * - `bar`：贴边轨道，胶囊还没有对应画板，窗口一律不声明。
 */
export type RingGeometry = "arc" | "ticks" | "bar"

/** 窗口标签配色：表示窗口身份，不随水位变化（沿用 `QuotaWindow.badgeTone`） */
export type QuotaBadgeTone = "purple" | "green" | "blue" | "neutral"

/** 额度查询入口：按连接查，还是读本机原工具登录 */
export type ProviderQuotaSource = "connection" | "local-codex" | "local-grok" | "local-zcode"

/** `useQuota` 的本机直查模式（`local` 参数）；`false` 表示按连接查 */
export type LocalQuotaMode = "codex" | "grok" | "zcode"

/**
 * 一个额度窗口的声明。`key` 是后端额度 DTO 的 `key`，全仓只在这里认识它：
 * 窗口名、环色、标签配色、占位行、是否算套餐都由这张条目给出，组件里不再按平台分支。
 */
export interface ProviderWindow {
  /** 后端窗口 key（5h / 7d / 30d / credits / quota …） */
  key: string
  /** 窗口形态 */
  kind: QuotaWindowKind
  /** 计量单位 */
  unit: QuotaUnit
  /** 是否参与「套餐制」判定（取代 `planCoverage` 里写死的 5h / 7d） */
  countsAsPlan: boolean
  /**
   * 环几何：`arc` 弧段环 / `ticks` 刻度环；`null` = 不画环。
   *
   * 几何跟着窗口走，所以同一条连接上的 5h / 7d（弧段环）与日额度桶（刻度环）可以同时出现。
   * 它只管「怎么画」，不参与套餐判定（`planCoverage` 仍只认 `countsAsPlan`）。
   */
  ringGeometry: RingGeometry | null
  /** 环图例的短名（胶囊）：5h → 「5 小时」；null = 回落后端 `window_name` */
  shortLabel: string | null
  /** 胶囊芯片名：5h → 「5h」；null = 回落后端 `window_name` */
  chipLabel: string | null
  /** `authQuotaRows` 的占位行名：5h → 「5 小时额度」；null = 不补占位 */
  placeholderName: string | null
  /** 标签底色 */
  badgeTone: QuotaBadgeTone
  /** 环色 CSS 变量 */
  ringColor: string
}

/**
 * `remainingDisplay` 的口径。两种来源严格分开：
 * - `unsupported`：确认查不到在线剩余额度（本机只有消耗记录 / 缺凭证 / 权限不够）；
 * - `online`：有在线查询，标题、来源、加载文案、行标签与未知占位都按平台声明。
 */
export type ProviderRemaining =
  | {
      kind: "unsupported"
      /** 卡片标题 */
      title: string
      /** 来源说明 */
      source: string
      /** 为什么读不到在线剩余额度 */
      reason: string
    }
  | {
      kind: "online"
      title: string
      source: string
      /** API 连接的说法（与订阅额度严格分开） */
      api: { title: string; source: string; reason: string }
      /** 还没有查询结果时的说明 */
      loadingReason: string
      /** 窗口 key → 整句剩余的标签 */
      windowLabels: Readonly<Record<string, string>>
      /** 未声明 key 的标签 */
      windowLabelFallback: string
      /** 数值未知时的占位符；同一仓库里有意按平台不同（`-` / `—`） */
      unknownText: string
      /** 来源返回精确剩余量时优先用它（Grok 只认已用百分比） */
      preferRemainingText: boolean
      /** 窗口在、数值未知时是否仍算查询成功（ZCode 看「有没有窗口」，Grok 看「有没有数」） */
      okWhenValueUnknown: boolean
      /** 一条窗口都没有时的原因 */
      emptyReason: string
    }

/** 卡片与胶囊上的整句文案（窗口级短名在 `ProviderWindow` 里） */
export interface ProviderLabels {
  /** 整句剩余 / 不可用原因 */
  remaining: ProviderRemaining
}

/** 配色缺省：未在 `windows` 里声明的 key 用它（改动前不会给未声明的 key 上色） */
export interface ProviderTone {
  /** 标签底色 */
  badge: QuotaBadgeTone
  /** 环色 CSS 变量 */
  ring: string
}

/** 一家供应商的额度描述符。新增平台 = 在 `registry.ts` 里加一条数据对象。 */
export interface ProviderDescriptor {
  /** 与 `platformCatalog.json` 的 id 一致 */
  platform: PlatformId
  /** 额度从哪来 */
  quotaSource: ProviderQuotaSource
  /** 该来源会出现的窗口形态（窄枚举）；空数组 = 没有可归类的窗口 */
  windowKinds: readonly QuotaWindowKind[]
  /** 额度环几何的**平台缺省**：只给未在 `windows` 里声明的窗口 key 用（ZCode 的日额度桶） */
  ringGeometry: RingGeometry
  /** 该来源会出现的窗口：形态、配色、文案都在窗口条目里，不按平台分支 */
  windows: readonly ProviderWindow[]
  /** 缺窗口时是否补 5h / 7d 未知占位（`authQuotaRows` 的既有行为）；Grok 不补 */
  placeholderWindows: boolean
  /** 整句文案 */
  labels: ProviderLabels
  /** 未声明窗口 key 的配色缺省 */
  tone: ProviderTone
}

/** 缺窗口时补的占位行（名字与配色都来自 `ProviderWindow`） */
export interface PlaceholderWindow {
  key: string
  name: string
  badgeTone: QuotaBadgeTone
}
