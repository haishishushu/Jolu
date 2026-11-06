import assert from "node:assert/strict"
import test from "node:test"
import {
  carriedDeltaCountKey,
  carriedDeltaSegment,
  carriedDeltaView,
  latchSessionDelta,
  todayReset,
} from "./carriedDelta.ts"

const latch = (previous, working, runningDelta, todayTokens) =>
  latchSessionDelta({ previous, working, runningDelta, todayTokens })

test("锁存：进行中记下本轮最新读数，数值变大跟着涨", () => {
  assert.equal(latch(null, true, 12_000, 2_300_000), 12_000)
  assert.equal(latch(12_000, true, 30_200, 2_300_000), 30_200)
})

test("锁存：新一轮数值变小也覆盖，不做只涨不跌的取最大", () => {
  assert.equal(latch(30_200, true, 4_800, 2_330_000), 4_800)
})

test("锁存：本轮刚起跑（0 / 未知）保持上一次读数，不被 0 抹掉", () => {
  assert.equal(latch(30_200, true, 0, 2_330_000), 30_200)
  assert.equal(latch(30_200, true, null, 2_330_000), 30_200)
})

test("锁存：会话结束后停在最后一个锁存值上，不再变化", () => {
  assert.equal(latch(30_200, false, null, 2_330_000), 30_200)
  // 后续心跳推送（今日累计继续涨）也不改锁存值
  assert.equal(latch(30_200, false, null, 2_400_000), 30_200)
})

test("清空：跨天今日累计归 0 且没有本轮值时清掉昨天的终值", () => {
  assert.equal(latch(30_200, false, null, 0), null)
  assert.equal(latch(30_200, false, null, null), null)
})

test("清空：本轮仍在写时跨过午夜不算「昨天的终值」，保持本轮读数", () => {
  assert.equal(latch(30_200, true, 31_000, 0), 31_000)
})

test("清空：手动刷新后没有本轮值，回到今日累计兜底（锁存已置空）", () => {
  assert.equal(latch(null, false, null, 2_330_000), null)
  assert.deepEqual(carriedDeltaView({ latched: null, todayTokens: 2_330_000, sourceFailed: false }),
    { value: 2_330_000, source: "today" })
})

test("今日累计归 0 / 未知的判定", () => {
  assert.equal(todayReset(0), true)
  assert.equal(todayReset(null), true)
  assert.equal(todayReset(1), false)
})

test("显示优先级：锁存值 ?? 今日累计", () => {
  assert.deepEqual(carriedDeltaView({ latched: 30_200, todayTokens: 2_300_000, sourceFailed: false }),
    { value: 30_200, source: "session" })
  assert.deepEqual(carriedDeltaView({ latched: null, todayTokens: 2_300_000, sourceFailed: false }),
    { value: 2_300_000, source: "today" })
})

test("显示：两者都为 0 / 未知时整段不渲染（value 为 null）", () => {
  assert.deepEqual(carriedDeltaView({ latched: null, todayTokens: 0, sourceFailed: false }),
    { value: null, source: null })
  assert.deepEqual(carriedDeltaView({ latched: null, todayTokens: null, sourceFailed: false }),
    { value: null, source: null })
})

test("显示：采集源出错时状态不可判定，拿不到可显示的增量", () => {
  assert.deepEqual(carriedDeltaView({ latched: 30_200, todayTokens: 2_300_000, sourceFailed: true }),
    { value: null, source: null })
})

test("追数键：进行中才订阅，结束后静态读数不跑逐帧", () => {
  assert.equal(carriedDeltaCountKey("session", true), "total")
  assert.equal(carriedDeltaCountKey("today", true), "today")
  assert.equal(carriedDeltaCountKey("session", false), undefined)
  assert.equal(carriedDeltaCountKey("today", false), undefined)
  assert.equal(carriedDeltaCountKey(null, true), undefined)
})

/* ---------- §5.8：段是否渲染（锁存 → 今日累计 → 写入中未知 → 不渲染） ---------- */

const segment = (overrides = {}) => carriedDeltaSegment({
  value: null, source: null, text: null, writing: false, unavailable: false, relay: false, ...overrides,
})

test("段：有锁存值就显示锁存值，来源标记为 session", () => {
  assert.deepEqual(segment({ value: 30_200, source: "session", writing: true }),
    { visible: true, value: 30_200, source: "session", unknown: false })
})

test("段：没有锁存时显示今日累计，来源标记为 today", () => {
  assert.deepEqual(segment({ value: 2_300_000, source: "today" }),
    { visible: true, value: 2_300_000, source: "today", unknown: false })
})

test("段：两个来源都没有但仍在写入时照常渲染，数值标成未知（显示 +—）", () => {
  assert.deepEqual(segment({ writing: true }),
    { visible: true, value: null, source: null, unknown: true })
})

test("段：两个来源都没有且不在写入时整段不渲染（静止态不凭空出现 +—）", () => {
  assert.deepEqual(segment({ writing: false }),
    { visible: false, value: null, source: null, unknown: false })
})

test("段：数值为 0 与未知同处理——写入中显示 +—，静止时隐藏", () => {
  assert.deepEqual(segment({ value: 0, writing: true }),
    { visible: true, value: null, source: null, unknown: true })
  assert.equal(segment({ value: 0 }).visible, false)
})

test("段：浏览器预览的示例文案优先于未知分支", () => {
  assert.deepEqual(segment({ text: "+12.8K", writing: true }),
    { visible: true, value: null, source: null, unknown: false })
})

test("段：连接不可用时不渲染，即使在写入", () => {
  assert.equal(segment({ writing: true, unavailable: true }).visible, false)
})

test("段：被网关转发路线占位时不渲染，即使在写入", () => {
  assert.equal(segment({ writing: true, relay: true }).visible, false)
})

