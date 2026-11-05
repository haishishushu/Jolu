// 真实 GatewayView + 完全隔离的合成 API；原生调用一律拒绝。
import assert from "node:assert/strict"
import { createRequire } from "node:module"
import { mkdir } from "node:fs/promises"
import { resolve } from "node:path"
const require = createRequire(import.meta.url)
const { chromium } = require(require.resolve("playwright-core", { paths: [resolve("output/acceptance/node")] }))
const base = process.env.JOLU_PREVIEW_URL || "http://127.0.0.1:5197"
assert.ok(["localhost", "127.0.0.1", "[::1]"].includes(new URL(base).hostname))
const output = resolve("output/acceptance/cc-switch")
await mkdir(output, { recursive: true })
const browser = await chromium.launch({ channel: "msedge", headless: true })
try {
  const page = await browser.newPage({ viewport: { width: 1400, height: 1100 } })
  const errors = []
  page.on("pageerror", error => errors.push(error.message))
  await page.addInitScript(() => {
    window.unexpectedNativeCalls = 0
    window.__TAURI_INTERNALS__ = { invoke: () => { window.unexpectedNativeCalls++; return Promise.reject(new Error("禁止真实原生调用")) } }
  })
  await page.route("**/src/hooks/useConnections.ts*", r => r.fulfill({ contentType: "application/javascript", body: 'const connections=[]; const reload=async()=>{}; export function useConnections(){return {connections,reload,loading:false,error:null}}' }))
  await page.route("**/gateway-confirm-check", r => r.fulfill({ contentType: "text/html", body: `<!doctype html><html><head><meta charset="UTF-8"></head><body><div id="root"></div><script type="module">
import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window);
window.$RefreshReg$=()=>{}; window.$RefreshSig$=()=>type=>type; window.__vite_plugin_react_preamble_installed__=true;
await import('/gateway-confirm-fixture');</script></body></html>` }))
  await page.route("**/gateway-confirm-fixture", r => r.fulfill({ contentType: "application/javascript", body: `
import React from '/node_modules/.vite/deps/react.js'; import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
import { GatewayView } from '/src/views/GatewayView.tsx';
import { gatewayApi } from '/src/lib/gatewayApi.ts';
import { ToastProvider } from '/src/components/ui/Toast.tsx'; import '/src/index.css';
const calls=[];
function deferred(kind,args){return new Promise((resolve,reject)=>calls.push({kind,args,resolve,reject}))}
const agents=[{agent:'claude-code',wired:true},{agent:'codex',wired:false,codex_mode:'oauth'},{agent:'gemini',wired:false},{agent:'opencode',wired:false,drift:'changed'}].map(e=>({config_path:'C:/synthetic/config',reason:null,notice:null,...e}));
gatewayApi.status=async()=>({mode:'relay',enabled:true,port:17890,circuit_threshold:3,auto_escape:true,default_connection:null,wiring:agents});
gatewayApi.groups=async()=>({sources:[{id:'fixture-group',name:'合成分组',sort:0,members:[]}],agents:[]});
gatewayApi.catalog=async()=>[];gatewayApi.routeLog=async()=>[];gatewayApi.sourceCapabilities=async()=>[];
gatewayApi.accessKey=async refresh=>refresh?deferred('key',{}):'jolu-****fixture';
gatewayApi.setWiring=(agent,wired)=>deferred('wire',{agent,wired});
gatewayApi.acceptWiring=agent=>deferred('accept',{agent}); gatewayApi.setMode=mode=>deferred('mode',{mode});
gatewayApi.deleteGroup=(kind,id)=>deferred('delete',{kind,id});
window.fixture={count:()=>calls.length,last:()=>calls.at(-1)?.args,success:()=>calls.at(-1).resolve('jolu-****new'),fail:()=>calls.at(-1).reject(new Error('合成配置错误'))};
ReactDOM.createRoot(document.getElementById('root')).render(React.createElement(ToastProvider,null,React.createElement('div',{className:'p-6'},React.createElement(GatewayView))));
` }))
  await page.goto(`${base}/gateway-confirm-check`)
  await page.getByRole("button", { name: "接入设置", exact: true }).waitFor()
  const count = () => page.evaluate(() => window.fixture.count())
  const setup = async () => { await page.getByRole("button", { name: "接入设置", exact: true }).click(); await page.getByRole("dialog", { name: "Agent 接线", exact: true }).waitFor() }
  const request = async label => { await setup(); await page.getByRole("dialog").getByRole("button", { name: label, exact: true }).first().click() }
  await request("断开")
  await page.getByRole("dialog", { name: /断开/ }).waitFor()
  assert.equal(await count(), 0)
  await page.getByRole("button", { name: "取消", exact: true }).click()
  await page.getByRole("dialog").waitFor({ state: "detached" })
  assert.ok((await page.evaluate(() => document.activeElement?.textContent)).includes("接入设置"), "关闭链式确认应回到外层接入设置按钮")
  await request("断开")
  await page.getByRole("button", { name: "确认断开", exact: true }).click()
  assert.equal(await count(), 1)
  assert.deepEqual(await page.evaluate(() => window.fixture.last()), { agent: "claude-code", wired: false })
  await page.keyboard.press("Escape")
  assert.equal(await page.getByRole("dialog").count(), 1)
  await page.evaluate(() => window.fixture.fail())
  await page.getByRole("dialog").getByRole("alert").filter({ hasText: "合成配置错误" }).waitFor()
  await page.getByRole("button", { name: "确认断开", exact: true }).click()
  await page.evaluate(() => window.fixture.success())
  await page.getByRole("dialog").waitFor({ state: "detached" })
  // 两个专用接线说明都必须保留到操作成功。
  for (const [agent, button, title] of [["codex", "接入", "接入 Codex"], ["gemini", "确认接入", "接入 Gemini CLI"]]) {
    await setup()
    const row = page.getByRole("dialog").getByText(agent === "codex" ? "Codex" : "Gemini CLI", { exact: true }).locator("../..")
    await row.getByRole("button", { name: "接线", exact: true }).click()
    const dialog = page.getByRole("dialog", { name: title, exact: true })
    await dialog.waitFor()
    const before = await count()
    await dialog.getByRole("button", { name: button, exact: true }).evaluate(e => { e.click(); e.click() })
    assert.equal(await count(), before + 1)
    await page.evaluate(() => window.fixture.fail())
    await dialog.getByRole("alert").filter({ hasText: "合成配置错误" }).waitFor()
    await dialog.getByRole("button", { name: button, exact: true }).click()
    await page.evaluate(() => window.fixture.success())
    await dialog.waitFor({ state: "detached" })
  }
  await request("接受现状")
  const beforeAccept = await count()
  await page.getByRole("dialog").getByRole("button", { name: "接受现状", exact: true }).click()
  assert.equal(await count(), beforeAccept + 1)
  await page.evaluate(() => window.fixture.success())
  await page.getByRole("dialog").waitFor({ state: "detached" })
  await page.getByRole("switch", { name: "开启模型转发", exact: true }).click()
  await page.getByRole("dialog", { name: "关闭模型转发？", exact: true }).waitFor()
  const beforeMode = await count()
  await page.getByRole("button", { name: "取消", exact: true }).click()
  await page.getByRole("dialog").waitFor({ state: "detached" })
  assert.equal(await count(), beforeMode)
  await page.getByRole("button", { name: "刷新 API Key", exact: true }).click()
  await page.getByRole("dialog", { name: "刷新网关 API Key？", exact: true }).waitFor()
  assert.equal(await count(), beforeMode)
  await page.screenshot({ path: `${output}/gateway-key-confirm-light.png` })
  await page.getByRole("button", { name: "确认刷新", exact: true }).click()
  assert.equal(await count(), beforeMode + 1)
  await page.evaluate(() => window.fixture.fail())
  await page.getByRole("dialog").getByRole("alert").filter({ hasText: "合成配置错误" }).waitFor()
  await page.getByRole("button", { name: "确认刷新", exact: true }).click()
  await page.evaluate(() => window.fixture.success())
  await page.getByRole("dialog").waitFor({ state: "detached" })
  assert.equal(await page.evaluate(() => window.unexpectedNativeCalls), 0)
  assert.deepEqual(errors, [])
  console.log("PASS Agent断开、Codex/Gemini异步失败重试、接受配置、关闭转发取消、Key刷新弹窗；无真实原生调用")
} finally { await browser.close() }
