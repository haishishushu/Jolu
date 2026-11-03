import assert from "node:assert/strict"
import test from "node:test"

import { convertFromUsd } from "./usageStats.ts"
import { formatMoney } from "./usageStats.ts"

const usd = { currency: "USD", rate: 1, asOf: "基准币种", fallback: false, loading: false }
const cny = { currency: "CNY", rate: 7.2, asOf: "2026-11-03", fallback: false, loading: false }
const cnyFallback = { currency: "CNY", rate: 7.2, asOf: "内置参考价", fallback: true, loading: false }

test("USD 为基准币种：换算不改变数值", () => {
  assert.equal(convertFromUsd(10, usd), 10)
  assert.equal(convertFromUsd(0, usd), 0)
  assert.equal(convertFromUsd(null, usd), null)
  assert.equal(convertFromUsd(undefined, usd), null)
})

test("CNY 按汇率换算，保留 null 语义", () => {
  assert.equal(convertFromUsd(10, cny), 72)
  assert.equal(convertFromUsd(0, cny), 0)
  assert.equal(convertFromUsd(null, cny), null, "未知费用不能变成 0")
})

test("兜底汇率与在线汇率换算结果一致，但状态可区分", () => {
  assert.equal(convertFromUsd(1, cny), convertFromUsd(1, cnyFallback))
  assert.notEqual(cny.fallback, cnyFallback.fallback)
})

test("formatMoney 按币种选择符号与精度", () => {
  assert.equal(formatMoney(72, "CNY"), "¥72.00")
  assert.equal(formatMoney(0.026 * 7.2, "CNY"), "¥0.187", "小于 1 保留三位小数")
  assert.equal(formatMoney(0, "CNY"), "¥0.00")
  assert.equal(formatMoney(null, "CNY"), "—")
  // USD 保持原有格式
  assert.equal(formatMoney(1.5, "USD"), "$1.50")
  assert.equal(formatMoney(0.026, "USD"), "$0.026")
})

test("大额与小额的显示边界", () => {
  // ≥1 两位小数，<1 三位小数
  assert.equal(formatMoney(1, "CNY"), "¥1.00")
  assert.equal(formatMoney(0.999, "CNY"), "¥0.999")
  assert.equal(formatMoney(12345.6789, "CNY"), "¥12345.68")
})
