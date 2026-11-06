import test from "node:test"
import assert from "node:assert/strict"
import { islandBucketQuotaView } from "./islandQuotaBuckets.ts"

const NOW = Date.parse("2026-11-06T12:00:00Z")

/** 灵动岛里的 `QuotaWindow`（由 `toQuotaWindows` 从后端 DTO 转出来） */
const window = (over = {}) => ({
  key: "bkt-1",
  windowName: "GLM-5.3",
  usedPercent: 40,
  usedText: "1200000 / 3000000 token",
  remainingText: "剩余 1800000 token",
  resetCountdown: "还剩 2h 9m",
  badgeTone: "neutral",
  ...over,
})

const island = (over = {}) => ({
  platform: "zcode",
  kind: "auth",
  windows: [window()],
  queryFailed: false,
  unavailable: false,
  ...over,
})

test("ZCode 本机登录的纯日额度桶：走桶卡，百分比 / 已用总量 / 剩余 / 重置都齐全", () => {
  const view = islandBucketQuotaView(island(), NOW)
  assert.ok(view)
  assert.equal(view.buckets.length, 1)
  assert.equal(view.buckets[0].name, "GLM-5.3")
  assert.equal(view.buckets[0].value, "40%")
  assert.equal(view.buckets[0].usedTotal, "1.2M / 3.0M")
  assert.equal(view.buckets[0].remaining, "1.8M")
  assert.equal(view.reset, "2h 9m", "区块头右侧写 `2h 9m 后重置`")
})

test("两个桶的视图本身是两列；岛侧组件仍按 380 宽固定逐行（columns=1）", () => {
  const view = islandBucketQuotaView(island({
    windows: [window({ key: "bkt-1" }), window({ key: "bkt-2", windowName: "GLM-5.3-Flash", usedPercent: 62 })],
  }), NOW)
  assert.equal(view.columns, 2)
  assert.deepEqual(view.buckets.map((bucket) => bucket.name), ["GLM-5.3", "GLM-5.3-Flash"])
})

test("真实 0% 照画 0%；未知不画条、不写剩余 0", () => {
  const zero = islandBucketQuotaView(island({ windows: [window({ usedPercent: 0, usedText: "0 / 3000000 token" })] }), NOW)
  assert.equal(zero.buckets[0].value, "0%")
  assert.equal(zero.buckets[0].percent, 0)
  assert.equal(zero.buckets[0].zero, true)

  // 登录态兜底窗口（后端 `login_state`：key=quota、三个数值字段全 None）
  const unknown = islandBucketQuotaView(island({
    windows: [window({ key: "quota", windowName: "套餐额度", usedPercent: null, usedText: null, remainingText: null, resetCountdown: null })],
  }), NOW)
  assert.ok(unknown, "未知额度仍然是一张桶卡，只是不画条")
  assert.equal(unknown.buckets[0].value, "—")
  assert.equal(unknown.buckets[0].usedTotal, null)
  assert.equal(unknown.buckets[0].remaining, null, "上屏写 `剩 —`")
  assert.equal(unknown.buckets[0].percent, null)
  assert.equal(unknown.buckets[0].zero, false)
  assert.equal(unknown.reset, null, "区块头写 `—`，不拼「后重置」")
})

test("5h / 7d 套餐窗口保留旧整句：只有套餐窗口、或混有套餐窗口都不走桶卡", () => {
  assert.equal(islandBucketQuotaView(island({
    windows: [window({ key: "5h", windowName: "5 小时额度" }), window({ key: "7d", windowName: "周额度" })],
  }), NOW), null, "纯套餐窗口")
  assert.equal(islandBucketQuotaView(island({
    windows: [window({ key: "5h", windowName: "5 小时额度" }), window({ key: "bkt-1" })],
  }), NOW), null, "混有套餐窗口：桶卡会吞掉套餐行")
})

test("其它平台的按桶列表不走桶卡（Claude / Codex / Grok 保持原样）", () => {
  for (const platform of ["claude", "codex", "grok", "gemini"]) {
    assert.equal(islandBucketQuotaView(island({ platform }), NOW), null, platform)
  }
})

test("API Key 计量 / 无窗口 / 失败 / 需重登 / 未连接：一律保留整句", () => {
  assert.equal(islandBucketQuotaView(island({ kind: "api" }), NOW), null, "API Key 计量")
  assert.equal(islandBucketQuotaView(island({ windows: [] }), NOW), null, "无窗口")
  assert.equal(islandBucketQuotaView(island({ windows: null }), NOW), null, "窗口未定义")
  assert.equal(islandBucketQuotaView(island({ windows: undefined }), NOW), null, "窗口缺省")
  assert.equal(islandBucketQuotaView(island({ queryFailed: true }), NOW), null, "查询失败 / 401")
  assert.equal(islandBucketQuotaView(island({ unavailable: true }), NOW), null, "未连接 / 连接过期")
})
