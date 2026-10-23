import assert from "node:assert/strict"
import test from "node:test"
import { accountQuotaWindows, subscriptionDisplay, resetCreditsText, filterAccounts, formatMoney, formatResetAt, formatSyncedAt, quotaTitle, redactAccountMessage } from "./accountDisplay.ts"

test("订阅本机快照不能冒充实时状态，缺失和无效到期日不猜测", () => {
  assert.equal(subscriptionDisplay(null).status, "未提供")
  const snapshot = { subscription_status: "active", subscription_source: "local_snapshot", expires_at: "2026-10-25T10:00:00Z", observed_at: null, reset_credits: null }
  assert.equal(subscriptionDisplay(snapshot).status, "待核实 · 本机记录")
  assert.match(subscriptionDisplay(snapshot).expires, /2026-10-25/)
  assert.match(subscriptionDisplay(snapshot).note, /官网/)
  assert.equal(subscriptionDisplay({ ...snapshot, expires_at: "invalid" }).expires, "未提供")
  assert.equal(subscriptionDisplay({ ...snapshot, subscription_source: "live" }).status, "有效")
  assert.equal(subscriptionDisplay({ ...snapshot, subscription_source: "live", subscription_status: "expired" }).status, "已到期")
})

test("重置卡区分零、未知与适用次数，不推断可以使用", () => {
  assert.equal(resetCreditsText(null), "未提供")
  assert.equal(resetCreditsText({ available_count: 1, applicable_available_count: 0 }), "剩余 1 次 · 当前可用 0 次")
  assert.equal(resetCreditsText({ available_count: 0, applicable_available_count: null }), "剩余 0 次 · 适用次数未提供")
  assert.equal(resetCreditsText({ available_count: null, applicable_available_count: 1 }), "总次数未提供 · 当前可用 1 次")
})

test("订阅卡片固定展示五小时和周额度，缺失不冒充满额且额外窗口不丢失", () => {
  const weekly = { key: "7d", window_name: "weekly", used_percent: 0, resets_at: null, amount_text: null }
  const model = { ...weekly, key: "7d-sonnet", window_name: "Sonnet", used_percent: 42 }
  const rows = accountQuotaWindows("codex", [model, weekly])
  assert.deepEqual(rows.map(row => row.key), ["5h", "7d", "7d-sonnet"])
  assert.equal(rows[0].used_percent, null)
  assert.equal(rows[1].used_percent, 0)
  assert.equal(rows[2].used_percent, 42)
  assert.equal(rows[0].resets_at, null)
  assert.deepEqual(accountQuotaWindows("claude", []).map(row => row.key), ["5h", "7d"])
  assert.deepEqual(accountQuotaWindows("zcode", [model]), [model])
})

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
