// 隔离组件验收：只挂载真实设置组件，以内存设置替代桌面持久化。
const assert = require('node:assert/strict')
const path = require('node:path')
const fs = require('node:fs')
const { chromium } = require(require.resolve('playwright-core', { paths: [path.resolve('output/acceptance/node')] }))
const base = process.env.JOLU_COMPONENT_URL || 'http://localhost:5173'
assert.ok(['localhost', '127.0.0.1'].includes(new URL(base).hostname))
const fixture = `
import React from '/node_modules/.vite/deps/react.js'
import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js'
import {NotificationSection} from '/src/components/settings/NotificationSection.tsx'
import {ToastProvider} from '/src/components/ui/Toast.tsx'
import '/src/index.css?import'
const h=React.createElement
function App(){
 const [settings,setSettings]=React.useState({balance_alert_threshold:8,balance_alert_currency:'USD',dnd:false,notify_task_resumed:true,notify_task_finished:true})
 const calls=React.useRef([])
 window.fixture={calls:calls.current,update:patch=>setSettings(s=>({...s,...patch}))}
 const cfg={settings,ready:true,isSaving:()=>false,setDnd:()=>{},setTaskNotify:()=>{}}
 return h('main',{className:'min-h-screen bg-jolu-bg p-6 text-jolu-text'},h('div',{className:'flex flex-col gap-5'},h(NotificationSection,{cfg,setDirty:()=>{}})))
}
ReactDOM.createRoot(document.getElementById('root')).render(h(ToastProvider,null,h(App)))
`
;(async () => {
 const browser = await chromium.launch({channel:'msedge',headless:true})
 try {
  const page = await browser.newPage({viewport:{width:1455,height:900}})
  const errors=[]
  page.on('pageerror',error=>errors.push(error.message))
  await page.addInitScript(()=>{
   window.__TAURI_INTERNALS__={invoke:async(command,args)=>{
    if(command!=='set_balance_alert')throw new Error('Unexpected command: '+command)
    window.fixture.calls.push(args)
    window.fixture.update({balance_alert_threshold:args.threshold,balance_alert_currency:args.currency})
   }}
  })
  await page.route('**/threshold-check.html',route=>route.fulfill({contentType:'text/html',body:`<html lang="zh-CN"><meta charset="UTF-8"><link rel="icon" href="data:,"><div id="root"></div><script type="module">import RefreshRuntime from '/@react-refresh';RefreshRuntime.injectIntoGlobalHook(window);window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>type=>type;window.__vite_plugin_react_preamble_installed__=true;await import('/threshold-fixture.mjs');</script></html>`}))
  await page.route('**/threshold-fixture.mjs',route=>route.fulfill({contentType:'application/javascript',body:fixture}))
  await page.goto(base+'/threshold-check.html')
  const input=page.getByRole('textbox',{name:'提醒阈值',exact:true})
  await input.waitFor()
  assert.equal(await input.inputValue(),'8','整数阈值不应显示 .00')
  assert.equal(await page.locator('input[aria-label="币种"]').count(),0,'币种不应可编辑')
  const currency=page.getByLabel('币种', {exact:true})
  assert.equal(await currency.textContent(),'USD')
  assert.equal(await currency.evaluate(el=>getComputedStyle(el).color!==getComputedStyle(el.previousElementSibling).color),true,'币种应使用灰色')
  const width=async()=> (await input.boundingBox()).width
  const shortWidth=await width()
  await input.fill('1000000000')
  assert.ok(await width()>shortWidth+40,'宽度随数字增长')
  assert.equal(await input.inputValue(),'1000000000')
  const metrics=await input.evaluate(el=>({scroll:el.scrollWidth,client:el.clientWidth,font:getComputedStyle(el).font,letterSpacing:getComputedStyle(el).letterSpacing}))
  assert.ok(metrics.scroll<=metrics.client+1,`合法最大阈值完整可见 ${JSON.stringify(metrics)}`)
  const save=page.getByRole('button',{name:'保存阈值',exact:true})
  await save.click()
  await page.waitForFunction(()=>window.fixture.calls.length===1)
  await input.fill('12.5')
  await page.evaluate(()=>window.fixture.update({dnd:true}))
  assert.equal(await input.inputValue(),'12.5','无关设置更新不覆盖输入')
  await save.click()
  await page.waitForFunction(()=>window.fixture.calls.length===2)
  assert.equal(await input.inputValue(),'12.5')
  await input.fill('')
  assert.equal(await save.isDisabled(),true)
  await input.fill('0')
  assert.equal(await save.isDisabled(),true)
  await input.fill('2')
  assert.ok(await width()<shortWidth+2,'短数字自动收窄')
  await save.click()
  await page.waitForFunction(()=>window.fixture.calls.length===3)
  assert.equal(await input.inputValue(),'2')
  assert.deepEqual(await page.evaluate(()=>window.fixture.calls),[{threshold:1000000000,currency:'USD'},{threshold:12.5,currency:'USD'},{threshold:2,currency:'USD'}])
  await page.evaluate(()=>window.fixture.update({balance_alert_threshold:null}))
  assert.equal(await input.inputValue(),'2','关闭提醒保留金额草稿')
  await input.fill('123456789')
  const out=path.resolve('output/acceptance/notification-threshold')
  fs.mkdirSync(out,{recursive:true})
  await page.screenshot({path:path.join(out,'light.png')})
  await page.evaluate(()=>document.documentElement.classList.add('dark'))
  await page.screenshot({path:path.join(out,'dark.png')})
  assert.deepEqual(errors,[])
  console.log('PASS: integer display, fixed muted currency, live input and auto width, decimal preservation, save and invalid values, draft retention')
 } finally { await browser.close() }
})().catch(error=>{console.error(error);process.exitCode=1})
