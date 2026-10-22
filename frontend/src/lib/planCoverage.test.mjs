import test from "node:test"
import assert from "node:assert/strict"
import { isPlanCovered } from "./planCoverage.ts"

const w = (key, windowName, amount_text = null, used_percent = null) =>
  ({ key, window_name: windowName, amount_text, used_percent, resets_at: null })

test("查到 5 小时或周额度窗口的就是套餐", () => {
  assert.equal(isPlanCovered([w("5h", "5 小时额度")]), true)
  assert.equal(isPlanCovered([w("7d", "周额度")]), true)
  assert.equal(isPlanCovered([w("5h", "5 小时额度"), w("7d", "周额度")]), true)
})

test("没有额度窗口的按 API Key 处理，照常显示估算金额", () => {
  assert.equal(isPlanCovered([]), false)
  assert.equal(isPlanCovered(null), false)
  assert.equal(isPlanCovered(undefined), false)
})

test("只有月额度或积分额度不算 5 小时 / 周套餐", () => {
  // 额度查询成功但窗口形态不同，不能一律当成套餐
  assert.equal(isPlanCovered([w("30d", "月额度")]), false)
  assert.equal(isPlanCovered([w("credits", "Grok 积分额度")]), false)
})
