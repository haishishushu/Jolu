// 统一网关（J6）预览验收：真实 GatewayView + 合成 API 与合成事件通道；不连接真实桌面、账号或上游。
// 用法：pnpm --dir frontend exec vite --port 5197 --strictPort；在仓库根目录 node scripts/acceptance/gateway-unified.e2e.mjs
// 依赖：output/acceptance/node 里的 playwright-core 与本机 Edge。截图在 output/acceptance/gateway-unified/。
import assert from "node:assert/strict"
import { createRequire } from "node:module"
import { mkdir } from "node:fs/promises"
import { resolve } from "node:path"
const require = createRequire(import.meta.url)
const { chromium } = require(require.resolve("playwright-core", { paths: [resolve("output/acceptance/node")] }))
const base = process.env.JOLU_PREVIEW_URL || "http://127.0.0.1:5197"
assert.ok(["localhost", "127.0.0.1", "[::1]"].includes(new URL(base).hostname))
const output = resolve("output/acceptance/gateway-unified")
await mkdir(output, { recursive: true })

const connections = [
  ["c1", "OpenAI · 工作账号", "codex", "auth", "openai-work", true],
  ["c2", "Anthropic · 个人账号", "claude", "auth", "claude-me", true],
  ["c3", "ZCode · 团队 Key", "zcode", "api", "zcode", true],
  ["c4", "OpenRouter · 备用", "custom", "api", "openrouter", true],
  ["c5", "Kimi · 测试 Key", "custom", "api", "kimi", false],
  ["c6", "Gemini · 个人", "gemini", "auth", "gemini", false],
].map(([id, baseName, platformId, kind, slug, serve]) => ({
  id, baseName, name: baseName, platformId, kind, slug, serve_enabled: serve, label: "", masked: "", status: "connected",
  lastSyncText: "", baseUrl: null, model: null, effort: null, context1m: null, protocol: null,
}))

const browser = await chromium.launch({ channel: "msedge", headless: true })
try {
  const page = await browser.newPage({ viewport: { width: 1392, height: 1900 } })
  const errors = []
  page.on("pageerror", (error) => { errors.push(error.message); if (process.env.DEBUG) console.log("pageerror", error.message) })
  // 预览环境噪声（Vite 热更新连接、图标 404）不算页面错误
  const noise = (text) => /WebSocket|\[vite\]|Failed to load resource/.test(text)
  page.on("console", (msg) => { if (msg.type() === "error" && !noise(msg.text())) { errors.push(msg.text()); if (process.env.DEBUG) console.log("console", msg.text()) } })
  await page.addInitScript(() => {
    window.unexpectedNativeCalls = []
    const callbacks = new Map()
    const listeners = new Map()
    let next = 1
    window.__TAURI_INTERNALS__ = {
      transformCallback(cb) { const id = next++; callbacks.set(id, cb); return id },
      unregisterCallback(id) { callbacks.delete(id) },
      invoke(cmd, args) {
        if (cmd === "plugin:event|listen") {
          const list = listeners.get(args.event) ?? []
          list.push(args.handler)
          listeners.set(args.event, list)
          return Promise.resolve(args.handler)
        }
        if (cmd === "plugin:event|unlisten") return Promise.resolve()
        window.unexpectedNativeCalls.push(cmd)
        return Promise.reject(new Error(`合成环境禁止原生调用：${cmd}`))
      },
    }
    window.__emit = (event, payload) => (listeners.get(event) ?? []).forEach((id) => callbacks.get(id)?.({ event, id, payload }))
  })
  await page.route("**/src/hooks/useConnections.ts*", (r) => r.fulfill({
    contentType: "application/javascript",
    body: `const connections=${JSON.stringify(connections)}; const reload=async()=>{}; export function useConnections(){return {connections,reload,loading:false,error:null}}`,
  }))
  await page.route("**/gateway-unified-check", (r) => r.fulfill({ contentType: "text/html", body: `<!doctype html><html><head><meta charset="UTF-8"></head><body class="bg-jolu-bg"><div id="root"></div><script type="module">
import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window);
window.$RefreshReg$=()=>{}; window.$RefreshSig$=()=>type=>type; window.__vite_plugin_react_preamble_installed__=true;
await import('/gateway-unified-fixture');</script></body></html>` }))
  await page.route("**/gateway-unified-fixture", (r) => r.fulfill({ contentType: "application/javascript", body: `
import React from '/node_modules/.vite/deps/react.js'; import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
import { GatewayView } from '/src/views/GatewayView.tsx';
import { gatewayApi } from '/src/lib/gatewayApi.ts';
import { ToastProvider } from '/src/components/ui/Toast.tsx'; import '/src/index.css';
const now=Date.now(); const min=60000;
const agents=[{agent:'claude-code',wired:true},{agent:'codex',wired:true},{agent:'gemini',wired:false},{agent:'opencode',wired:false}].map(e=>({config_path:'C:/synthetic/config',reason:null,notice:null,...e}));
gatewayApi.status=async()=>({mode:'relay',enabled:true,port:17890,circuit_threshold:5,auto_escape:true,default_connection:null,wiring:agents});
gatewayApi.groups=async()=>({sources:[{id:'g1',name:'常用',sort:0,members:['c1','c2']}],agents:[{id:'ag1',name:'编码组',sort:0,members:['codex','claude-code']}]});
gatewayApi.catalog=async()=>['openai-work/gpt-5-codex','claude-me/claude-sonnet-4.5','zcode/glm-4.6','openrouter/deepseek-chat','kimi/kimi-k2'].map(id=>({id,source:id.split('/')[0],slug:id.split('/')[0],model:id.split('/')[1],kind:'api',status:'connected'}));
gatewayApi.sourceCapabilities=async()=>['c1','c2','c3','c4','c5','c6'].map((id,i)=>({connection_id:id,can_serve:i!==4,state:i===4?'needs_auth':'ready',reason:''}));
gatewayApi.accessKey=async()=>'jolu-••••••••a1b2';
gatewayApi.sourceUsage=async()=>[{connection_id:'c1',last_ts:now-2*min,requests:42,failed:1},{connection_id:'c3',last_ts:now-3*min,requests:7,failed:0}];
const link=(c,slug,model,ago,n=1)=>({session_platform:'x',session_id:'x',connection_id:c,source:slug,requested_model:slug+'/'+model,upstream_model:null,agent:null,last_ts:now-ago,requests:n});
const tokens=(t)=>t===null?{fresh_input:null,cache_read:null,cache_write:0,output:0,total:null,state:'partial'}:{fresh_input:t,cache_read:0,cache_write:0,output:0,total:t,state:'known'};
const S=(id,platform,title,cwd,ago,t,links,status='done')=>({platform,session_id:id,agent:platform==='claude'?'claude-code':'codex',title,cwd,thread_last_ts:now-ago,gateway_last_ts:now-ago,thread_status:status,tokens:tokens(t),links});
const sessions=[
 S('s1','codex','修复网关请求转发','D:/develop/cc-usage',0,12840,[link('c1','openai-work','gpt-5-codex',0,9)],'recent'),
 S('s2','claude','调整设置页面布局','D:/develop/jolu',0,8400,[link('c2','claude-me','claude-sonnet-4.5',0,4)],'recent'),
 S('s3','codex','梳理账号连接流程','D:/develop/cc-usage',3*min,24600,[link('c3','zcode','glm-4.6',3*min)]),
 S('s4','codex','检查会话压缩逻辑','D:/develop/cc-usage',12*min,6200,[link('c1','openai-work','gpt-5-codex',12*min)]),
 S('s5','claude','整理供应商模型列表','D:/develop/jolu',28*min,null,[link('c4','openrouter','deepseek-chat',28*min)]),
 S('s6','codex','整理项目交接记录','D:/develop/cc-usage',40*min,0,[link('c1','openai-work','gpt-5-codex',40*min)]),
 S('s7','claude','检查模型配置','D:/develop/jolu',65*min,1530000,[link('c3','zcode','glm-4.6',65*min)]),
 S('s8','codex','查阅供应商接入说明',null,3*60*min,980,[link('c4','openrouter','deepseek-chat',3*60*min)]),
];
const inflight=[{request_id:101,ts:now-1500,agent:'codex',requested_model:'openai-work/gpt-5-codex',source:'openai-work',connection_id:'c1',session_platform:'codex',session_id:'s1'},
 {request_id:102,ts:now-900,agent:'claude-code',requested_model:'claude-me/claude-sonnet-4.5',source:'claude-me',connection_id:'c2',session_platform:'claude',session_id:'s2'},
 {request_id:103,ts:now-500,agent:'gemini',requested_model:'openrouter/deepseek-chat',source:'openrouter',connection_id:'c4',session_platform:null,session_id:null}];
window.overviewCalls=0;
gatewayApi.sessionsOverview=async()=>{window.overviewCalls++;return {seq:10,at:now,in_flight:window.overviewCalls===1?inflight:inflight.filter(r=>window.liveIds?.has(r.request_id)??true),sessions};};
const rows=Array.from({length:23},(_,i)=>({id:100-i,ts:now-i*45000,agent:i%2?'codex':'claude-code',requested_model:i%3?'openai-work/gpt-5-codex':'zcode/glm-4.6',source:i%3?'openai-work':'zcode',upstream_model:i%3?'gpt-5-codex':null,status_code:i===4?502:200,result:i===4?'failed':i===7?'rejected':'ok',duration_ms:i===7?null:800+i*37,connection_id:i%3?'c1':'c3',session_platform:null,session_id:null}));
window.pageCalls=[];
gatewayApi.routeLogPage=async(p,size,upper)=>{window.pageCalls.push([p,upper]);const total=rows.length;const count=Math.ceil(total/size);const pg=Math.min(Math.max(1,p),count);return {rows:rows.slice((pg-1)*size,pg*size),page:pg,page_size:size,total,page_count:count,upper_id:100};};
gatewayApi.setServe=async()=>{}; gatewayApi.setServeMany=async(ids,en)=>ids.map((id,i)=>({connection_id:id,ok:i===0,error:i===0?null:'合成失败'}));
ReactDOM.createRoot(document.getElementById('root')).render(React.createElement(ToastProvider,null,React.createElement('div',{className:'h-screen p-8 pb-0 bg-jolu-side'},React.createElement(GatewayView))));
` }))

  await page.goto(`${base}/gateway-unified-check`)
  await page.getByRole("heading", { name: "统一网关" }).waitFor()
  await page.getByText("修复网关请求转发").waitFor()
  await page.waitForTimeout(600)

  // 连接栏：状态胶囊、Base URL 实际端口、掩码
  await page.getByRole("button", { name: /已接入/ }).first().waitFor()
  assert.ok(await page.getByText("http://127.0.0.1:17890", { exact: true }).isVisible())
  // 运行优先：两个运行中的会话在前，默认 5 条，展开入口存在
  const titles = await page.locator("section[aria-label='模型来源与最近会话'] .gw6-card span.text-\\[14px\\]").allTextContents()
  assert.deepEqual(titles.slice(0, 2).sort(), ["修复网关请求转发", "调整设置页面布局"].sort())
  assert.equal(await page.getByText("运行中", { exact: true }).count(), 2)
  // 没有会话标识的 Gemini 请求：来源卡显示运行中，不生成会话卡
  assert.ok(await page.getByText("运行中 · Gemini CLI", { exact: true }).isVisible())
  // Token：真实零、未知、M 缩写
  const graph = page.locator("section[aria-label='模型来源与最近会话']")
  assert.ok(await graph.getByText("12.8K", { exact: true }).isVisible())
  assert.ok((await graph.locator(".gw6-flow").count()) >= 2, "运行中的关联要有流光")
  await page.screenshot({ path: `${output}/main.png`, fullPage: true })
  await graph.screenshot({ path: `${output}/graph-default.png` })

  // 展开更多
  await page.getByRole("button", { name: /展开更多最近会话/ }).click()
  await page.waitForTimeout(400)
  assert.ok(await graph.getByText("1.5M", { exact: true }).isVisible())
  assert.ok(await graph.getByText("0", { exact: true }).isVisible(), "完整数据合计为零显示 0")
  await graph.screenshot({ path: `${output}/graph-expanded.png` })
  await page.getByRole("button", { name: "收起更多会话" }).click()

  // 结束一个请求：运行状态解除、流光停止（序号大于快照）
  await page.evaluate(() => { window.liveIds = new Set([102, 103]); window.__emit("gateway-route", { request_id: 101, seq: 11, ts: Date.now(), agent: "codex", requested_model: "openai-work/gpt-5-codex", source: "openai-work", upstream_model: "gpt-5-codex", status_code: 200, result: "ok", duration_ms: 1200, connection_id: "c1", session_platform: "codex", session_id: "s1" }) })
  await page.waitForTimeout(400)
  assert.equal(await page.getByText("运行中", { exact: true }).count(), 1)
  // 乱序：已结束请求的迟到开始事件不应再登记
  await page.evaluate(() => window.__emit("gateway-route-start", { request_id: 101, seq: 9, ts: Date.now(), agent: "codex", requested_model: "openai-work/gpt-5-codex", source: "openai-work", connection_id: "c1", session_platform: "codex", session_id: "s1" }))
  await page.waitForTimeout(200)
  assert.equal(await page.getByText("运行中", { exact: true }).count(), 1)
  await graph.screenshot({ path: `${output}/graph-after-end.png` })

  // 路由流水：10 条每页、翻页带回查询上界、非法页码不翻页
  const log = page.locator("section[aria-label='路由流水']")
  assert.ok(await log.getByText("第 1–10 条 · 共 23 条").isVisible())
  await log.getByRole("button", { name: "3", exact: true }).click()
  await log.getByText("第 21–23 条 · 共 23 条").waitFor()
  assert.deepEqual(await page.evaluate(() => window.pageCalls.at(-1)), [3, 100])
  await log.getByLabel("跳转页码").fill("9")
  await log.getByRole("button", { name: "跳转" }).click()
  await log.getByRole("alert").filter({ hasText: "页码范围为 1–3" }).waitFor()
  await log.screenshot({ path: `${output}/route-log.png` })

  // 管理来源
  await page.getByRole("button", { name: "管理来源" }).click()
  const dialog = page.getByRole("dialog", { name: "管理模型来源" })
  await dialog.waitFor()
  await dialog.getByText("需要重新登录").waitFor()
  await dialog.getByRole("button", { name: /未接入 \d/ }).click()
  assert.equal(await dialog.getByRole("checkbox", { name: /^选择 / }).count(), 2)
  await page.waitForTimeout(250)
  await page.screenshot({ path: `${output}/manage.png` })
  await dialog.getByRole("button", { name: /全部 \d/ }).click()
  await dialog.getByRole("button", { name: "常用" }).click()
  await page.waitForTimeout(150)
  await page.screenshot({ path: `${output}/manage-group.png` })
  await dialog.getByRole("button", { name: "编码组" }).click()
  await page.waitForTimeout(150)
  await page.screenshot({ path: `${output}/manage-agents.png` })

  assert.deepEqual(errors, [], `页面错误：${errors.join(" | ")}`)
  assert.deepEqual(await page.evaluate(() => window.unexpectedNativeCalls), [])
  console.log("gateway-unified 预览验收通过，截图在", output)
} finally {
  await browser.close()
}
