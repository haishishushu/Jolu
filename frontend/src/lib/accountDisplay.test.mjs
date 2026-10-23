import assert from "node:assert/strict"
import test from "node:test"
import { filterAccounts, formatMoney, formatResetAt, formatSyncedAt, quotaTitle, redactAccountMessage } from "./accountDisplay.ts"

test("额度窗口标题", () => {
  assert.equal(quotaTitle("5h", "5 小时窗口"), "5 小时额度")
  assert.equal(quotaTitle("7d", "7 天窗口"), "周额度")
  assert.equal(quotaTitle("1d", "每日额度"), "每日额度")
})

test("重置时间按当天 / 其他日期显示，缺失不推测", () => {
  const now = new Date(2026, 9, 22, 10, 0)
  assert.equal(formatResetAt(new Date(2026, 9, 22, 18, 30).toISOString(), now), "今天 18:30 重置")
  assert.equal(formatResetAt(new Date(2026, 9, 26, 9, 0).toISOString(), now), "10/26 09:00 重置")
  assert.equal(formatResetAt(null, now), null)
  assert.equal(formatResetAt("not-a-date", now), null)
})

test("同步时间", () => {
  const now = new Date(2026, 9, 22, 10, 30)
  assert.equal(formatSyncedAt(null, now), "尚未同步")
  assert.equal(formatSyncedAt(new Date(2026, 9, 22, 10, 29, 40), now, "余额"), "余额刚刚同步")
  assert.equal(formatSyncedAt(new Date(2026, 9, 22, 10, 20), now), "10 分钟前同步")
  assert.equal(formatSyncedAt(new Date(2026, 9, 22, 8, 5), now), "08:05 同步")
})

test("金额带两位小数，美元加 $", () => {
  assert.equal(formatMoney(128.5, "USD"), "$128.50")
  assert.equal(formatMoney(0, "CNY"), "0.00")
})

test("搜索与供应商筛选共同生效", () => {
  const accounts = [
    { platformId: "claude", baseName: "Momo", name: "Momo", label: "Max" },
    { platformId: "codex", baseName: "团队 API", name: "团队 API · sk-…7k2a", label: "API Key" },
  ]
  assert.equal(filterAccounts(accounts, null, "").length, 2)
  assert.deepEqual(filterAccounts(accounts, "codex", "").map((a) => a.baseName), ["团队 API"])
  assert.deepEqual(filterAccounts(accounts, null, "7K2A").map((a) => a.baseName), ["团队 API"])
  assert.equal(filterAccounts(accounts, "claude", "团队").length, 0)
})

test("邮箱搜索独立于自定义连接名，错误脱敏保留诊断与金额", () => {
  const accounts = [{ platformId: "codex", baseName: "工作账号", name: "工作账号", label: "Pro", accountEmail: "person@example.test" }]
  assert.equal(filterAccounts(accounts, null, "PERSON@EXAMPLE").length, 1)
  assert.equal(filterAccounts(accounts, "claude", "person").length, 0)
  assert.equal(redactAccountMessage("person@example.test 请求 https://private.example.test/v1 失败，余额 $128.50"), "[账号已隐藏] 请求 [地址已隐藏] 失败，余额 $128.50")
})
