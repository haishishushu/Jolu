import assert from "node:assert/strict"
import test from "node:test"
import { botExpression, quotaJustReset, SLEEP_AFTER_MS } from "./botExpression.ts"

const base = { connected: true, queryFailed: false, quotaUsed: [12, 59], dragging: false, refreshing: false, working: false }
const expr = (patch) => botExpression({ ...base, ...patch }).expr

test("没有任何状态时空闲", () => assert.equal(expr({}), "idle"))

test("优先级逐级压过低级", () => {
  assert.equal(expr({ queryFailed: true, relay: "rejected", quotaUsed: [100], dragging: true, working: true }), "error")
  assert.equal(expr({ relay: "rejected", quotaUsed: [100], dragging: true }), "worry")
  assert.equal(expr({ quotaUsed: [100, 90], dragging: true, working: true }), "sad")
  assert.equal(expr({ dragging: true, quotaUsed: [85], working: true }), "drag")
  assert.equal(expr({ quotaUsed: [85], waiting: true, refreshing: true }), "worry")
  assert.equal(expr({ waiting: true, refreshing: true, working: true }), "wait")
  assert.equal(expr({ refreshing: true, relay: "in", working: true }), "refresh")
  assert.equal(expr({ relay: "in", working: true }), "relay")
  assert.equal(expr({ relay: "ok", working: true }), "happy")
  assert.equal(expr({ working: true, shot: "surprise" }), "work")
})

test("一次性表情压过睡着与打招呼，打招呼低于睡着", () => {
  assert.equal(expr({ shot: "surprise", inactiveMs: SLEEP_AFTER_MS }), "surprise")
  assert.equal(expr({ shot: "greet", inactiveMs: SLEEP_AFTER_MS }), "sleep")
  assert.equal(expr({ shot: "greet", docked: true, hover: true }), "greet")
})

test("未知额度不当作 0，也不触发告警", () => {
  assert.equal(expr({ quotaUsed: [null, null] }), "idle")
  assert.equal(expr({ quotaUsed: [null, 80] }), "worry")
})

test("未连接睡着，且不报查询失败", () => {
  assert.equal(expr({ connected: false, queryFailed: true }), "sleep")
  assert.equal(expr({ inactiveMs: SLEEP_AFTER_MS - 1 }), "idle")
  assert.equal(expr({ inactiveMs: SLEEP_AFTER_MS }), "sleep")
})

test("停靠偷看低于悬停之外的低优先级，高于看向指针", () => {
  assert.equal(expr({ docked: true, hover: true }), "peek")
  assert.equal(expr({ hover: true }), "look")
})

test("额度重置只在高位回到低位时判定", () => {
  assert.equal(quotaJustReset({ "5h": 97 }, { "5h": 3 }), true)
  assert.equal(quotaJustReset({ "5h": 60 }, { "5h": 55 }), false)
  assert.equal(quotaJustReset({ "5h": null }, { "5h": 2 }), false)
  assert.equal(quotaJustReset({}, { "5h": 2 }), false)
})
