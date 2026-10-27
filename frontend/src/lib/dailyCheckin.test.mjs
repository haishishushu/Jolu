import assert from "node:assert/strict"
import test from "node:test"
import { CHECKIN_EDITIONS, applyCheckinUpdate, checkinTargets, checkinSummary, initialCheckinAccount, rewardLabel, currentOutcome, checkinAccountLabel } from "./dailyCheckin.ts"

const account = (id, extra = {}) => ({ ...initialCheckinAccount({ id, platform: "qoder", edition: "cn", name: id, connection_status: "connected" }), account_key: `identity-${id}`, outcome: { state: "ready", message: "可领取", reward: 100, unit: "Credits", period_key: "2026-10-27", reset_at: null }, ...extra })

test("签到显示官方账号，不把旧连接名称或凭证当账号，隐私态隐藏标识", () => {
  assert.equal(checkinAccountLabel({name:'测试',account:'person@example.test'}),'person@example.test')
  assert.equal(checkinAccountLabel({name:'测试',account:null}),'未获取账号')
  assert.equal(checkinAccountLabel({name:'测试',account:'person@example.test'},true),'账号已隐藏')
})

test("身份更新同步已验证账号，缺失显示值必须清除旧账号标签", () => {
  const previous=account('a',{account:'first@example.test',account_key:'identity-first'})
  const update={id:'a',account_key:'identity-second',account:'second@example.test',checked_at:123,outcome:previous.outcome,statistics:null}
  assert.equal(applyCheckinUpdate([previous],update)[0].account,'second@example.test')
  assert.equal(applyCheckinUpdate([previous],{...update,account:null})[0].account,null)
})

test("六种版本明确区分，仅真实提供手动签到的版本可领取", () => {
  assert.equal(CHECKIN_EDITIONS.length, 6)
  assert.deepEqual(CHECKIN_EDITIONS.filter(e => e.manual).map(e => e.id), ["workbuddy-cn", "qoder-cn", "trae-cn", "qoder-global"])
  const ai = initialCheckinAccount({ id: "ai", platform: "workbuddy", edition: "ai", name: "AI", connection_status: "connected" })
  assert.equal(ai.outcome.state, "unsupported")
  assert.deepEqual(checkinTargets([ai, account("paused", { connection_status: "paused" })]), [])
})

test("批量按真实账号去重，未知身份不合并，不串版本", () => {
  const rows = [account("a", { account_key: "same-cn" }), account("alias", { account_key: "same-cn" }), account("b"), account("c"), account("global", { edition: "global", account_key: "same-global" }), account("done", { outcome: { state: "claimed", message: "已领" } })]
  assert.deepEqual(checkinTargets(rows).map(a => a.id), ["a", "b", "c", "global"])
  assert.deepEqual(checkinSummary(rows), { ready: 4, claimed: 1, errors: 0 })
})

test("奖励未知不补零，真实零保留，不累计不同平台单位", () => {
  assert.equal(rewardLabel({ reward: null, unit: "积分" }), "—")
  assert.equal(rewardLabel({ reward: 0, unit: "积分" }), "0 积分")
  assert.equal(rewardLabel({ reward: 100, unit: "Credits" }), "100 Credits")
  assert.equal(rewardLabel({ reward: Number.NaN, unit: "积分" }), "—")
})

test("新轮次不能继续显示上一轮可领取或已领取", () => {
  const row = account("old", { checked_at: 90_000, outcome: { state: "claimed", message: "已领", reward: 100, server_time: 90, reset_at: 100, unit: "Credits" } })
  assert.equal(currentOutcome(row, 99_999).state, "claimed")
  assert.equal(currentOutcome(row, 100_000).state, "unknown")
  assert.deepEqual(checkinTargets([row], 100_000), [])
})

test("系统时钟快十分钟仍按服务端剩余有效期展示，可签到必须有已核实身份", () => {
  const server = Date.parse("2026-10-27T15:55:00Z") / 1000
  const received = (server + 600) * 1000
  const row = account("clock", { checked_at: received, outcome: { state: "ready", message: "可领取", reward: 100, unit: "积分", server_time: server, reset_at: server + 300 } })
  assert.equal(currentOutcome(row, received).state, "ready")
  assert.equal(checkinTargets([row], received).length, 1)
  assert.equal(currentOutcome(row, received + 300_000).state, "unknown")
  assert.equal(checkinTargets([account("unverified", { account_key: null })]).length, 0)
})

test("单账号结果仅同步相同真实身份；刷新后变更身份不影响旧账号", () => {
  const rows = [account("a", { account_key: "user-a" }), account("alias", { account_key: "user-a" }), account("b", { account_key: "user-b" })]
  const update = { id: "a", account_key: "user-a", checked_at: 1, outcome: { state: "claimed", message: "已领", reward: null, unit: "积分", reset_at: null, period_key: null } }
  const next = applyCheckinUpdate(rows, update)
  assert.deepEqual(next.map(a => a.outcome.state), ["claimed", "claimed", "ready"])
  const switched = applyCheckinUpdate(rows, { ...update, account_key: "new-user" })
  assert.deepEqual(switched.map(a => a.outcome.state), ["claimed", "ready", "ready"])
  assert.deepEqual(applyCheckinUpdate(rows, { ...update, id: "deleted" }), rows)
})

test("签到统计绑定真实身份，未知金额不补零，失败不保留旧账号统计", () => {
  const statistics = { has_records: true, unit: "积分", week: { days: 2, reward: 0 }, month: { days: 18, reward: null }, all: { days: 26, reward: null } }
  const rows = [account("a", { account_key: "user-a" }), account("alias", { account_key: "user-a" }), account("b", { account_key: "user-b", statistics: { ...statistics, week: { days: 1, reward: 100 } } })]
  const update = { id: "a", account_key: "user-a", checked_at: 1, outcome: rows[0].outcome, statistics }
  const next = applyCheckinUpdate(rows, update)
  assert.deepEqual(next[0].statistics, statistics)
  assert.deepEqual(next[1].statistics, statistics)
  assert.equal(next[2].statistics.week.reward, 100)
  assert.equal(next[0].statistics.month.reward, null)
  assert.equal(next[0].statistics.week.reward, 0)
  const failed = applyCheckinUpdate(next, { ...update, account_key: "new-user", statistics: null })
  assert.equal(failed[0].statistics, null)
  assert.deepEqual(failed[1].statistics, statistics)
  assert.equal(initialCheckinAccount({ id: "new", platform: "trae", edition: "cn", name: "新账号", connection_status: "connected" }).statistics, null)
})
