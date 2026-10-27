import assert from "node:assert/strict"
import test from "node:test"
import { checkinPeriodDisplay, checkinQuotaDisplay, checkinQuotaSummary } from "./checkinStats.ts"

const quotaWindow = (key, extra = {}) => ({ key, window_name: key, used_percent: null, amount_text: null, remaining_text: null, resets_at: null, ...extra })
const quota = (windows, extra = {}) => ({ state: { state: "ok", windows, plan: null }, ...extra })
const statistics = (extra = {}) => ({ has_records: true, unit: "Credits", week: { days: 0, reward: 0 }, month: { days: 2, reward: 250.5 }, all: { days: 4, reward: 500 }, ...extra })
const rawQuota = (amount_ranges) => ({ state: { state: "ok", windows: [quotaWindow("credits", { amount_text: "60 / 100 credits", remaining_text: "剩余 40 credits" })], plan: null,
  account: { subscription_status: "active", expires_at: null, subscription_source: "live", observed_at: null, reset_credits: null, amount_ranges } } })

test("签到统计区分未获取、暂无记录、奖励未知和真实零", () => {
  assert.deepEqual(checkinPeriodDisplay(null, "week"), { days: "未获取", reward: "—" })
  assert.deepEqual(checkinPeriodDisplay(statistics({ has_records: false }), "week"), { days: "暂无记录", reward: "—" })
  assert.deepEqual(checkinPeriodDisplay(statistics(), "week"), { days: "0 天", reward: "0 Credits" })
  assert.deepEqual(checkinPeriodDisplay(statistics(), "month"), { days: "2 天", reward: "250.5 Credits" })
  assert.deepEqual(checkinPeriodDisplay(statistics({ all: { days: 4, reward: null } }), "all"), { days: "4 天", reward: "—" })
  assert.equal(checkinPeriodDisplay(statistics({ unit: "", all: { days: 4, reward: 500 } }), "all").reward, "—")
})

test("已核实的后端额度文本保留千分位小数和零", () => {
  const display = checkinQuotaDisplay(quota([quotaWindow("qoder:userQuota", { amount_text: "1,000.25 / 2,000.50 credits", remaining_text: "剩余 0.00 credits" })]))
  assert.deepEqual(display.rows.map(({ total, remaining }) => ({ total, remaining })), [{ total: "2,000.50 credits", remaining: "0.00 credits" }])
  assert.equal(display.rows[0].reason, null)
  assert.equal(display.message, null)
})

test("总量与已用不相减，没有官方精确剩余始终显示未知", () => {
  const display = checkinQuotaDisplay(quota([quotaWindow("credits", { used_percent: 25, amount_text: "25 / 100 credits" })]))
  assert.equal(display.rows[0].total, "100 credits")
  assert.equal(display.rows[0].remaining, "—")
  assert.match(display.rows[0].reason, /精确剩余/)
  const percentageOnly = checkinQuotaDisplay(quota([quotaWindow("qoder:overall", { used_percent: 0 })]))
  assert.equal(percentageOnly.rows[0].total, "—")
  assert.equal(percentageOnly.rows[0].remaining, "—")
  assert.match(percentageOnly.rows[0].reason, /百分比/)
})

test("Qoder 套餐、加购、组织和专属额度逐范围保留，百分比总览不混成金额", () => {
  const display = checkinQuotaDisplay(quota([
    quotaWindow("qoder:overall", { used_percent: 40 }),
    quotaWindow("qoder:userQuota", { window_name: "套餐额度", amount_text: "20 / 100 Credits", remaining_text: "剩余 80 Credits" }),
    quotaWindow("qoder:addOnQuota", { window_name: "加购额度", remaining_text: "剩余 0 Credits" }),
    quotaWindow("qoder:orgResourcePackage", { window_name: "组织额度", amount_text: "200 / 800 积分", remaining_text: "剩余 600 积分" }),
    quotaWindow("qoder:dedicated:package-a", { window_name: "专属包 A", amount_text: "0 / 10 次", remaining_text: "剩余 10 次" }),
  ]))
  assert.deepEqual(display.rows.map(({ label, total, remaining }) => ({ label, total, remaining })), [
    { label: "套餐额度", total: "100 Credits", remaining: "80 Credits" },
    { label: "加购额度", total: "—", remaining: "0 Credits" },
    { label: "组织额度", total: "800 积分", remaining: "600 积分" },
    { label: "专属包 A", total: "10 次", remaining: "10 次" },
  ])
  assert.match(display.rows[1].reason, /总额度/)
})

test("Trae 明确总量、已用未知与不限量均保留来源范围和单位", () => {
  const display = checkinQuotaDisplay(quota([
    quotaWindow("trae:fast", { window_name: "快速请求", amount_text: "0 / 200 次" }),
    quotaWindow("trae:slow", { window_name: "慢速请求", amount_text: "总额度 1,000 次；已用未知" }),
    quotaWindow("trae:unlimited", { window_name: "不限量模型", amount_text: "不限量" }),
    quotaWindow("trae:used-unlimited", { window_name: "慢速不限量", amount_text: "已用 5 次；不限量" }),
  ]))
  assert.deepEqual(display.rows.map(({ label, total, remaining }) => ({ label, total, remaining })), [
    { label: "快速请求", total: "200 次", remaining: "—" },
    { label: "慢速请求", total: "1,000 次", remaining: "—" },
    { label: "不限量模型", total: "不限量", remaining: "—" },
    { label: "慢速不限量", total: "不限量（次）", remaining: "—" },
  ])
})

test("畸形、负数与百分比文本不能伪造成绝对额度，明确剩余可以独立存在", () => {
  for (const amount_text of ["-1 / 100 credits", "1 / -100 credits", "0 / 1,00 credits", "50%", "已用 10 次", "0 / Infinity credits"]) {
    const display = checkinQuotaDisplay(quota([quotaWindow("unknown", { amount_text, remaining_text: "剩余 80%" })]))
    assert.equal(display.rows[0].total, "—", amount_text)
    assert.equal(display.rows[0].remaining, "—", amount_text)
  }
  const display = checkinQuotaDisplay(quota([quotaWindow("only-remaining", { remaining_text: "0.5 Credits" })]))
  assert.equal(display.rows[0].total, "—")
  assert.equal(display.rows[0].remaining, "0.5 Credits")
})

test("未获取、加载、失败、不支持、无权限和登录失效分别表达，不补零", () => {
  assert.equal(checkinQuotaDisplay({ state: null }).message, "未获取")
  assert.equal(checkinQuotaDisplay({ state: null, loading: true }).message, "正在查询额度…")
  for (const [state, label] of [["failed", "查询失败"], ["unsupported", "不支持查询"], ["forbidden", "无权限"], ["unauthorized", "登录已过期"], ["rate_limited", "请求受限"]]) {
    const display = checkinQuotaDisplay({ state: { state, reason: "来源说明" } })
    assert.match(display.message, new RegExp(label))
    assert.match(display.message, /来源说明/)
    assert.equal(display.rows[0].total, "—")
    assert.equal(display.rows[0].remaining, "—")
  }
  assert.match(checkinQuotaDisplay(quota([])).rows[0].reason, /未提供/)
})

test("刷新失败保留旧额度时明确标过期与失败原因", () => {
  const display = checkinQuotaDisplay(quota([quotaWindow("credits", { amount_text: "0 / 100 credits", remaining_text: "剩余 100 credits" })], { stale: true, failure: { state: "failed", reason: "网络不可用" } }))
  assert.equal(display.rows[0].remaining, "100 credits")
  assert.equal(display.stale, true)
  assert.match(display.message, /过期/)
  assert.match(display.message, /查询失败/)
  assert.match(display.message, /网络不可用/)
})

test("WorkBuddy 原始资源包未提供精确剩余时不能使用旧窗口的推算剩余", () => {
  const display = checkinQuotaDisplay(rawQuota([{ key: "wb:plan", name: "套餐资源包", total: 100, remaining: null, unit: "credits" }]))
  assert.deepEqual(display.rows.map(({ label, total, remaining }) => ({ label, total, remaining })), [{ label: "套餐资源包", total: "100 credits", remaining: "—" }])
  assert.match(display.rows[0].reason, /精确剩余/)
})

test("WorkBuddy 原始资源包仅有剩余时不能使用旧窗口的聚合总量", () => {
  const display = checkinQuotaDisplay(rawQuota([{ key: "wb:plan", name: "套餐资源包", total: null, remaining: 40, unit: "credits" }]))
  assert.equal(display.rows[0].total, "—")
  assert.equal(display.rows[0].remaining, "40 credits")
  assert.match(display.rows[0].reason, /总额度/)
})

test("原始资源包逐包展示名称、单位、小数与真实零，不计算资源包合计", () => {
  const display = checkinQuotaDisplay(rawQuota([
    { key: "wb:plan", name: "套餐资源包", total: 2000.5, remaining: 0, unit: "credits" },
    { key: "wb:bonus", name: "签到资源包", total: 0, remaining: 0, unit: "积分" },
    { key: "wb:addon", name: "加购资源包", total: null, remaining: 0.125, unit: "credits" },
  ]))
  assert.deepEqual(display.rows.map(({ label, total, remaining }) => ({ label, total, remaining })), [
    { label: "套餐资源包", total: "2,000.5 credits", remaining: "0 credits" },
    { label: "签到资源包", total: "0 积分", remaining: "0 积分" },
    { label: "加购资源包", total: "—", remaining: "0.125 credits" },
  ])
})

test("原始资源包为空也不能回退旧窗口的聚合数值", () => {
  const display = checkinQuotaDisplay(rawQuota([]))
  assert.equal(display.rows.length, 1)
  assert.equal(display.rows[0].total, "—")
  assert.equal(display.rows[0].remaining, "—")
  assert.match(display.rows[0].reason, /未提供/)
})

test("原始资源包的缺失、负数、非有限值和非数值保持未知", () => {
  for (const value of [null, undefined, -1, Number.NaN, Number.POSITIVE_INFINITY, "0"]) {
    const display = checkinQuotaDisplay(rawQuota([{ key: "wb:invalid", name: "资源包", total: value, remaining: value, unit: "credits" }]))
    assert.equal(display.rows[0].total, "—")
    assert.equal(display.rows[0].remaining, "—")
  }
})

test("官方返回日期及活动汇总优先，未知历史金额不由本机少量记录补齐", () => {
  const official = {unit:'积分',scope:'本期活动',week:{days:3,reward:null},month:{days:12,reward:null},all:{days:null,reward:1800},note:'官方返回记录'}
  const local = {has_records:true,unit:'积分',week:{days:1,reward:100},month:{days:1,reward:100},all:{days:1,reward:100}}
  assert.deepEqual(checkinPeriodDisplay(local,'week',official),{days:'3 天',reward:'—'})
  assert.deepEqual(checkinPeriodDisplay(local,'all',official),{days:'—',reward:'1,800 积分'})
  assert.deepEqual(checkinPeriodDisplay(null,'month',{...official,month:{days:0,reward:0}}),{days:'0 天',reward:'0 积分'})
})

test("计算可用量必须明示来源，明确剩余优先，矛盾计算值不采用", () => {
  const display = checkinQuotaDisplay(rawQuota([{key:'trae:a',name:'积分包',total:100,used:30,remaining:null,calculated_remaining:70,unit:'积分'}]))
  assert.equal(display.rows[0].remaining,'70 积分')
  assert.equal(display.rows[0].remainingLabel,'可用额度（计算）')
  assert.equal(display.rows[0].used,'30 积分')
  const direct = checkinQuotaDisplay(rawQuota([{key:'trae:a',name:'积分包',total:100,used:30,remaining:60,calculated_remaining:70,unit:'积分'}]))
  assert.equal(direct.rows[0].remaining,'60 积分')
  assert.equal(direct.rows[0].remainingLabel,'可用额度')
  for(const calculated_remaining of [null,-1,1000,Number.NaN]) {
    const row=checkinQuotaDisplay(rawQuota([{key:'trae:a',name:'积分包',total:100,used:30,remaining:null,calculated_remaining,unit:'积分'}])).rows[0]
    assert.equal(row.remaining,'—')
  }
})

test("原始不限量范围保留能力与已用，不把不限量当未知或计算成零", () => {
  const display=checkinQuotaDisplay(rawQuota([{key:'trae:unlimited',name:'快速请求',total:null,used:5,remaining:null,unit:'次',unlimited:true}]))
  assert.equal(display.rows[0].total,'不限量（次）')
  assert.equal(display.rows[0].used,'5 次')
  assert.equal(display.rows[0].remaining,'—')
})

test("摘要单项保留真实零及计算标记，多项不拼合，明确汇总可直接显示", () => {
  const single=checkinQuotaSummary(rawQuota([{key:'wb:0',name:'积分资源包',total:100,remaining:0,unit:'credits'}]))
  assert.equal(single.total,'100 credits')
  assert.equal(single.remaining,'0 credits')
  assert.equal(single.scope,'积分资源包')
  const multiple=checkinQuotaSummary(rawQuota([{key:'qoder:a',name:'套餐',total:100,remaining:50,unit:'Credits'},{key:'qoder:b',name:'加购',total:100,remaining:20,unit:'Credits'}]))
  assert.equal(multiple.total,'多项额度')
  assert.equal(multiple.remaining,'多项额度')
  assert.equal(multiple.scope,'共 2 项额度')
  const summary=checkinQuotaSummary(rawQuota([{key:'trae:credits',name:'积分汇总',total:4700,remaining:null,used:0,calculated_remaining:4700,unit:'积分'},{key:'trae:0',name:'积分包',total:100,remaining:null,unit:'积分'}]))
  assert.equal(summary.total,'4,700 积分')
  assert.equal(summary.remaining,'4,700 积分')
  assert.equal(summary.remainingLabel,'可用额度（计算）')
})

test("摘要保留失败和过期状态，详细原因仍由详情展示", () => {
  assert.equal(checkinQuotaSummary({state:{state:'failed',reason:'合成错误'}}).status,'查询失败')
  assert.equal(checkinQuotaSummary({state:null,loading:true}).status,'查询中…')
  assert.equal(checkinQuotaSummary({...quota([quotaWindow('credits',{amount_text:'0 / 100 credits',remaining_text:'剩余 100 credits'})]),stale:true}).status,'数据过期')
})
