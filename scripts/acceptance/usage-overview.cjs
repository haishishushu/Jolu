// 合成统计响应 + 真实概览组件；不访问桌面、账号或原始会话。
const path=require('node:path'),assert=require('node:assert/strict'),fs=require('node:fs')
const {chromium}=require(require.resolve('playwright-core',{paths:[path.resolve(__dirname,'../../output/acceptance/node')]}))
const origin=process.env.JOLU_COMPONENT_URL??'http://localhost:5314'
assert.ok(['localhost','127.0.0.1'].includes(new URL(origin).hostname))
;(async()=>{
 const browser=await chromium.launch({channel:'msedge',headless:true})
 try{
  const page=await browser.newPage({viewport:{width:1280,height:900}}),errors=[]
  page.on('pageerror',e=>errors.push(e.message))
  await page.route('**/usage-overview-check',r=>r.fulfill({contentType:'text/html',body:`<!doctype html><html><head><meta charset="utf-8"></head><body><div id="root"></div><script type="module">import R from '/@react-refresh';R.injectIntoGlobalHook(window);window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>t=>t;window.__vite_plugin_react_preamble_installed__=true;await import('/usage-overview-module');</script></body></html>`}))
  await page.route('**/usage-overview-module',r=>r.fulfill({contentType:'application/javascript',body:`
   import React from '/node_modules/.vite/deps/react.js';import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
   import {OverviewPanel} from '/src/components/usage/OverviewPanel.tsx';import '/src/index.css';
   const root=ReactDOM.createRoot(document.getElementById('root'));
   const base={tokens:{fresh_input:null,output:null,cache_read:null,cache_write:null,total:null},token_coverage:{known_total:150,known_requests:2,cache_hit_rate:0.5,cache_requests:2},requests:3,failed:0,cache_hit_rate:null,cost:{amount:0,priced_requests:0,requests:3},previous_total:100,trend:{bucket:'2h',points:[]},models:[{platform:'codex',model:'gpt-5',requests:1,tokens:120,cache_hit_rate:0.6,avg_duration_ms:null,cost:null},{platform:'qoder',model:'qfmodel',requests:1,tokens:null,cache_hit_rate:null,avg_duration_ms:null,cost:null}],providers:[],attributed:0,sessions:[]};
   base.trend.points=[0,10,null,35,10].map((value,i)=>({start:Date.UTC(2026,9,26,i*2),end:Date.UTC(2026,9,26,i*2+2),label:i*2+':00',fresh_input:value,output:value,cache_read:value,cache_write:value,total:value===null?null:value*4,requests:1,cost:1}));
   window.renderOverview=patch=>root.render(React.createElement('div',{className:'flex flex-col gap-4 p-8'},React.createElement(OverviewPanel,{data:{...base,...patch},isToday:true,onDrillBucket:()=>{},onDrillModel:()=>{},onOpenSession:()=>{}})));
   window.renderOverview({});
  `}))
  await page.goto(origin+'/usage-overview-check')
  const metric=page.locator('section').first()
  await metric.getByText('已知 Token',{exact:true}).waitFor({timeout:5000})
  await metric.getByText('150',{exact:true}).waitFor()
  await metric.getByText('2 / 3 次有 Token · 1 次未提供',{exact:true}).waitFor()
  assert.equal(await metric.getByText(/较昨天/).count(),0)
  const cache=page.locator('section').nth(3)
  await cache.getByText('50.0%',{exact:true}).waitFor()
  await cache.getByText('2 / 3 次可计算 · 基于已知输入',{exact:true}).waitFor()
  await page.getByRole('button').filter({hasText:'GPT-5'}).getByText('120 · 80.0%',{exact:true}).waitFor()
  const chart=page.getByRole('img',{name:'用量趋势'})
  const paths=await chart.locator('path[stroke]').evaluateAll(nodes=>nodes.map(node=>node.getAttribute('d')))
  assert.equal(paths.length,4)
  for(const line of paths){assert.equal((line.match(/M/g)||[]).length,1);assert.equal((line.match(/C/g)||[]).length,3)}
  const bounds=await chart.boundingBox()
  await chart.hover({position:{x:bounds.width/2,y:40}})
  await page.getByRole('tooltip').getByText('此时段部分数据未提供，折线跨过缺失点连接。',{exact:true}).waitFor()
  const output=path.resolve(__dirname,'../../output/acceptance/usage-overview');fs.mkdirSync(output,{recursive:true})
  await page.screenshot({path:path.join(output,'connected-known-data.png'),fullPage:true})
  await page.evaluate(()=>window.renderOverview({token_coverage:{known_total:null,known_requests:0,cache_hit_rate:null,cache_requests:0}}))
  await metric.getByText('—',{exact:true}).waitFor()
  await cache.getByText('—',{exact:true}).waitFor()
  await page.evaluate(()=>window.renderOverview({tokens:{fresh_input:0,output:0,cache_read:0,cache_write:0,total:0},token_coverage:{known_total:0,known_requests:3,cache_hit_rate:null,cache_requests:3}}))
  await metric.getByText('总 Token',{exact:true}).waitFor()
  await metric.getByText('0',{exact:true}).waitFor()
  assert.deepEqual(errors,[])
  console.log('PASS: mixed known/unknown total and cache coverage, model ratios, continuous four series across missing points, truthful tooltip, all-unknown and real zero')
 }finally{await browser.close()}
})().catch(e=>{console.error(e);process.exitCode=1})
