import test from "node:test"
import assert from "node:assert/strict"
import { UNKNOWN_REMAINING, remainingDisplay } from "./remainingDisplay.ts"

test("Grok 将真实已用百分比换算为剩余，保留重置时间", () => {
  const view = remainingDisplay("grok", "auth", {
    state: "ok", plan: null, windows: [{ key: "7d", window_name: "周额度", used_percent: 37.5, amount_text: null, resets_at: "2026-09-30T00:00:00Z" }],
  })
  assert.equal(view.rows[0].value, "62.5%")
  assert.equal(view.rows[0].label, "SuperGrok 周剩余")
  assert.equal(view.rows[0].resetsAt, "2026-09-30T00:00:00Z")
})

test("Grok 缺失百分比不会变成 100% 剩余", () => {
  const view = remainingDisplay("grok", "auth", {
    state: "ok", plan: null, windows: [{ key: "credits", window_name: "积分", used_percent: null, amount_text: null, resets_at: null }],
  })
  assert.equal(view.rows[0].value, "—")
  assert.match(view.reason, /未提供/)
})

test("其他平台缺少远程授权时不把本机消耗冒充余额", () => {
  for (const platform of ["gemini", "trae", "qoder", "workbuddy"]) {
    const view = remainingDisplay(platform, "auth", null)
    assert.equal(view.rows.length, 0)
    assert.equal(view.state, "unsupported")
    assert.ok(view.reason.length > 0)
  }
})

test("Grok API Key 与 SuperGrok 订阅额度严格分开", () => {
  const view = remainingDisplay("grok", "api", null)
  assert.equal(view.rows.length, 0)
  assert.match(view.reason, /Management Key/)
})

test("ZCode 显示 BigModel API Key 返回的精确剩余积分", () => {
  const view = remainingDisplay("zcode", "auth", {
    state: "ok", plan: "pro", windows: [
      { key: "5h", window_name: "5 小时额度", used_percent: 25, amount_text: "3000 / 12000 credits", remaining_text: "8999 Credits", resets_at: null },
      { key: "7d", window_name: "周额度", used_percent: 0, amount_text: "0 / 60000 credits", remaining_text: "60000 Credits", resets_at: null },
    ],
  })
  assert.equal(view.rows[0].value, "8999 Credits")
  assert.equal(view.rows[1].value, "60000 Credits")
  assert.match(view.source, /API Key/)
})

test("ZCode 未知额度显示 -，不冒充 0 也不算查询失败", () => {
  // 后端 login_state 兜底：窗口在，三个数值字段都为 None
  const view = remainingDisplay("zcode", "auth", {
    state: "ok", plan: null, windows: [
      { key: "quota", window_name: "套餐额度", used_percent: null, amount_text: null, remaining_text: null, resets_at: null },
    ],
  })
  assert.equal(view.rows.length, 1)
  assert.equal(view.rows[0].value, UNKNOWN_REMAINING)
  assert.equal(view.rows[0].value, "-")
  assert.notEqual(view.rows[0].value, "0", "未知不等于用尽")
  assert.equal(view.rows[0].usedText, null)
  assert.equal(view.state, "ok", "查询成功但数值未知，不降级成不支持")
})

test("ZCode 真实 0 剩余仍显示 0，不被改写成 -", () => {
  const exact = remainingDisplay("zcode", "auth", {
    state: "ok", plan: null, windows: [
      { key: "quota", window_name: "套餐额度", used_percent: 100, amount_text: "3M / 3M token", remaining_text: "0", resets_at: null },
    ],
  })
  assert.equal(exact.rows[0].value, "0")
  assert.equal(exact.state, "ok")
  // 只给已用百分比时换算出的也是 0%，不是未知占位符
  const computed = remainingDisplay("zcode", "auth", {
    state: "ok", plan: null, windows: [
      { key: "quota", window_name: "套餐额度", used_percent: 100, amount_text: null, remaining_text: null, resets_at: null },
    ],
  })
  assert.equal(computed.rows[0].value, "0%")
})

test("ZCode 查询失败 / 未授权 / 需要重新登录保持各自表达，不变成 -", () => {
  for (const state of ["failed", "unauthorized", "forbidden", "rate_limited"]) {
    const view = remainingDisplay("zcode", "auth", { state, plan: null, windows: [], reason: "查询未完成" })
    assert.equal(view.state, state)
    assert.equal(view.rows.length, 0, `${state} 不伪造剩余行`)
    assert.equal(view.reason, "查询未完成")
  }
  // 一条窗口都没有才是「不支持」，同样不是 -
  const empty = remainingDisplay("zcode", "auth", { state: "ok", plan: null, windows: [] })
  assert.equal(empty.state, "unsupported")
  assert.equal(empty.rows.length, 0)
})
