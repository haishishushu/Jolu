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

test("快捷日期范围覆盖今天、近 N 天、本月与上月", async () => {
  const { RANGE_PRESETS } = await import("./usageStats.ts")
  const now = new Date(2026, 10, 2, 15, 30) // 2026-11-02
  const get = (key) => RANGE_PRESETS.find((p) => p.key === key).range(now)

  assert.deepEqual(get("today"), { start: "2026-11-02", end: "2026-11-02" })
  assert.deepEqual(get("yesterday-today"), { start: "2026-11-01", end: "2026-11-02" })
  assert.deepEqual(get("last-3"), { start: "2026-10-31", end: "2026-11-02" })
  assert.deepEqual(get("last-7"), { start: "2026-10-27", end: "2026-11-02" })
  assert.deepEqual(get("this-month"), { start: "2026-11-01", end: "2026-11-02" })
  assert.deepEqual(get("last-month"), { start: "2026-10-01", end: "2026-10-31" })
  assert.deepEqual(get("last-month-to-today"), { start: "2026-10-01", end: "2026-11-02" })
})

test("快捷范围的起止都不晚于今天且开始不晚于结束", async () => {
  const { RANGE_PRESETS } = await import("./usageStats.ts")
  const now = new Date(2026, 0, 15, 9, 0) // 跨年边界：2026-01-15
  for (const preset of RANGE_PRESETS) {
    const { start, end } = preset.range(now)
    assert.ok(start <= end, `${preset.key} 开始不能晚于结束：${start} > ${end}`)
    assert.ok(end <= "2026-01-15", `${preset.key} 不能选到未来：${end}`)
  }
  // 跨年时上月应为 2025-12
  assert.deepEqual(RANGE_PRESETS.find((p) => p.key === "last-month").range(now), {
    start: "2025-12-01",
    end: "2025-12-31",
  })
})

test("未计价说明区分「全部已计价」与三种未计价原因", async () => {
  const { unpricedNote } = await import("./usageStats.ts")

  // 全部已计价
  assert.equal(
    unpricedNote({ requests: 100, priced_requests: 100 }),
    "100 次全部已计价 · 不含月费",
  )

  // 模型未收录
  const noPrice = unpricedNote({
    requests: 396, priced_requests: 0,
    unpriced_no_price: ["zcode:deepseek-v4.1-flash-1017"],
  })
  assert.match(noPrice, /已计价 0 \/ 396 次/)
  assert.match(noPrice, /deepseek-v4\.1-flash-1017/)
  assert.match(noPrice, /未收录价目/)

  // 多个模型：卡片只有一行，报个数；完整清单交给悬停提示（unpricedModels）
  const many = unpricedNote({
    requests: 100, priced_requests: 10,
    unpriced_no_price: ["zcode:a", "claude:b", "codex:c"],
  })
  assert.match(many, /3 个模型未收录价目/)

  const { unpricedModels } = await import("./usageStats.ts")
  assert.deepEqual(
    unpricedModels({ unpriced_no_price: ["zcode:a", "claude:b", "codex:c"] }),
    ["a", "b", "c"],
    "悬停提示要拿到去掉 platform 前缀的完整清单",
  )
  assert.deepEqual(
    unpricedModels({ unpriced_no_price: ["zcode:<synthetic>", "zcode:real"] }),
    ["real"],
    "<synthetic> 不该出现在未收录清单",
  )

  // 失败占位 <synthetic> 不算「未收录模型」
  const synthetic = unpricedNote({
    requests: 10, priced_requests: 7,
    unpriced_no_price: ["zcode:<synthetic>"],
  })
  assert.doesNotMatch(synthetic, /未收录价目/, "<synthetic> 不该出现在未收录列表")

  // 无用量数据
  const noUsage = unpricedNote({ requests: 50, priced_requests: 40, unpriced_no_usage: 10 })
  assert.match(noUsage, /10 次无用量/)

  // 两种原因并存
  const both = unpricedNote({
    requests: 100, priced_requests: 50,
    unpriced_no_price: ["zcode:x"], unpriced_no_usage: 5,
  })
  assert.match(both, /未收录价目/)
  assert.match(both, /5 次无用量/)
})
