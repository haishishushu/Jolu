import assert from "node:assert/strict"
import test from "node:test"
import { accountQuotaWindows, subscriptionDisplay, resetCreditsText, filterAccounts, formatMoney, formatResetAt, formatSyncedAt, quotaTitle, redactAccountMessage } from "./accountDisplay.ts"
import { quotaValue, accountQueries } from "./accountDisplay.ts"

test("账号查询包括所有平台及自定义服务，暂停账号不联网", () => {
  const api = { id: "z", platformId: "zcode", kind: "api", baseUrl: "https://open.bigmodel.cn/api/coding/paas/v4", status: "connected" }
  assert.deepEqual(accountQueries(api), { quota: "z", balance: "z" })
  assert.deepEqual(accountQueries({ ...api, kind: "auth", baseUrl: null }), { quota: "z", balance: null })
  assert.deepEqual(accountQueries({ ...api, platformId: "custom" }), { quota: "z", balance: "z" })
  assert.deepEqual(accountQueries({ ...api, platformId: "custom", status: "paused" }), { quota: null, balance: null })
  assert.deepEqual(accountQueries({ ...api, status: "paused" }), { quota: null, balance: null })
  assert.deepEqual(accountQueries({ ...api, baseUrl: null }), { quota: "z", balance: null })
})

test("额度优先显示真实剩余次数，百分比非法时不显示满额", () => {
  assert.equal(quotaValue({ remaining_text: "剩余 0 次", used_percent: null }), "剩余 0 次")
  assert.equal(quotaValue({ remaining_text: null, used_percent: 0 }), "剩余 100%")
  for (const used_percent of [null, undefined, NaN, Infinity, -1, 101]) {
    assert.equal(quotaValue({ used_percent }), "未提供")
  }
})

test("订阅到期日期区分本机记录，缺失和无效日期不猜测", () => {
  assert.equal(subscriptionDisplay(null).expires, "未提供")
  const snapshot = { subscription_status: "active", subscription_source: "local_snapshot", expires_at: "2026-10-25T10:00:00Z", observed_at: null, reset_credits: null }
  assert.match(subscriptionDisplay(snapshot).expires, /2026-10-25/)
  assert.match(subscriptionDisplay(snapshot).note, /官网/)
  assert.equal(subscriptionDisplay({ ...snapshot, expires_at: "invalid" }).expires, "未提供")
  assert.equal(subscriptionDisplay({ ...snapshot, subscription_source: "live" }).note, null)
})

test("重置卡主卡显示总可用数，当前适用数不能覆盖它，零和未知分别保留", () => {
  assert.equal(resetCreditsText(null), "未提供")
  assert.equal(resetCreditsText({ available_count: 1, applicable_available_count: 0 }, true), "可用 1 次")
  assert.equal(resetCreditsText({ available_count: 1, applicable_available_count: 0 }), "可用 1 次 · 当前适用 0 次")
  assert.equal(resetCreditsText({ available_count: 0, applicable_available_count: null }, true), "可用 0 次")
  assert.equal(resetCreditsText({ available_count: 0, applicable_available_count: null }), "可用 0 次 · 适用次数未提供")
  assert.equal(resetCreditsText({ available_count: null, applicable_available_count: 1 }, true), "可用次数未提供")
  assert.equal(resetCreditsText({ available_count: null, applicable_available_count: 1 }), "可用次数未提供 · 当前适用 1 次")
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

test("账号类型按真实 kind 筛选，与供应商、搜索取交集", () => {
  const accounts=[
    {platformId:"claude",kind:"auth",baseName:"官方订阅",name:"官方订阅",label:"Pro"},
    {platformId:"claude",kind:"api",baseName:"中转服务",name:"中转服务",label:"API Key"},
    {platformId:"custom",kind:"api",baseName:"自定义服务",name:"自定义服务",label:"API Key"},
  ]
  assert.deepEqual(filterAccounts(accounts,null,"","auth").map(a=>a.baseName),["官方订阅"])
  assert.deepEqual(filterAccounts(accounts,null,"","api").map(a=>a.baseName),["中转服务","自定义服务"])
  assert.deepEqual(filterAccounts(accounts,"claude","服务","api").map(a=>a.baseName),["中转服务"])
  assert.equal(filterAccounts(accounts,"claude","服务","auth").length,0)
  assert.equal(filterAccounts(accounts,null,"",null).length,3)
})

test("账号六版本分别筛选，区域未知账号只留在全部供应商", () => {
  const accounts = [
    ["qoder", "cn"], ["qoder", "global"], ["workbuddy", "cn"],
    ["workbuddy", "ai"], ["trae", "cn"], ["trae", "global"], ["qoder", null],
  ].map(([platformId, edition], index) => ({ id: `browser:${index}`, platformId, edition, baseName: "账号", name: "账号", label: "订阅" }))
  const providers = ["qoder-cn", "qoder", "workbuddy", "workbuddy-ai", "trae-cn", "trae"]
  for (const [index, provider] of providers.entries()) {
    assert.deepEqual(filterAccounts(accounts, provider, "账号").map(a => a.id), [`browser:${index}`])
  }
  assert.equal(filterAccounts(accounts, null, "").length, 7)
})

test("邮箱搜索独立于自定义连接名，错误脱敏保留诊断与金额", () => {
  const accounts = [{ platformId: "codex", baseName: "工作账号", name: "工作账号", label: "Pro", accountEmail: "person@example.test" }]
  assert.equal(filterAccounts(accounts, null, "PERSON@EXAMPLE").length, 1)
  assert.equal(filterAccounts(accounts, "claude", "person").length, 0)
  assert.equal(redactAccountMessage("person@example.test 请求 https://private.example.test/v1 失败，余额 $128.50"), "[账号已隐藏] 请求 [地址已隐藏] 失败，余额 $128.50")
})
