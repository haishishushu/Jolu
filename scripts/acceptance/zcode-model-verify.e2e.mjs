// 真实连接弹窗 + 合成 API；不连接桌面、不发送模型请求、不保存真实账号。
import assert from "node:assert/strict"
import { mkdir } from "node:fs/promises"
import { createRequire } from "node:module"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const require = createRequire(import.meta.url)
const { chromium } = require(require.resolve("playwright-core", { paths: [join(repo, "output/acceptance/node")] }))
const baseUrl = new URL(process.env.JOLU_COMPONENT_URL ?? "http://localhost:5311")
assert.ok(["localhost", "127.0.0.1", "[::1]"].includes(baseUrl.hostname), "只允许本机开发服务器")
const artifacts = join(repo, "output/acceptance/zcode-model")
await mkdir(artifacts, { recursive: true })
const fixture = `
import React from '/node_modules/.vite/deps/react.js';
import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
import { ConnectDialog } from '/src/components/providers/ConnectDialog.tsx';
import { api } from '/src/lib/api.ts';
import { TipProvider } from '/src/components/ui/Tip.tsx';
import { ToastProvider } from '/src/components/ui/Toast.tsx';
import '/src/index.css';
const calls=[];
const deferred=(kind,args)=>new Promise((resolve,reject)=>calls.push({kind,args,resolve,reject}));
api.fetchRemoteModels=(...args)=>deferred('fetch',args);
api.verifyCustomModel=(...args)=>deferred('verify',args);
const actions={connections:[],readLocal:()=>Promise.reject(Error('禁止本机读取')),add:input=>deferred('save',input)};
const root=ReactDOM.createRoot(document.getElementById('root'));
let serial=0;
window.fixture={
 show(platform='zcode') {root.render(React.createElement(TipProvider,null,React.createElement(ToastProvider,null,
   React.createElement(ConnectDialog,{key:++serial,platform,actions,onClose:()=>{}}))));},
 count:kind=>calls.filter(c=>c.kind===kind).length,
 complete(kind,index,value){calls.filter(c=>c.kind===kind)[index].resolve(value);},
 fail(kind,index){calls.filter(c=>c.kind===kind)[index].reject(Error(kind==='fetch'?'HTTP 404 模型列表不可用':'合成模型权限失败'));},
 argsMatch(kind,index,expected){return JSON.stringify(calls.filter(c=>c.kind===kind)[index].args)===JSON.stringify(expected);},
};`
const browser = await chromium.launch({ channel: "msedge", headless: true })
try {
  const page = await browser.newPage({ viewport: { width: 1400, height: 1100 } })
  const errors = []
  page.on("pageerror", error => errors.push(error.message))
  await page.addInitScript(() => {
    window.unexpectedNativeCalls = 0
    window.__TAURI_INTERNALS__ = { invoke: () => {
      window.unexpectedNativeCalls++
      return Promise.reject(Error("合成验收禁止真实原生调用"))
    } }
  })
  await page.route("**/zcode-model-check", route => route.fulfill({ contentType: "text/html", body: `
    <!doctype html><html><head><meta charset="UTF-8"></head><body><div id="root"></div><script type="module">
    import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window);
    window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>type=>type;window.__vite_plugin_react_preamble_installed__=true;
    await import('/zcode-model-fixture');</script></body></html>` }))
  await page.route("**/zcode-model-fixture", route => route.fulfill({ contentType: "application/javascript", body: fixture }))
  await page.goto(new URL("/zcode-model-check", baseUrl).href)
  await page.waitForFunction(() => !!window.fixture)
  const invoke = (method, ...args) => page.evaluate(([name, values]) => window.fixture[name](...values), [method, args])
  const waitRequest = (kind, count) => page.waitForFunction(([k, n]) => window.fixture.count(k) === n, [kind, count])
  const settle = async (method, ...args) => {
    await invoke(method, ...args)
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  }
  const save = () => page.getByRole("button", { name: "保存连接", exact: true })
  const key = () => page.getByLabel("API Key *", { exact: true })
  const manual = () => page.getByPlaceholder("输入服务支持的完整模型 ID")
  const verify = () => page.getByRole("button", { name: "验证模型", exact: true })
  const startVerify = async () => {
    const count = await invoke("count", "verify")
    await verify().click()
    await page.getByRole("alertdialog", { name: "验证模型", exact: true }).getByText(/可能产生少量 API 用量/).waitFor()
    assert.equal(await invoke("count", "verify"), count, "确认之前不得发送模型请求")
    await page.getByRole("button", { name: "继续验证", exact: true }).dblclick()
    await waitRequest("verify", count + 1)
    assert.ok(await manual().isDisabled(), "验证时模型输入禁用")
    assert.ok(await save().isDisabled())
    return count
  }
  let fetches = 0
  const prepare = async (platform = "zcode") => {
    await invoke("show", platform)
    if (platform === "zcode") await page.getByRole("tab", { name: "API Key", exact: true }).click()
    await page.getByLabel(platform === "custom" ? "名称 *" : "连接名称 *", { exact: true }).fill("合成验证")
    if (platform === "custom") await page.getByLabel("Base URL *", { exact: true }).fill("https://synthetic.invalid/v1")
    await key().fill("synthetic-key-a")
    await page.getByRole("button", { name: "获取模型", exact: true }).click()
    await waitRequest("fetch", ++fetches)
    await settle("fail", "fetch", fetches - 1)
    await manual().waitFor({ timeout: 5000 })
    await manual().fill("synthetic-model")
  }
  const regions = [
    ["国内 · Anthropic", "https://open.bigmodel.cn/api/anthropic", "anthropic"],
    ["国内 · OpenAI", "https://open.bigmodel.cn/api/coding/paas/v4", "openai"],
    ["国际 · Anthropic", "https://api.z.ai/api/anthropic", "anthropic"],
    ["国际 · OpenAI", "https://api.z.ai/api/coding/paas/v4", "openai"],
  ]
  const chooseRegion = async label => {
    await page.getByRole("button", { name: "服务区域与协议", exact: true }).click()
    await page.getByRole("menuitemradio", { name: label, exact: true }).click()
  }
  await prepare()
  assert.ok(await save().isDisabled(), "手填未验证模型不能保存")
  await verify().click()
  await page.screenshot({ path: join(artifacts, "confirmation.png") })
  await page.getByRole("alertdialog", { name: "验证模型", exact: true }).getByRole("button", { name: "取消", exact: true }).click()
  assert.equal(await invoke("count", "verify"), 0)
  const old = await startVerify()
  await key().fill("synthetic-key-b")
  await settle("complete", "verify", old)
  assert.ok(await save().isDisabled(), "换 Key 后旧成功不能恢复保存能力")
  const oldError = await startVerify()
  await chooseRegion(regions[1][0])
  const latest = await startVerify()
  await settle("complete", "verify", latest)
  await settle("fail", "verify", oldError)
  assert.equal(await save().isDisabled(), false, "旧失败不能覆盖新成功")
  assert.equal(await page.getByText(/合成模型权限失败/).count(), 0)
  await manual().fill("synthetic-model-edited")
  assert.ok(await save().isDisabled(), "更改模型后必须重新验证")
  const failed = await startVerify()
  await settle("fail", "verify", failed)
  assert.equal(await manual().inputValue(), "synthetic-model-edited")
  assert.ok(await save().isDisabled())
  await page.getByText(/合成模型权限失败/).waitFor()
  await page.screenshot({ path: join(artifacts, "failed.png") })
  const recovered = await startVerify()
  await settle("complete", "verify", recovered)
  await page.getByRole("button", { name: /^(重新获取|获取模型)$/ }).click()
  await waitRequest("fetch", ++fetches)
  assert.ok(await manual().isDisabled(), "手填模式获取模型期间输入禁用，不能使请求序号失效而卡住加载")
  assert.ok(await save().isDisabled(), "重新获取期间旧验证不能允许保存")
  await settle("complete", "fetch", fetches - 1, ["synthetic-enumerated"])
  await page.getByRole("button", { name: "选择模型", exact: true }).click()
  await page.getByRole("option", { name: "synthetic-enumerated", exact: true }).click()
  assert.equal(await save().isDisabled(), false, "枚举选择模型保持既有保存流程")
  assert.equal(await verify().count(), 0)
  console.log("PASS ZCode: explicit consent, no duplicate verification, stale results ignored, failed draft retained")
  for (const [label, base, protocol] of regions) {
    await prepare()
    if (label !== regions[0][0]) await chooseRegion(label)
    const index = await startVerify()
    assert.ok(await invoke("argsMatch", "verify", index, [base, "synthetic-key-a", protocol, "synthetic-model"]), "验证参数必须匹配服务区域与协议")
    await settle("complete", "verify", index)
    assert.equal(await save().isDisabled(), false)
    if (label === regions[0][0]) await page.screenshot({ path: join(artifacts, "verified.png") })
    await save().click()
    const saved = await invoke("count", "save")
    assert.ok(await invoke("argsMatch", "save", saved - 1, { platform: "zcode", service: null, kind: "api", name: "合成验证", secret: "synthetic-key-a", base_url: base, model: "synthetic-model", protocol }))
  }
  await prepare("custom")
  assert.equal(await save().isDisabled(), false, "自定义供应商既有手填保存规则不改变")
  assert.equal(await verify().count(), 0)
  assert.equal(errors.length, 0, "组件无运行时异常")
  assert.equal(await page.evaluate(() => window.unexpectedNativeCalls), 0, "没有真实原生调用")
  console.log("PASS ZCode: four regional/protocol combinations verified before save; custom-provider rules unchanged")
} finally {
  await browser.close()
}
