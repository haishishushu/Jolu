// 独立无头 Edge：挂载真实通知和 Radix 弹窗，只注入合成操作结果。
const assert = require('node:assert/strict')
const path = require('node:path')
const fs = require('node:fs')
const { chromium } = require(require.resolve('playwright-core', { paths: [path.resolve('output/acceptance/node')] }))
const base = process.env.JOLU_COMPONENT_URL || 'http://localhost:5314'
if (!['localhost', '127.0.0.1', '[::1]'].includes(new URL(base).hostname)) throw new Error('Only localhost preview is allowed')
const out = path.resolve('output/acceptance/toast-cards')
const fixture = `
import React from '/node_modules/.vite/deps/react.js'
import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js'
import * as Dialog from '/node_modules/.vite/deps/@radix-ui_react-dialog.js'
import {ToastProvider,ToastViewport,useToast} from '/src/components/ui/Toast.tsx'
import '/src/index.css?import'
const h=React.createElement
function App(){
  const toast=useToast(),[open,setOpen]=React.useState(false)
  window.fixture={show:(tone,title,detail)=>toast.show({tone,title,detail}),action:()=>toast.show({tone:'danger',title:'可重试错误',detail:'稍后重试即可恢复',action:{label:'重新查询',onClick:()=>{window.actionCount=(window.actionCount||0)+1}}})}
  const retryFailure=()=>toast.show({tone:'danger',title:'连续失败',detail:'服务仍暂时不可用',action:{label:'再次重试',onClick:retryFailure}})
  window.fixture.failedAction=retryFailure
  return h('div',{style:{position:'relative',height:'100vh',width:'100%',overflow:'hidden'},className:'bg-jolu-bg text-jolu-text'},
    h('header',{className:'h-[30px] border-b border-jolu-border px-5 text-xs leading-[30px]'},'Jolu'),
    h('div',{className:'flex h-16 items-center border-b border-jolu-border px-8 font-semibold'},'供应商 · 账号管理'),
    h('main',{className:'flex flex-col items-start gap-5 p-8'},
      h('h1',{className:'text-xl font-semibold'},'操作通知预览'),
      h('input',{'aria-label':'页面输入',className:'rounded-lg border border-jolu-border p-2',defaultValue:'继续当前操作'}),
      h('button',{onClick:()=>setOpen(true),className:'rounded-lg bg-jolu-accent px-4 py-2 text-white'},'打开测试弹窗')),
    h(ToastViewport),
    h(Dialog.Root,{open,onOpenChange:setOpen},h(Dialog.Portal,null,
      h(Dialog.Overlay,{className:'fixed inset-0 z-50 bg-black/35'}),
      h(Dialog.Content,{'aria-describedby':undefined,style:{position:'fixed',left:'50%',top:'50%',transform:'translate(-50%,-50%)',width:420,maxWidth:'calc(100% - 32px)'},className:'z-50 rounded-2xl border border-jolu-border bg-jolu-bg p-6 shadow-dialog'},
        h(Dialog.Title,{className:'text-lg font-semibold'},'连接设置'),
        h('input',{'aria-label':'连接名称',className:'mt-5 w-full rounded-lg border border-jolu-border p-2'}),
        h('button',{onClick:()=>toast.danger('连接验证失败','请确认服务地址后重新查询'),className:'mt-5 rounded-lg bg-jolu-accent px-4 py-2 text-white'},'测试错误'),
        h(Dialog.Close,{className:'ml-4'},'取消')))))
}
ReactDOM.createRoot(document.getElementById('root')).render(h(React.StrictMode,null,h(ToastProvider,null,h(App))))
`
;(async () => {
  fs.mkdirSync(out, { recursive: true })
  const browser = await chromium.launch({ channel: 'msedge', headless: true })
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
    const errors = []
    page.on('pageerror', error => { errors.push(error.message); console.error('fixture:',error.message) })
    await page.clock.install()
    await page.route('**/toast-cards-check.html', route => route.fulfill({ contentType:'text/html', body:`<html lang="zh-CN"><meta charset="UTF-8"><link rel="icon" href="data:,"><div id="root"></div><script type="module">import RefreshRuntime from '/@react-refresh';RefreshRuntime.injectIntoGlobalHook(window);window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>type=>type;window.__vite_plugin_react_preamble_installed__=true;await import('/toast-cards-fixture.mjs');</script></html>` }))
    await page.route('**/toast-cards-fixture.mjs', route => route.fulfill({ contentType:'application/javascript', body:fixture }))
    await page.goto(base + '/toast-cards-check.html')
    await page.getByRole('heading', { name:'操作通知预览' }).waitFor()
    const show = (tone, title, detail) => page.evaluate(args => window.fixture.show(...args), [tone,title,detail])
    const cards = page.locator('[data-toast-tone]')
    const card = title => cards.filter({ has:page.getByText(title, { exact:true }) })
    async function finishGone(title) {
      // React 的退出 effect 在时钟推进后的渲染提交，单独推进退出动画。
      for(let attempt=0;attempt<4 && await card(title).count();attempt++) await page.clock.runFor(180)
      assert.equal(await card(title).count(),0)
    }
    async function clear(){
      while (await cards.count()) {
        await cards.first().locator('button[aria-label^="关闭提示"]').click()
        await page.clock.runFor(180)
      }
      await page.mouse.move(5,5)
      await page.getByRole('textbox',{name:'页面输入'}).focus()
    }

    await page.getByRole('textbox',{name:'页面输入'}).focus()
    await show('success','连接已保存','WorkBuddy 已添加到账号管理')
    assert.equal(await page.getByRole('textbox',{name:'页面输入'}).evaluate(el=>el===document.activeElement),true)
    await show('warn','查询暂时受限','请稍后再试，已有数据仍然保留')
    const detail = '网关暂时无法返回余额，请检查服务地址和网络连接。'.repeat(8)+'完整原因结束'
    await show('danger','余额查询失败',detail)
    await show('danger','余额查询失败',detail)
    assert.equal(await cards.count(),3)
    await page.getByLabel('重复 2 次').waitFor()
    const position = await cards.first().boundingBox()
    assert.ok(position.x>850 && position.y>=100 && position.y<130)
    assert.ok(position.x+position.width<=1264)
    await card('余额查询失败').getByRole('button',{name:'展开原因'}).click()
    assert.equal(await card('余额查询失败').locator('p').last().evaluate(el=>el.scrollHeight<=el.clientHeight+1),true)
    await card('余额查询失败').getByRole('button',{name:'收起原因'}).click()
    await page.mouse.move(5,5)
    await page.getByRole('textbox',{name:'页面输入'}).focus()
    await page.clock.runFor(200)
    await page.getByRole('region',{name:'操作通知'}).evaluate(el=>{el.scrollTop=0})
    await page.evaluate(()=>window.scrollTo(0,0))
    assert.equal(await page.getByRole('region',{name:'操作通知'}).evaluate(el=>el.scrollHeight>el.clientHeight+1),false,'三张普通卡片应完整显示')
    assert.deepEqual(await page.getByRole('region',{name:'操作通知'}).evaluate(el=>{
      const style=getComputedStyle(el)
      return {background:style.backgroundColor,overflow:style.overflowY}
    }),{background:'rgba(0, 0, 0, 0)',overflow:'visible'},'外层应透明且不裁切卡片阴影')
    await page.screenshot({ path:path.join(out,'light.png') })
    await page.evaluate(()=>document.documentElement.classList.add('dark'))
    await page.screenshot({ path:path.join(out,'dark.png') })
    await page.clock.fastForward(6500)
    await page.clock.runFor(180)
    assert.equal(await cards.count(),1)
    await card('余额查询失败').waitFor()
    await clear()

    await show('info','暂停测试','鼠标与键盘分别暂停倒计时')
    await page.clock.runFor(500)
    await card('暂停测试').hover()
    await page.clock.fastForward(4000)
    await card('暂停测试').waitFor()
    await card('暂停测试').locator('button').last().focus()
    await page.mouse.move(5,5)
    await page.clock.fastForward(4000)
    await card('暂停测试').waitFor()
    await page.getByRole('textbox',{name:'页面输入'}).focus()
    await page.clock.runFor(2300)
    await card('暂停测试').waitFor()
    await page.clock.runFor(600)
    await finishGone('暂停测试')
    assert.equal(await cards.count(),0)

    for(let i=1;i<=3;i++) await show('danger',`保留错误 ${i}`,'等待鼠鼠处理')
    await show('success','排队成功','显示后才开始三秒倒计时')
    await page.clock.fastForward(20000)
    assert.equal(await cards.count(),3)
    assert.equal(await card('排队成功').count(),0)
    await card('保留错误 2').locator('button').last().click()
    assert.equal(await cards.count(),3)
    await page.clock.runFor(180)
    await card('排队成功').waitFor()
    await page.mouse.move(5,5)
    await page.getByRole('textbox',{name:'页面输入'}).focus()
    await page.clock.runFor(2800)
    await card('排队成功').waitFor()
    await page.clock.runFor(800)
    await finishGone('排队成功')
    assert.equal(await cards.count(),2)
    await clear()

    await page.evaluate(()=>window.fixture.action())
    await page.getByRole('button',{name:'重新查询'}).click()
    await finishGone('可重试错误')
    assert.equal(await page.evaluate(()=>window.actionCount),1)
    assert.equal(await cards.count(),0)
    await page.evaluate(()=>window.fixture.failedAction())
    await page.getByRole('button',{name:'再次重试'}).click()
    await page.clock.runFor(180)
    await page.evaluate(()=>0)
    await page.clock.runFor(180)
    assert.equal(await card('连续失败').count(),1,'重试仍失败时不能关闭合并后的错误')
    assert.equal(await card('连续失败').getAttribute('data-state'),'open')
    await page.getByLabel('重复 2 次').waitFor()
    await clear()

    await page.getByRole('button',{name:'打开测试弹窗'}).click()
    await page.getByRole('dialog',{name:'连接设置'}).waitFor()
    await page.getByRole('button',{name:'测试错误'}).click()
    await card('连接验证失败').waitFor()
    await card('连接验证失败').locator('button[aria-label^="关闭提示"]').focus()
    assert.equal(await card('连接验证失败').locator('button').last().evaluate(el=>el===document.activeElement),true,'表单打开时通知也应能键盘操作')
    const modalCard = await card('连接验证失败').boundingBox()
    assert.ok(modalCard.x>850 && modalCard.y>=100 && modalCard.y<130,'有表单时通知仍固定在窗口右上角')
    await page.screenshot({path:path.join(out,'modal.png')})
    await page.keyboard.press('Escape')
    await finishGone('连接验证失败')
    assert.equal(await page.getByRole('dialog',{name:'连接设置'}).count(),1,'Esc 关闭通知不能顺带关闭表单')
    assert.equal(await card('连接验证失败').count(),0)
    await page.getByRole('button',{name:'测试错误'}).click()
    await card('连接验证失败').locator('button[aria-label^="关闭提示"]').click()
    await finishGone('连接验证失败')
    assert.equal(await page.getByRole('dialog',{name:'连接设置'}).count(),1,'鼠标关闭提示不能顺带关闭表单')
    await page.getByRole('button',{name:'取消',exact:true}).click()

    await page.setViewportSize({width:420,height:720})
    await show('danger','服务查询失败','https://synthetic.example.test/'+ 'a'.repeat(250))
    await page.clock.runFor(200)
    const rect = await cards.first().boundingBox()
    assert.ok(rect.x>=12 && rect.x+rect.width<=408)
    assert.equal(await cards.first().evaluate(el=>el.scrollWidth>el.clientWidth),false)
    await page.emulateMedia({reducedMotion:'reduce'})
    assert.equal(await cards.first().evaluate(el=>getComputedStyle(el).transform),'none')
    await page.screenshot({path:path.join(out,'narrow.png')})
    await clear()
    await show('danger','长原因滚动检查','较长的合成错误原因，用来验证完整信息仍然可以查看。'.repeat(160))
    await card('长原因滚动检查').getByRole('button',{name:'展开原因'}).click()
    await page.waitForFunction(()=>getComputedStyle(document.querySelector('[aria-label="操作通知"]')).overflowY==='auto')
    assert.equal(await page.getByRole('region',{name:'操作通知'}).evaluate(el=>el.scrollHeight>el.clientHeight),true)
    assert.deepEqual(errors,[])
    console.log('PASS: 右上角卡片、三条排队、分级停留、错误保留、合并次数、关闭、鼠标与键盘暂停、原因展开、动作、模态交互及浅深色和窄窗口')
  } finally { await browser.close() }
})().catch(error=>{ console.error(error);process.exitCode=1 })
