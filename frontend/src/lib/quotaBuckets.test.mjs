import test from "node:test"
import assert from "node:assert/strict"
import { MAX_BUCKET_COLUMNS, quotaBucketView, quotaBucketViewFromWindows } from "./quotaBuckets.ts"

const NOW = Date.parse("2026-11-06T12:00:00Z")

const window = (over = {}) => ({
  key: "bkt-1",
  window_name: "GLM-5.3",
  used_percent: 40,
  amount_text: "1200000 / 3000000 token",
  remaining_text: "剩余 1800000 token",
  resets_at: "2026-11-06T17:12:00Z",
  ...over,
})

test("百分比优先：主值 40%，副值压缩成 1.2M / 3.0M 与 剩 1.8M", () => {
  const view = quotaBucketView([window()], NOW)
  assert.ok(view)
  assert.equal(view.buckets.length, 1)
  assert.deepEqual(
    {
      name: view.buckets[0].name,
      value: view.buckets[0].value,
      usedTotal: view.buckets[0].usedTotal,
      remaining: view.buckets[0].remaining,
      percent: view.buckets[0].percent,
      zero: view.buckets[0].zero,
      tone: view.buckets[0].tone,
      reset: view.buckets[0].reset,
    },
    {
      name: "GLM-5.3",
      value: "40%",
      usedTotal: "1.2M / 3.0M",
      remaining: "1.8M",
      percent: 40,
      zero: false,
      tone: null,
      reset: "5h 12m",
    },
  )
  assert.equal(view.reset, "5h 12m")
  assert.equal(view.columns, 1)
})

test("没有百分比才回落 已用 / 总量；都未知写 —（不写 0、不写剩余整句）", () => {
  const pair = quotaBucketView([window({ used_percent: null })], NOW)
  assert.equal(pair.buckets[0].value, "1.2M / 3.0M")
  assert.equal(pair.buckets[0].usedTotal, "1.2M / 3.0M")
  assert.equal(pair.buckets[0].percent, null)

  const empty = quotaBucketView([window({ used_percent: null, amount_text: null, remaining_text: null })], NOW)
  assert.equal(empty.buckets[0].value, "—")
  assert.equal(empty.buckets[0].usedTotal, null)
  assert.equal(empty.buckets[0].remaining, null)
  assert.equal(empty.buckets[0].percent, null)
  assert.ok(!/剩余|token/.test(empty.buckets[0].value), "主值不得是整句剩余额度")
})

test("真实 0% 是 0% 与空进度条，不是未知；未知不画进度条", () => {
  const zero = quotaBucketView([window({ used_percent: 0, amount_text: "0 / 3000000 token" })], NOW)
  assert.equal(zero.buckets[0].value, "0%")
  assert.equal(zero.buckets[0].percent, 0)
  assert.equal(zero.buckets[0].zero, true)
  assert.equal(zero.buckets[0].remaining, "1.8M")

  const unknown = quotaBucketView([window({ used_percent: null, amount_text: null })], NOW)
  assert.equal(unknown.buckets[0].percent, null)
  assert.equal(unknown.buckets[0].zero, false)
})

test("桶数 > 2 改成逐行；≤ 2 保持两列（沿用灵动岛最多两块）", () => {
  const two = quotaBucketView([
    window({ key: "a", window_name: "GLM-5.3" }),
    window({ key: "b", window_name: "GLM-5.3-Flash", used_percent: 62 }),
  ], NOW)
  assert.equal(two.columns, 2)
  assert.equal(MAX_BUCKET_COLUMNS, 2)

  const three = quotaBucketView([
    window({ key: "a" }),
    window({ key: "b", window_name: "GLM-5.3-Flash" }),
    window({ key: "c", window_name: "GLM-5.3-Air" }),
  ], NOW)
  assert.equal(three.columns, 1)
  assert.equal(three.buckets.length, 3, "逐行不丢桶")
})

test("桶顺序沿用后端顺序，不按水位重排", () => {
  const view = quotaBucketView([
    window({ key: "bkt-1", window_name: "GLM-5.3", used_percent: 40 }),
    window({ key: "bkt-2", window_name: "GLM-5.3-Flash", used_percent: 62 }),
  ], NOW)
  assert.deepEqual(view.buckets.map((bucket) => bucket.name), ["GLM-5.3", "GLM-5.3-Flash"])
  assert.deepEqual(view.buckets.map((bucket) => bucket.value), ["40%", "62%"])
})

test("区块头重置取最近一次；没有重置时间时为未知", () => {
  const view = quotaBucketView([
    window({ key: "a", resets_at: "2026-11-06T18:00:00Z" }),
    window({ key: "b", resets_at: "2026-11-06T17:12:00Z" }),
  ], NOW)
  assert.equal(view.reset, "5h 12m")
  assert.deepEqual(view.buckets.map((bucket) => bucket.reset), ["6h 0m", "5h 12m"])

  const unknown = quotaBucketView([window({ resets_at: null })], NOW)
  assert.equal(unknown.reset, null)
  assert.equal(unknown.buckets[0].reset, null)
})

test("5h / 7d 套餐窗口不进入按桶视图，其它窗口保持原样", () => {
  assert.equal(quotaBucketView([
    window({ key: "5h", window_name: "5 小时额度" }),
    window({ key: "7d", window_name: "周额度" }),
  ], NOW), null)
  assert.equal(quotaBucketView([], NOW), null)
  assert.equal(quotaBucketView(null, NOW), null)
})

test("来源已压缩的写法不二次换算；百分数收口并标告警色", () => {
  const view = quotaBucketView([
    window({ amount_text: "1.5M / 3M token", remaining_text: "剩余 1.2K token", used_percent: 120 }),
  ], NOW)
  assert.equal(view.buckets[0].usedTotal, "1.5M / 3M")
  assert.equal(view.buckets[0].remaining, "1.2K")
  assert.equal(view.buckets[0].value, "100%")
  assert.equal(view.buckets[0].tone, "danger")
})

test("桶名为空时写 —，不从 key 猜名字", () => {
  const view = quotaBucketView([window({ window_name: "  " })], NOW)
  assert.equal(view.buckets[0].name, "—")
})

/** 灵动岛拿到的 `QuotaWindow`：只有格式化好的倒计时，没有 `resets_at` */
const quotaWindow = (over = {}) => ({
  key: "bkt-1",
  windowName: "GLM-5.3",
  usedPercent: 40,
  usedText: "1200000 / 3000000 token",
  remainingText: "剩余 1800000 token",
  resetCountdown: "还剩 2h 9m",
  badgeTone: "neutral",
  ...over,
})

test("灵动岛的 QuotaWindow 走同一份口径：直接用格式化倒计时，不要求 ISO 时间", () => {
  const view = quotaBucketViewFromWindows([quotaWindow()], NOW)
  assert.ok(view)
  assert.equal(view.buckets[0].name, "GLM-5.3")
  assert.equal(view.buckets[0].value, "40%")
  assert.equal(view.buckets[0].usedTotal, "1.2M / 3.0M")
  assert.equal(view.buckets[0].remaining, "1.8M")
  assert.equal(view.buckets[0].reset, "2h 9m", "`还剩` 前缀去掉，与主窗口同一写法")
  assert.equal(view.reset, "2h 9m")
  assert.equal(view.columns, 1)

  const unknown = quotaBucketViewFromWindows([
    quotaWindow({ usedPercent: null, usedText: null, remainingText: null, resetCountdown: null }),
  ], NOW)
  assert.equal(unknown.buckets[0].value, "—")
  assert.equal(unknown.buckets[0].percent, null, "未知不画 0% 条")
  assert.equal(unknown.buckets[0].zero, false)
  assert.equal(unknown.reset, null)
  assert.equal(quotaBucketViewFromWindows([quotaWindow({ key: "5h" }), quotaWindow({ key: "7d" })], NOW), null)
  assert.equal(quotaBucketViewFromWindows([], NOW), null)
  assert.equal(quotaBucketViewFromWindows(null, NOW), null)
})
