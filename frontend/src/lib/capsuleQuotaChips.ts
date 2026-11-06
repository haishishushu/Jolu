/**
 * 胶囊灵动岛收缩态的额度芯片（Jolu.pen 模块 A3，收缩态 `.cap-main`）。
 *
 * 一个窗口 = 一个芯片：`<弱化色窗口名> <数值>`，与已有的 5h/7d 芯片同一结构
 * （`.cap-q` 里 `<i>` 是弱化窗口名、`<b>` 是数值）。
 *
 * 为什么不用 `remainingDisplay` 的 `rows`：那是卡片式整句（「剩余额度」+「剩余 3000000 token」），
 * 收缩态既没空间，也没有窗口名、看不出用了多少。实测渲染成
 * `Zcode剩余额度剩余 3000000 token剩余额度剩余 5000000 token+20.25M`（宽 568px），
 * 胶囊被撑成很长的条。这里只取结构化字段（`window_name` / `used_percent` / `amount_text` /
 * `remaining_text`），按「窗口名 + 已用」拼芯片，宽度由 `CHIP_MAX_WIDTH` 兜住。
 *
 * 取值优先级（高 → 低）：
 *   1. `usedPercent` → `GLM-5.3 0%`、`Flash 62%`（与额度环、5h/7d 芯片同一口径：已用百分比）；
 *   2. `amountText` 的 `已用 / 总量` → `GLM-5.3 0/3.0M`（来源没给分母时没有百分比，用绝对数）；
 *   3. 只有 `remainingText`（例如 `total_units = 0`，总量未知但剩余量给了）→ `余3.0M`；
 *   4. 都没有 → `-`（未知占位符，与 `remainingDisplay.UNKNOWN_REMAINING` 同义，不写假的 0）。
 *
 * 未知 / 真实 0 / 不支持 / 失败 / 需要重新登录这几类表达不属于本函数：它们由
 * `remainingDisplay` 与 `IslandData` 的 `state` 决定，调用方在芯片之外照旧处理。
 */

/** 窗口名字符上限：超出走短形式，仍超出才截断加省略号（`GLM-5.3-Flash` → `Flash`） */
export const MAX_WINDOW_NAME = 6
/** 收缩态最多显示几个芯片：超出只留最紧的一个 + `+N` */
export const MAX_CHIPS = 2
/** 单芯片宽度上限（px）：外壳宽度按内容测量，名字再长也只占这么宽 */
export const CHIP_MAX_WIDTH = 96
/** 芯片里名字的宽度上限（px）：96 减掉数值（`100%` / `0/3.0M` 约 40px）与 3px 间距 */
export const CHIP_NAME_MAX_WIDTH = 56

export interface CapsuleChipWindow {
  /** 窗口标识（后端 bucket id / 5h / 7d）；只用于 React key 与去重，不上屏 */
  key: string
  /** 窗口名：后端 `window_name`（`GLM-5.3` / `GLM-5.3-Flash`）；5h / 7d 传 `5h` / `7d` */
  name: string
  /** 已用百分比；`null` 表示来源没给分母（未知），不是 0% */
  usedPercent: number | null
  /** `已用 / 总量` 的文字描述，如 `0 / 3000000 token` */
  amountText?: string | null
  /** 来源直接给出的精确剩余量，如 `剩余 3000000 token` */
  remainingText?: string | null
  /** 重置倒计时，只进悬停说明，不占芯片宽度 */
  reset?: string | null
}

export interface CapsuleQuotaChip {
  key: string
  /** 上屏用的短窗口名（弱化色） */
  name: string
  /** `0%` / `0/3.0M` / `余3.0M` / `-` */
  value: string
  /** 悬停说明：完整窗口名 + 取值口径 + 剩余原文 + 重置倒计时 */
  title: string
  /** ≥80% 橙、100% 红；其余 null（复用 `.cap-o` / `.cap-r`，不新增颜色变量） */
  tone: "warn" | "danger" | null
}

export interface CapsuleQuotaChips {
  chips: CapsuleQuotaChip[]
  /** 被折叠的窗口数（>2 时显示 `+N`） */
  more: number
}

/**
 * 数字压缩：`3000000` → `3.0M`、`12500` → `12.5K`、`1.2e9` → `1.2B`、`999` → `999`。
 * 千以上一律保留一位小数，所以尾随零最多只有 `.0` 这一个（`3.00M` 这种不会出现）。
 *
 * 不复用 `api.ts` 的 `compactTokens`：它对 M / B 固定两位小数（`3000000` → `3.00M`），
 * 收缩态要的是一位小数，改它会影响 Token 增量、会话列表等所有既有调用点。
 */
export function compactQuotaNumber(value: number): string {
  if (!Number.isFinite(value)) return "-"
  const abs = Math.abs(value)
  const sign = value < 0 ? "-" : ""
  const scaled = (size: number, unit: string) => `${sign}${(abs / size).toFixed(1)}${unit}`
  if (abs >= 1_000_000_000) return scaled(1_000_000_000, "B")
  if (abs >= 1_000_000) return scaled(1_000_000, "M")
  if (abs >= 1_000) return scaled(1_000, "K")
  return `${sign}${Number.isInteger(abs) ? abs : abs.toFixed(1)}`
}

/** 取一段文字开头的数量：`3000000 token` → `3.0M`；已经是 `3M` 这种压缩写法时原样保留。 */
function leadingQuantity(text: string): string | null {
  const match = text.trim().match(/^([\d][\d,]*(?:\.\d+)?)\s*([KkMmBb])?/)
  if (!match) return null
  const raw = match[1].replace(/,/g, "")
  const value = Number(raw)
  if (!Number.isFinite(value)) return null
  const unit = match[2]?.toUpperCase()
  // 来源已经压缩过（`3M / 3M token`）：不再二次换算，避免 `3M` 变成 `3.0M` 以外的形态
  if (unit) return `${match[1]}${unit}`
  return compactQuotaNumber(value)
}

/** `已用 / 总量`：`0 / 3000000 token` → `0/3.0M`；结构不符（没有 `/` 或取不到数）时返回 null。 */
export function usedOfTotal(amountText: string | null | undefined): string | null {
  if (!amountText) return null
  const parts = amountText.split("/")
  if (parts.length !== 2) return null
  const used = leadingQuantity(parts[0])
  const total = leadingQuantity(parts[1])
  if (used === null || total === null) return null
  return `${used}/${total}`
}

/** 只有剩余量能取到时压成 `余3.0M`（不定量表述，替代「剩余 3000000 token」整句）。 */
function remainingOnly(remainingText: string | null | undefined): string | null {
  if (!remainingText) return null
  const match = remainingText.match(/(\d[\d,]*(?:\.\d+)?)\s*([KkMmBb])?/)
  if (!match) return null
  const raw = match[1].replace(/,/g, "")
  const value = Number(raw)
  if (!Number.isFinite(value)) return null
  const unit = match[2]?.toUpperCase()
  return `余${unit ? `${match[1]}${unit}` : compactQuotaNumber(value)}`
}

/**
 * 同一胶囊里最像「基准名」的那一个：同组里最短、且是别的窗口名的前缀
 * （`GLM-5.3` 之于 `GLM-5.3-Flash`）。只有一个窗口时它自己就是基准名。
 * 名字之间没有前缀关系时返回 null，各名字自行截断。
 */
export function referenceWindowName(names: readonly string[]): string | null {
  const distinct = [...new Set(names.map((name) => name.trim()).filter(Boolean))]
  if (distinct.length === 0) return null
  const shortest = [...distinct].sort((a, b) => a.length - b.length)[0]
  if (distinct.length === 1) return shortest
  return distinct.some((name) => name !== shortest && name.startsWith(shortest)) ? shortest : null
}

/**
 * 窗口名的短形式：
 * - 与基准名同源的变体去掉公共前缀（`GLM-5.3-Flash` → `Flash`），同一胶囊里保持一致；
 * - 不超过 6 字符原样显示（`GLM-5.3` 之外的短名、`7d`、`5h`）；
 * - 基准名本身不截断——它是最短的那个身份（`GLM-5.3`），宽度由 96px 芯片上限与省略号兜住；
 * - 其余超过 6 字符的截成 5 字符 + `…`。
 */
export function shortWindowName(name: string, reference: string | null = null): string {
  const clean = name.trim()
  if (!clean) return "-"
  if (reference && clean !== reference && clean.startsWith(reference)) {
    const rest = clean.slice(reference.length).replace(/^[-_/\s.·]+/, "").trim()
    if (rest.length > 0 && rest.length <= MAX_WINDOW_NAME) return rest
  }
  if (clean.length <= MAX_WINDOW_NAME) return clean
  if (reference && clean === reference) return clean
  // 截断点在分隔符后面时把分隔符一起去掉（`Kimi-K2-Thinking` → `Kimi…`，不出现 `Kimi-…`）
  const head = clean.slice(0, MAX_WINDOW_NAME - 1).replace(/[-_/\s.·]+$/, "")
  return `${head || clean.slice(0, MAX_WINDOW_NAME - 1)}…`
}

/** ≥80% 橙、100% 红；未知（null）不着色，避免把未知画成告警。 */
export function chipTone(usedPercent: number | null): "warn" | "danger" | null {
  if (usedPercent == null || !Number.isFinite(usedPercent)) return null
  if (usedPercent >= 100) return "danger"
  if (usedPercent >= 80) return "warn"
  return null
}

function chipValue(window: CapsuleChipWindow): { value: string; detail: string } {
  const percent = window.usedPercent
  if (percent != null && Number.isFinite(percent)) {
    const shown = Math.round(Math.min(100, Math.max(0, percent)))
    return { value: `${shown}%`, detail: `已用 ${shown}%` }
  }
  const pair = usedOfTotal(window.amountText)
  if (pair) return { value: pair, detail: `已用 ${pair}` }
  const absolute = remainingOnly(window.remainingText)
  if (absolute) return { value: absolute, detail: `来源只给剩余量：${absolute.slice(1)}` }
  return { value: "-", detail: "来源未提供可用数量" }
}

/**
 * 组装收缩态芯片：`>2` 个窗口时只留最紧的一个（已用百分比最高；拿不到百分比的排在其后，
 * 全未知时保留第一个），其余折成 `+N`。内容按行内元素渲染，不新增样式类。
 */
export function capsuleQuotaChips(
  windows: readonly CapsuleChipWindow[],
  limit = MAX_CHIPS,
): CapsuleQuotaChips {
  const visible = windows.filter((window) => Boolean(window.key) || Boolean(window.name))
  if (visible.length === 0) return { chips: [], more: 0 }
  const capped = Math.max(1, limit)
  let shown = [...visible]
  let more = 0
  if (visible.length > capped) {
    const tightest = [...visible].sort((a, b) => (b.usedPercent ?? -1) - (a.usedPercent ?? -1))[0]
    shown = [tightest]
    more = visible.length - 1
  }
  // 基准名按**全部**窗口算：`>2` 折叠后只留最紧的那个时，短形式仍与没显示出来的窗口一致
  const reference = referenceWindowName(visible.map((window) => window.name.trim()))
  const chips = shown.map((window) => {
    const name = shortWindowName(window.name, reference)
    const { value, detail } = chipValue(window)
    // 整句塞不进芯片，但完整的剩余原文与重置倒计时仍要能读到：全部放在悬停说明里
    const title = [
      window.name.trim() || name,
      detail,
      window.remainingText?.trim() || null,
      window.reset ? `${window.reset} 后重置` : null,
    ]
      .filter(Boolean)
      .join(" · ")
    return { key: window.key, name, value, title, tone: chipTone(window.usedPercent) }
  })
  return { chips, more }
}
