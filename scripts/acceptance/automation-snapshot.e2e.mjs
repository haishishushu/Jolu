// 真实任务详情组件 + 合成桥接；不读取真实账号或启动自动化。
import assert from "node:assert/strict"
import { createRequire } from "node:module"
import { dirname, join, resolve } from "node:path"
import { mkdir } from "node:fs/promises"
import { fileURLToPath } from "node:url"
const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const require = createRequire(import.meta.url)
const { chromium } = require(require.resolve("playwright-core", { paths: [join(repo, "output/acceptance/node")] }))
const base = new URL(process.env.JOLU_COMPONENT_URL ?? "http://localhost:5193")
assert.ok(["localhost", "127.0.0.1", "[::1]"].includes(base.hostname))
const fixture = `
import React from '/node_modules/.vite/deps/react.js';
import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
import { TaskDetail } from '/src/components/automation/TaskDetail.tsx';
import { automationApi } from '/src/lib/automationApi.ts';
import { demoTaskDetail } from '/src/mock/automation.ts';
import { TipProvider } from '/src/components/ui/Tip.tsx';
import '/src/index.css';
const root=ReactDOM.createRoot(document.getElementById('root')); const noop=()=>{};
window.calls=[];window.sessions=[];window.edits=[];
automationApi.quota=async (id)=>{window.calls.push(id);return {state:'available',reason:'合成额度',windows:[],resets_at:null};};
window.show=(unknown=false)=>{
 const detail=demoTaskDetail(24,Date.now());
 const original={...detail.item,platform:'claude',connection_id:'old-account',cwd:'D:/old-project',prompt:'原运行指令',auto_resume:true,wait_deadline:null};
 detail.item={...detail.item,platform:'codex',connection_id:'new-account',cwd:'D:/new-project',prompt:'新任务指令',auto_resume:false,wait_deadline:Date.now()+60000};
 detail.item.active={...detail.item.active,platform:unknown?'unknown':'claude',connection_id:unknown?'':'old-account',account:'原账号',session_id:'original-session',status:unknown?'attention':'waiting',block_kind:unknown?'snapshot_missing':'quota',reason:unknown?'缺少原始执行快照':null};
 detail.execution=unknown?null:original;
 if(unknown) detail.queue=[];
 root.render(React.createElement(TipProvider,null,React.createElement('div',{className:'p-6 bg-jolu-bg'},React.createElement(TaskDetail,{detail,now:Date.now(),account:unknown?undefined:{name:'原账号',kind:'auth'},onBack:noop,onAction:async()=>{},onEdit:()=>window.edits.push(detail.item.prompt),onCancel:noop,onDelete:noop,onOpenRun:noop,onOpenProviders:noop,onOpenSession:(...args)=>window.sessions.push(args)}))));
};window.show();`
const browser = await chromium.launch({ channel: "msedge", headless: true })
try {
  const page = await browser.newPage({ viewport: { width: 1500, height: 1100 } })
  const errors = []
  page.on("pageerror", error => errors.push(error.message))
  await page.addInitScript(() => { window.nativeCalls=0;window.__TAURI_INTERNALS__={invoke:()=>{window.nativeCalls++;throw Error("禁止原生桥接")}} })
  // Vite HMR 可能给组件 import 加时间戳；对每个实际模块实例注入同一合成桥接。
  await page.route("**/src/lib/automationApi.ts*", async route => {
    const response = await route.fetch()
    await route.fulfill({response,body:await response.text()+`\nautomationApi.quota=async(id)=>{window.calls.push(id);return {state:'available',reason:'合成额度',windows:[],resets_at:null}};`})
  })
  await page.route("**/automation-snapshot-check", route => route.fulfill({ contentType:"text/html",body:`<!doctype html><meta charset="UTF-8"><div id="root"></div><script type="module">import RefreshRuntime from '/@react-refresh';RefreshRuntime.injectIntoGlobalHook(window);window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>t=>t;window.__vite_plugin_react_preamble_installed__=true;await import('/automation-snapshot-fixture');</script>` }))
  await page.route("**/automation-snapshot-fixture", route=>route.fulfill({contentType:"application/javascript",body:fixture}))
  await page.goto(new URL("/automation-snapshot-check",base).href)
  await page.waitForFunction(()=>window.calls?.length===1,{},{timeout:15000}).catch(async error=>{throw new Error(`${error.message}; browser=${JSON.stringify(errors)}; state=${JSON.stringify(await page.evaluate(()=>({calls:window.calls,body:document.body.innerText.slice(0,200)})))}`)})
  assert.deepEqual(await page.evaluate(()=>window.calls),["old-account"])
  await page.getByText("原运行指令",{exact:true}).waitFor()
  assert.equal(await page.getByText("新任务指令",{exact:true}).count(),0)
  await page.getByText("D:/old-project",{exact:true}).waitFor()
  assert.equal(await page.getByText("D:/new-project",{exact:true}).count(),0)
  await page.getByText("Claude Code / 原账号",{exact:true}).waitFor()
  await page.getByText("自动检查后续跑",{exact:true}).waitFor()
  await page.getByText("不限制",{exact:true}).waitFor()
  await page.getByRole("button",{name:"查看关联会话"}).click()
  assert.deepEqual(await page.evaluate(()=>window.sessions),[["claude","original-session"]])
  await page.getByRole("button",{name:"任务更多操作"}).click()
  await page.getByRole("menuitem",{name:"编辑任务"}).click()
  assert.deepEqual(await page.evaluate(()=>window.edits),["新任务指令"])
  await page.getByRole("menu").waitFor({state:"detached"})
  const out=join(repo,"output/acceptance/automation-snapshot")
  await mkdir(out,{recursive:true})
  await page.screenshot({path:join(out,"snapshot-detail.png"),fullPage:true})
  await page.evaluate(()=>window.show(true))
  await page.getByText("缺少原始执行快照",{exact:true}).first().waitFor()
  await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))))
  assert.deepEqual(await page.evaluate(()=>window.calls),["old-account"],"未知快照不查询当前任务账号")
  assert.equal(await page.getByText("新任务指令",{exact:true}).count(),0)
  assert.equal(await page.getByRole("button",{name:"查看关联会话"}).count(),0)
  assert.equal(await page.getByText("正在读取额度…",{exact:true}).count(),0)
  assert.ok(await page.getByRole("button",{name:"重新排队"}).isDisabled())
  assert.equal(await page.evaluate(()=>window.nativeCalls),0)
  assert.deepEqual(errors,[])
  await page.screenshot({path:join(out,"unknown-detail.png"),fullPage:true})
  console.log("PASS automation snapshot: original execution display, quota, session; current edit; unknown blocked")
} finally { await browser.close() }
