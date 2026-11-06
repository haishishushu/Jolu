import assert from "node:assert/strict"
import test from "node:test"
import {
  DEFAULT_ACTIVE_WINDOW_SECONDS,
  FUTURE_TOLERANCE_MS,
  islandWriting,
  lastWriteAt,
} from "./islandWriting.ts"

const NOW = 1_800_000_000_000
/** 会话列表：`state` + 最近写入时刻（后端 last_seen_ms） */
const session = (state, agoMs = null) => ({
  state,
  updatedAtMs: agoMs == null ? null : NOW - agoMs,
})
const work = (sessions, overrides = {}) => islandWriting({
  sessions,
  recentWindow: true,
  activeWindowSeconds: DEFAULT_ACTIVE_WINDOW_SECONDS,
  inFlight: 0,
  now: NOW,
  ...overrides,
})

test("写入信号：有 state === running 的会话就算在写（codex / claude 生命周期来源）", () => {
  const result = work([session("running")], { recentWindow: false })
  assert.equal(result.working, true)
  assert.equal(result.running, 1)
  assert.equal(result.recentWrite, false)
  assert.equal(result.gatewayInFlight, false)
})

test("写入信号：最近活跃来源没有 running 行时，窗口内仍有写入就算在写（ZCode 真实情形）", () => {
  // 实测 ZCode 的 model_usage 只有 completed / error / cancelled，state 永远不是 running
  const result = work([session("done", 5_000)])
  assert.equal(result.running, 0)
  assert.equal(result.recentWrite, true)
  assert.equal(result.working, true)
})

test("写入信号：最近活跃窗口内没有新写入就不再算工作中", () => {
  const result = work([session("done", (DEFAULT_ACTIVE_WINDOW_SECONDS + 1) * 1_000)])
  assert.equal(result.recentWrite, false)
  assert.equal(result.working, false)
})

test("写入信号：窗口边界上的写入仍算在写，越界一毫秒就不算", () => {
  assert.equal(work([session("done", DEFAULT_ACTIVE_WINDOW_SECONDS * 1_000)]).working, true)
  assert.equal(work([session("done", DEFAULT_ACTIVE_WINDOW_SECONDS * 1_000 + 1)]).working, false)
})

test("写入信号：来自生命周期状态时，最近活跃不算工作（不把 done 当 running）", () => {
  const result = work([session("done", 1_000), session("failed", 2_000)], { recentWindow: false })
  assert.equal(result.working, false)
  assert.equal(result.recentWrite, false)
})

test("写入信号：网关请求在途单独就能驱动 working（会话列表为空、采集源出错也一样）", () => {
  const result = work([], { recentWindow: false, inFlight: 1 })
  assert.equal(result.gatewayInFlight, true)
  assert.equal(result.working, true)
})

test("写入信号：没有会话、没有在途请求时不算工作中", () => {
  const result = work([])
  assert.deepEqual(
    { working: result.working, running: result.running, recentWrite: result.recentWrite, gatewayInFlight: result.gatewayInFlight },
    { working: false, running: 0, recentWrite: false, gatewayInFlight: false },
  )
  assert.equal(result.lastWriteMs, null)
})

test("写入信号：未来时间戳超出容差不算写入，容差内才算", () => {
  const future = (ms) => islandWriting({
    sessions: [{ state: "done", updatedAtMs: NOW + ms }],
    recentWindow: true,
    activeWindowSeconds: DEFAULT_ACTIVE_WINDOW_SECONDS,
    inFlight: 0,
    now: NOW,
  })
  assert.equal(future(FUTURE_TOLERANCE_MS).working, true)
  assert.equal(future(FUTURE_TOLERANCE_MS + 1).working, false)
})

test("写入信号：active_window_seconds 为 0 时退回默认 90 秒窗口", () => {
  const result = islandWriting({
    sessions: [session("done", 60_000)],
    recentWindow: true,
    activeWindowSeconds: 0,
    inFlight: 0,
    now: NOW,
  })
  assert.equal(result.recentWrite, true)
})

test("lastWriteAt：取最新的会话时刻，时刻未知的会话不参与", () => {
  assert.equal(lastWriteAt([session("done", 9_000), session("done", 1_000), session("done")]), NOW - 1_000)
  assert.equal(lastWriteAt([session("done")]), null)
  assert.equal(lastWriteAt([]), null)
})
