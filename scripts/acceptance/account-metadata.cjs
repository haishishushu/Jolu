// Synthetic component verification only; real desktop/backend checks are recorded separately.
const path=require('node:path');
const {chromium}=require(require.resolve('playwright-core',{paths:[path.resolve(__dirname,'../../output/acceptance/node')]}));
const assert=require('node:assert/strict');
const fs=require('node:fs');
(async()=>{
 const browser=await chromium.launch({channel:'msedge',headless:true});
 try {
 const page=await browser.newPage({viewport:{width:1280,height:1000}}); const errors=[];page.on('pageerror',e=>{errors.push(e.message);console.log('PAGEERROR',e.message)});page.on('console',m=>{if(m.type()==='error')console.log('CONSOLE',m.text())});
 await page.route('**/account-metadata-check',r=>r.fulfill({contentType:'text/html',body:`<!doctype html><html><head><meta charset="UTF-8"/></head><body><div id="root"></div><script type="module">
 import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window); window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>type=>type;window.__vite_plugin_react_preamble_installed__=true;
 await import('/account-metadata-module');</script></body></html>`}));
 await page.route('**/account-metadata-module',r=>r.fulfill({contentType:'application/javascript',body:`import React from '/node_modules/.vite/deps/react.js'; import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js'; import {AccountsPanel} from '/src/components/providers/AccountsPanel.tsx';import '/src/index.css';import {TipProvider} from '/src/components/ui/Tip.tsx';
 const noop=async()=>{};const connections=['codex','claude','codex','claude'].map((platformId,i)=>({id:String(i),platformId,kind:'auth',name:'测试账号'+i,baseName:'测试账号'+i,accountEmail:'fixture'+i+'@example.test',label:'',masked:'',status:'active',lastSyncText:'刚刚同步',baseUrl:null}));
 ReactDOM.createRoot(document.getElementById('root')).render(React.createElement(TipProvider,null,React.createElement('div',{className:'flex flex-col gap-4 p-8 bg-jolu-bg text-jolu-text'},React.createElement(AccountsPanel,{conn:{connections,loading:false,error:null,remove:noop,reload:noop},onConnect:noop,onBrowseProviders:noop}))));` }));
 await page.route('**/src/hooks/useQuota.ts*',r=>r.fulfill({contentType:'application/javascript',body:`const common={loading:false,stale:false,fetchedAt:new Date(),refresh:async()=>{}};export function useQuota(id){return {...common,state:{state:'ok',plan:'Pro',windows:[{key:'7d',window_name:'周额度',used_percent:15,resets_at:'2026-10-30T10:00:00Z',amount_text:null}],account:id==='0'?{subscription_status:'unknown',subscription_source:'local_snapshot',expires_at:'2026-10-25T10:00:00Z',observed_at:'2026-10-20T10:00:00Z',reset_credits:{available_count:1,applicable_available_count:0}}:null}}}export function useBalance(){return {...common,state:null}}`}));
 await page.goto('http://localhost:5193/account-metadata-check'); await page.locator('article').first().waitFor();
 assert.equal(await page.getByText('5 小时额度',{exact:true}).count(),4);assert.equal(await page.getByText('周额度',{exact:true}).count(),4);
 assert.equal(await page.getByText('剩余 1 次 · 当前可用 0 次',{exact:true}).count(),1);assert.equal(await page.getByText('2026-10-25',{exact:true}).count(),1);
 assert.equal(await page.getByRole('button',{name:'查询重置卡',exact:true}).count(),2);
 assert.equal(await page.getByRole('button',{name:'设置为使用',exact:true}).count(),4);
 const cards=await page.locator('article').evaluateAll(es=>es.map(e=>({x:e.getBoundingClientRect().x,y:e.getBoundingClientRect().y,overflow:e.scrollWidth>e.clientWidth})));
 assert.equal(new Set(cards.map(x=>x.y)).size,1);assert.ok(cards.every(x=>!x.overflow));
 fs.mkdirSync('output/playwright',{recursive:true});await page.screenshot({path:'output/playwright/account-metadata-light.png',fullPage:true});
 await page.getByRole('button',{name:'隐藏敏感信息',exact:true}).click();assert.ok(!(await page.locator('article').first().innerHTML()).includes('fixture0@example.test'));
 await page.evaluate(()=>document.documentElement.classList.add('dark'));await page.screenshot({path:'output/playwright/account-metadata-dark.png',fullPage:true});
 assert.deepEqual(errors,[]);console.log(JSON.stringify({pass:true,cards,checks:'four columns, both windows with unknown value, snapshot date, reset credits zero, promotion and query controls, privacy, no runtime errors'}));
 }finally{await browser.close()}
})().catch(e=>{console.error(e);process.exit(1)});
