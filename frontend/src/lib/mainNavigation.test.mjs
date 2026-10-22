import assert from "node:assert/strict"
import test from "node:test"
import {
  PRIMARY_PAGES,
  SIDEBAR_KEY,
  pageForIntent,
  pageLabel,
  readSidebarCollapsed,
  writeSidebarCollapsed,
} from "./mainNavigation.ts"

test("侧栏模块顺序与 Jolu.pen 画板 01 一致", () => {
  assert.deepEqual(PRIMARY_PAGES.map((item) => item.label), ["灵动岛", "供应商", "用量统计", "会话线程", "自动化", "Skills 管理"])
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
