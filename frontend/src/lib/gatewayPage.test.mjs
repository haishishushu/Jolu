import { test } from "node:test"
import assert from "node:assert/strict"

import {
  agentLabel, canEscape, canServe, circuitText, gatewayRunLine, nextMode, wiredCount, wiringLine,
} from "./gatewayPage.ts"

const baseStatus = {
  mode: "relay",
  enabled: true,
  port: 12731,
  circuit_threshold: 5,
  auto_escape: true,
  default_connection: null,
  wiring: [
    { agent: "claude-code", wired: true, config_path: "/c/settings.json", notice: null, reason: null },
    { agent: "codex", wired: false, config_path: "/c/config.toml", notice: null, reason: null },
    { agent: "zcode", wired: false, config_path: null, notice: null, reason: "尚未安装或未生成配置文件" },
  ],
  status: undefined,
}

test("运行状态行区分启用、未启动与未知模式", () => {
  assert.equal(gatewayRunLine(null).text, "读取中…")
  assert.equal(gatewayRunLine(baseStatus).text, "网关运行中 · 127.0.0.1:12731")
  assert.equal(gatewayRunLine(baseStatus).tone, "on")

  const stopped = { ...baseStatus, enabled: false }
  assert.equal(gatewayRunLine(stopped).tone, "danger")
  assert.match(gatewayRunLine(stopped).text, /监听未启动/)

  const observe = { ...baseStatus, mode: "observe" }
  assert.match(gatewayRunLine(observe).text, /只记录/)

  const unknown = { ...baseStatus, mode: "weird" }
  assert.equal(gatewayRunLine(unknown).tone, "danger")
  assert.match(gatewayRunLine(unknown).text, /未知模式/)
})

test("接线行把原因当作独立状态而不是笼统未接入", () => {
  assert.deepEqual(wiringLine(baseStatus.wiring[0]), { text: "已接入网关", tone: "on" })
  assert.deepEqual(wiringLine(baseStatus.wiring[1]), { text: "未接入", tone: "muted" })
  assert.deepEqual(wiringLine(baseStatus.wiring[2]), {
    text: "尚未安装或未生成配置文件",
    tone: "muted",
  })
})

test("已接入计数与模式切换目标", () => {
  assert.equal(wiredCount(null), 0)
  assert.equal(wiredCount(baseStatus), 1)
  assert.equal(nextMode(baseStatus), "observe")
  assert.equal(nextMode({ ...baseStatus, mode: "observe" }), "relay")
  assert.equal(nextMode(null), "relay")
})

test("逃生门只在 relay 模式可用", () => {
  assert.equal(canEscape(baseStatus), true)
  assert.equal(canEscape({ ...baseStatus, mode: "observe" }), false)
  assert.equal(canEscape(null), false)
})

test("关闭自动逃生时文案必须说明只能手动切回", () => {
  assert.equal(circuitText(baseStatus), "连续失败 5 次后自动切回直连")
  const manual = circuitText({ ...baseStatus, auto_escape: false })
  assert.match(manual, /已关闭自动逃生/)
  assert.match(manual, /只能手动切回/)
})

test("模型池准入与后端 set_serve 保持一致", () => {
  assert.equal(canServe("custom", "api"), true)
  assert.equal(canServe("claude", "api"), true)
  assert.equal(canServe("claude", "auth"), true)
  assert.equal(canServe("codex", "auth"), true)
  // 无适配器的平台订阅不能开启
  assert.equal(canServe("gemini", "auth"), false)
  assert.equal(canServe("qoder", "auth"), false)
})

test("工具展示名回退到原始 id 而不是空", () => {
  assert.equal(agentLabel("claude-code"), "Claude Code")
  assert.equal(agentLabel("zcode"), "ZCode")
  assert.equal(agentLabel("unknown-agent"), "unknown-agent")
})
