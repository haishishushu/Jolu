import assert from "node:assert/strict"
import test from "node:test"
import { PROXY_URL_HINT, diagnosisSummary, isNetworkDraft, latencyText, networkStatusLine, normalizeNetworkMode, probeStatus, proxyUrlError } from "./networkSettings.ts"

const saved = (mode, url = null) => ({ network_mode: mode, network_proxy_url: url })

test("联网方式只接受三种取值，其余按跟随系统", () => {
  assert.equal(normalizeNetworkMode("direct"), "direct")
  assert.equal(normalizeNetworkMode("manual"), "manual")
  assert.equal(normalizeNetworkMode("pac"), "system")
  assert.equal(normalizeNetworkMode(""), "system")
})

test("代理地址规则与后端一致：只收 http / https、必须带端口", () => {
  assert.equal(proxyUrlError("http://127.0.0.1:7890"), null)
  assert.equal(proxyUrlError(" https://proxy.test:443/ "), null)
  assert.equal(proxyUrlError("http://[::1]:7890"), null)
  for (const raw of ["socks5://x:1", "127.0.0.1", "http://host", "", "http://host:0", "http://host:70000", "http://user:pw@host:1", "http://host:1/path"]) {
    assert.equal(proxyUrlError(raw), PROXY_URL_HINT, raw)
  }
})

test("状态行：三种方式各自的文案，未保存的手动代理提示草稿", () => {
  assert.equal(networkStatusLine("system", saved("system"), ""), "使用系统当前网络设置")
  assert.equal(networkStatusLine("direct", saved("direct"), ""), "不使用代理，直接连接")
  assert.equal(networkStatusLine("manual", saved("system"), "http://127.0.0.1:7890"), "代理地址草稿 · 点击保存后生效")
  assert.equal(networkStatusLine("manual", saved("manual", "http://127.0.0.1:7890"), "http://127.0.0.1:7891"), "代理地址草稿 · 点击保存后生效")
  assert.equal(networkStatusLine("manual", saved("manual", "http://127.0.0.1:7890"), "http://127.0.0.1:7890"), "已通过 http://127.0.0.1:7890 连接")
})

test("草稿判断：只有手动代理会产生未保存的草稿", () => {
  assert.equal(isNetworkDraft("system", "", saved("system")), false)
  assert.equal(isNetworkDraft("manual", "http://a:1", saved("system")), true)
  assert.equal(isNetworkDraft("manual", " http://a:1 ", saved("manual", "http://a:1")), false)
  assert.equal(isNetworkDraft("manual", "http://a:2", saved("manual", "http://a:1")), true)
})

test("延迟缺失显示「—」，有值显示毫秒", () => {
  assert.equal(latencyText({ ok: false, latency_ms: null, error: "无法建立连接", skipped: false }), "—")
  assert.equal(latencyText({ ok: true, latency_ms: 0, error: null, skipped: false }), "0 ms")
  assert.equal(latencyText({ ok: true, latency_ms: 128, error: null, skipped: false }), "128 ms")
})

test("单个连接的状态：可达、失败、未检测分开表达", () => {
  assert.deepEqual(probeStatus({ ok: true, latency_ms: 12, error: null, skipped: false }), { tone: "ok", text: "可达" })
  assert.deepEqual(probeStatus({ ok: false, latency_ms: null, error: "连接超时（5 秒）", skipped: false }), { tone: "danger", text: "连接超时（5 秒）" })
  assert.deepEqual(probeStatus({ ok: false, latency_ms: null, error: null, skipped: false }), { tone: "danger", text: "不可达" })
  assert.deepEqual(probeStatus({ ok: false, latency_ms: null, error: "代理不可达，未检测", skipped: true }), { tone: "muted", text: "代理不可达，未检测" })
})

test("诊断摘要：代理、额度接口分别汇总", () => {
  const ok = { ok: true, latency_ms: 30, error: null, skipped: false }
  const bad = { ok: false, latency_ms: null, error: "无法建立连接", skipped: false }
  const skip = { ok: false, latency_ms: null, error: "代理不可达，未检测", skipped: true }
  const row = (probe) => ({ connection_name: "x", platform: "claude", probe })
  assert.deepEqual(diagnosisSummary({ proxy: ok, endpoints: [row(ok)] }), [{ tone: "ok", text: "代理可达" }, { tone: "ok", text: "额度接口可达" }])
  assert.deepEqual(diagnosisSummary({ proxy: null, endpoints: [row(ok), row(bad), row(bad)] }), [{ tone: "danger", text: "2 个连接不可达" }])
  assert.deepEqual(diagnosisSummary({ proxy: bad, endpoints: [row(skip)] }), [{ tone: "danger", text: "代理不可达" }, { tone: "muted", text: "额度接口未检测" }])
  assert.deepEqual(diagnosisSummary({ proxy: null, endpoints: [] }), [{ tone: "muted", text: "没有已保存的连接" }])
})
