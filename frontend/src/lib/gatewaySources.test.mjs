import { test } from "node:test"
import assert from "node:assert/strict"

import { batchTargets, capabilityLine, filterSources, inScope, pruneSelection, serveCounts, usageLine } from "./gatewaySources.ts"

const sources = [
  { id: "a", baseName: "OpenAI · 工作", platformId: "codex", kind: "auth", slug: "openai-work", serve_enabled: true },
  { id: "b", baseName: "Kimi 团队", platformId: "custom", kind: "api", slug: "kimi", serve_enabled: false },
  { id: "c", baseName: "智谱", platformId: "zcode", kind: "api", slug: "glm", serve_enabled: true },
]
const groups = [{ id: "g1", name: "常用", sort: 0, members: ["c", "a"] }]

test("分组范围：全部 / 未分组 / 分组按保存顺序", () => {
  assert.equal(inScope(sources, groups, { kind: "all" }).length, 3)
  assert.deepEqual(inScope(sources, groups, { kind: "ungrouped" }).map((s) => s.id), ["b"])
  assert.deepEqual(inScope(sources, groups, { kind: "group", id: "g1" }).map((s) => s.id), ["c", "a"])
  assert.deepEqual(inScope(sources, groups, { kind: "group", id: "missing" }), [])
})

test("搜索名称、菜单 id 与平台，并按已接入 / 未接入筛选", () => {
  assert.deepEqual(filterSources(sources, "KIMI", "all").map((s) => s.id), ["b"])
  assert.deepEqual(filterSources(sources, "glm", "all").map((s) => s.id), ["c"])
  assert.deepEqual(filterSources(sources, "", "on").map((s) => s.id), ["a", "c"])
  assert.deepEqual(filterSources(sources, "", "off").map((s) => s.id), ["b"])
  assert.deepEqual(serveCounts(sources), { all: 3, on: 2, off: 1 })
})

test("选择集随筛选裁剪；批量只改需要变化的来源", () => {
  const selected = new Set(["a", "b", "x"])
  assert.deepEqual([...pruneSelection(selected, sources)], ["a", "b"])
  assert.deepEqual(batchTargets(sources, selected, true), ["b"])
  assert.deepEqual(batchTargets(sources, selected, false), ["a"])
})

test("调用能力与最近使用文案", () => {
  assert.equal(capabilityLine(undefined, false).text, "正在检查调用能力")
  assert.deepEqual(capabilityLine({ connection_id: "a", can_serve: false, state: "needs_auth", reason: "" }, true), { text: "需要重新登录", tone: "danger" })
  assert.equal(capabilityLine({ connection_id: "a", can_serve: false, state: "unsupported", reason: "" }, true).text, "已选择 · 转发适配尚未就绪")
  assert.equal(capabilityLine({ connection_id: "a", can_serve: true, state: "ready", reason: "" }, true).tone, "ok")
  assert.equal(usageLine(undefined, 0), "暂无经网关的请求")
  assert.equal(usageLine({ last_ts: 0, requests: 12, failed: 1 }, 5 * 60_000), "最近使用 5 分钟前 · 12 次（失败 1）")
})
