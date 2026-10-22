import assert from "node:assert/strict"
import test from "node:test"
import {
  activityLevel, calendarWeeks, callStatus, canConfirmRange, changeText, effortLabel, formatTokens, formatUsd,
  modelDisplayName, monthGrid, monthLabels, outputTps, pageWindow, pickDate, rangeDays, rangeLabel, rangeToMillis, zoneName,
} from "./usageStats.ts"

const now = new Date(2026, 9, 21, 12, 38)

test("日期范围文案与天数", () => {
  assert.equal(rangeLabel({ start: "2026-10-21", end: "2026-10-21" }, now), "今天")
  assert.equal(rangeLabel({ start: "2026-10-15", end: "2026-10-15" }, now), "10/15")
  assert.equal(rangeLabel({ start: "2026-10-15", end: "2026-10-21" }, now), "10/15—10/21")
  assert.equal(rangeDays({ start: "2026-10-15", end: "2026-10-21" }), 7)
})

test("结束日是今天时只统计到当前时刻", () => {
  const { start, end } = rangeToMillis({ start: "2026-10-21", end: "2026-10-21" }, now)
  assert.equal(start, new Date(2026, 9, 21).getTime())
  assert.equal(end, now.getTime())
  const past = rangeToMillis({ start: "2026-10-01", end: "2026-10-02" }, now)
  assert.equal(past.end, new Date(2026, 9, 3).getTime())
})

test("草稿点选与确认校验", () => {
  let draft = pickDate({ start: null, end: null }, "2026-10-15")
  assert.deepEqual(draft, { start: "2026-10-15", end: null })
  assert.equal(canConfirmRange(draft.start, draft.end), false)
  draft = pickDate(draft, "2026-10-21")
  assert.deepEqual(draft, { start: "2026-10-15", end: "2026-10-21" })
  assert.equal(canConfirmRange("2026-10-15", "2026-10-15"), true, "允许同一天")
  assert.equal(canConfirmRange("2026-10-22", "2026-10-21"), false)
  assert.deepEqual(pickDate(draft, "2026-10-01"), { start: "2026-10-01", end: null }, "完整范围后重新开始")
})

test("月历周一开头", () => {
  const weeks = monthGrid(2026, 9)
  assert.deepEqual(weeks[0].slice(0, 4), [null, null, null, "2026-10-01"])
  assert.equal(weeks.every((w) => w.length === 7), true)
})

test("数值格式与缺失值", () => {
  assert.equal(formatTokens(12_480_000), "12.48 M")
  assert.equal(formatTokens(8_400), "8.4 K")
  assert.equal(formatTokens(846), "846")
  assert.equal(formatTokens(null), "—")
  assert.equal(formatUsd(18.42), "$18.42")
  assert.equal(formatUsd(0.026), "$0.026")
  assert.equal(formatUsd(null), "—")
  assert.equal(changeText(118.6, 100), "+18.6%")
  assert.equal(changeText(100, 0), null)
})

test("调用状态、思考强度与 TPS", () => {
  assert.deepEqual(callStatus({ failed: false, status_code: null }), { label: "成功", ok: true })
  assert.deepEqual(callStatus({ failed: true, status_code: 429 }), { label: "429 限流", ok: false })
  assert.deepEqual(callStatus({ failed: true, status_code: 504 }), { label: "超时", ok: false })
  assert.equal(effortLabel("high"), "高")
  assert.equal(effortLabel(null), "—")
  assert.equal(outputTps({ output: 100, duration_ms: 3_000, first_token_ms: 500 }), 40)
  assert.equal(outputTps({ output: 100, duration_ms: null, first_token_ms: 500 }), null)
})

test("模型展示名", () => {
  assert.equal(modelDisplayName("claude-sonnet-4-5-20250929"), "Claude Sonnet 4.5")
  assert.equal(modelDisplayName("gpt-5-mini"), "GPT-5 mini")
  assert.equal(modelDisplayName("gemini-2.5-flash"), "Gemini 2.5 Flash")
  assert.equal(modelDisplayName("grok-4"), "Grok 4")
  assert.equal(modelDisplayName(null), "未知模型")
})

test("分页页码折叠", () => {
  assert.deepEqual(pageWindow(1, 85), [1, 2, 3, 4, 5, "…", 85])
  assert.deepEqual(pageWindow(40, 85), [1, "…", 39, 40, 41, "…", 85])
  assert.deepEqual(pageWindow(2, 3), [1, 2, 3])
})

test("全年日历按周分列，月份标签落在第一个周一所在列", () => {
  const weeks = calendarWeeks("2025-10-20", "2026-10-21")
  assert.equal(weeks[0][0], "2025-10-20")
  assert.equal(weeks.at(-1).includes("2026-10-21"), true)
  assert.equal(weeks.at(-1).at(-1), null)
  const labels = monthLabels(weeks)
  assert.equal(labels[0].label, "10月")
  assert.deepEqual(labels.slice(0, 2), [{ column: 0, label: "10月" }, { column: 2, label: "11月" }])
})

test("活跃等级按四分位", () => {
  const sorted = [1, 2, 3, 4, 5, 6, 7, 8]
  assert.equal(activityLevel(0, sorted), 0)
  assert.equal(activityLevel(1, sorted), 1)
  assert.equal(activityLevel(8, sorted), 4)
})

test("时区名", () => {
  assert.equal(zoneName("Asia/Shanghai"), "上海")
  assert.equal(zoneName("Europe/Paris"), "Europe/Paris")
})
