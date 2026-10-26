// 合成接口 + 真实组件与 hook；不连接桌面、不读取凭据、不请求供应商。
const path = require('node:path')
const fs = require('node:fs')
const assert = require('node:assert/strict')
const { chromium } = require(require.resolve('playwright-core', { paths: [path.resolve(__dirname, '../../output/acceptance/node')] }))
const origin = process.env.JOLU_PREVIEW_URL ?? 'http://localhost:5193'
assert.ok(['localhost', '127.0.0.1', '[::1]'].includes(new URL(origin).hostname))

;(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true })
  try {
    const page = await browser.newPage({ viewport: { width: 1400, height: 1100 } })
    const errors = []
    page.on('pageerror', e => errors.push(e.message))
    await page.addInitScript(() => {
      window.fixture = { calls: [], resolve: null, customRefreshes: 0 }
      const account = { subscription_source: 'live', subscription_status: 'active', expires_at: '2026-11-30T00:00:00Z', reset_credits: { available_count: 2, applicable_available_count: 0 } }
      window.__TAURI_INTERNALS__ = { invoke: async (command, args) => {
        if (command === 'current_connections') return []
        window.fixture.calls.push({ command, ...args })
        if (command === 'connection_quota') {
          if(args.id==='grok' && window.fixture.expired) return {state:'unauthorized',reason:'synthetic authorization expired'}
          if (args.id === 'gateway' && args.force) return new Promise(resolve => { window.fixture.resolve = () => resolve({ state: 'failed', reason: 'synthetic quota offline' }) })
          return { state: 'ok', plan: 'Test plan', account: {...account, reset_credits: ['gateway','openai'].includes(args.id) ? account.reset_credits : null}, windows: [{ key: args.id === 'gemini' ? 'model' : '5h', window_name: '模型额度', used_percent: args.id === 'gemini' ? null : 25, remaining_text: args.id === 'gemini' ? '剩余 0 次' : null, resets_at: null }] }
        }
        if (command === 'connection_balance') {
          if (args.id === 'custom') {
            if (args.force && ++window.fixture.customRefreshes === 1) return {state:'failed',reason:'synthetic balance offline'}
            if (window.fixture.customRefreshes > 1) return {state:'unsupported',reason:'synthetic unknown billing API'}
            return {state:'ok',balance:0,used:null,currency:'CNY'}
          }
          return { state: 'ok', balance: 12.5, used: 3, currency: 'USD' }
        }
        throw new Error(`Unexpected command ${command}`)
      } }
    })
    await page.route('**/src/hooks/useSettings.ts*', r => r.fulfill({ contentType: 'application/javascript', body: 'export function useSettings(){return {settings:{refresh_minutes:5,island_clones:[]}}}' }))
    await page.route('**/account-sources-check', r => r.fulfill({ contentType: 'text/html', body: `<!doctype html><html><head><meta charset="utf-8"></head><body><div id="root"></div><script type="module">
      import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window); window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>type=>type;window.__vite_plugin_react_preamble_installed__=true;
      await import('/account-sources-module');</script></body></html>` }))
    await page.route('**/account-sources-module', r => r.fulfill({ contentType: 'application/javascript', body: `
      import React from '/node_modules/.vite/deps/react.js';import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
      import {AccountsPanel} from '/src/components/providers/AccountsPanel.tsx';import {TipProvider} from '/src/components/ui/Tip.tsx';import '/src/index.css';
      const connections=[['gateway','claude','api'],['gemini','gemini','auth'],['openai','opencode','auth'],['custom','custom','api'],['codex','codex','auth'],['grok','grok','auth'],['workbuddy','workbuddy','auth'],['trae','trae','auth'],['qoder','qoder','auth'],['zcode','zcode','auth']].map(([id,platformId,kind])=>({id,platformId,kind,name:id,baseName:id,accountEmail:null,label:'',masked:'sk-…test',status:'connected',lastSyncText:'尚未同步',baseUrl:kind==='api'?'https://example.test/v1':null}));
      const noop=async()=>{};ReactDOM.createRoot(document.getElementById('root')).render(React.createElement(TipProvider,null,React.createElement('div',{style:{height:'100vh'},className:'flex min-h-0 flex-col gap-3 p-8 bg-jolu-bg text-jolu-text'},React.createElement(AccountsPanel,{conn:{connections,loading:false,error:null,remove:noop,reload:noop},onConnect:noop,onBrowseProviders:noop}))));` }))
    await page.goto(`${origin}/account-sources-check`)
    const cards = page.locator('article')
    await cards.first().getByText('$12.50', { exact: true }).waitFor()
    await cards.first().getByRole('button',{name:'查看账号详情'}).click()
    await page.getByRole('dialog',{name:'账号详情'}).getByText('剩余 75%', { exact: true }).waitFor()
    await page.getByRole('dialog',{name:'账号详情'}).getByText('重置卡',{exact:true}).waitFor()
    await page.getByRole('dialog',{name:'账号详情'}).getByRole('button',{name:'关闭',exact:true}).last().click()
    await cards.first().getByText(/^2026-11-30/).waitFor()
    assert.equal(await cards.first().getByText('$3.00 USD', { exact: true }).count(), 1)
    assert.equal(await cards.nth(1).getByText('剩余 0 次', { exact: true }).count(), 1)
    assert.equal(await cards.getByText('订阅状态', {exact:true}).count(),0)
    assert.equal(await cards.getByRole('button', { name: '查询重置卡', exact: true }).count(), 1)
    assert.equal(await cards.nth(6).getByText('重置卡', {exact:true}).count(), 0)
    await cards.nth(2).getByRole('button', {name:'查询重置卡',exact:true}).click()
    await page.waitForFunction(()=>window.fixture.calls.some(c=>c.command==='connection_quota'&&c.id==='openai'&&c.force))
    assert.deepEqual(await page.evaluate(() => window.fixture.calls.filter(c => c.force).map(c => [c.command,c.id])), [['connection_quota','openai']])
    assert.equal(await cards.nth(2).getByText('可用 2 次', { exact: true }).count(), 1)
    await cards.nth(3).getByText('0.00', {exact:true}).waitFor()
    assert.equal(await cards.nth(3).getByText('CNY',{exact:true}).count(),1)
    assert.deepEqual((await page.evaluate(() => window.fixture.calls.filter(c => c.id === 'custom').map(c=>c.command))).sort(), ['connection_balance','connection_quota'])
    const customRefresh = cards.nth(3).getByRole('button', {name:'刷新当前账号',exact:true})
    await customRefresh.click()
    await cards.nth(3).getByText('刷新失败 · 上次金额',{exact:true}).waitFor()
    assert.equal(await cards.nth(3).getByText('0.00',{exact:true}).count(),1)
    const refresh = cards.first().getByRole('button', { name: '刷新当前账号', exact: true })
    await refresh.click()
    await page.waitForFunction(() => window.fixture.resolve !== null)
    assert.equal(await refresh.isDisabled(), true)
    await page.evaluate(() => window.fixture.resolve())
    await cards.first().getByText(/刷新失败/).first().waitFor()
    await cards.first().getByRole('button',{name:'查看账号详情'}).click()
    await page.getByRole('dialog',{name:'账号详情'}).getByText('剩余 75%', { exact: true }).waitFor()
    await page.getByRole('dialog',{name:'账号详情'}).getByRole('button',{name:'关闭',exact:true}).last().click()
    assert.equal(await cards.first().getByText('$12.50', { exact: true }).count(), 1)
    assert.equal(await page.evaluate(() => window.fixture.calls.filter(c => c.id === 'gateway' && c.force).length), 2)
    assert.ok(await cards.evaluateAll(nodes => nodes.every(n => n.scrollWidth <= n.clientWidth)))
    const output = path.resolve(__dirname, '../../output/playwright')
    fs.mkdirSync(output, { recursive: true })
    await page.screenshot({ path: path.join(output, 'account-sources-light.png'), fullPage: true })
    await page.evaluate(() => document.documentElement.classList.add('dark'))
    await page.screenshot({ path: path.join(output, 'account-sources-dark.png'), fullPage: true })
    await customRefresh.click()
    await cards.nth(3).getByRole('button',{name:'查看账号详情'}).click()
    await page.getByRole('dialog',{name:'账号详情'}).getByText('余额 · synthetic unknown billing API',{exact:true}).waitFor()
    await page.getByRole('dialog',{name:'账号详情'}).getByRole('button',{name:'关闭',exact:true}).last().click()
    assert.equal(await cards.nth(3).getByText('0.00',{exact:true}).count(),0)
    assert.equal(await page.evaluate(()=>window.fixture.calls.filter(c=>c.id==='custom'&&c.force&&c.command==='connection_balance').length),2)
    await page.evaluate(()=>window.fixture.expired=true)
    await cards.nth(5).getByRole('button',{name:'刷新当前账号',exact:true}).click()
    await cards.nth(5).getByText('额度 · 账号认证已失效，重新登录后同步',{exact:true}).waitFor()
    assert.equal(await cards.getByText('订阅状态',{exact:true}).count(),0)
    assert.deepEqual(errors, [])
    console.log('PASS: API quota+balance+expiry, exact zero, OpenCode reset credits, custom balance+zero+stale+unsupported, scoped refresh, no overflow/pageerror')
  } finally { await browser.close() }
})().catch(error => { console.error(error); process.exitCode = 1 })
