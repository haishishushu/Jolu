import assert from "node:assert/strict"
import test from "node:test"
import {
  groupByProject, listTime, middleEllipsis, projectName, sessionTotals, sinceFor, spanLabel, statusLabel, visibleRows,
} from "./sessionThreads.ts"

const now = new Date(2026, 9, 21, 12, 40)
const at = (d, h, m) => new Date(2026, 9, d, h, m).getTime()
const item = (id, cwd, last, tokens = [10, 5, 1, 2]) => ({
  platform: "claude", session_id: id, title: null, cwd, first_ts: last - 60_000, last_ts: last, requests: 1,
  fresh_input: tokens[0], cache_read: tokens[1], cache_write: tokens[2], output: tokens[3], connection_id: null, status: "done", has_source: true,
})

test("项目名与分组：未识别项目放最后", () => {
  assert.equal(projectName("D:/develop/jolu"), "jolu")
  assert.equal(projectName(String.raw`C:\work\jolu-docs\ `.trim()), "jolu-docs")
  assert.equal(projectName(null), "未识别项目")
  const groups = groupByProject([item("a", null, at(21, 12, 0)), item("b", "D:/x/old", at(19, 9, 0)), item("c", "D:/x/new", at(21, 11, 0))])
  assert.deepEqual(groups.map((g) => g.name), ["new", "old", "未识别项目"])
})

test("时间文案", () => {
  assert.equal(listTime(at(21, 12, 38), now), "12:38")
  assert.equal(listTime(at(20, 9, 0), now), "昨天")
  assert.equal(listTime(at(19, 9, 0), now), "10/19")
  assert.equal(spanLabel(at(21, 12, 12), at(21, 12, 38), now), "今天 12:12—12:38")
  assert.equal(spanLabel(at(19, 23, 0), at(20, 1, 5), now), "10/19 23:00—昨天 01:05")
})

test("状态、省略与时间筛选", () => {
  assert.equal(statusLabel("recent"), "最近活跃")
  assert.equal(statusLabel("usage_only"), "仅用量")
  assert.equal(middleEllipsis("session_demo_82acxxxxxxxx91f0"), "session_demo…91f0")
  assert.equal(sinceFor("all", now), null)
  assert.equal(sinceFor("today", now), new Date(2026, 9, 21).getTime())
})

test("顶部统计：输入含缓存，缺字段显示未知", () => {
  const t = sessionTotals([item("a", null, 1, [10, 30, 0, 5]), item("b", null, 2, [0, 10, 0, 5])])
  assert.equal(t.input, 50)
  assert.equal(t.cache, 40)
  assert.equal(t.consumption, 60)
  assert.equal(t.hitRate, 0.8)
  assert.equal(sessionTotals([item("c", null, 1, [null, 1, 1, 1])]).input, null)
  assert.equal(sessionTotals([]).input, 0)
})

test("顶部统计：部分会话 Token 未知时只汇总已知部分并计数", () => {
  const t = sessionTotals([item("a", null, 1, [10, 30, 0, 5]), item("b", null, 2, [null, null, null, null])])
  assert.equal(t.input, 40)
  assert.equal(t.consumption, 45)
  assert.equal(t.unknown, 1)
})

test("项目分组不区分盘符大小写与分隔符", () => {
  const groups = groupByProject([item("a", String.raw`d:\develop\jolu`, 2), item("b", "D:/develop/jolu/", 1)])
  assert.equal(groups.length, 1)
  assert.equal(groups[0].sessions.length, 2)
  assert.equal(groups[0].path, String.raw`d:\develop\jolu`)
})

test("键盘导航只包含展开项目的会话", () => {
  const groups = groupByProject([item("a", "D:/p", 2), item("b", "D:/q", 1)])
  assert.equal(visibleRows(groups, new Set(["d:/p"])).length, 3)
})
