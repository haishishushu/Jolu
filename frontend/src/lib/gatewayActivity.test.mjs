import assert from "node:assert/strict"
import test from "node:test"
import {
  activityFromLog, dayKey, emptyActivity, formatDuration, modelPart, nextTickAt, onEnd, onStart, onTick,
  OK_HOLD_MS, rejectReason, REJECT_HOLD_MS, STALE_IN_FLIGHT_MS,
} from "./gatewayActivity.ts"

const T0 = new Date(2026, 10, 4, 12, 0, 0).getTime()
const start = (id, agent = "opencode", model = "codex/gpt-5-codex") => ({ request_id: id, ts: T0, agent, requested_model: model, source: model.split("/")[0] })
const end = (id, result = "ok", extra = {}) => ({
  request_id: id, ts: T0, agent: "opencode", requested_model: "codex/gpt-5-codex", source: result === "rejected" ? null : "codex",
  upstream_model: null, status_code: result === "ok" ? 200 : 429, result, duration_ms: 6000, ...extra,
})

test("开始与结束按 id 配对，沿用同一条显示", () => {
  let s = onStart(emptyActivity(T0), start(1), T0)
  assert.equal(s.shown.phase, "in")
  assert.equal(s.shown.model, "gpt-5-codex")
  s = onEnd(s, end(1), T0 + 6000)
  assert.equal(s.inFlight.length, 0)
  assert.equal(s.shown.phase, "ok")
  assert.equal(s.shown.startedAt, T0, "保留开始时刻")
  assert.equal(s.today.ok, 1)
  assert.equal(s.lastByAgent.opencode.source, "codex")
})

test("只收到结束事件时直接显示结果", () => {
  const s = onEnd(emptyActivity(T0), end(5, "rejected"), T0)
  assert.equal(s.shown.phase, "rejected")
  assert.equal(s.shown.source, null)
  assert.equal(s.today.rejected, 1)
})

test("结果停留到期后收起：成功 2.4 秒，被拒 4.8 秒", () => {
  let s = onEnd(emptyActivity(T0), end(1), T0)
  assert.equal(nextTickAt(s), T0 + OK_HOLD_MS)
  assert.equal(onTick(s, T0 + OK_HOLD_MS - 1).shown.phase, "ok")
  assert.equal(onTick(s, T0 + OK_HOLD_MS).shown, null)
  s = onEnd(emptyActivity(T0), end(2, "rejected"), T0)
  assert.equal(onTick(s, T0 + OK_HOLD_MS).shown.phase, "rejected")
  assert.equal(onTick(s, T0 + REJECT_HOLD_MS).shown, null)
})

test("只有开始没有结束的请求 120 秒后收回", () => {
  const s = onStart(emptyActivity(T0), start(1), T0)
  assert.equal(nextTickAt(s), T0 + STALE_IN_FLIGHT_MS)
  assert.equal(onTick(s, T0 + STALE_IN_FLIGHT_MS - 1).shown.phase, "in")
  const done = onTick(s, T0 + STALE_IN_FLIGHT_MS)
  assert.equal(done.shown, null)
  assert.equal(done.inFlight.length, 0)
})

test("并发时显示最新开始的一条；结果停留结束后回到仍在进行的那条", () => {
  let s = onStart(emptyActivity(T0), start(1, "codex"), T0)
  s = onStart(s, start(2, "opencode"), T0 + 100)
  assert.equal(s.shown.requestId, 2)
  assert.equal(s.inFlight.length, 2)
  s = onEnd(s, { ...end(2), agent: "opencode" }, T0 + 1000)
  assert.equal(s.shown.requestId, 2)
  assert.equal(s.shown.phase, "ok")
  s = onTick(s, T0 + 1000 + OK_HOLD_MS)
  assert.equal(s.shown.requestId, 1, "回到进行中的请求")
  assert.equal(s.shown.phase, "in")
})

test("重复的开始事件不重复计入", () => {
  const s = onStart(emptyActivity(T0), start(1), T0)
  assert.equal(onStart(s, start(1), T0 + 10), s)
})

test("今日统计跨零点重新计数", () => {
  const late = new Date(2026, 10, 4, 23, 59, 59).getTime()
  let s = onEnd(emptyActivity(late), end(1), late)
  assert.equal(s.today.ok, 1)
  const next = late + 2000
  s = onTick(s, next)
  assert.equal(s.today.day, dayKey(next))
  assert.equal(s.today.ok, 0)
})

test("从流水恢复最近去向与今日统计；未知 Agent 不计入去向", () => {
  const rows = [
    { ts: T0 + 2, agent: "codex", requested_model: "zhipu/glm-4.6", source: "zhipu", upstream_model: "glm-4.6", status_code: 200, result: "ok", duration_ms: 800 },
    { ts: T0 + 1, agent: null, requested_model: "x/y", source: null, upstream_model: null, status_code: 404, result: "rejected", duration_ms: 2 },
    { ts: T0, agent: "codex", requested_model: "codex/gpt-5", source: "codex", upstream_model: null, status_code: 502, result: "failed", duration_ms: 1400 },
    { ts: T0 - 86_400_000, agent: "opencode", requested_model: "a/b", source: "a", upstream_model: null, status_code: 200, result: "ok", duration_ms: 10 },
  ]
  const s = activityFromLog(rows, T0 + 10)
  assert.deepEqual(s.today, { day: dayKey(T0), ok: 1, rejected: 1, failed: 1 })
  assert.equal(s.lastByAgent.codex.source, "zhipu", "按时间取最新一次")
  assert.equal(s.lastByAgent.codex.model, "glm-4.6")
  assert.equal(s.lastByAgent.opencode.ok, true)
  assert.equal(Object.keys(s.lastByAgent).length, 2)
})

test("被拒原因按状态码映射", () => {
  assert.equal(rejectReason(429, "rejected"), "冷却中")
  assert.equal(rejectReason(401, "rejected"), "凭据已失效")
  assert.equal(rejectReason(402, "rejected"), "额度已用尽")
  assert.equal(rejectReason(503, "rejected"), "已暂停")
  assert.equal(rejectReason(null, "failed"), "上游不可达")
  assert.equal(rejectReason(502, "failed"), "上游返回 502")
})

test("耗时与模型名格式", () => {
  assert.equal(formatDuration(800), "800ms")
  assert.equal(formatDuration(6000), "6.0s")
  assert.equal(formatDuration(null), "—")
  assert.equal(modelPart("codex/gpt-5", "codex"), "gpt-5")
  assert.equal(modelPart("gpt-5", null), "gpt-5")
})

test("合并快照：快照之后结束的实时请求补加到今日统计，去向取较新", async () => {
  const { mergeSnapshot } = await import("./gatewayActivity.ts")
  let live = onEnd(emptyActivity(T0), end(1), T0 + 50)
  const snapshot = {
    lastByAgent: { opencode: { source: "old", model: "m", ts: T0, ok: true, durationMs: 1, statusCode: 200 }, codex: { source: "zhipu", model: "glm", ts: T0, ok: true, durationMs: 1, statusCode: 200 } },
    today: { day: dayKey(T0), ok: 40, rejected: 2, failed: 1 },
  }
  const merged = mergeSnapshot(live, snapshot, { ok: 1, rejected: 0, failed: 0 })
  assert.deepEqual(merged.today, { day: dayKey(T0), ok: 41, rejected: 2, failed: 1 })
  assert.equal(merged.lastByAgent.opencode.source, "codex", "实时事件比快照新")
  assert.equal(merged.lastByAgent.codex.source, "zhipu", "快照补齐实时没有的 Agent")
  assert.equal(merged.shown.phase, "ok", "保留正在显示的结果")
})
