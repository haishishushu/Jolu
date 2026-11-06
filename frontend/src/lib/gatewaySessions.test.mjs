import { test } from "node:test"
import assert from "node:assert/strict"

import {
  applyLiveEvent, buildSessionViews, compactTokens, defaultWindowSize, liveUnattributed, credentialLabel, emptyLive, liveBySession, liveFromSnapshot,
  nextPriorityExpiry, PRIORITY_HOLD_MS, projectName, relativeTime, routeTypeTag, sessionKey, sessionLines,
  sessionStatus, sessionTitle, sessionWindow, sourceSummary, tokenDisplay, updatePriority,
} from "./gatewaySessions.ts"

const start = (id, seq, session = "s1", conn = "c1", ts = 1000) => ({
  kind: "start",
  event: { request_id: id, seq, ts, agent: "codex", requested_model: "kimi/k2", source: "kimi", connection_id: conn, session_platform: "codex", session_id: session },
})
const end = (id, seq) => ({
  kind: "end",
  event: { request_id: id, seq, ts: 2000, agent: "codex", requested_model: "kimi/k2", source: "kimi", upstream_model: null, status_code: 200, result: "ok", duration_ms: 5 },
})

const session = (id, extra = {}) => ({
  platform: "codex", session_id: id, agent: "codex", title: `标题 ${id}`, cwd: "D:/work/cc-usage", thread_last_ts: 0, gateway_last_ts: 0,
  thread_status: "recent", tokens: { fresh_input: 1, cache_read: 0, cache_write: 0, output: 1, total: 2, state: "known" }, links: [], ...extra,
})

test("并发请求逐个结束：全部结束才停止运行", () => {
  let live = emptyLive()
  live = applyLiveEvent(live, start(1, 1))
  live = applyLiveEvent(live, start(2, 2))
  assert.equal(liveBySession(live).get(sessionKey("codex", "s1")).count, 2)
  live = applyLiveEvent(live, end(1, 3))
  assert.equal(liveBySession(live).get(sessionKey("codex", "s1")).count, 1, "结束一个请求不代表会话已完成")
  live = applyLiveEvent(live, end(2, 4))
  assert.equal(liveBySession(live).has(sessionKey("codex", "s1")), false)
})

test("重复与乱序事件不会产生负计数或永久运行", () => {
  let live = emptyLive()
  live = applyLiveEvent(live, end(7, 2))
  live = applyLiveEvent(live, start(7, 1))
  assert.equal(Object.keys(live.requests).length, 0, "结束先到，迟到的开始不再登记")
  live = applyLiveEvent(live, end(7, 3))
  live = applyLiveEvent(live, start(8, 4))
  live = applyLiveEvent(live, start(8, 4))
  assert.equal(liveBySession(live).get(sessionKey("codex", "s1")).count, 1)
})

test("快照之后只应用序号更大的事件", () => {
  const snapshot = [{ request_id: 5, ts: 1, agent: null, requested_model: "a/b", source: "a", connection_id: "c1", session_platform: "codex", session_id: "s1" }]
  const live = liveFromSnapshot(10, snapshot, [end(5, 9), start(6, 11, "s2")])
  assert.ok(live.requests[5], "序号不大于快照的结束事件已反映在快照里，不能再删")
  assert.ok(live.requests[6])
  const after = applyLiveEvent(live, end(5, 12))
  assert.equal(after.requests[5], undefined)
})

test("无法验证会话身份的请求不归到任何会话", () => {
  let live = emptyLive()
  live = applyLiveEvent(live, { kind: "start", event: { request_id: 1, seq: 1, ts: 1, agent: null, requested_model: "a/b", source: "a" } })
  assert.equal(liveBySession(live).size, 0)
})

test("运行优先、组内按进入运行时刻、其余按最近活动；同时间按稳定标识", () => {
  const sessions = [session("a", { thread_last_ts: 300 }), session("b", { thread_last_ts: 500 }), session("c", { thread_last_ts: 500 }), session("d", { thread_last_ts: 100 })]
  let live = emptyLive()
  live = applyLiveEvent(live, start(1, 1, "d"))
  const map = liveBySession(live)
  const priority = updatePriority({}, map.keys(), 1000)
  const order = buildSessionViews(sessions, map, priority).map((v) => v.session.session_id)
  assert.deepEqual(order, ["d", "b", "c", "a"])
})

test("Token 更新不改变排序", () => {
  const base = [session("a", { thread_last_ts: 300 }), session("b", { thread_last_ts: 200 })]
  const grown = [session("a", { thread_last_ts: 300 }), session("b", { thread_last_ts: 200, tokens: { ...base[1].tokens, total: 99999 } })]
  const ids = (list) => buildSessionViews(list, new Map(), {}).map((v) => v.session.session_id)
  assert.deepEqual(ids(base), ids(grown))
})

test("运行结束后在保留期内不掉出前部，过期后回到按活动排序", () => {
  const key = sessionKey("codex", "x")
  let p = updatePriority({}, [key], 1000)
  p = updatePriority(p, [], 2000)
  assert.equal(p[key].since, 1000)
  assert.equal(nextPriorityExpiry(p), 2000 + PRIORITY_HOLD_MS)
  p = updatePriority(p, [key], 3000)
  assert.equal(p[key].since, 1000, "保留期内再次运行，位置不变")
  p = updatePriority(p, [], 4000)
  p = updatePriority(p, [], 4000 + PRIORITY_HOLD_MS + 1)
  assert.equal(p[key], undefined)
})

test("默认窗口 5 条，没有更多时不显示入口", () => {
  const list = [1, 2, 3, 4, 5, 6, 7, 8]
  assert.deepEqual(sessionWindow(list, false), { shown: [1, 2, 3, 4, 5], hidden: 3 })
  assert.deepEqual(sessionWindow(list, true).shown.length, 8)
  assert.deepEqual(sessionWindow([1, 2], false), { shown: [1, 2], hidden: 0 })
})

test("会话连线：运行中的连接 + 最近一次关联", () => {
  const view = {
    key: "k", running: true, liveConnections: ["c2"], activity: 0,
    session: session("a", { links: [{ connection_id: "c1", source: "kimi", requested_model: "kimi/k2", upstream_model: null, last_ts: 9, requests: 1 }] }),
  }
  assert.deepEqual(sessionLines(view), [{ connectionId: "c2", active: true }, { connectionId: "c1", active: false }])
  assert.deepEqual(sessionLines({ ...view, liveConnections: ["c1"] }), [{ connectionId: "c1", active: true }])
})

test("来源卡：关联会话按会话去重，模型取最近一次", () => {
  const views = buildSessionViews([
    session("a", { links: [{ connection_id: "c1", source: "kimi", requested_model: "kimi/old", upstream_model: null, last_ts: 1, requests: 3 }] }),
    session("b", { links: [{ connection_id: "c1", source: "kimi", requested_model: "kimi/k2", upstream_model: null, last_ts: 5, requests: 1 }] }),
    session("c", { links: [{ connection_id: "c9", source: "x", requested_model: "x/y", upstream_model: "y-up", last_ts: 7, requests: 1 }] }),
  ], new Map(), {})
  assert.deepEqual(sourceSummary(views, "c1"), { sessions: 2, model: "k2" })
  assert.deepEqual(sourceSummary(views, "c9"), { sessions: 1, model: "y-up" })
  assert.deepEqual(sourceSummary(views, "none"), { sessions: 0, model: null })
})

test("Token 缩写：小于 1000 整数，其余 K / M 最多一位小数", () => {
  assert.equal(compactTokens(0), "0")
  assert.equal(compactTokens(999), "999")
  assert.equal(compactTokens(1000), "1K")
  assert.equal(compactTokens(12_840), "12.8K")
  assert.equal(compactTokens(999_960), "1M")
  assert.equal(compactTokens(24_600_000), "24.6M")
})

test("Token 状态：真实零、未知、缺失、失败与过期分别表达", () => {
  const zero = tokenDisplay({ fresh_input: 0, cache_read: 0, cache_write: 0, output: 0, total: 0, state: "known" }, "fresh")
  assert.equal(zero.text, "0")
  assert.equal(zero.known, true)
  assert.equal(tokenDisplay({ fresh_input: 1, cache_read: null, cache_write: 0, output: 0, total: null, state: "partial" }, "fresh").text, "—")
  assert.equal(tokenDisplay(undefined, "fresh").text, "—")
  const failed = tokenDisplay({ fresh_input: 1, cache_read: 0, cache_write: 0, output: 0, total: 1, state: "known" }, "failed")
  assert.equal(failed.text, "1")
  assert.match(failed.flag, /刷新失败/)
  assert.match(tokenDisplay(zero && { fresh_input: 0, cache_read: 0, cache_write: 0, output: 0, total: 0, state: "known" }, "stale").flag, /过期/)
})

test("状态文案不从耗时推测阶段，没有完成证据时保持最近活跃", () => {
  assert.deepEqual(sessionStatus(true, "done"), { label: "运行中", tone: "running" })
  assert.equal(sessionStatus(false, "done").label, "已完成")
  assert.equal(sessionStatus(false, "running").label, "最近活跃", "会话线程的状态不等于经网关正在运行")
  assert.equal(sessionStatus(false, null).label, "最近活跃")
})

test("显示文案", () => {
  assert.equal(projectName("D:\\develop\\cc-usage"), "cc-usage")
  assert.equal(projectName(null), null)
  assert.equal(sessionTitle({ title: null, session_id: "0123456789ab" }), "会话 01234567")
  assert.equal(sessionTitle({ title: null, session_id: "opencode:ses_e500e3935ffe" }), "会话 ses_e500")
  assert.equal(relativeTime(10_000, 30_000), "刚刚")
  assert.equal(relativeTime(0, 1), "—")
  assert.equal(relativeTime(0 + 1, 1 + 3 * 60_000), "3 分钟前")
  assert.equal(credentialLabel("auth", "claude"), "官方订阅")
  assert.equal(credentialLabel("api", "custom"), "第三方 API Key")
  assert.equal(credentialLabel("api", "zcode"), "官方 API Key")
  assert.equal(routeTypeTag("api", "custom"), "第三方")
})

test("运行中会话超过 5 条时全部进入默认窗口", () => {
  const views = (n, running) => Array.from({ length: n }, (_, i) => ({ prioritized: i < running }))
  assert.equal(defaultWindowSize(views(8, 2)), 5)
  assert.equal(defaultWindowSize(views(9, 7)), 7)
})

test("Token 角标：失败与过期分别标注，最新数据没有角标", () => {
  const known = { fresh_input: 1, cache_read: 0, cache_write: 0, output: 0, total: 1, state: "known" }
  assert.equal(tokenDisplay(known, "fresh").badge, null)
  assert.deepEqual(tokenDisplay(known, "failed").badge, { kind: "failed", label: "刷新失败" })
  assert.deepEqual(tokenDisplay(undefined, "stale").badge, { kind: "stale", label: "已过期" })
})

test("没有会话标识的在途请求只按来源汇总，不归到会话", () => {
  let live = emptyLive()
  const gemini = (id, seq) => ({ kind: "start", event: { request_id: id, seq, ts: 1, agent: "gemini", requested_model: "a/b", source: "a", connection_id: "c1", session_platform: null, session_id: null } })
  live = applyLiveEvent(live, gemini(1, 1))
  live = applyLiveEvent(live, gemini(2, 2))
  live = applyLiveEvent(live, start(3, 3))
  assert.deepEqual(liveUnattributed(live).get("c1"), { count: 2, agents: ["gemini"] })
  assert.equal(liveBySession(live).size, 1, "有会话的请求照常归到会话")
  live = applyLiveEvent(live, end(1, 4))
  live = applyLiveEvent(live, end(2, 5))
  assert.equal(liveUnattributed(live).has("c1"), false)
})
