// 合成账号与响应，真实前端组件；不连接桌面或读取真实授权。
const path=require('node:path'),assert=require('node:assert/strict'),fs=require('node:fs');
const {chromium}=require(require.resolve('playwright-core',{paths:[path.resolve(__dirname,'../../output/acceptance/node')]}));
const origin=process.env.JOLU_COMPONENT_URL??'http://localhost:5314';assert.ok(['localhost','127.0.0.1'].includes(new URL(origin).hostname));
(async()=>{const browser=await chromium.launch({channel:'msedge',headless:true});try{
const page=await browser.newPage({viewport:{width:1360,height:900}});const errors=[];page.on('pageerror',e=>errors.push(e.message));page.setDefaultTimeout(10000);
await page.addInitScript(()=>{
 window.calls=[];window.connections=[['codex-a','Codex A','codex',true],['claude-a','Claude A','claude',false],['claude-b','Claude B','claude',false],['qoder-a','Qoder A','qoder',false]].map(([id,name,platformId,serve_enabled])=>({id,baseName:name,name,platformId,kind:'auth',status:'connected',slug:id,serve_enabled,masked:'已加密保存'}));
 window.capabilities=[{connection_id:'codex-a',can_serve:true,state:'ready',reason:'支持 API 调用 · Chat / Responses'},{connection_id:'claude-a',can_serve:true,state:'ready',reason:'支持 API 调用 · Messages 非流式文本'},{connection_id:'claude-b',can_serve:false,state:'needs_auth',reason:'所选账号缺少完整授权，请在账号管理中重新登录并保存；不会使用其他账号代替'},{connection_id:'qoder-a',can_serve:false,state:'unsupported',reason:'暂未支持订阅转发；已保存登录不代表可以调用模型'}];
 window.invoke=async(cmd,args)=>{window.calls.push([cmd,args]);switch(cmd){case 'gateway_status':return {mode:'relay',enabled:true,port:12731,wiring:[],auto_escape:true,circuit_threshold:5};case 'gateway_groups':return {sources:[],agents:[]};case 'gateway_catalog':return [{id:'codex-a/model-a',source:'Codex A',status:'connected',kind:'auth',model:'model-a',slug:'codex-a'}];case 'gateway_route_log':return [];case 'gateway_source_capabilities':return window.capabilities;case 'gateway_access_key':return '••••••••1234';case 'gateway_set_serve':window.connections=window.connections.map(c=>c.id===args.connectionId?{...c,serve_enabled:args.enabled}:c);return null;default:throw new Error('unexpected command '+cmd)}};
 Object.defineProperty(navigator,'clipboard',{value:{writeText:async text=>window.calls.push(['clipboard',text])}});
});
await page.route(/\/src\/lib\/api\.ts/,r=>r.fulfill({contentType:'application/javascript',body:'export const isTauri=true; export const api={};export const invoke=(...args)=>window.invoke(...args);'}));
await page.route(/\/src\/hooks\/useConnections\.ts/,r=>r.fulfill({contentType:'application/javascript',body:'export function useConnections(){return {connections:window.connections,reload:async()=>{}}}'}));
await page.route('**/subscription-check',r=>r.fulfill({contentType:'text/html',body:'<!doctype html><html><head><meta charset="utf-8"></head><body><div id="root"></div><script type="module">import R from "/@react-refresh";R.injectIntoGlobalHook(window);window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>t=>t;window.__vite_plugin_react_preamble_installed__=true;await import("/subscription-module");</script></body></html>'}));
await page.route('**/subscription-module',r=>r.fulfill({contentType:'application/javascript',body:`import React from '/node_modules/.vite/deps/react.js';import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';import {GatewayView} from '/src/views/GatewayView.tsx';import '/src/index.css';ReactDOM.createRoot(document.getElementById('root')).render(React.createElement('div',{className:'p-5'},React.createElement(GatewayView)));`}));
await page.goto(origin+'/subscription-check');
await page.getByText('支持 API 调用 · Chat / Responses',{exact:true}).waitFor();
assert.equal(await page.getByRole('switch',{name:'Claude A 对外服务',exact:true}).isEnabled(),true);
assert.equal(await page.getByRole('switch',{name:'Claude B 对外服务',exact:true}).isEnabled(),true);
assert.equal(await page.getByRole('switch',{name:'Qoder A 对外服务',exact:true}).isEnabled(),true);
await page.getByRole('switch',{name:'Qoder A 对外服务',exact:true}).click();
assert.equal(await page.getByRole('switch',{name:'Qoder A 对外服务',exact:true}).getAttribute('aria-checked'),'true');
await page.getByText('已选择 · 转发适配尚未就绪',{exact:true}).waitFor();
await page.getByRole('switch',{name:'Claude A 对外服务',exact:true}).click();
assert.ok((await page.evaluate(()=>window.calls)).some(([cmd,args])=>cmd==='gateway_set_serve'&&args.connectionId==='claude-a'&&args.enabled));
const out=path.resolve(__dirname,'../../output/acceptance/subscription-api');fs.mkdirSync(out,{recursive:true});await page.screenshot({path:path.join(out,'sources-light.png'),fullPage:true});
await page.getByRole('button',{name:'管理模型池',exact:true}).click();await page.getByRole('button',{name:'复制模型名',exact:true}).click();
assert.ok((await page.evaluate(()=>window.calls)).some(([cmd,value])=>cmd==='clipboard'&&value==='codex-a/model-a'));
await page.screenshot({path:path.join(out,'copy-model.png'),fullPage:true});
assert.deepEqual(errors,[]);console.log('PASS: backend capability states, all subscription switches selectable, pending adapter state explicit, selected ID enabled, model copied');
}finally{await browser.close()}})().catch(e=>{console.error(e);process.exitCode=1});
