// 当前页复用真实连接弹窗；所有授权和账号数据均为合成 IPC。
const assert = require('node:assert/strict'), path = require('node:path'), fs = require('node:fs')
const {chromium} = require(require.resolve('playwright-core', {paths:[path.resolve(__dirname,'../../output/acceptance/node')]}))
const origin = process.env.JOLU_COMPONENT_URL || 'http://localhost:5173'
assert.ok(['localhost','127.0.0.1'].includes(new URL(origin).hostname))
;(async()=>{
 const browser=await chromium.launch({channel:'msedge',headless:true})
 try{
  const page=await browser.newPage({viewport:{width:1280,height:900}}), errors=[]
  page.on('pageerror',e=>errors.push(e.message))
  await page.addInitScript(()=>{
   const row=(id,platform,edition,name)=>({id,platform,edition,name,kind:'auth',label:'浏览器授权',masked:'已加密保存',status:'connected',last_sync_text:null,base_url:null,model:null,effort:null,context_1m:null,protocol:null,service:null,account_email:null})
   const f={rows:[row('trae-a','trae','cn','Trae 账号 A')],calls:[],callbacks:{},listeners:{},attempt:0,statusWaiters:[],settings:{default_page:'providers',last_page:'providers',list_density:'comfortable',reduce_motion:true,island_clones:[],island_visible:true,theme:'light'}}
   window.fixture=f
   f.emit=(event,payload)=>{for(const l of Object.values(f.listeners))if(l.event===event)f.callbacks[l.handler]({event,id:l.handler,payload})}
   window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}}
   window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},transformCallback:cb=>{const id=Object.keys(f.callbacks).length+1;f.callbacks[id]=cb;return id},unregisterCallback(){},async invoke(command,args){
    f.calls.push({command,...args})
    if(command==='plugin:event|listen'){f.listeners[args.handler]={event:args.event,handler:args.handler};return args.handler}
    if(command==='plugin:event|unlisten'){delete f.listeners[args.eventId];return}
    if(command==='get_settings')return f.settings
    if(command==='take_main_intent')return null
    if(command==='take_app_notices')return []
    if(command==='main_panel_ready'||command==='remember_page'||command==='cancel_browser_authorization')return
    if(command==='plugin:window|is_visible')return true
    if(command==='plugin:window|is_minimized'||command==='plugin:window|is_maximized')return false
    if(command==='list_connections')return f.rows
    if(command==='current_connections')return []
    if(command==='connection_quota')return {state:'unsupported',reason:'此个人账号未提供额度查询'}
    if(command==='daily_checkin_overview'){if(f.failOverview)throw Error('synthetic overview error');return {accounts:f.rows.map(r=>({...r,account:r.id==='trae-a'?'first@example.test':'verified@example.test',connection_status:r.status})),records:[]}}
    if(command==='daily_checkin_status'){if(f.holdStatus)await new Promise(resolve=>f.statusWaiters.push(resolve));return {id:args.id,account:args.id==='trae-a'?'first@example.test':'verified@example.test',account_key:'identity-'+args.id,checked_at:Date.now(),outcome:{state:'ready',message:'可领取',reward:100,unit:'积分',reset_at:null,period_key:null,server_time:null}}}
    if(command==='start_browser_authorization'){f.authorization={platform:args.platform,edition:args.edition};return 'attempt-'+(++f.attempt)}
    if(command==='browser_authorization_status')return {state:'ready',detection:{found:true,account:'verified@example.test',plan:null,sources:['官方身份已验证'],masked:'已验证',message:'账号身份有效；额度查询暂不支持'},message:null}
    if(command==='save_browser_connection'){
     if(f.holdSave)await new Promise(resolve=>f.releaseSave=resolve)
     const id='saved-'+f.attempt
     f.rows.push(row(id,f.authorization.platform,f.authorization.edition,args.name));f.emit('connections-changed');return id
    }
    throw Error('unexpected '+command)
   }}
  })
  await page.route('**/src/hooks/useSettings.ts*',r=>r.fulfill({contentType:'application/javascript',body:'export function useSettings(){return {settings:window.fixture.settings,ready:true}}'}))
  await page.route('**/account-connect-check',r=>r.fulfill({contentType:'text/html',body:`<!doctype html><html lang="zh-CN"><meta charset="utf-8"><link rel="icon" href="data:,"><div id="root"></div><script type="module">import R from '/@react-refresh';R.injectIntoGlobalHook(window);window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>t=>t;window.__vite_plugin_react_preamble_installed__=true;await import('/account-connect-fixture');</script></html>`}))
  await page.route('**/account-connect-fixture',r=>r.fulfill({contentType:'application/javascript',body:`import React from '/node_modules/.vite/deps/react.js';import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';import {MainPanelWindow} from '/src/views/MainPanelWindow.tsx';import '/src/index.css';ReactDOM.createRoot(document.getElementById('root')).render(React.createElement('div',{style:{height:'100vh'}},React.createElement(MainPanelWindow,{embedded:true,initialPage:'providers'})));`}))
  await page.goto(origin+'/account-connect-check')
  const nav=page.locator('nav[aria-label="主导航"]'), dialog=page.getByRole('dialog')
  await page.getByRole('tab',{name:'账号管理',exact:true}).click()
  await page.getByRole('button',{name:'添加账号',exact:true}).first().click()
  await dialog.waitFor({timeout:3000})
  assert.equal(await page.getByRole('tab',{name:'账号管理',exact:true,includeHidden:true}).getAttribute('aria-selected'),'true')
  await dialog.getByRole('button',{name:'取消',exact:true}).click();await dialog.waitFor({state:'hidden'})
  await nav.getByRole('button',{name:'一键签到',exact:true}).click()
  await page.getByRole('heading',{name:'每日福利，一次领取'}).waitFor()
  await page.waitForFunction(()=>[...document.querySelectorAll('[data-checkin-toolbar] button')].some(n=>n.textContent==='刷新状态'&&!n.disabled))
  assert.equal(await dialog.count(),0,'自动查询不弹窗')
  const out=path.resolve(__dirname,'../../output/acceptance/account-connect-dialogs');fs.mkdirSync(out,{recursive:true})
  const toolbar=page.locator('[data-checkin-toolbar]')
  await toolbar.getByRole('button',{name:'添加账号',exact:true}).waitFor({timeout:3000})
  assert.deepEqual(await toolbar.getByRole('button').allTextContents(),['','刷新状态','添加账号','一键签到'])
  assert.equal(await page.locator('[data-checkin-edition] header button').count(),0,'账号卡片移除加号')
  await toolbar.getByRole('button',{name:'添加账号',exact:true}).click()
  await dialog.getByRole('heading',{name:'添加账号',exact:true}).waitFor()
  assert.equal(await dialog.getByRole('button',{name:'WorkBuddy AI 国际版',exact:true}).isDisabled(),true)
  assert.equal(await dialog.getByRole('button',{name:'Trae 国际版',exact:true}).isDisabled(),true)
  await page.screenshot({path:path.join(out,'checkin-provider-picker-light.png')})
  await page.evaluate(()=>document.documentElement.classList.add('dark'))
  await page.screenshot({path:path.join(out,'checkin-provider-picker-dark.png')})
  await page.evaluate(()=>document.documentElement.classList.remove('dark'))
  await page.setViewportSize({width:480,height:600})
  const pickerBounds=await dialog.evaluate(n=>({width:n.getBoundingClientRect().width,overflow:n.scrollWidth>n.clientWidth+1}))
  assert.equal(pickerBounds.overflow,false)
  assert.ok(pickerBounds.width<=480-48)
  await dialog.getByRole('button',{name:'Qoder 国际版',exact:true}).scrollIntoViewIfNeeded()
  await page.screenshot({path:path.join(out,'checkin-provider-picker-narrow.png')})
  await page.setViewportSize({width:1280,height:900})
  await dialog.getByRole('button',{name:'取消',exact:true}).click();await dialog.waitFor({state:'hidden'})
  await page.evaluate(()=>window.fixture.holdStatus=true)
  await toolbar.getByRole('button',{name:'刷新状态',exact:true}).click()
  await page.waitForFunction(()=>window.fixture.statusWaiters.length>0)
  assert.equal(await dialog.count(),0,'刷新状态不打开居中弹窗')
  assert.equal(await page.locator('[aria-label="每日签到"] > p[role="status"]').count(),0,'刷新时不显示页面内进度条')
  assert.equal(await toolbar.getByRole('button',{name:'刷新状态',exact:true}).isDisabled(),true,'查询期间禁止重复刷新')
  await page.evaluate(()=>{window.fixture.holdStatus=false;window.fixture.statusWaiters.splice(0).forEach(resolve=>resolve())})
  await page.waitForFunction(()=>[...document.querySelectorAll('[data-checkin-toolbar] button')].some(n=>n.textContent==='刷新状态'&&!n.disabled))
  assert.ok(await page.evaluate(()=>window.fixture.calls.some(c=>c.command==='connection_quota'&&c.id==='trae-a'&&c.force===true)),'手动刷新强制更新该账号额度缓存')
  const refreshed=page.locator('[data-toast-id]').filter({has:page.getByText('签到状态已刷新',{exact:true})})
  await refreshed.waitFor()
  assert.ok((await refreshed.boundingBox()).x>640,'刷新结果显示在主窗口右侧')
  assert.equal(await dialog.count(),0)
  await page.screenshot({path:path.join(out,'checkin-refresh-toast-light.png')})
  await page.evaluate(()=>document.documentElement.classList.add('dark'))
  await page.screenshot({path:path.join(out,'checkin-refresh-toast-dark.png')})
  await page.evaluate(()=>document.documentElement.classList.remove('dark'))
  await refreshed.getByRole('button',{name:'关闭提示：签到状态已刷新',exact:true}).click()
  const forcedBeforeHidden=await page.evaluate(()=>window.fixture.calls.filter(c=>c.command==='connection_quota'&&c.force===true).length)
  await page.evaluate(()=>window.fixture.holdStatus=true)
  await toolbar.getByRole('button',{name:'刷新状态',exact:true}).click()
  await page.waitForFunction(()=>window.fixture.statusWaiters.length>0)
  await nav.getByRole('button',{name:'供应商',exact:true}).click()
  await page.evaluate(()=>{window.fixture.holdStatus=false;window.fixture.statusWaiters.splice(0).forEach(resolve=>resolve())})
  await page.waitForFunction(()=>[...document.querySelectorAll('[data-checkin-toolbar] button')].some(n=>n.textContent==='刷新状态'&&!n.disabled))
  assert.equal(await page.evaluate(()=>window.fixture.calls.filter(c=>c.command==='connection_quota'&&c.force===true).length),forcedBeforeHidden,'切走后状态返回不再发起强制额度查询')
  await nav.getByRole('button',{name:'一键签到',exact:true}).click()
  await toolbar.getByRole('button',{name:'刷新状态',exact:true}).waitFor()
  await page.waitForFunction(()=>[...document.querySelectorAll('[data-checkin-toolbar] button')].some(n=>n.textContent==='刷新状态'&&!n.disabled))
  const hiddenRefreshToast=page.locator('[data-toast-id]').filter({has:page.getByText('签到状态已刷新',{exact:true})})
  if(await hiddenRefreshToast.count())await hiddenRefreshToast.getByRole('button',{name:'关闭提示：签到状态已刷新',exact:true}).click()
  await page.evaluate(()=>window.fixture.failOverview=true)
  await toolbar.getByRole('button',{name:'刷新状态',exact:true}).click()
  const failed=page.locator('[data-toast-id]').filter({has:page.getByText('签到状态刷新失败',{exact:true})})
  await failed.getByRole('button',{name:'关闭提示：签到状态刷新失败',exact:true}).click()
  assert.equal(await dialog.count(),0)
  await page.evaluate(()=>window.fixture.failOverview=false)
  for(const [id,name,edition]of [['workbuddy-cn','WorkBuddy','cn'],['qoder-cn','Qoder CN','cn'],['qoder-global','Qoder','global']]){
   await page.locator(`[data-checkin-edition="${id}"]`).getByRole('button',{name:'去连接',exact:true}).click()
   await dialog.waitFor({timeout:3000})
   assert.equal(await nav.getByRole('button',{name:'一键签到',exact:true,includeHidden:true}).getAttribute('aria-current'),'page')
   await dialog.getByText(name,{exact:true}).first().waitFor()
   await dialog.getByRole('button',{name:'浏览器授权',exact:true}).click()
   await dialog.getByText('verified@example.test',{exact:true}).waitFor()
   assert.equal(await page.evaluate(()=>window.fixture.authorization.edition),edition)
   assert.equal(await dialog.getByRole('textbox',{name:'连接名称 *',exact:true}).count(),0,'订阅账号不需要填写名称')
   await dialog.getByRole('button',{name:'保存连接',exact:true}).click()
   await dialog.waitFor({state:'hidden'})
   await page.locator(`[data-checkin-edition="${id}"] [data-checkin-account]`).waitFor()
  }
  const trae=page.locator('[data-checkin-edition="trae-cn"]')
  await toolbar.getByRole('button',{name:'添加账号',exact:true}).click()
  await dialog.getByRole('button',{name:'Trae 国内版',exact:true}).click()
  await dialog.getByText('Trae CN',{exact:true}).first().waitFor()
  await dialog.getByRole('button',{name:'浏览器授权',exact:true}).click()
  await dialog.getByText('verified@example.test',{exact:true}).waitFor()
  assert.equal(await dialog.getByRole('textbox',{name:'连接名称 *',exact:true}).count(),0)
  await page.evaluate(()=>window.fixture.holdSave=true)
  const save=dialog.getByRole('button',{name:'保存连接',exact:true})
  const before=await page.evaluate(()=>window.fixture.calls.filter(c=>c.command==='save_browser_connection').length)
  await save.evaluate(n=>{n.click();n.click();n.click()});await page.waitForFunction(()=>typeof window.fixture.releaseSave==='function')
  for(const name of ['更换平台','浏览器授权','读取本地登录信息'])assert.equal(await dialog.getByRole('button',{name,exact:true}).isDisabled(),true,`保存时禁用${name}`)
  assert.equal(await dialog.getByRole('textbox',{name:'连接名称 *',exact:true}).count(),0)
  assert.equal(await page.evaluate(()=>window.fixture.calls.filter(c=>c.command==='save_browser_connection').length),before+1)
  await page.evaluate(()=>window.fixture.releaseSave())
  await dialog.waitFor({state:'hidden'})
  await page.waitForFunction(()=>document.querySelectorAll('[data-checkin-edition="trae-cn"] [data-checkin-account]').length===2)
  await trae.getByText('账号：first@example.test',{exact:true}).waitFor();await trae.getByText('账号：verified@example.test',{exact:true}).waitFor()
  await page.screenshot({path:path.join(out,'checkin-multiple-accounts-light.png')})
  await toolbar.getByRole('button',{name:'添加账号',exact:true}).click()
  await dialog.getByRole('button',{name:'Trae 国内版',exact:true}).click()
  await dialog.waitFor();await page.screenshot({path:path.join(out,'checkin-connect-modal-light.png')})
  await page.evaluate(()=>document.documentElement.classList.add('dark'))
  await page.screenshot({path:path.join(out,'checkin-connect-modal-dark.png')})
  assert.deepEqual(errors,[])
  console.log('PASS: 顶部添加账号与供应商选择/卡片无加号/刷新结果右侧通知/无居中弹窗/失败通知/账号原地弹窗/版本预选/Trae多账号/保存冻结与防连点/浅深色')
 }finally{await browser.close()}
})().catch(e=>{console.error(e);process.exitCode=1})
