import assert from "node:assert/strict"
import test from "node:test"
import {
  PROVIDERS,
  canConnect,
  connectionBadge,
  defaultMode,
  filterModels,
  providerEntry,
  showsModeTabs,
  singleModeReason,
  validateConnectionName,
} from "./providerCatalog.ts"

test("供应商顺序与 Jolu.pen 画板 05 一致，Trae 补在最后", () => {
  assert.deepEqual(PROVIDERS.map((p) => p.name), ["Claude Code", "Codex", "Gemini", "Grok", "OpenCode", "ZCode", "Qoder", "WorkBuddy", "Trae"])
})

test("连接方式：两种都实现才显示页签，待接入不能连接", () => {
  assert.equal(showsModeTabs(providerEntry("claude")), true)
  assert.equal(showsModeTabs(providerEntry("zcode")), false)
  assert.equal(defaultMode(providerEntry("zcode")), "subscription")
  assert.match(singleModeReason(providerEntry("zcode")), /待接入/)
  assert.match(singleModeReason(providerEntry("trae")), /暂不支持 API Key/)
  assert.equal(singleModeReason(providerEntry("codex")), null)
  assert.equal(canConnect(providerEntry("opencode")), false)
  assert.equal(canConnect(providerEntry("trae")), true)
  assert.equal(providerEntry("gemini").baseUrl.locked, true)
})

test("数量徽标", () => {
  assert.equal(connectionBadge(0), "尚未连接")
  assert.equal(connectionBadge(2), "2 个账号")
})

test("连接名称去空白、同平台同方式不得重名", () => {
  const existing = [{ platformId: "codex", kind: "api", baseName: "工作 API" }]
  assert.equal(validateConnectionName("  ", existing, "codex", "api"), "请填写连接名称")
  assert.match(validateConnectionName(" 工作 API ", existing, "codex", "api"), /同名/)
  assert.equal(validateConnectionName("工作 API", existing, "codex", "auth"), null)
  assert.equal(validateConnectionName("工作 API", existing, "claude", "api"), null)
})

test("模型搜索不区分大小写并保留顺序", () => {
  const models = ["coding-main", "Coding-Fast", "reasoning-pro"]
  assert.deepEqual(filterModels(models, "coding"), ["coding-main", "Coding-Fast"])
  assert.deepEqual(filterModels(models, " "), models)
})
