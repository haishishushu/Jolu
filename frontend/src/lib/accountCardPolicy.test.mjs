import assert from "node:assert/strict"
import test from "node:test"
import { accountCardPolicy } from "./accountCardPolicy.ts"

const auth = { kind: "auth", baseUrl: null, status: "connected", label: "浏览器授权" }
const api = { ...auth, kind: "api", baseUrl: "https://gateway.example.test/v1" }
const metadata = { subscription_status: "unknown", expires_at: null, subscription_source: "unavailable", observed_at: null, reset_credits: null }
const quota = { state: "ok", plan: null, windows: [], account: metadata }
const window = { key: "7d", window_name: "周额度", used_percent: 0, amount_text: null, remaining_text: null, resets_at: null }

test("重置卡只在来源返回对象时出现，真实零与未知次数分别保留", () => {
  assert.equal(accountCardPolicy(auth, null).resetCredits, null)
  assert.equal(accountCardPolicy(auth, { state: "failed", reason: "网络失败" }).resetCredits, null)
  assert.equal(accountCardPolicy(auth, quota).resetCredits, null)
  for (const credits of [
    { available_count: 0, applicable_available_count: 0 },
    { available_count: null, applicable_available_count: 1 },
    { available_count: 1, applicable_available_count: null },
  ]) {
    assert.deepEqual(accountCardPolicy(auth, { ...quota, account: { ...metadata, reset_credits: credits } }).resetCredits, credits)
  }
})

test("套餐只取服务响应，不把连接标签和身份来源当成套餐", () => {
  for (const label of ["浏览器授权", "本机来源", "API 服务", "Pro"]) {
    assert.equal(accountCardPolicy({ ...auth, label }, quota).plan, null)
  }
  assert.equal(accountCardPolicy(auth, { ...quota, plan: "  Pro  " }).plan, "Pro")
  assert.equal(accountCardPolicy(auth, { ...quota, plan: "  " }).plan, null)
  assert.equal(accountCardPolicy(auth, { state: "failed", reason: "失败" }).plan, null)
})

test("到期时间只展示有效日期，并保留本机记录来源", () => {
  assert.equal(accountCardPolicy(auth, quota).expiresAt, null)
  assert.equal(accountCardPolicy(auth, { ...quota, account: { ...metadata, expires_at: "invalid" } }).expiresAt, null)
  const local = accountCardPolicy(auth, { ...quota, account: { ...metadata, expires_at: "2026-10-01T00:00:00Z", subscription_source: "local_snapshot" } })
  assert.equal(local.expiresAt, "2026-10-01T00:00:00Z")
  assert.equal(local.expiryIsSnapshot, true)
  const live = accountCardPolicy(auth, { ...quota, account: { ...metadata, expires_at: "2026-10-01T00:00:00Z", subscription_source: "live" } })
  assert.equal(live.expiryIsSnapshot, false)
})

test("额度只保留实际返回窗口，不按平台补造五小时窗口", () => {
  const result = accountCardPolicy({ ...auth, platformId: "codex" }, { ...quota, windows: [window] })
  assert.deepEqual(result.windows, [window])
  assert.equal(result.showQuota, true)
  assert.equal(accountCardPolicy(auth, quota).showQuota, false)
  assert.equal(accountCardPolicy(auth, { ...quota, plan: "个人套餐已到期" }).showQuota, true)
  assert.equal(accountCardPolicy(auth, { state: "unsupported", reason: "未提供" }).showQuota, false)
  assert.equal(accountCardPolicy(auth, { state: "unauthorized", reason: "登录失效" }).showQuota, true)
})

test("余额按实际API连接及响应决定，不支持不保留占位，失败仍能说明", () => {
  const ok = { state: "ok", balance: 0, currency: "USD", used: 0 }
  assert.equal(accountCardPolicy(auth, quota, ok).showBalance, false)
  assert.equal(accountCardPolicy({ ...api, baseUrl: null }, quota, ok).showBalance, false)
  assert.equal(accountCardPolicy(api, quota, { state: "unsupported", reason: "没有钱包" }).showBalance, false)
  assert.equal(accountCardPolicy(api, quota, { state: "failed", reason: "请求失败" }).showBalance, true)
  const result = accountCardPolicy(api, quota, ok)
  assert.equal(result.showBalance, true)
  assert.equal(result.balanceAmount, 0)
  assert.equal(result.balanceCurrency, "USD")
  assert.equal(result.used, 0)
  assert.equal(accountCardPolicy(api, quota, { ...ok, used: null }).used, null)
  assert.equal(accountCardPolicy(api, quota, { state: "failed", reason: "失败" }).used, null)
})

test("刷新失败由hook保留成功响应时，卡片继续使用上次真实金额与额度", () => {
  const hookResult = { state: { state: "ok", balance: -2, currency: "USD", used: 2 }, stale: true, failure: { state: "failed", reason: "网络异常" } }
  const result = accountCardPolicy(api, { ...quota, windows: [window] }, hookResult.state)
  assert.equal(result.balanceAmount, -2)
  assert.equal(result.used, 2)
  assert.deepEqual(result.windows, [window])
})
