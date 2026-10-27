// 真实账号管理、选择和确认组件；仅通过合成 IPC 删除内存连接。
const assert = require('node:assert/strict'), path = require('node:path'), fs = require('node:fs')
const {chromium} = require(require.resolve('playwright-core', {paths:[path.resolve(__dirname,'../../output/acceptance/node')]}))
const origin = process.env.JOLU_COMPONENT_URL || 'http://localhost:5173'
assert.ok(['localhost','127.0.0.1'].includes(new URL(origin).hostname))
;(async()=>{
 const browser = await chromium.launch({channel:'msedge',headless:true})
 try {
  const page = await browser.newPage({viewport:{width:1280,height:900}}), errors=[]
  page.setDefaultTimeout(15000)
  page.on('pageerror',e=>errors.push(e.message))
  await page.addInitScript(()=>{
   const row=(id,platform,kind,name)=>({id,platform,edition:null,kind,name,label:'浏览器授权',masked:kind==='api'?'sk-****test':'',status:id==='claude-b'?'paused':'connected',last_sync_text:'刚刚同步',base_url:kind==='api'?'https://synthetic.example/v1':null,model:kind==='api'?'synthetic-model':null,effort:null,context_1m:null,protocol:kind==='api'?'openai':null,service:null,account_email:kind==='auth'?name:null})
   const f={rows:[row('codex-a','codex','auth','current@example.test'),row('codex-b','codex','auth','failed@example.test'),row('claude-a','claude','auth','untouched@example.test'),row('claude-b','claude','auth','paused@example.test'),row('api-a','custom','api','合成 API')],calls:[],callbacks:{},listeners:{},failures:{},waiters:[],hold:false,explicitReloads:0,history:[{id:'history-a',connection_id:'codex-a',amount:100},{id:'history-b',connection_id:'codex-b',amount:0}],settings:{refresh_minutes:5,reduce_motion:true,island_clones:[],island_visible:true,theme:'light'}}
   window.fixture=f
   f.emit=(event,payload)=>{for(const l of Object.values(f.listeners))if(l.event===event)f.callbacks[l.handler]({event,id:l.handler,payload})}
   window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}}
   window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},transformCallback:cb=>{const id=Object.keys(f.callbacks).length+1;f.callbacks[id]=cb;return id},unregisterCallback(){},async invoke(command,args){
    f.calls.push({command,...args})
    if(command==='plugin:event|listen'){f.listeners[args.handler]={event:args.event,handler:args.handler};return args.handler}
    if(command==='plugin:event|unlisten'){delete f.listeners[args.eventId];return}
    if(command==='list_connections')return f.rows
    if(command==='current_connections')return f.rows.some(r=>r.id==='codex-a')?['codex-a']:[]
    if(command==='connection_quota'||command==='connection_balance')return {state:'unsupported',reason:'合成供应商未提供查询'}
    if(command==='remove_connection'){
     if(f.hold)await new Promise(resolve=>f.waiters.push(resolve))
     if(f.failures[args.id]){f.failures[args.id]--;throw Error('合成删除失败 failed@example.test sk-secret-synthetic')}
     assertKnownId(args.id)
     f.rows=f.rows.filter(r=>r.id!==args.id)
     f.emit('connections-changed')
     return
    }
    throw Error('unexpected '+command)
   }}
   function assertKnownId(id){if(!f.rows.some(r=>r.id===id))throw Error('unknown synthetic connection')}
  })
  await page.route('**/*',route=>{
   const url=new URL(route.request().url())
   if(url.origin!==new URL(origin).origin)return route.abort('blockedbyclient')
   return route.continue()
  })
  await page.route('**/src/hooks/useSettings.ts*',r=>r.fulfill({contentType:'application/javascript',body:'export function useSettings(){return {settings:window.fixture.settings,ready:true}}'}))
  await page.route('**/bulk-account-check',r=>r.fulfill({contentType:'text/html',body:`<!doctype html><html lang="zh-CN"><meta charset="utf-8"><link rel="icon" href="data:,"><div id="root"></div><script type="module">import R from '/@react-refresh';R.injectIntoGlobalHook(window);window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>t=>t;window.__vite_plugin_react_preamble_installed__=true;await import('/bulk-account-fixture');</script></html>`}))
  await page.route('**/bulk-account-fixture',r=>r.fulfill({contentType:'application/javascript',body:`
   import React from '/node_modules/.vite/deps/react.js';import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
   import {AccountsPanel} from '/src/components/providers/AccountsPanel.tsx';import {useConnections} from '/src/hooks/useConnections.ts';import {TipProvider} from '/src/components/ui/Tip.tsx';import {ToastProvider} from '/src/components/ui/Toast.tsx';import '/src/index.css';
   function App(){const conn=useConnections(),[target,setTarget]=React.useState(null);window.fixture.setTarget=setTarget;const bridge={...conn,reload:async()=>{window.fixture.explicitReloads++;await conn.reload()}};return React.createElement('main',{className:'flex flex-col bg-jolu-bg p-6',style:{height:'100vh'}},React.createElement(AccountsPanel,{conn:bridge,target,onTargetHandled:()=>{window.fixture.targetHandled=true},onConnect:()=>{window.fixture.addClicked=true}}))}
   ReactDOM.createRoot(document.getElementById('root')).render(React.createElement(TipProvider,null,React.createElement(ToastProvider,null,React.createElement(App))));
  `}))
  await page.goto(origin+'/bulk-account-check')
  const grid=page.locator('[data-account-grid]'), card=id=>grid.locator(`[data-account-id="${id}"]`)
  await page.waitForFunction(()=>document.querySelectorAll('[data-account-grid] article').length===5)
  assert.equal(await page.getByRole('button',{name:'批量管理',exact:true}).count(),1,'账号管理提供批量管理入口')
  await page.getByRole('button',{name:'添加账号',exact:true}).click()
  assert.equal(await page.evaluate(()=>window.fixture.addClicked),true,'保留原添加入口')
  await page.getByRole('textbox',{name:'搜索账号',exact:true}).fill('untouched')
  await page.waitForFunction(()=>document.querySelectorAll('[data-account-grid] article').length===1)
  await page.evaluate(()=>window.fixture.setTarget({connectionId:'codex-a',nonce:1}))
  await page.waitForFunction(()=>window.fixture.targetHandled&&document.querySelector('[data-account-id="codex-a"]').dataset.accountLocated==='true')
  assert.equal(await grid.locator('article').count(),5,'保留通知目标清筛选并定位')
  const calls=command=>page.evaluate(command=>window.fixture.calls.filter(c=>c.command===command),command)
  const removed=async()=> (await calls('remove_connection')).map(c=>c.id)
  const enter=()=>page.getByRole('button',{name:'批量管理',exact:true}).click()
  const exit=()=>page.getByRole('button',{name:'退出',exact:true}).click()
  const checkbox=id=>card(id).getByRole('checkbox',{includeHidden:true})
  const selected=()=>page.getByText(/已选\s*\d+\s*\//).first()
  const openDelete=()=>page.getByRole('button',{name:/删除所选/}).click()
  const dialog=page.getByRole('dialog',{name:'删除所选账号？',exact:true})
  const confirm=()=>dialog.getByRole('button',{name:/删除 \d+ 个账号|正在删除/})

  // 筛选剔除隐藏项，全选只覆盖当前列表；取消绝不触发删除。
  await enter()
  assert.equal(await grid.getByRole('checkbox').count(),5)
  assert.equal(await page.getByRole('button',{name:/删除所选/}).isDisabled(),true,'空选择不能提交')
  assert.equal(await card('codex-a').getByText('当前使用',{exact:true}).count(),1)
  assert.equal(await grid.getByRole('button',{name:'删除账号',exact:true}).count(),0)
  assert.equal(await grid.getByRole('button',{name:'编辑连接',exact:true}).count(),0)
  await checkbox('codex-a').check()
  await page.getByRole('button',{name:'Claude Code',exact:true}).click()
  await page.waitForFunction(()=>document.querySelectorAll('[data-account-grid] article').length===2)
  assert.match(await selected().innerText(),/已选\s*0\s*\//)
  await page.getByRole('button',{name:'全选当前列表',exact:true}).click()
  assert.equal(await grid.getByRole('checkbox',{checked:true}).count(),2)
  await openDelete();await dialog.waitFor()
  assert.equal((await dialog.innerText()).includes('current@example.test'),false)
  await dialog.getByRole('button',{name:'取消',exact:true}).click()
  assert.deepEqual(await removed(),[])
  await page.getByRole('button',{name:'全部供应商',exact:true}).click()
  await page.waitForFunction(()=>document.querySelectorAll('[data-account-grid] article').length===5)
  assert.equal(await checkbox('codex-a').isChecked(),false)
  assert.equal(await checkbox('claude-a').isChecked(),true)
  await exit();await enter()
  assert.equal(await grid.getByRole('checkbox',{checked:true}).count(),0)

  // 确认冻结精确 ID；一项失败仍继续，重试仅调用失败项，当前账号不切换 CLI。
  await checkbox('codex-a').check();await checkbox('codex-b').check();await checkbox('claude-b').check()
  await page.evaluate(()=>{window.fixture.failures['codex-b']=1;window.fixture.hold=true})
  await openDelete();await dialog.getByText('当前使用',{exact:true}).waitFor()
  assert.match(await dialog.innerText(),/原工具/)
  const reloadsBefore=await page.evaluate(()=>window.fixture.explicitReloads)
  await confirm().evaluate(button=>{button.click();button.click()})
  await page.waitForFunction(()=>window.fixture.waiters.length===1)
  assert.equal(await confirm().isDisabled(),true)
  assert.equal(await dialog.getByRole('button',{name:'取消',exact:true}).isDisabled(),true)
  await page.keyboard.press('Escape');assert.equal(await dialog.count(),1)
  await page.evaluate(()=>{window.fixture.hold=false;window.fixture.waiters.splice(0).forEach(resolve=>resolve())})
  await dialog.getByRole('alert').waitFor()
  assert.deepEqual(await removed(),['codex-a','codex-b','claude-b'])
  assert.equal(await page.evaluate(()=>window.fixture.explicitReloads)-reloadsBefore,1,'一次批量只显式重读一次列表；真实连接事件也可更新')
  assert.equal(await card('codex-a').count(),0)
  assert.equal(await card('claude-b').count(),0)
  assert.equal(await checkbox('codex-b').isChecked(),true)
  assert.equal(await checkbox('claude-a').isChecked(),false)
  assert.equal(await checkbox('api-a').isChecked(),false)
  await dialog.getByRole('button',{name:/重试失败/}).click()
  await dialog.waitFor({state:'hidden'})
  assert.deepEqual(await removed(),['codex-a','codex-b','claude-b','codex-b'])
  assert.deepEqual(await calls('enable_connection'),[])
  assert.deepEqual(await page.evaluate(()=>window.fixture.rows.map(r=>r.id)),['claude-a','api-a'])
  assert.deepEqual(await page.evaluate(()=>window.fixture.history),[{id:'history-a',connection_id:'codex-a',amount:100},{id:'history-b',connection_id:'codex-b',amount:0}])

  // 失败后关闭保留勾选；隐私名称、失败理由、无障碍名称均不出现合成敏感值。
  await checkbox('claude-a').check()
  await page.getByRole('button',{name:'隐藏敏感信息',exact:true}).click()
  await page.evaluate(()=>window.fixture.failures['claude-a']=2)
  await openDelete();await confirm().click();await dialog.getByRole('alert').waitFor()
  const html=await dialog.innerHTML()
  for(const value of ['untouched@example.test','failed@example.test','sk-secret-synthetic'])assert.equal(html.includes(value),false,`隐私弹窗不泄漏 ${value}`)
  await dialog.getByRole('button',{name:/取消|关闭/}).last().click();await dialog.waitFor({state:'hidden'})
  assert.equal(await checkbox('claude-a').isChecked(),true,'关闭后保留失败账号勾选')
  assert.equal(await checkbox('api-a').isChecked(),false)
  assert.deepEqual(await removed(),['codex-a','codex-b','claude-b','codex-b','claude-a'])
  assert.equal((await grid.innerHTML()).includes('untouched@example.test'),false)
  const output=path.resolve(__dirname,'../../output/acceptance/bulk-account-management');fs.mkdirSync(output,{recursive:true})
  await page.screenshot({path:path.join(output,'selection-light.png')})
  await page.evaluate(()=>document.documentElement.classList.add('dark'))
  await page.screenshot({path:path.join(output,'selection-dark.png')})
  await openDelete()
  await page.screenshot({path:path.join(output,'confirm-dark.png')})
  await page.setViewportSize({width:680,height:700})
  assert.equal(await dialog.evaluate(n=>n.scrollWidth>n.clientWidth+1),false)
  await page.screenshot({path:path.join(output,'confirm-narrow.png')})
  await dialog.getByRole('button',{name:'取消',exact:true}).click()
  // 删除全部剩余连接后显示原空状态，仍可通过原入口添加。
  await page.setViewportSize({width:1280,height:900})
  await page.evaluate(()=>{window.fixture.failures['claude-a']=0;window.fixture.addClicked=false})
  await checkbox('api-a').check()
  await openDelete();await confirm().click();await dialog.waitFor({state:'hidden'})
  await page.getByText('还没有账号',{exact:true}).waitFor()
  assert.deepEqual(await page.evaluate(()=>window.fixture.rows),[])
  await page.getByRole('button',{name:'添加账号',exact:true}).first().click()
  assert.equal(await page.evaluate(()=>window.fixture.addClicked),true,'批量删空后保留添加入口')
  assert.equal(await page.evaluate(()=>window.fixture.history.length),2)
  assert.deepEqual(errors,[])
  console.log('PASS bulk account management: filtered selection, cancel, exact IDs, partial failure, retry, freeze, privacy and preserved unselected rows/history')
 } finally {await browser.close()}
})().catch(error=>{console.error(error);process.exitCode=1})
