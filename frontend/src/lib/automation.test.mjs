import assert from "node:assert/strict"
import test from "node:test"
import {
  blockBanner, clock, countByFilter, dayTime, durationText, filterTasks, launchLabel, nextFire, quotaTip, runCode, runTokens,
  sessionTaskName, taskCode, taskStatus, untilText, validateForm,
} from "./automation.ts"

const now = new Date(2026, 9, 22, 16, 48).getTime()
const at = (d, h, m) => new Date(2026, 9, d, h, m).getTime()

const run = (patch = {}) => ({
  id: 91, task_id: 24, task_name: "完善登录页面", platform: "claude", connection_id: "c1", source: "now", status: "queued",
  block_kind: null, reason: null, resets_at: null, last_check_at: null, next_check_at: null, check_failures: 0, resume_pending: false,
  session_id: null, account: "Momo", model: null, created_at: at(22, 16, 2), started_at: null, ended_at: null, exec_ms: 0, wait_ms: 0,
  exec_started_at: null, wait_started_at: null, waits: 0, input_tokens: null, cache_read_tokens: null, output_tokens: null,
  result: null, files: [], error: null, ...patch,
})

const task = (patch = {}) => ({
  id: 24, name: "完善登录页面", platform: "claude", connection_id: "c1", model: null, cwd: "D:/develop/jolu", session_id: null,
  prompt: "完善表单", skills: [], launch: "now", schedule: null, auto_resume: true, wait_deadline: null, state: "active",
  next_run_at: null, missed_at: null, created_at: at(22, 16, 2), updated_at: at(22, 16, 2), active: null, last: null, queue_ahead: null, ...patch,
})

test("时间与时长文案", () => {
  assert.equal(dayTime(at(22, 18, 30), now), "今天 18:30")
  assert.equal(dayTime(at(23, 9, 0), now), "明天 09:00")
  assert.equal(dayTime(at(24, 9, 0), now), "10/24 09:00")
  assert.equal(clock(272_000), "04:32")
  assert.equal(clock(8_043_000), "02:14:03")
  assert.equal(clock(0), "—")
  assert.equal(durationText(1_122_000), "18 分 42 秒")
  assert.equal(durationText(8_040_000), "2 时 14 分")
  assert.equal(untilText(at(22, 18, 30), now), "约 1 小时 42 分钟后")
  assert.equal(taskCode(24), "#AT-024")
  assert.equal(runCode(91), "#RUN-091")
  assert.equal(runTokens({ input_tokens: null, output_tokens: null }), null)
  assert.equal(runTokens({ input_tokens: 46_200, output_tokens: 8_600 }), 54_800)
})

test("定时：每天 / 每周 / 单次", () => {
  const daily = { freq: "daily", time: "09:00", weekday: null, date: null, timezone: "Asia/Shanghai" }
  assert.equal(nextFire(daily, now), at(23, 9, 0))
  assert.equal(nextFire({ ...daily, freq: "weekly", weekday: 1 }, now), at(26, 9, 0))
  assert.equal(nextFire({ ...daily, freq: "once", date: "2026-10-22" }, now), null)
  assert.equal(launchLabel({ launch: "schedule", schedule: daily }), "每天 09:00")
  assert.equal(launchLabel({ launch: "schedule", schedule: { ...daily, freq: "weekly", weekday: 3 } }), "每周三 09:00")
  assert.equal(launchLabel({ launch: "quota", schedule: null }), "额度恢复后")
})

test("任务状态：执行中、等待额度、排队、暂停、已错过与已启用", () => {
  assert.equal(taskStatus(task({ active: run({ status: "running", exec_started_at: now - 272_000 }) }), now).next, "正在执行 · 04:32")
  const waiting = taskStatus(task({ active: run({ status: "waiting", block_kind: "quota", resets_at: at(22, 18, 30) }) }), now)
  assert.deepEqual([waiting.label, waiting.next, waiting.action], ["等待额度", "预计今天 18:30", "查看等待"])
  assert.equal(taskStatus(task({ active: run({ status: "waiting", block_kind: "quota_unknown" }) }), now).next, "恢复时间未知")
  assert.equal(taskStatus(task({ active: run(), queue_ahead: 1 }), now).next, "同账号前方 1 个任务")
  assert.equal(taskStatus(task({ state: "paused", active: run({ status: "waiting" }) }), now).label, "已暂停")
  assert.equal(taskStatus(task({ launch: "schedule", missed_at: at(22, 9, 0) }), now).label, "已错过")
  assert.equal(taskStatus(task({ launch: "schedule", next_run_at: at(23, 9, 0) }), now, "上海").next, "明天 09:00 · 上海")
  assert.equal(taskStatus(task({ last: run({ status: "failed", error: "登录失效；请重新登录" }) }), now).next, "登录失效")
  assert.equal(taskStatus(task({ state: "finished", last: run({ status: "completed", ended_at: at(22, 16, 30) }) }), now).label, "已完成")
})

test("任务筛选与计数", () => {
  const items = [
    task({ id: 1, active: run({ status: "running" }) }),
    task({ id: 2, name: "整理项目文档", cwd: "D:/jolu-docs", connection_id: "c2", active: run({ status: "waiting" }) }),
    task({ id: 3, state: "paused" }),
  ]
  assert.deepEqual(countByFilter(items, now), { all: 3, running: 1, waiting: 1, paused: 1, done: 0 })
  assert.deepEqual(filterTasks(items, "waiting", null, "", now).map((t) => t.id), [2])
  assert.deepEqual(filterTasks(items, "all", "c2", "", now).map((t) => t.id), [2])
  assert.deepEqual(filterTasks(items, "all", null, "jolu-docs", now).map((t) => t.id), [2])
})

test("等待提示与额度提示", () => {
  const banner = blockBanner(run({ status: "waiting", block_kind: "quota", resume_pending: true, resets_at: at(22, 18, 30) }), now)
  assert.equal(banner.title, "等待额度 · 已保留任务与进度")
  assert.equal(blockBanner(run({ status: "attention", block_kind: "reauth" }), now).danger, true)
  const tip = quotaTip({
    state: "blocked", reason: null, resets_at: at(22, 18, 30),
    windows: [{ key: "5h", name: "5 小时窗口", used_percent: 100, resets_at: at(22, 18, 30), blocking: true }, { key: "7d", name: "7 天窗口", used_percent: 58, resets_at: null, blocking: false }],
  }, "Momo", now)
  assert.equal(tip.title, "等待 5 小时额度恢复")
  assert.equal(tip.body, "Momo 的 5 小时额度已用完，周额度剩余 42%。\n预计今天 18:30 恢复；届时检查实际可用额度后启动。")
  assert.equal(quotaTip({ state: "unsupported", reason: "API Key 不提供订阅额度", resets_at: null, windows: [] }, "Key", now).disabled, true)
})

test("表单校验", () => {
  const form = {
    name: "", platform: "claude", connectionId: "", model: "", cwd: "", sessionId: null, prompt: " ", launch: "schedule",
    freq: "once", time: "09:00", weekday: 1, date: "2026-10-22", timezone: "Asia/Shanghai", autoResume: true, deadline: "",
  }
  const e = validateForm(form, now)
  assert.equal(e.connectionId, "请选择账号后再创建任务。")
  assert.equal(e.time, "单次执行时间已经过去")
  assert.ok(e.name && e.cwd && e.prompt)
  assert.deepEqual(validateForm({ ...form, name: "a", connectionId: "c1", cwd: "D:/x", prompt: "b", launch: "now" }, now), {})
  assert.equal(validateForm({ ...form, name: "a", connectionId: "c1", cwd: "D:/x", prompt: "b", launch: "quota" }, now, "不支持读取额度").launch, "不支持读取额度")
  assert.equal(sessionTaskName("优化 Jolu 的会话界面"), "继续开发：优化 Jolu 的会话界面")
})
