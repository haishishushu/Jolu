import test from "node:test"
import assert from "node:assert/strict"
import { authQuotaRows, quotaRowsFor } from "./authQuotaRows.ts"
import { toQuotaWindows } from "./quotaMap.ts"
import { isPlanCovered } from "./planCoverage.ts"
import { remainingDisplay } from "./remainingDisplay.ts"
import {
  AGENT_PLATFORMS,
  PLACEHOLDER_WINDOWS,
  PLAN_WINDOW_KEYS,
  PROVIDERS,
  isPlaceholderWindowKey,
  isPlanWindowKey,
  providerDescriptor,
  windowBadgeTone,
  windowChipLabel,
  windowRingColor,
  windowRingGeometry,
  windowShortLabel,
  windowSpec,
} from "./providers/registry.ts"

/**
 * 搬移等价性：`lib/providers/registry.ts` 的每个值都必须与改造前的散点实现逐项相等。
 * 这些断言就是「搬移前 vs 搬移后」的核对表在代码里的版本——值变了就红。
 */

const RING_CYAN = "var(--pn-capsule-cyan)"
const RING_GREEN = "var(--pn-capsule-green)"

test("registry 只填真有额度逻辑的平台", () => {
  assert.deepEqual(Object.keys(PROVIDERS).sort(), [
    "claude", "codex", "custom", "gemini", "grok", "opencode", "qoder", "trae", "workbuddy", "zcode",
  ])
  // 未接入的平台没有描述符，调用方各自兜底
  assert.equal(providerDescriptor("custom-unknown"), null)
})

/** 改动前散落在各文件里的平台事实，逐条搬到这里做对照 */
const BEFORE = {
  claude: { quotaSource: "connection", windowKinds: ["rolling-5h", "weekly-7d"], windows: ["5h", "7d", "7d-opus", "7d-sonnet"], placeholderWindows: true, ringGeometry: "arc" },
  codex: { quotaSource: "connection", windowKinds: ["rolling-5h", "weekly-7d"], windows: ["5h", "7d"], placeholderWindows: true, ringGeometry: "arc" },
  opencode: { quotaSource: "connection", windowKinds: ["rolling-5h", "weekly-7d"], windows: ["5h", "7d"], placeholderWindows: true, ringGeometry: "arc" },
  // ZCode 的日额度桶 key 是 bucket_id（不在表里），平台缺省因此是刻度环
  zcode: { quotaSource: "local-zcode", windowKinds: ["rolling-5h", "weekly-7d", "daily", "credit"], windows: ["5h", "7d", "quota"], placeholderWindows: true, ringGeometry: "ticks" },
  grok: { quotaSource: "local-grok", windowKinds: ["weekly-7d", "monthly", "credit"], windows: ["7d", "30d", "credits"], placeholderWindows: false, ringGeometry: "arc" },
  gemini: { quotaSource: "connection", windowKinds: ["none"], windows: [], placeholderWindows: true, ringGeometry: "arc" },
  qoder: { quotaSource: "connection", windowKinds: ["credit"], windows: [], placeholderWindows: true, ringGeometry: "arc" },
  trae: { quotaSource: "connection", windowKinds: ["credit"], windows: [], placeholderWindows: true, ringGeometry: "arc" },
  workbuddy: { quotaSource: "connection", windowKinds: ["credit"], windows: ["credits"], placeholderWindows: true, ringGeometry: "arc" },
  custom: { quotaSource: "connection", windowKinds: ["rolling-5h", "daily", "weekly-7d", "monthly", "credit"], windows: ["5h", "1d", "7d", "30d", "quota"], placeholderWindows: true, ringGeometry: "arc" },
}

test("descriptor 的平台事实与改动前一致", () => {
  for (const [platform, expected] of Object.entries(BEFORE)) {
    const descriptor = providerDescriptor(platform)
    assert.ok(descriptor, `${platform} 必须有描述符`)
    assert.equal(descriptor.platform, platform)
    assert.equal(descriptor.quotaSource, expected.quotaSource, `${platform} quotaSource`)
    assert.deepEqual([...descriptor.windowKinds], expected.windowKinds, `${platform} windowKinds`)
    assert.deepEqual(descriptor.windows.map((window) => window.key), expected.windows, `${platform} windows`)
    assert.equal(descriptor.placeholderWindows, expected.placeholderWindows, `${platform} placeholderWindows`)
    // 平台级 ringGeometry 只是「未声明窗口 key」的缺省：只有 ZCode 的日额度桶需要刻度环
    assert.equal(descriptor.ringGeometry, expected.ringGeometry, `${platform} ringGeometry`)
    // 未声明的 key 用平台缺省：改动前一律中性标签 + 绿色环
    assert.deepEqual(descriptor.tone, { badge: "neutral", ring: RING_GREEN }, `${platform} tone`)
  }
})

test("环几何下沉到窗口条目：5h / 7d 弧段环，日额度与积分桶刻度环", () => {
  // 窗口自己带几何，不再由平台一刀切（平台级只剩「未声明 key」的缺省）
  const before = {
    "5h": "arc",
    "7d": "arc",
    "1d": "ticks",
    "credits": "ticks",
    "quota": "ticks",
    // 30d 画板还没有定义，保守保持弧段环（待确认）
    "30d": "arc",
    // 卡片里的额外周子窗口不画环
    "7d-opus": null,
    "7d-sonnet": null,
  }
  for (const [key, geometry] of Object.entries(before)) {
    assert.equal(windowSpec(key).ringGeometry, geometry, key)
    // 已声明的 key 与平台无关，结果都一样
    assert.equal(windowRingGeometry(key), geometry, key)
    assert.equal(windowRingGeometry(key, "claude"), geometry, key)
    assert.equal(windowRingGeometry(key, "zcode"), geometry, key)
  }
  // 未声明的 key 用平台缺省：ZCode 的 bucket_id 是刻度环，其余平台仍是弧段环
  assert.equal(windowRingGeometry("bkt-1", "zcode"), "ticks")
  assert.equal(windowRingGeometry("bkt-1", "claude"), "arc")
  // 没写进注册表的平台回到改动前的弧段环
  assert.equal(windowRingGeometry("gemini:pro", "gemini"), "arc")
  assert.equal(windowRingGeometry("whatever", "custom-unknown"), "arc")
  assert.equal(windowRingGeometry("whatever"), "arc")
})


test("窗口标签配色与改动前 quotaMap 的 TONE 一致", () => {
  // 改动前：5h 淡紫；7d / 7d-opus / 7d-sonnet 淡绿；1d / 30d 淡蓝；其余中性
  const before = {
    "5h": "purple",
    "7d": "green",
    "7d-opus": "green",
    "7d-sonnet": "green",
    "1d": "blue",
    "30d": "blue",
    "credits": "neutral",
    "quota": "neutral",
  }
  for (const [key, tone] of Object.entries(before)) assert.equal(windowBadgeTone(key), tone, key)
  // 未声明的 key（来源自定义）保持中性，任何平台都一样
  assert.equal(windowBadgeTone("gemini:pro"), "neutral")
  assert.equal(windowBadgeTone("gemini:pro", "gemini"), "neutral")
  assert.equal(windowBadgeTone("trae:0:credits"), "neutral")
})

test("后端 DTO 映射出的标签配色与改动前一致", () => {
  const dto = (key) => ({ key, window_name: key, used_percent: null, amount_text: null, resets_at: null })
  const rows = toQuotaWindows([
    dto("5h"), dto("7d"), dto("7d-opus"), dto("7d-sonnet"), dto("1d"), dto("30d"), dto("credits"), dto("qoder:overall"),
  ])
  assert.deepEqual(rows.map((row) => row.badgeTone), [
    "purple", "green", "green", "green", "blue", "blue", "neutral", "neutral",
  ])
})

test("环色与改动前的 ringColor 分支一致", () => {
  assert.equal(windowRingColor("5h"), RING_CYAN)
  for (const key of ["7d", "7d-opus", "7d-sonnet", "1d", "30d", "credits", "quota", "custom-key"]) {
    assert.equal(windowRingColor(key), RING_GREEN, key)
  }
  // 带平台查同一份缺省，结果不变
  assert.equal(windowRingColor("custom-key", "grok"), RING_GREEN)
})

test("胶囊文案与改动前的 LABEL / 芯片名一致", () => {
  // 改动前 LABEL = { "5h": "5 小时", "7d": "7 天" }，未命中回落后端 window_name
  assert.equal(windowShortLabel("5h"), "5 小时")
  assert.equal(windowShortLabel("7d"), "7 天")
  for (const key of ["7d-opus", "7d-sonnet", "1d", "30d", "credits", "quota", "gemini:pro"]) {
    assert.equal(windowShortLabel(key), null, key)
  }
  // 改动前芯片名：5h / 7d 用 key 原样，其余用后端 window_name
  assert.equal(windowChipLabel("5h"), "5h")
  assert.equal(windowChipLabel("7d"), "7d")
  for (const key of ["7d-opus", "1d", "30d", "credits", "quota", "gemini:pro"]) {
    assert.equal(windowChipLabel(key), null, key)
  }
})

test("套餐窗口与改动前写死的 5h / 7d 完全一致", () => {
  assert.deepEqual([...PLAN_WINDOW_KEYS], ["5h", "7d"])
  for (const key of ["5h", "7d"]) assert.equal(isPlanWindowKey(key), true, key)
  // 月额度、积分额度、日额度、来源自定义窗口都不算套餐
  for (const key of ["1d", "30d", "7d-opus", "7d-sonnet", "credits", "quota", "gemini:pro"]) {
    assert.equal(isPlanWindowKey(key), false, key)
  }
  // 与 planCoverage 的对外行为对齐（新增声明后这里会一起变，正是要盯的地方）
  assert.equal(isPlanCovered([{ key: "5h" }]), true)
  assert.equal(isPlanCovered([{ key: "7d" }]), true)
  assert.equal(isPlanCovered([{ key: "30d" }, { key: "credits" }]), false)
})

test("缺窗口补的占位行与改动前 authQuotaRows 的写死数据一致", () => {
  // 改动前：{ key: "5h", windowName: "5 小时额度", badgeTone: "purple" } / { key: "7d", windowName: "周额度", badgeTone: "green" }
  assert.deepEqual([...PLACEHOLDER_WINDOWS], [
    { key: "5h", name: "5 小时额度", badgeTone: "purple" },
    { key: "7d", name: "周额度", badgeTone: "green" },
  ])
  assert.deepEqual(authQuotaRows(), [
    { key: "5h", windowName: "5 小时额度", usedPercent: null, usedText: null, resetCountdown: null, badgeTone: "purple" },
    { key: "7d", windowName: "周额度", usedPercent: null, usedText: null, resetCountdown: null, badgeTone: "green" },
  ])
  assert.equal(isPlaceholderWindowKey("5h"), true)
  assert.equal(isPlaceholderWindowKey("7d"), true)
  assert.equal(isPlaceholderWindowKey("30d"), false)
})

test("Grok 不补 5h / 7d 占位，其余平台沿用占位", () => {
  const weekly = [{ key: "7d", windowName: "周额度", usedPercent: 37.5, usedText: null, resetCountdown: null, badgeTone: "green" }]
  // 改动前：platform === "grok" 时直接用后端原始窗口，不会凭空多出一条 5 小时
  assert.deepEqual(quotaRowsFor("grok", weekly), weekly)
  assert.deepEqual(quotaRowsFor("grok", undefined), [])
  // 其他平台（含未接入描述符的平台）仍是占位补全：缺 5h 给未知行
  const claudeRows = quotaRowsFor("claude", weekly)
  assert.deepEqual(claudeRows.map((row) => row.key), ["5h", "7d"])
  assert.equal(claudeRows[0].usedPercent, null)
  assert.deepEqual(claudeRows[1], weekly[0])
  assert.deepEqual(quotaRowsFor("custom-unknown", weekly).map((row) => row.key), ["5h", "7d"])
})

const ZCODE_BASE = { title: "ZCode · BigModel 套餐剩余", source: "本机 ZCode BigModel Coding Plan API Key" }
const GROK_BASE = { title: "Grok · SuperGrok 剩余额度", source: "本机 Grok OAuth · 订阅额度" }

test("ZCode 的整句剩余与改动前完全一致", () => {
  assert.deepEqual(remainingDisplay("zcode", "api", null), {
    ...ZCODE_BASE, rows: [], state: "unsupported", reason: "当前 API 连接未绑定本机 ZCode BigModel Key。",
  })
  assert.deepEqual(remainingDisplay("zcode", "auth", null), {
    ...ZCODE_BASE, rows: [], state: "loading", reason: "正在查询 API Key 套餐额度…",
  })
  assert.deepEqual(remainingDisplay("zcode", "auth", { state: "failed", reason: "查询未完成" }), {
    ...ZCODE_BASE, rows: [], state: "failed", reason: "查询未完成",
  })
  assert.deepEqual(remainingDisplay("zcode", "auth", { state: "ok", plan: null, windows: [] }), {
    ...ZCODE_BASE, rows: [], state: "unsupported", reason: "来源未提供可计算的剩余额度。",
  })
  const view = remainingDisplay("zcode", "auth", {
    state: "ok", plan: null, windows: [
      { key: "5h", window_name: "5 小时额度", used_percent: 25, amount_text: "3000 / 12000 credits", remaining_text: "8999 Credits", resets_at: null },
      { key: "7d", window_name: "周额度", used_percent: 0, amount_text: "0 / 60000 credits", remaining_text: "60000 Credits", resets_at: "2026-09-30T00:00:00Z" },
      { key: "quota", window_name: "套餐额度", used_percent: null, amount_text: null, remaining_text: null, resets_at: null },
    ],
  })
  assert.deepEqual(view.rows, [
    { label: "5 小时剩余", value: "8999 Credits", usedText: "3000 / 12000 credits", resetsAt: null },
    { label: "周剩余", value: "60000 Credits", usedText: "0 / 60000 credits", resetsAt: "2026-09-30T00:00:00Z" },
    { label: "剩余额度", value: "-", usedText: null, resetsAt: null },
  ])
  assert.equal(view.state, "ok")
  assert.equal(view.reason, "")
  // 只给已用百分比时换算成剩余，真实 0 仍是 0%（不是未知占位符）
  const computed = remainingDisplay("zcode", "auth", {
    state: "ok", plan: null, windows: [
      { key: "quota", window_name: "套餐额度", used_percent: 100, amount_text: null, remaining_text: null, resets_at: null },
    ],
  })
  assert.equal(computed.rows[0].value, "0%")
  assert.equal(computed.rows[0].usedText, "已用 100%")
})

test("Grok 的整句剩余与改动前完全一致", () => {
  assert.deepEqual(remainingDisplay("grok", "api", null), {
    title: "xAI API 预付余额", source: "xAI 团队钱包", rows: [], state: "unsupported",
    reason: "此余额需要独立的 Management Key 和团队 ID；API Key 无法读取。",
  })
  assert.deepEqual(remainingDisplay("grok", "auth", null), {
    ...GROK_BASE, rows: [], state: "loading", reason: "正在查询订阅额度…",
  })
  const view = remainingDisplay("grok", "auth", {
    state: "ok", plan: null, windows: [
      { key: "7d", window_name: "周额度", used_percent: 37.5, amount_text: null, resets_at: "2026-09-30T00:00:00Z" },
      { key: "30d", window_name: "月额度", used_percent: 12, amount_text: null, resets_at: null },
      { key: "credits", window_name: "积分", used_percent: null, amount_text: null, resets_at: null },
    ],
  })
  assert.deepEqual(view.rows, [
    { label: "SuperGrok 周剩余", value: "62.5%", usedText: "已用 37.5%", resetsAt: "2026-09-30T00:00:00Z" },
    { label: "SuperGrok 月剩余", value: "88%", usedText: "已用 12%", resetsAt: null },
    { label: "SuperGrok 剩余", value: "—", usedText: null, resetsAt: null },
  ])
  assert.equal(view.state, "ok")
  assert.equal(view.reason, "")
  // 有窗口但一个百分比都没有：不支持，不冒充「剩余 100%」
  const unknown = remainingDisplay("grok", "auth", {
    state: "ok", plan: null, windows: [{ key: "credits", window_name: "积分", used_percent: null, amount_text: null, resets_at: null }],
  })
  assert.equal(unknown.state, "unsupported")
  assert.equal(unknown.reason, "来源未提供可计算的已用百分比。")
})

test("查不到在线剩余额度的平台文案与改动前一致", () => {
  const before = {
    gemini: ["Gemini 剩余额度", "Gemini API 项目 / 账号", "本机 Gemini CLI 记录只包含消耗；当前未接入可验证的在线剩余额度查询。"],
    trae: ["Trae 剩余额度", "Trae 个人 / 企业账号", "企业额度查询需要管理员授权；个人账号暂无可验证的剩余额度接口。"],
    qoder: ["Qoder 剩余积分", "Qoder 账号 Credits", "本机记录是已上报积分；账号剩余积分需接入已登录的 Qoder Agent SDK。"],
    workbuddy: ["WorkBuddy 剩余积分", "WorkBuddy 个人账号", "本机记录是已上报积分；个人剩余积分需第三方应用授权及积分读取权限。"],
  }
  for (const [platform, [title, source, reason]] of Object.entries(before)) {
    assert.deepEqual(remainingDisplay(platform, "auth", null), { title, source, rows: [], state: "unsupported", reason })
    // API 连接同一套说法，不会变成订阅额度
    assert.deepEqual(remainingDisplay(platform, "api", null), { title, source, rows: [], state: "unsupported", reason })
  }
  // 没写进 UNAVAILABLE 的平台（claude / codex / opencode / custom）走原来的兜底文案
  for (const platform of ["claude", "codex", "opencode", "custom"]) {
    assert.deepEqual(remainingDisplay(platform, "auth", null), {
      title: "剩余额度", source: "在线账号", rows: [], state: "unsupported", reason: "当前未接入在线剩余额度查询。",
    })
  }
})

test("同一窗口 key 在多家的声明必须逐字段一致", () => {
  const seen = new Map()
  for (const descriptor of Object.values(PROVIDERS)) {
    for (const window of descriptor.windows) {
      const known = seen.get(window.key)
      if (!known) { seen.set(window.key, window); continue }
      assert.deepEqual({ ...window }, { ...known }, `${window.key} 在多家声明里不一致`)
      // 全局查表取第一份，因此重复声明必须完全相同，不能各自为政
      assert.deepEqual({ ...windowSpec(window.key) }, { ...known })
    }
  }
  assert.deepEqual([...seen.keys()].sort(), ["1d", "30d", "5h", "7d", "7d-opus", "7d-sonnet", "credits", "quota"])
})

test("Agent → 平台标识与改动前 CapsuleGateway 的写死映射一致", () => {
  assert.deepEqual({ ...AGENT_PLATFORMS }, {
    "claude-code": "claude", codex: "codex", gemini: "gemini", opencode: "opencode", zcode: "zcode",
  })
})
