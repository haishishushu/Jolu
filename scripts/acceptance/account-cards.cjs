// 真实供应商页与账号组件，合成桥接；不访问桌面或真实供应商。
const path=require('node:path'),assert=require('node:assert/strict'),fs=require('node:fs')
const {chromium}=require(require.resolve('playwright-core',{paths:[path.resolve(__dirname,'../../output/acceptance/node')]}))
const origin=process.env.JOLU_COMPONENT_URL??'http://localhost:5314'
assert.ok(['localhost','127.0.0.1'].includes(new URL(origin).hostname))
;(async()=>{
 const browser=await chromium.launch({channel:'msedge',headless:true})
 try{
  const page=await browser.newPage({viewport:{width:1280,height:900}}),errors=[]
  page.on('pageerror',e=>{errors.push(e.message);console.error(e.message)})
  await page.addInitScript(()=>{
   const entries=[['codex','codex',null,'auth'],['claude','claude',null,'auth'],['gemini','gemini',null,'auth'],['workbuddy','workbuddy','cn','auth'],['trae-cn','trae','cn','auth'],['qoder-cn','qoder','cn','auth'],['zcode','zcode',null,'auth'],['custom','custom',null,'api'],['workbuddy-ai','workbuddy','ai','auth'],['trae','trae','global','auth'],['qoder','qoder','global','auth'],['opencode','opencode',null,'auth'],['grok','grok',null,'auth'],['api','codex',null,'api']]
   const rows=entries.map(([id,platform,edition,kind])=>({id,platform,edition,kind,name:id==='codex'?'codex@example.test':id,label:'浏览器授权',masked:kind==='api'?'sk-****test':'',status:'connected',last_sync_text:'刚刚同步',base_url:kind==='api'?'https://synthetic.example/v1':null,model:kind==='api'?'synthetic-model':null,effort:null,context_1m:null,protocol:kind==='api'?'openai':null,service:null,account_email:kind==='auth'?`${id}@example.test`:null}))
   const win=(key,remaining=null)=>({key,window_name:key,used_percent:remaining===null?25:null,remaining_text:remaining,resets_at:'2026-11-30T10:00:00Z',amount_text:null})
   const settings={refresh_minutes:5,island_clones:[],default_page:'providers',last_page:'providers',main_zoom:100,list_density:'comfortable',reduce_motion:false,theme:'light',island_visible:true}
   window.fixture={calls:[],callbacks:{},rows,expired:false,settings}
   window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},transformCallback:callback=>{const id=Object.keys(window.fixture.callbacks).length+1;window.fixture.callbacks[id]=callback;return id},unregisterCallback:()=>{},invoke:async(command,args)=>{
    window.fixture.calls.push({command,...args})
    if(command==='plugin:event|listen')return 1
    if(command==='plugin:event|unlisten')return
    if(command==='get_settings')return settings
    if(command==='take_main_intent')return null
    if(command==='main_panel_ready'||command==='remember_page')return
    if(command==='plugin:window|is_visible')return true
    if(command==='plugin:window|is_minimized'||command==='plugin:window|is_maximized')return false
    if(command==='list_connections')return rows
    if(command==='current_connections')return ['codex']
    if(command==='connection_balance')return ['custom','api'].includes(args.id)?{state:'ok',balance:0,used:0,currency:'USD'}:{state:'unsupported',reason:'该服务没有钱包余额'}
    if(command==='connection_quota'){
      if(args.id==='grok'&&window.fixture.expiryState)return {state:window.fixture.expiryState,reason:'synthetic expiry query state'}
      if(args.id==='custom')return {state:'unsupported',reason:'该服务不提供套餐额度'}
      if(args.id==='grok'&&window.fixture.expired)return {state:'unauthorized',reason:'synthetic expired'}
      const reset=['codex','opencode','api'].includes(args.id)?{available_count:0,applicable_available_count:0}:null
      const windows=args.id==='gemini'?Array.from({length:6},(_,i)=>win(`gemini-model-${i+1}`,'剩余 0 次')):args.id==='claude'?[win('5h'),win('7d'),win('7d-sonnet')]:args.id==='codex'||args.id==='opencode'?[win('5h'),win('7d')]:[win('credits','剩余 90 积分')]
      return {state:'ok',plan:args.id==='claude'||args.id==='grok'?null:'Pro',windows,account:{email:`${args.id}@example.test`,subscription_source:args.id==='codex'?'local_snapshot':'live',subscription_status:'unknown',expires_at:['codex','qoder','qoder-cn','api'].includes(args.id)?'2026-11-30T00:00:00Z':null,observed_at:'2026-10-26T00:00:00Z',reset_credits:reset}}
    }
    throw Error(`unexpected ${command}`)
   }}
  })
  await page.route('**/src/hooks/useSettings.ts*',r=>r.fulfill({contentType:'application/javascript',body:'export function useSettings(){return {settings:window.fixture.settings,ready:true}}'}))
  await page.route('**/account-cards-check',r=>r.fulfill({contentType:'text/html',body:`<!doctype html><html><head><meta charset="utf-8"></head><body><div id="root"></div><script type="module">import R from '/@react-refresh';R.injectIntoGlobalHook(window);window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>t=>t;window.__vite_plugin_react_preamble_installed__=true;await import('/account-cards-module');</script></body></html>`}))
  await page.route('**/account-cards-module',r=>r.fulfill({contentType:'application/javascript',body:`
    import React from '/node_modules/.vite/deps/react.js';import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
    import {MainPanelWindow} from '/src/views/MainPanelWindow.tsx';import '/src/index.css';
    const e=React.createElement;
    ReactDOM.createRoot(document.getElementById('root')).render(e('div',{style:{height:'100vh'}},e(MainPanelWindow,{embedded:true,initialPage:'providers'})));
  `}));
  await page.goto(`${origin}/account-cards-check`)
  await page.getByRole('tab',{name:'账号管理',exact:true}).click()
  const grid=page.locator('[data-account-grid]'),cards=grid.locator('article')
  try {await cards.first().getByText('剩余 75%',{exact:true}).first().waitFor({timeout:10000})}
  catch(error){console.error(await page.locator('body').innerText());throw error}
  assert.equal(await cards.count(),14)
  for(const name of ['Claude Code','Codex'])assert.equal(await page.getByRole('button',{name,exact:true}).locator('span[aria-hidden="true"] [role="img"]').count(),1)
  await page.getByRole('button',{name:'更多供应商',exact:true}).click()
  const menu=page.getByRole('menu'),items=menu.getByRole('menuitem')
  assert.equal(await items.count(),14)
  for(let i=0;i<14;i++)assert.equal(await items.nth(i).locator(':scope > span[aria-hidden="true"]').count(),1)
  assert.equal(await menu.locator('img').evaluateAll(nodes=>nodes.every(node=>node.complete&&node.naturalWidth>0)),true)
  assert.equal(await items.locator(':scope > span[title]').evaluateAll(nodes=>nodes.some(node=>node.scrollWidth>node.clientWidth)),false)
  const iconOutput=path.resolve(__dirname,'../../output/acceptance/account-cards');fs.mkdirSync(iconOutput,{recursive:true})
  await page.screenshot({path:path.join(iconOutput,'provider-filter-icons-light.png')})
  await menu.getByRole('menuitem',{name:'WorkBuddy AI · 1',exact:true}).click()
  await page.waitForFunction(()=>document.querySelectorAll('[data-account-grid] article').length===1)
  assert.equal(await page.getByRole('button',{name:'WorkBuddy AI',exact:true}).locator('span[aria-hidden="true"] > :is(svg,img,[role="img"])').count(),1)
  await page.getByRole('button',{name:'全部供应商',exact:true}).click()
  await page.waitForFunction(()=>document.querySelectorAll('[data-account-grid] article').length===14)
  const geometry=()=>page.evaluate(()=>{
   const grid=document.querySelector('[data-account-grid]'),r=grid.getBoundingClientRect(),items=[...grid.querySelectorAll('article')].map(n=>{const b=n.getBoundingClientRect();return {top:b.top,bottom:b.bottom,left:b.left,width:b.width,overflow:n.scrollWidth>n.clientWidth,summaryOverflow:n.querySelector('[data-account-summary]').scrollHeight>n.querySelector('[data-account-summary]').clientHeight+1}})
   return {height:innerHeight,top:r.top,bottom:r.bottom,visible:items.filter(n=>n.top>=r.top-1&&n.bottom<=r.bottom+1).length,items,outerOverflow:document.querySelector('[data-panel-scroll]').scrollHeight>document.querySelector('[data-panel-scroll]').clientHeight+1}
  })
  for(const [width,height] of [[1280,900],[1920,1080]]){
   await page.setViewportSize({width,height});await page.waitForTimeout(100)
   const g=await geometry()
   assert.ok(g.bottom<=height,JSON.stringify(g))
   assert.equal(g.visible,8,JSON.stringify(g));assert.equal(g.outerOverflow,false,JSON.stringify(g))
   assert.equal(new Set(g.items.slice(0,4).map(n=>Math.round(n.top))).size,1)
   assert.equal(new Set(g.items.slice(0,4).map(n=>Math.round(n.left))).size,4)
   assert.equal(g.items.some(n=>n.overflow||n.summaryOverflow),false,JSON.stringify(g))
  }
  assert.equal(await cards.getByText('重置卡',{exact:true}).count(),2)
  assert.equal(await cards.getByText('订阅到期',{exact:true}).count(),12)
  await cards.first().getByText('2026-11-30 · 本机',{exact:true}).waitFor()
  await cards.nth(3).getByText('未提供',{exact:true}).waitFor()
  assert.equal(await cards.getByText('今日消费',{exact:true}).count(),0)
  assert.equal(await cards.getByText('累计充值',{exact:true}).count(),0)
  assert.equal(await cards.nth(1).getByText('浏览器授权',{exact:true}).count(),0)
  await cards.nth(7).getByText('$0.00',{exact:true}).first().waitFor()
  await cards.nth(2).getByRole('button',{name:'查看账号详情'}).click()
  const dialog=page.getByRole('dialog',{name:'账号详情'})
  await dialog.getByText('订阅到期',{exact:true}).waitFor()
  await dialog.getByText('未提供',{exact:true}).waitFor()
  for(let i=1;i<=6;i++)await dialog.getByText(`gemini-model-${i}`,{exact:true}).waitFor()
  await dialog.getByRole('button',{name:'关闭',exact:true}).last().click()
  for(let i=0;i<14;i++){
   await cards.nth(i).getByRole('button',{name:'查看账号详情'}).click()
   if([7,13].includes(i)){
    assert.equal(await dialog.getByText('订阅到期',{exact:true}).count(),0)
    await dialog.getByText('sk-****test',{exact:true}).waitFor()
    if(i===13){await dialog.getByText('套餐到期',{exact:true}).waitFor();await dialog.getByText('credits',{exact:true}).waitFor();await dialog.getByText('重置卡',{exact:true}).waitFor()}
   }
   else{
    await dialog.getByText('订阅到期',{exact:true}).waitFor()
    await dialog.getByText([0,5,10].includes(i)?(i===0?'2026-11-30 · 本机':'2026-11-30'):'未提供',{exact:true}).waitFor()
   }
   await dialog.getByRole('button',{name:'关闭',exact:true}).last().click()
  }
  await grid.evaluate(n=>n.scrollTop=0)
  await cards.first().getByRole('button',{name:'查询重置卡'}).click()
  await page.waitForFunction(()=>window.fixture.calls.some(c=>c.command==='connection_quota'&&c.id==='codex'&&c.force))
  assert.deepEqual(await page.evaluate(()=>window.fixture.calls.filter(c=>c.force).map(c=>c.id)),['codex'])
  for(const [state,label] of [['unauthorized','登录失效'],['forbidden','无查询权限'],['failed','查询失败'],['rate_limited','请求受限']]){
   await page.evaluate(state=>{window.fixture.expiryState=state},state)
   await cards.nth(12).getByRole('button',{name:'刷新当前账号',exact:true}).click()
   await cards.nth(12).getByText(label,{exact:true}).waitFor()
   await cards.nth(12).getByRole('button',{name:'查看账号详情'}).click()
   await dialog.getByText(label,{exact:true}).waitFor()
   await dialog.getByRole('button',{name:'关闭',exact:true}).last().click()
  }
  await grid.evaluate(n=>n.scrollTop=0)
  await grid.hover();await page.mouse.wheel(0,750)
  await page.waitForFunction(()=>document.querySelector('[data-account-grid]').scrollTop>0)
  assert.ok((await cards.nth(8).boundingBox()).y<await grid.evaluate(n=>n.getBoundingClientRect().bottom))
  await page.getByRole('textbox',{name:'搜索账号'}).fill('workbuddy-ai')
  await page.waitForFunction(()=>document.querySelectorAll('[data-account-grid] article').length===1)
  assert.equal(await grid.evaluate(n=>n.scrollTop),0)
  await page.getByRole('textbox',{name:'搜索账号'}).fill('')
  await page.waitForFunction(()=>document.querySelectorAll('[data-account-grid] article').length===14)
  const typeFilter=page.getByRole('group',{name:'账号类型',exact:true})
  await typeFilter.getByRole('button',{name:'第三方ApiKey',exact:true}).click()
  await page.waitForFunction(()=>document.querySelectorAll('[data-account-grid] article').length===2)
  assert.equal(await typeFilter.getByRole('button',{name:'第三方ApiKey',exact:true}).getAttribute('aria-pressed'),'true')
  assert.equal(await cards.getByText('订阅到期',{exact:true}).count(),0)
  assert.equal(await cards.getByText('sk-****test',{exact:true}).count(),2)
  assert.equal(await cards.getByText('synthetic-model',{exact:true}).count(),2)
  assert.equal(await cards.getByText('https://synthetic.example/v1',{exact:true}).count(),2)
  await page.getByRole('button',{name:'Codex',exact:true}).click()
  await page.waitForFunction(()=>document.querySelectorAll('[data-account-grid] article').length===1)
  await cards.first().getByText('api',{exact:true}).waitFor()
  await page.getByRole('textbox',{name:'搜索账号'}).fill('custom')
  await page.getByText('没有找到匹配的账号',{exact:true}).waitFor()
  await page.getByRole('button',{name:'清除筛选',exact:true}).click()
  await page.waitForFunction(()=>document.querySelectorAll('[data-account-grid] article').length===14)
  await typeFilter.getByRole('button',{name:'Auth官方订阅',exact:true}).click()
  await page.waitForFunction(()=>document.querySelectorAll('[data-account-grid] article').length===12)
  assert.equal(await cards.getByText('订阅到期',{exact:true}).count(),12)
  assert.equal(await grid.evaluate(n=>n.scrollTop),0)
  await typeFilter.getByRole('button',{name:'全部类型',exact:true}).click()
  await page.waitForFunction(()=>document.querySelectorAll('[data-account-grid] article').length===14)
  await page.setViewportSize({width:1280,height:900})
  await grid.evaluate(n=>n.scrollTop=0)
  await page.waitForTimeout(150)
  const finalGeometry=await geometry()
  assert.ok(finalGeometry.bottom<=900,JSON.stringify(finalGeometry))
  assert.equal(finalGeometry.visible,8,JSON.stringify(finalGeometry))
  assert.equal(finalGeometry.outerOverflow,false,JSON.stringify(finalGeometry))
  const output=path.resolve(__dirname,'../../output/acceptance/account-cards');fs.mkdirSync(output,{recursive:true})
  await page.screenshot({path:path.join(output,'eight-cards-light.png')})
  await page.evaluate(()=>document.documentElement.classList.add('dark'))
  await page.screenshot({path:path.join(output,'eight-cards-dark.png')})
  assert.deepEqual(errors,[])
  await page.evaluate(()=>document.documentElement.classList.remove('dark'))
  await typeFilter.getByRole('button',{name:'第三方ApiKey',exact:true}).click()
  await page.screenshot({path:path.join(output,'api-key-cards-light.png')})
  await page.evaluate(()=>document.documentElement.classList.add('dark'))
  await page.screenshot({path:path.join(output,'api-key-cards-dark.png')})
  const beforePrivacy=await cards.first().boundingBox()
  await page.getByRole('button',{name:'隐藏敏感信息',exact:true}).click()
  assert.equal(await cards.locator('.privacy-mosaic').count(),4)
  const afterPrivacy=await cards.first().boundingBox()
  assert.equal(beforePrivacy.width,afterPrivacy.width);assert.equal(beforePrivacy.height,afterPrivacy.height)
  assert.equal((await cards.evaluateAll(nodes=>nodes.map(n=>n.innerHTML).join(''))).includes('synthetic.example'),false)
  assert.equal((await cards.evaluateAll(nodes=>nodes.map(n=>n.innerHTML).join(''))).includes('sk-****test'),false)
  await cards.first().getByRole('button',{name:'查看账号详情'}).click()
  assert.equal((await dialog.innerHTML()).includes('synthetic.example'),false)
  assert.equal((await dialog.innerHTML()).includes('sk-****test'),false)
  await dialog.getByRole('button',{name:'关闭',exact:true}).last().click()
  await typeFilter.getByRole('button',{name:'全部类型',exact:true}).click()
  await page.waitForFunction(()=>document.querySelectorAll('[data-account-grid] article').length===14)
  assert.equal(await cards.first().getByRole('img',{name:'账号名称已隐藏',exact:true}).count(),1)
  assert.equal((await cards.first().innerHTML()).includes('codex@example.test'),false)
  assert.equal(await cards.locator('.privacy-mosaic').first().evaluate(node=>getComputedStyle(node,'::before').backgroundImage!=='none'),true)
  await page.evaluate(()=>document.documentElement.classList.remove('dark'))
  await page.screenshot({path:path.join(output,'privacy-mosaic-light.png')})
  await page.evaluate(()=>document.documentElement.classList.add('dark'))
  await page.screenshot({path:path.join(output,'privacy-mosaic-dark.png')})
  await page.getByRole('button',{name:'显示敏感信息',exact:true}).click()
  await cards.first().getByText('codex@example.test',{exact:true}).first().waitFor()
  assert.equal(await cards.locator('.privacy-mosaic').count(),0)
  console.log('PASS: real main-window layout, eight first-screen cards, distinct Auth/API cards, type/provider/search intersections and clear, details, real zero, privacy-safe API metadata and scoped queries')
 }finally{await browser.close()}
})().catch(error=>{console.error(error);process.exitCode=1})
