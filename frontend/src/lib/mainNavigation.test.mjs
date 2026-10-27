import assert from "node:assert/strict"
import test from "node:test"
import {
  PRIMARY_PAGES,
  SIDEBAR_KEY,
  pageForIntent,
  pageLabel,
  readSidebarCollapsed,
  resolveOpenPage,
  writeSidebarCollapsed,
} from "./mainNavigation.ts"

test("签到在供应商同级下方，与 Jolu.pen 模块 I 导航一致", () => {
  assert.deepEqual(PRIMARY_PAGES.map((item) => item.label), ["灵动岛", "供应商", "一键签到", "用量统计", "会话线程", "自动化", "Skills 管理"])
  assert.equal(resolveOpenPage("checkin", null), "checkin")
  assert.equal(resolveOpenPage("last", "checkin"), "checkin")
  assert.equal(pageLabel("settings"), "设置")
})

test("托盘意图换算成页面", () => {
  assert.equal(pageForIntent({ kind: "overview" }), "usage")
  assert.equal(pageForIntent({ kind: "settings", section: "island" }), "island")
  assert.equal(pageForIntent({ kind: "settings", section: "connections" }), "providers")
  assert.equal(pageForIntent({ kind: "settings", section: "general" }), "settings")
})

test("侧栏收起状态读写，存储不可用时按展开处理", () => {
  const map = new Map()
  const storage = { getItem: (key) => map.get(key) ?? null, setItem: (key, value) => map.set(key, value) }
  assert.equal(readSidebarCollapsed(storage), false)
  writeSidebarCollapsed(true, storage)
  assert.equal(map.get(SIDEBAR_KEY), "1")
  assert.equal(readSidebarCollapsed(storage), true)
  const broken = { getItem: () => { throw new Error("blocked") }, setItem: () => { throw new Error("blocked") } }
  assert.equal(readSidebarCollapsed(broken), false)
  assert.doesNotThrow(() => writeSidebarCollapsed(true, broken))
})

test("resolveOpenPage", () => {
  assert.equal(resolveOpenPage("last", "skills"), "skills")
  assert.equal(resolveOpenPage("last", null), "usage")
  assert.equal(resolveOpenPage("last", "bogus"), "usage")
  assert.equal(resolveOpenPage("sessions", "skills"), "sessions")
  assert.equal(resolveOpenPage("bogus", null), "usage")
})

test("托盘打开主窗口按默认打开页面设置换算", () => {
  assert.equal(pageForIntent({ kind: "overview", settings: { default_page: "last", last_page: "automation" } }), "automation")
  assert.equal(pageForIntent({ kind: "overview", settings: { default_page: "providers", last_page: "automation" } }), "providers")
  assert.equal(pageForIntent({ kind: "overview", settings: { default_page: "last", last_page: null } }), "usage")
})

test("点击任务通知打开自动化页", () => {
  assert.equal(pageForIntent({ kind: "automation", task_id: 24 }), "automation")
})
test("点击余额通知打开供应商账号管理", () => {
  assert.equal(pageForIntent({ kind: "providers", connection_id: "synthetic-api" }), "providers")
})
