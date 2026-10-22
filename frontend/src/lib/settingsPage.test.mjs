import assert from "node:assert/strict"
import test from "node:test"
import {
  SETTINGS_TABS, cleanupCutoffDate, formatBytes, formatCount, parseThreshold, portError, saveState, tabForSection,
} from "./settingsPage.ts"

test("分类顺序与画板 86 一致", () => {
  assert.deepEqual(SETTINGS_TABS.map((t) => t.label), ["常规", "外观", "网络与代理", "数据与存储", "通知", "关于与更新"])
})

test("旧分区 id 映射到新分类，认不出的回到常规", () => {
  assert.equal(tabForSection("proxy"), "network")
  assert.equal(tabForSection("plan"), "general")
  assert.equal(tabForSection("data"), "data")
  assert.equal(tabForSection("appearance"), "appearance")
  assert.equal(tabForSection("general"), "general")
  assert.equal(tabForSection("whatever"), "general")
  assert.equal(tabForSection(null), "general")
})

test("容量与计数区分未知值和真实的零", () => {
  assert.equal(formatBytes(null), null)
  assert.equal(formatBytes(0), "0.0 KB")
  assert.equal(formatBytes(28.6 * 1024 * 1024), "28.6 MB")
  assert.equal(formatCount(undefined), null)
  assert.equal(formatCount(0), "0")
  assert.equal(formatCount(12846), "12,846")
})

test("清理截止日期按本地日期往前推", () => {
  assert.equal(cleanupCutoffDate(90, new Date(2026, 9, 21, 23, 59)), "2026-07-23")
  assert.equal(cleanupCutoffDate(30, new Date(2026, 0, 15)), "2025-12-16")
  assert.equal(cleanupCutoffDate(90, new Date(2026, 9, 21), new Date(2026, 6, 1, 8).getTime()), "2026-07-01")
})

test("端口只接受 1024–65535 的整数", () => {
  assert.equal(portError("12731"), null)
  assert.equal(portError("70000"), "请输入 1024–65535 的整数")
  assert.equal(portError("80"), "请输入 1024–65535 的整数")
  assert.equal(portError("12a"), "请输入 1024–65535 的整数")
  assert.equal(portError(""), "请输入 1024–65535 的整数")
})

test("余额阈值必须大于 0 且不超过 10 亿", () => {
  assert.equal(parseThreshold("5.00"), 5)
  assert.equal(parseThreshold("0"), null)
  assert.equal(parseThreshold("-1"), null)
  assert.equal(parseThreshold("abc"), null)
  assert.equal(parseThreshold("2000000000"), null)
})

test("页头保存状态：失败 > 保存中 > 未保存 > 已保存", () => {
  assert.equal(saveState({ saving: true, error: "x", dirty: true }), "failed")
  assert.equal(saveState({ saving: true, error: null, dirty: true }), "saving")
  assert.equal(saveState({ saving: false, error: null, dirty: true }), "dirty")
  assert.equal(saveState({ saving: false, error: null, dirty: false }), "saved")
})
