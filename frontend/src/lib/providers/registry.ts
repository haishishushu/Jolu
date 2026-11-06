import type { PlatformId } from "@/types"
import type {
  PlaceholderWindow,
  ProviderDescriptor,
  ProviderRemaining,
  ProviderTone,
  ProviderWindow,
  QuotaBadgeTone,
  RingGeometry,
} from "./types.ts"

/**
 * 供应商额度注册表 —— 平台特判的唯一出处。
 *
 * 这一步是纯搬移：表里的每个值都与改造前的散点实现逐项相等（对照表见
 * `providerRegistry.test.mjs`）。新增一家供应商先在这里加一条数据对象，
 * 组件（尤其 `components/island/`）只查表，不再出现 `platform === "…"`。
 *
 * 平台键与 `frontend/src/lib/platformCatalog.json` 的 id 一致。
 */

/** 胶囊环色：5 小时青、其余绿（与改动前 `ringColor()` 的分支一致） */
const RING_CYAN = "var(--pn-capsule-cyan)"
const RING_GREEN = "var(--pn-capsule-green)"

/**
 * 窗口条目。同一个 key 在多家声明时必须逐字段一致（`providerRegistry.test.mjs` 断言）。
 * `7d-opus` / `7d-sonnet` 是 Claude 账号额度里的 Sonnet / Opus 周窗口，配色沿用周额度。
 */
const W5H: ProviderWindow = {
  key: "5h",
  kind: "rolling-5h",
  unit: "percent",
  countsAsPlan: true,
  // 订阅窗口：弧段环（A3-5 / A3-6），几何与改造前一致
  ringGeometry: "arc",
  shortLabel: "5 小时",
  chipLabel: "5h",
  placeholderName: "5 小时额度",
  badgeTone: "purple",
  ringColor: RING_CYAN,
}
const W7D: ProviderWindow = {
  key: "7d",
  kind: "weekly-7d",
  unit: "percent",
  countsAsPlan: true,
  ringGeometry: "arc",
  shortLabel: "7 天",
  chipLabel: "7d",
  placeholderName: "周额度",
  badgeTone: "green",
  ringColor: RING_GREEN,
}
const W7D_OPUS: ProviderWindow = {
  key: "7d-opus",
  kind: "weekly-7d",
  unit: "percent",
  countsAsPlan: false,
  // 卡片里的额外周子窗口：改动前就不参与环（只并列在 5h / 7d 之外的文字行里），
  // 声明成可环化会按水位把 5h / 7d 挤出环，等于改了订阅环的渲染结果
  ringGeometry: null,
  shortLabel: null,
  chipLabel: null,
  placeholderName: null,
  badgeTone: "green",
  ringColor: RING_GREEN,
}
const W7D_SONNET: ProviderWindow = {
  key: "7d-sonnet",
  kind: "weekly-7d",
  unit: "percent",
  countsAsPlan: false,
  ringGeometry: null,
  shortLabel: null,
  chipLabel: null,
  placeholderName: null,
  badgeTone: "green",
  ringColor: RING_GREEN,
}
/** 自建网关（sub2api 订阅型分组）的日消费窗口 */
const W1D: ProviderWindow = {
  key: "1d",
  kind: "daily",
  unit: "currency",
  countsAsPlan: false,
  // 日额度 = 刻度环（A3-13 / A3-14）
  ringGeometry: "ticks",
  shortLabel: null,
  chipLabel: null,
  placeholderName: null,
  badgeTone: "blue",
  ringColor: RING_GREEN,
}
/**
 * 月额度。Grok 的 SuperGrok 月窗口给已用百分比，网关的 `30d` 是月消费金额；
 * 同一个 key 两种单位，`unit` 只做声明、不参与换算，这里取额度语义。
 */
const W30D: ProviderWindow = {
  key: "30d",
  kind: "monthly",
  unit: "percent",
  countsAsPlan: false,
  // A3-13 / A3-14 只定义了日额度的刻度环；月窗口还没有画板，保持弧段环（待鼠鼠确认）
  ringGeometry: "arc",
  shortLabel: null,
  chipLabel: null,
  placeholderName: null,
  badgeTone: "blue",
  ringColor: RING_GREEN,
}
/** 积分 / 额度包（Grok 积分额度、WorkBuddy 资源包） */
const WCREDITS: ProviderWindow = {
  key: "credits",
  kind: "credit",
  unit: "credit",
  countsAsPlan: false,
  // 积分桶和日额度同族：刻度环
  ringGeometry: "ticks",
  shortLabel: null,
  chipLabel: null,
  placeholderName: null,
  badgeTone: "neutral",
  ringColor: RING_GREEN,
}
/** 总配额 / 套餐额度（ZCode 本机登录兜底、自建网关总额度） */
const WQUOTA: ProviderWindow = {
  key: "quota",
  kind: "credit",
  unit: "percent",
  countsAsPlan: false,
  ringGeometry: "ticks",
  shortLabel: null,
  chipLabel: null,
  placeholderName: null,
  badgeTone: "neutral",
  ringColor: RING_GREEN,
}

/** 未声明窗口 key 的缺省：中性标签 + 绿色环 */
const DEFAULT_TONE: ProviderTone = { badge: "neutral", ring: RING_GREEN }

/**
 * 平台没写进注册表时的缺省几何（改动前所有环都是弧段环）。
 * 描述符里的 `ringGeometry` 是**平台缺省**，只用于未在 `windows` 里声明的 key。
 */
const DEFAULT_RING_GEOMETRY: RingGeometry = "arc"

/** 改动前 `remainingDisplay` 对没有平台分支的走法：一律「未接入在线剩余额度」 */
const NO_ONLINE_REMAINING: ProviderRemaining = {
  kind: "unsupported",
  title: "剩余额度",
  source: "在线账号",
  reason: "当前未接入在线剩余额度查询。",
}

/**
 * platform → descriptor。只填这一步真有额度逻辑的平台。
 *
 * 说明：
 * - `bigmodel` 不是独立平台：ZCode 的 BigModel 个人订阅与 BigModel API Key 都落在 `zcode`
 *   这一条（后端 `zcode_quota.rs` 的 login family `bigmodel`），因此没有单独的 `bigmodel` 键。
 * - `custom` 是自定义供应商 / 自建网关（sub2api、new-api），会按 5h / 日 / 周 / 月 / 总额度
 *   返回窗口，这些 key 的配色与标签也必须在这里声明，否则会退回中性色。
 */
export const PROVIDERS: Readonly<Partial<Record<PlatformId, ProviderDescriptor>>> = {
  claude: {
    platform: "claude",
    // 订阅额度按连接查（Claude OAuth）；API Key 只给组织用量，不是订阅额度
    quotaSource: "connection",
    windowKinds: ["rolling-5h", "weekly-7d"],
    ringGeometry: "arc",
    windows: [W5H, W7D, W7D_OPUS, W7D_SONNET],
    placeholderWindows: true,
    labels: { remaining: NO_ONLINE_REMAINING },
    tone: DEFAULT_TONE,
  },
  codex: {
    platform: "codex",
    quotaSource: "connection",
    windowKinds: ["rolling-5h", "weekly-7d"],
    ringGeometry: "arc",
    windows: [W5H, W7D],
    placeholderWindows: true,
    labels: { remaining: NO_ONLINE_REMAINING },
    tone: DEFAULT_TONE,
  },
  opencode: {
    platform: "opencode",
    // OpenCode 里的 OpenAI 订阅登录按 Codex 的账号额度解析，窗口同样是 5h + 周
    quotaSource: "connection",
    windowKinds: ["rolling-5h", "weekly-7d"],
    ringGeometry: "arc",
    windows: [W5H, W7D],
    placeholderWindows: true,
    labels: { remaining: NO_ONLINE_REMAINING },
    tone: DEFAULT_TONE,
  },
  zcode: {
    platform: "zcode",
    // 灵动岛走本机 ZCode 登录（local-zcode）；API Key 连接走 BigModel 编码套餐（5h + 周）
    quotaSource: "local-zcode",
    windowKinds: ["rolling-5h", "weekly-7d", "daily", "credit"],
    // 平台缺省给未声明的 key：本机登录返回的是按 bucket_id 的日额度桶（不在这里声明），
    // 它们按刻度环画；5h / 7d 在窗口条目里各自声明成弧段环，不受这条影响
    ringGeometry: "ticks",
    // 账号额度是按 bucket 返回的日额度，key 是 bucket_id，不在这里声明（回落后端窗口名）
    windows: [W5H, W7D, WQUOTA],
    placeholderWindows: true,
    labels: {
      remaining: {
        kind: "online",
        title: "ZCode · BigModel 套餐剩余",
        source: "本机 ZCode BigModel Coding Plan API Key",
        api: {
          title: "ZCode · BigModel 套餐剩余",
          source: "本机 ZCode BigModel Coding Plan API Key",
          reason: "当前 API 连接未绑定本机 ZCode BigModel Key。",
        },
        loadingReason: "正在查询 API Key 套餐额度…",
        windowLabels: { "5h": "5 小时剩余", "7d": "周剩余" },
        windowLabelFallback: "剩余额度",
        unknownText: "-",
        preferRemainingText: true,
        okWhenValueUnknown: true,
        emptyReason: "来源未提供可计算的剩余额度。",
      },
    },
    tone: DEFAULT_TONE,
  },
  grok: {
    platform: "grok",
    // SuperGrok 订阅额度读本机 Grok OAuth（local-grok）
    quotaSource: "local-grok",
    windowKinds: ["weekly-7d", "monthly", "credit"],
    ringGeometry: "arc",
    windows: [W7D, W30D, WCREDITS],
    // Grok 的窗口是周 / 月 / 积分，没有 5 小时窗口；补占位会凭空多出一条「5 小时」
    placeholderWindows: false,
    labels: {
      remaining: {
        kind: "online",
        title: "Grok · SuperGrok 剩余额度",
        source: "本机 Grok OAuth · 订阅额度",
        api: {
          title: "xAI API 预付余额",
          source: "xAI 团队钱包",
          reason: "此余额需要独立的 Management Key 和团队 ID；API Key 无法读取。",
        },
        loadingReason: "正在查询订阅额度…",
        windowLabels: { "7d": "SuperGrok 周剩余", "30d": "SuperGrok 月剩余" },
        windowLabelFallback: "SuperGrok 剩余",
        unknownText: "—",
        preferRemainingText: false,
        okWhenValueUnknown: false,
        emptyReason: "来源未提供可计算的已用百分比。",
      },
    },
    tone: DEFAULT_TONE,
  },
  gemini: {
    platform: "gemini",
    quotaSource: "connection",
    // 本机登录只给按模型的 bucket（key 是 gemini:<model>[:<tokenType>]），归不进共用形态
    windowKinds: ["none"],
    ringGeometry: "arc",
    windows: [],
    placeholderWindows: true,
    labels: {
      remaining: {
        kind: "unsupported",
        title: "Gemini 剩余额度",
        source: "Gemini API 项目 / 账号",
        reason: "本机 Gemini CLI 记录只包含消耗；当前未接入可验证的在线剩余额度查询。",
      },
    },
    tone: DEFAULT_TONE,
  },
  qoder: {
    platform: "qoder",
    quotaSource: "connection",
    // 账号额度是积分：key 由原生来源决定（qoder:overall / qoder:<key>）
    windowKinds: ["credit"],
    ringGeometry: "arc",
    windows: [],
    placeholderWindows: true,
    labels: {
      remaining: {
        kind: "unsupported",
        title: "Qoder 剩余积分",
        source: "Qoder 账号 Credits",
        reason: "本机记录是已上报积分；账号剩余积分需接入已登录的 Qoder Agent SDK。",
      },
    },
    tone: DEFAULT_TONE,
  },
  trae: {
    platform: "trae",
    quotaSource: "connection",
    // 额度按套餐 / 奖励包返回积分，key 由原生来源决定（trae:<index>:<limit>）
    windowKinds: ["credit"],
    ringGeometry: "arc",
    windows: [],
    placeholderWindows: true,
    labels: {
      remaining: {
        kind: "unsupported",
        title: "Trae 剩余额度",
        source: "Trae 个人 / 企业账号",
        reason: "企业额度查询需要管理员授权；个人账号暂无可验证的剩余额度接口。",
      },
    },
    tone: DEFAULT_TONE,
  },
  workbuddy: {
    platform: "workbuddy",
    quotaSource: "connection",
    // 积分资源包：汇总窗口 key 是 credits，逐包是 workbuddy:package:<index>
    windowKinds: ["credit"],
    ringGeometry: "arc",
    windows: [WCREDITS],
    placeholderWindows: true,
    labels: {
      remaining: {
        kind: "unsupported",
        title: "WorkBuddy 剩余积分",
        source: "WorkBuddy 个人账号",
        reason: "本机记录是已上报积分；个人剩余积分需第三方应用授权及积分读取权限。",
      },
    },
    tone: DEFAULT_TONE,
  },
  custom: {
    platform: "custom",
    quotaSource: "connection",
    // 自定义供应商 / 自建网关（sub2api、new-api）：5h、日 / 周 / 月消费、总额度
    windowKinds: ["rolling-5h", "daily", "weekly-7d", "monthly", "credit"],
    ringGeometry: "arc",
    windows: [W5H, W1D, W7D, W30D, WQUOTA],
    placeholderWindows: true,
    labels: { remaining: NO_ONLINE_REMAINING },
    tone: DEFAULT_TONE,
  },
}

/**
 * 模型入口的 Agent 标识 → 平台。
 * 胶囊头部的头像用它挑平台图标（原先写在 `components/island/CapsuleGateway.tsx`）；
 * 与额度无关，但同属「平台标识」这一处，组件里不再出现平台字面量。
 */
export const AGENT_PLATFORMS: Readonly<Record<string, PlatformId>> = {
  "claude-code": "claude", codex: "codex", gemini: "gemini", opencode: "opencode", zcode: "zcode",
}

const BY_PLATFORM: Readonly<Partial<Record<string, ProviderDescriptor>>> = PROVIDERS

/** 查一家供应商的描述符；未接入的平台返回 null（调用方各自兜底） */
export function providerDescriptor(platform: string): ProviderDescriptor | null {
  return BY_PLATFORM[platform] ?? null
}

/** key → 窗口条目：同一个 key 多平台声明时取第一份；各份必须一致（测试断言） */
function buildWindowIndex(): ReadonlyMap<string, ProviderWindow> {
  const index = new Map<string, ProviderWindow>()
  for (const descriptor of Object.values(PROVIDERS)) {
    for (const window of descriptor?.windows ?? []) {
      if (!index.has(window.key)) index.set(window.key, window)
    }
  }
  return index
}
const WINDOW_INDEX = buildWindowIndex()

/** 查窗口 key 的声明；未声明的 key 返回 null */
export function windowSpec(key: string): ProviderWindow | null {
  return WINDOW_INDEX.get(key) ?? null
}

/**
 * 窗口标签底色。未声明的 key 用该平台的缺省（各家都声明为中性），
 * 因此 `quotaMap` 这种不带平台的调用与带平台的结果一致。
 */
export function windowBadgeTone(key: string, platform?: string | null): QuotaBadgeTone {
  const own = WINDOW_INDEX.get(key)
  if (own) return own.badgeTone
  return (platform ? providerDescriptor(platform)?.tone.badge : undefined) ?? DEFAULT_TONE.badge
}

/** 窗口环色。未声明的 key 用该平台的缺省（改动前一律绿色） */
export function windowRingColor(key: string, platform?: string | null): string {
  const own = WINDOW_INDEX.get(key)
  if (own) return own.ringColor
  return (platform ? providerDescriptor(platform)?.tone.ring : undefined) ?? DEFAULT_TONE.ring
}

/**
 * 窗口的环几何：几何按**窗口**声明，不再按平台。
 *
 * 已声明的 key 用它自己的 `ringGeometry`（`null` = 不画环：如 7d-opus / 7d-sonnet）；
 * 未声明的 key（ZCode 的 `bucket_id`、Gemini 的 `gemini:<model>`、Qoder / Trae 的来源自定义 key）
 * 回落到平台缺省（`ProviderDescriptor.ringGeometry`），平台没写进注册表时回到弧段环。
 *
 * 返回 `null` 的窗口不参与环——它就是「不画环」的唯一判断，和 `planCoverage`
 * （算不算套餐窗口）互不影响。
 */
export function windowRingGeometry(key: string, platform?: string | null): RingGeometry | null {
  const own = WINDOW_INDEX.get(key)
  if (own) return own.ringGeometry
  return (platform ? providerDescriptor(platform)?.ringGeometry : undefined) ?? DEFAULT_RING_GEOMETRY
}

/** 环图例短名；未声明（或条目没给短名）时返回 null，由调用方回落后端窗口名 */
export function windowShortLabel(key: string): string | null {
  return WINDOW_INDEX.get(key)?.shortLabel ?? null
}

/** 胶囊芯片名；未声明时返回 null，由调用方回落后端窗口名 */
export function windowChipLabel(key: string): string | null {
  return WINDOW_INDEX.get(key)?.chipLabel ?? null
}

/** 这个 key 是否补未知占位行（`authQuotaRows`） */
export function isPlaceholderWindowKey(key: string): boolean {
  return (WINDOW_INDEX.get(key)?.placeholderName ?? null) !== null
}

/**
 * 算「套餐制」的窗口 key：描述符声明过、且 `countsAsPlan` 为真。
 * 改动前是写死的 `["5h", "7d"]`；日 / 月 / 积分窗口都不参与。
 */
export const PLAN_WINDOW_KEYS: readonly string[] = [...WINDOW_INDEX.values()]
  .filter((window) => window.countsAsPlan)
  .map((window) => window.key)
const PLAN_WINDOW_KEY_SET = new Set(PLAN_WINDOW_KEYS)

/** 这个窗口是否算订阅套餐窗口（`planCoverage` 的唯一依据） */
export function isPlanWindowKey(key: string): boolean {
  return PLAN_WINDOW_KEY_SET.has(key)
}

/** 缺数据时要补的占位行；顺序即声明顺序（当前是 5h、7d） */
export const PLACEHOLDER_WINDOWS: readonly PlaceholderWindow[] = [...WINDOW_INDEX.values()]
  .flatMap((window) =>
    window.placeholderName
      ? [{ key: window.key, name: window.placeholderName, badgeTone: window.badgeTone }]
      : [],
  )
