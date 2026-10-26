// 真实连接组件+合成原生桥接；不读取登录、不开真实授权页、不联网。
const path=require('node:path'),assert=require('node:assert/strict'),fs=require('node:fs');
const {chromium}=require(require.resolve('playwright-core',{paths:[path.resolve(__dirname,'../../output/acceptance/node')]}));
const origin=process.env.JOLU_COMPONENT_URL??'http://localhost:5314';
assert.ok(['localhost','127.0.0.1'].includes(new URL(origin).hostname));
(async()=>{
 const browser=await chromium.launch({channel:'msedge',headless:true});
 try {
  const page=await browser.newPage({viewport:{width:1100,height:1000}}),errors=[];
  page.on('pageerror',e=>errors.push(e.message));
  await page.addInitScript(()=>{
   window.fixture={calls:[],ready:false,closed:false};
   window.__TAURI_INTERNALS__={invoke:async(command,args)=>{
    window.fixture.calls.push({command,...args});
    if(command==='start_browser_authorization')return window.fixture.holdStart?new Promise(resolve=>window.fixture.resolveStart=resolve):'synthetic-attempt';
    if(command==='browser_authorization_status')return window.fixture.ready?{state:'ready',detection:{found:true,account:'synthetic@example.test',plan:'Test plan',sources:['浏览器授权'],masked:'已验证'},message:null}:{state:'waiting',detection:null,message:'等待官网登录'};
    if(command==='cancel_browser_authorization')return;
    if(command==='save_browser_connection')return 'browser:synthetic-attempt';
    if(command==='detect_local_login')return {found:true,account:'local@example.test',plan:null,sources:[],masked:null};
    if(command==='read_local_connections')return {added:1,existing:0};
    throw Error(`unexpected command ${command}`);
   }};
  });
  await page.route('**/provider-auth-check*',r=>r.fulfill({contentType:'text/html',body:`<!doctype html><html><head><meta charset="utf-8"></head><body><div id="root"></div><script type="module">import R from '/@react-refresh';R.injectIntoGlobalHook(window);window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>t=>t;window.__vite_plugin_react_preamble_installed__=true;await import('/provider-auth-module');</script></body></html>`}));
  await page.route('**/provider-auth-module',r=>r.fulfill({contentType:'application/javascript',body:`import React from '/node_modules/.vite/deps/react.js';import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';import {ConnectDialog} from '/src/components/providers/ConnectDialog.tsx';import {TipProvider} from '/src/components/ui/Tip.tsx';import {ToastProvider} from '/src/components/ui/Toast.tsx';import '/src/index.css';const actions={connections:[],readLocal:(platform,kind,name,edition)=>window.__TAURI_INTERNALS__.invoke('read_local_connections',{platform,kind,name,edition}),add:async()=>{throw Error('unexpected API save')}};const root=ReactDOM.createRoot(document.getElementById('root'));window.fixture.unmount=()=>root.unmount();root.render(React.createElement(TipProvider,null,React.createElement(ToastProvider,null,React.createElement(ConnectDialog,{platform:(new URLSearchParams(location.search).get('provider')??'workbuddy'),actions,onClose:()=>{window.fixture.closed=true}}))));`}));
  await page.goto(`${origin}/provider-auth-check`);
  await page.getByRole('button',{name:'读取本地登录信息',exact:true}).waitFor();
  const authorize=page.getByRole('button',{name:'浏览器授权',exact:true});
  await authorize.waitFor();
  const output=path.resolve(__dirname,'../../output/acceptance/provider-auth');fs.mkdirSync(output,{recursive:true});
  await page.screenshot({path:path.join(output,'two-entries-light.png'),fullPage:true});
  await page.evaluate(()=>document.documentElement.classList.add('dark'));
  await page.screenshot({path:path.join(output,'two-entries-dark.png'),fullPage:true});
  await page.evaluate(()=>document.documentElement.classList.remove('dark'));
  await page.getByRole('textbox',{name:'连接名称 *',exact:true}).fill('浏览器测试');
  await authorize.click();
  await page.waitForFunction(()=>window.fixture.calls.some(c=>c.command==='browser_authorization_status'));
  assert.equal(await page.getByRole('button',{name:'保存连接',exact:true}).isDisabled(),true);
  await page.evaluate(()=>window.fixture.ready=true);
  await page.getByText('synthetic@example.test',{exact:true}).waitFor();
  await page.getByRole('button',{name:'保存连接',exact:true}).click();
  await page.waitForFunction(()=>window.fixture.closed);
  const calls=await page.evaluate(()=>window.fixture.calls);
  assert.equal(calls.filter(c=>c.command==='start_browser_authorization').length,1);
  assert.equal(calls.filter(c=>c.command==='save_browser_connection').length,1);
  assert.equal(calls.filter(c=>c.command==='read_local_connections').length,0);
  assert.equal(calls.find(c=>c.command==='start_browser_authorization').edition,'cn');
  await page.goto(`${origin}/provider-auth-check?provider=workbuddy-ai`);
  await page.getByRole('textbox',{name:'连接名称 *',exact:true}).fill('本地国际版');
  await page.getByRole('button',{name:'读取本地登录信息',exact:true}).click();
  await page.getByText('local@example.test',{exact:true}).waitFor();
  await page.getByRole('button',{name:'保存连接',exact:true}).click();
  await page.waitForFunction(()=>window.fixture.closed);
  const local=await page.evaluate(()=>window.fixture.calls);
  assert.equal(local.find(c=>c.command==='detect_local_login').edition,'ai');
  assert.equal(local.find(c=>c.command==='read_local_connections').edition,'ai');
  assert.equal(local.filter(c=>c.command==='save_browser_connection').length,0);
  await page.reload();
  await page.getByRole('button',{name:'浏览器授权',exact:true}).waitFor();
  await page.evaluate(()=>window.fixture.holdStart=true);
  await page.getByRole('button',{name:'浏览器授权',exact:true}).click();
  await page.waitForFunction(()=>window.fixture.resolveStart);
  await page.evaluate(()=>{window.fixture.unmount();window.fixture.resolveStart('late-attempt')});
  await page.waitForFunction(()=>window.fixture.calls.some(c=>c.command==='cancel_browser_authorization'&&c.id==='late-attempt'));
  assert.equal(await page.evaluate(()=>window.fixture.calls.filter(c=>c.command==='save_browser_connection').length),0);
  for (const [provider,family,edition] of [
    ['workbuddy','workbuddy','cn'],['workbuddy-ai','workbuddy','ai'],
    ['trae-cn','trae','cn'],['trae','trae','global'],['qoder-cn','qoder','cn'],['qoder','qoder','global'],
    ['claude','claude','default'],['codex','codex','default'],['gemini','gemini','default'],
    ['grok','grok','default'],['zcode','zcode','default'],['opencode','opencode','default'],
  ]) {
    await page.goto(`${origin}/provider-auth-check?provider=${provider}`);
    if(provider==='opencode') {
      await page.getByRole('button',{name:'模型服务',exact:true}).click();
      await page.getByRole('menuitemradio',{name:'OpenAI 订阅',exact:true}).click();
    }
    const button=page.getByRole('button',{name:'浏览器授权',exact:true});
    await button.waitFor();assert.equal(await button.isEnabled(),true,provider);
    await button.click();
    await page.waitForFunction(()=>window.fixture.calls.some(c=>c.command==='browser_authorization_status'));
    const started=await page.evaluate(()=>window.fixture.calls.find(c=>c.command==='start_browser_authorization'));
    assert.equal(started.platform,family,provider);assert.equal(started.edition,edition,provider);
    await page.evaluate(()=>window.fixture.ready=true);
    await page.getByText('synthetic@example.test',{exact:true}).waitFor();
    assert.equal(await page.getByRole('button',{name:'保存连接',exact:true}).isDisabled(),true,provider);
    await page.getByText('请填写连接名称后保存。',{exact:true}).waitFor();
    await page.getByRole('textbox',{name:'连接名称 *',exact:true}).fill(`测试 ${provider}`);
    await page.getByRole('button',{name:'保存连接',exact:true}).click();
    await page.waitForFunction(()=>window.fixture.closed);
    assert.equal(await page.evaluate(()=>window.fixture.calls.filter(c=>c.command==='save_browser_connection').length),1,provider);
  }
  assert.deepEqual(errors,[]);
  const grid=await browser.newPage({viewport:{width:1280,height:1100}});
  await grid.route('**/provider-grid-check',r=>r.fulfill({contentType:'text/html',body:`<!doctype html><html><head><meta charset="utf-8"></head><body><div id="root"></div><script type="module">import R from '/@react-refresh';R.injectIntoGlobalHook(window);window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>t=>t;window.__vite_plugin_react_preamble_installed__=true;await import('/provider-grid-module');</script></body></html>`}));
  await grid.route('**/provider-grid-module',r=>r.fulfill({contentType:'application/javascript',body:`import React from '/node_modules/.vite/deps/react.js';import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';import {ProvidersView} from '/src/views/ProvidersView.tsx';import {TipProvider} from '/src/components/ui/Tip.tsx';import {ToastProvider} from '/src/components/ui/Toast.tsx';import '/src/index.css';ReactDOM.createRoot(document.getElementById('root')).render(React.createElement(TipProvider,null,React.createElement(ToastProvider,null,React.createElement('div',{className:'p-6 bg-jolu-bg'},React.createElement(ProvidersView)))));`}));
  await grid.goto(`${origin}/provider-grid-check`);
  for(const name of ['WorkBuddy','WorkBuddy AI','Trae CN','Trae','Qoder CN','Qoder'])await grid.getByRole('button',{name:`连接 ${name}`,exact:true}).waitFor();
  assert.equal(await grid.getByRole('button',{name:/^连接 /}).count(),12);
  await grid.screenshot({path:path.join(output,'all-provider-cards-light.png'),fullPage:true});
  await grid.evaluate(()=>document.documentElement.classList.add('dark'));
  await grid.screenshot({path:path.join(output,'all-provider-cards-dark.png'),fullPage:true});
  console.log('PASS: all 12 subscription entries, six edition routes, browser save gate, local edition binding, late authorization cancellation');
 }finally{await browser.close()}
})().catch(error=>{console.error(error);process.exitCode=1});
