import assert from "node:assert/strict"
import test from "node:test"
import {
  PROVIDERS,
  canConnect,
  canStartVerify,
  canEnterManualModel,
  connectionBadge,
  defaultMode,
  editSaveState,
  fallsBackToManualModel,
  filterModels,
  modelBusyAfter,
  modelFetchErrorText,
  protocolBaseUrlHint,
  protocolLabel,
  providerEntry,
  requestGate,
  showsModeTabs,
  singleModeReason,
  validateConnectionName,
} from "./providerCatalog.ts"

test("供应商顺序与 Jolu.pen 画板 05 一致：自定义供应商在最前，Trae 补在最后", () => {
  assert.deepEqual(PROVIDERS.map((p) => p.name), ["自定义供应商", "Claude Code", "Codex", "Gemini", "Grok", "OpenCode", "ZCode", "Qoder", "WorkBuddy", "Trae"])
})

test("连接方式：两种都实现才显示页签，待接入不能连接", () => {
  assert.equal(showsModeTabs(providerEntry("claude")), true)
  assert.equal(showsModeTabs(providerEntry("zcode")), true)
  assert.equal(defaultMode(providerEntry("zcode")), "subscription")
  assert.equal(singleModeReason(providerEntry("zcode")), null)
  assert.match(singleModeReason(providerEntry("trae")), /暂不支持 API Key/)
  assert.equal(singleModeReason(providerEntry("codex")), null)
  assert.equal(canConnect(providerEntry("opencode")), true)
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

test("自定义供应商只有 API Key 方式，可以连接且不显示方式页签", () => {
  const custom = providerEntry("custom")
  assert.equal(custom.id, "custom")
  assert.equal(canConnect(custom), true)
  assert.equal(showsModeTabs(custom), false)
  assert.equal(defaultMode(custom), "apiKey")
  assert.equal(singleModeReason(custom), null)
  assert.equal(custom.baseUrl, undefined)
})

test("自定义供应商协议：Base URL 示例与显示名", () => {
  assert.equal(protocolLabel("openai"), "OpenAI 兼容")
  assert.equal(protocolLabel("anthropic"), "Anthropic 兼容")
  assert.equal(protocolLabel(null), "未记录协议")
  assert.match(protocolBaseUrlHint("openai"), /\/v1$/)
  assert.doesNotMatch(protocolBaseUrlHint("anthropic"), /\/v1$/)
})

test("编辑自定义供应商：更换密钥后必须重新获取模型或验证手填模型才能保存（画板 27）", () => {
  const base = { platformId: "custom", kind: "api", nameChanged: false, nameValid: true, newSecret: "", model: "", modelReady: false }
  // 没有任何改动不能保存；只改名称无需重新验证
  assert.equal(editSaveState(base).canSave, false)
  assert.deepEqual(editSaveState({ ...base, nameChanged: true }), { canSave: true, needsModel: false, reason: null })
  // 更换密钥：需要模型，且模型须由新密钥获取或验证过
  const replaced = { ...base, newSecret: "sk-new" }
  assert.equal(editSaveState(replaced).needsModel, true)
  assert.equal(editSaveState(replaced).canSave, false)
  assert.match(editSaveState(replaced).reason, /重新获取模型/)
  assert.equal(editSaveState({ ...replaced, model: "m-1" }).canSave, false)
  assert.equal(editSaveState({ ...replaced, model: "m-1", modelReady: true }).canSave, true)
  assert.equal(editSaveState({ ...replaced, model: "  ", modelReady: true }).canSave, false)
  // 名称无效时一律不能保存
  assert.equal(editSaveState({ ...replaced, model: "m-1", modelReady: true, nameValid: false }).canSave, false)
  // 其他平台更换密钥仍由后端验证，不要求重新获取模型
  assert.deepEqual(editSaveState({ ...base, platformId: "codex", newSecret: "sk-new" }), { canSave: true, needsModel: false, reason: null })
})

test("自定义供应商获取模型失败：接口不可用时改为手填，密钥被拒时不改（画板 29A 状态 A / 画板 27）", () => {
  assert.equal(fallsBackToManualModel("获取模型失败：HTTP 404 Not Found，请检查 Key 与地址"), true)
  assert.equal(fallsBackToManualModel("网关返回了空模型列表"), true)
  assert.equal(fallsBackToManualModel("无法连接网关：error sending request"), true)
  assert.equal(fallsBackToManualModel("获取模型失败：HTTP 401 Unauthorized，请检查 Key 与地址"), false)
  assert.equal(fallsBackToManualModel("获取模型失败：HTTP 403 Forbidden，请检查 Key 与地址"), false)
  assert.equal(modelFetchErrorText("网关返回了空模型列表"), "获取模型失败：网关返回了空模型列表")
  assert.equal(modelFetchErrorText("获取模型失败：HTTP 404 Not Found"), "获取模型失败：HTTP 404 Not Found")
})

test("请求闸门：新请求或失效后，旧请求的结果一律丢弃（换 Key 期间的验证不能标记新 Key）", () => {
  const gate = requestGate()
  const verifyWithOldKey = gate.begin()
  assert.equal(gate.isCurrent(verifyWithOldKey), true)
  gate.invalidate() // 用户改了 Key
  assert.equal(gate.isCurrent(verifyWithOldKey), false)
  const fetchA = gate.begin()
  const fetchB = gate.begin()
  assert.equal(gate.isCurrent(fetchA), false)
  assert.equal(gate.isCurrent(fetchB), true)
})

test("「手动填写模型」入口：自定义供应商 401 / 403 时不提供，其他平台照旧", () => {
  assert.equal(canEnterManualModel("custom", "获取模型失败：HTTP 401 Unauthorized，请检查 Key 与地址"), false)
  assert.equal(canEnterManualModel("custom", "获取模型失败：HTTP 403 Forbidden，请检查 Key 与地址"), false)
  assert.equal(canEnterManualModel("custom", "网关返回了空模型列表"), true)
  assert.equal(canEnterManualModel("codex", "获取模型失败：HTTP 401 Unauthorized，请检查 Key 与地址"), true)
})

test("模型区忙碌状态：获取与验证互斥，过期请求结束不改状态，任何顺序都不会残留进行中", () => {
  // 复现审查步骤：手填状态下先获取，未返回时开始验证
  let busy = "idle"
  busy = modelBusyAfter(busy, { type: "start", kind: "fetching" })
  assert.equal(canStartVerify(busy), false) // ① 获取进行中不能验证
  busy = modelBusyAfter(busy, { type: "start", kind: "verifying" }) // ② 即使绕过也只保留一种进行中状态
  assert.equal(busy, "verifying")
  busy = modelBusyAfter(busy, { type: "end", current: false }) // 过期的获取结束
  assert.equal(busy, "verifying")
  busy = modelBusyAfter(busy, { type: "end", current: true }) // 验证结束
  assert.equal(busy, "idle")
  assert.equal(canStartVerify(busy), true)
  // 反向：验证中重新获取，过期的验证结束不影响获取
  busy = modelBusyAfter(modelBusyAfter("idle", { type: "start", kind: "verifying" }), { type: "start", kind: "fetching" })
  assert.equal(modelBusyAfter(busy, { type: "end", current: false }), "fetching")
  assert.equal(canStartVerify("verifying"), false)
})

test("Qoder 与 WorkBuddy 密钥在原工具配置",()=>{for(const id of ["qoder","workbuddy"]){const p=providerEntry(id);assert.equal(p.apiKey,"none");assert.equal(singleModeReason(p),`${p.name} 的自定义模型在 ${p.name} 客户端内配置，Jolu 不代管密钥。`)}})
test("ZCode 四种官方地址与鉴权协议匹配",()=>{const p=providerEntry("zcode");assert.equal(p.baseUrlChoices.length,4);for(const c of p.baseUrlChoices)assert.equal(c.protocol,c.value.endsWith("anthropic")?"anthropic":"openai")})
