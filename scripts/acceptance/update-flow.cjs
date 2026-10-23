// 使用合成 API 验证更新交互；不连接真实账号、不安装程序。
const fs = require('node:fs'), path = require('node:path'), assert = require('node:assert/strict');
const root = process.cwd(), fixture = path.join(root, 'frontend/update-audit-check.tsx'), html = path.join(root, 'frontend/update-audit-check.html');
if (fs.existsSync(fixture) || fs.existsSync(html)) throw Error('Fixture exists');
fs.writeFileSync(fixture, `import {createRoot} from 'react-dom/client';import {UpdateProvider,useUpdate} from './src/components/panel/UpdateContext';import {UpdateDialog} from './src/components/panel/UpdateDialog';import './src/index.css';function App(){const u=useUpdate();window.controls=u;return <main><button onClick={()=>u.checkNow()}>检查</button><button onClick={u.startDownload}>更新</button><span data-phase>{u.phase}</span><UpdateDialog onViewTasks={()=>window.tasksOpened=true}/></main>}createRoot(document.getElementById('root')!).render(<UpdateProvider><App/></UpdateProvider>);`);
fs.writeFileSync(html, '<div id="root"></div><script type="module" src="/update-audit-check.tsx"></script>');
process.on('exit', () => { fs.unlinkSync(fixture); fs.unlinkSync(html); });
const {chromium} = require(require.resolve('playwright-core', {paths:[path.join(root,'output/acceptance/node')]}));
(async () => {
 const browser = await chromium.launch({channel:'msedge',headless:true});
 try {
  const page = await browser.newPage({viewport:{width:1000,height:800}}), errors=[];
  page.on('pageerror',e=>errors.push(e.message));
  await page.route('**/src/lib/api.ts*',r=>r.fulfill({contentType:'application/javascript',body:`export const isTauri=true;window.updateMock={downloads:0,installs:0,running:2,listeners:0,checks:0};const m=window.updateMock;export const listenEvent=async(name,fn)=>{if(name==='settings-changed'){m.settingsChanged=fn;return()=>{}};m.listeners++;if(name==='automation-changed')m.changed=fn;return()=>{m.listeners--}};const dto={available:true,current_version:'0.1.0',available_version:'0.1.1'};export const api={getSettings:async()=>({auto_check_updates:false}),checkAppUpdate:async()=>{m.checks++;return dto},downloadAppUpdate:()=>{m.downloads++;return new Promise((resolve,reject)=>{m.resolve=()=>resolve(dto);m.reject=reject})},cancelAppUpdateDownload:async()=>m.reject('更新下载已取消'),installDownloadedUpdate:async()=>{m.installs++;if(m.failInstall)throw Error('安装器启动失败')}};export const invoke=async()=>({running:m.running});` }));
  await page.goto((process.env.JOLU_PREVIEW_URL||'http://127.0.0.1:5197')+'/update-audit-check.html');
  await page.waitForTimeout(1200);assert.equal(await page.evaluate(()=>window.updateMock.checks),0,'关闭自动检查时不请求版本');
  await page.getByRole('button',{name:'检查',exact:true}).click();await page.getByRole('button',{name:'更新',exact:true}).click();await page.getByRole('button',{name:'取消下载'}).click();await page.getByRole('dialog').waitFor({state:'hidden'});
  await page.getByRole('button',{name:'更新',exact:true}).click();await page.waitForFunction(()=>window.updateMock.downloads===2);await page.evaluate(()=>window.updateMock.reject('离线'));await page.getByRole('button',{name:'重试下载'}).click();await page.waitForFunction(()=>window.updateMock.downloads===3);await page.evaluate(()=>window.updateMock.resolve());
  await page.getByRole('button',{name:'查看任务',exact:true}).click();await page.getByRole('dialog').waitFor({state:'hidden'});assert.equal(await page.evaluate(()=>window.tasksOpened),true);
  await page.getByRole('button',{name:'更新',exact:true}).click();await page.getByRole('button',{name:'稍后安装'}).waitFor();assert.equal(await page.evaluate(()=>window.updateMock.downloads),3,'已校验包不重复下载');
  await page.evaluate(()=>{window.updateMock.running=0;window.updateMock.changed()});await page.getByRole('button',{name:'重启并安装'}).waitFor();await page.evaluate(()=>window.updateMock.failInstall=true);await page.getByRole('button',{name:'重启并安装'}).click();await page.getByRole('alert').waitFor();await page.getByRole('button',{name:'稍后安装'}).click();await page.getByRole('dialog').waitFor({state:'hidden'});
  await page.getByRole('button',{name:'更新',exact:true}).click();await page.evaluate(()=>window.updateMock.failInstall=false);await page.getByRole('button',{name:'重启并安装'}).click();await page.getByText('安装完成',{exact:true}).waitFor();await page.getByRole('button',{name:'关闭',exact:true}).click();assert.equal(await page.evaluate(()=>window.updateMock.installs),2);assert.equal(await page.evaluate(()=>window.updateMock.listeners),0);assert.equal(await page.evaluate(()=>window.updateMock.checks),1);await page.evaluate(()=>window.updateMock.settingsChanged({auto_check_updates:true}));await page.waitForFunction(()=>window.updateMock.checks===2);assert.equal(await page.evaluate(()=>window.updateMock.downloads),3,'自动检查不自动下载');assert.deepEqual(errors,[]);
  console.log('PASS 更新取消、离线重试、运行任务导航、任务完成、稍后安装、缓存复用、安装失败重试、监听清理');
 } finally { await browser.close(); }
})().catch(error=>{console.error(error);process.exitCode=1});
