// Real AccountsPanel/useQuota with synthetic API responses only; no desktop or credentials.
const path = require('node:path')
const assert = require('node:assert/strict')
const { chromium } = require(require.resolve('playwright-core', { paths: [path.resolve(__dirname, '../../output/acceptance/node')] }))
const origin = process.env.JOLU_PREVIEW_URL ?? 'http://localhost:5193'
assert.ok(['localhost', '127.0.0.1', '[::1]'].includes(new URL(origin).hostname))

;(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true })
  try {
    const page = await browser.newPage({ viewport: { width: 1400, height: 1000 } })
    const errors = []
    page.on('pageerror', error => errors.push(error.message))
    await page.addInitScript(() => {
      const state = count => ({ state: 'ok', plan: 'Pro', windows: [{ key: '5h', window_name: '5 小时额度', used_percent: 20, resets_at: null, amount_text: null }, { key: '7d', window_name: '周额度', used_percent: 40, resets_at: null, amount_text: null }], account: { subscription_status: 'unknown', subscription_source: 'unavailable', expires_at: null, observed_at: null, reset_credits: { available_count: count, applicable_available_count: 0 } } })
      window.fixture = { calls: [], resolve: null, reject: null, unexpected: [] }
      window.__TAURI_INTERNALS__ = { invoke: async (command, args) => {
        if (command === 'current_connections') return []
        if (command === 'connection_quota') {
          window.fixture.calls.push({ id: args.id, force: args.force })
          if (args.force) return new Promise((resolve, reject) => {
            window.fixture.resolve = () => resolve(state(2))
            window.fixture.reject = () => reject(new Error('synthetic offline'))
          })
          return state(1)
        }
        window.fixture.unexpected.push(command)
        throw new Error(`Unexpected native command: ${command}`)
      } }
    })
    await page.route('**/account-refresh-check', route => route.fulfill({ contentType: 'text/html', body: `<!doctype html><html><head><meta charset="utf-8"></head><body><div id="root"></div><script type="module">
      import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window);
      window.$RefreshReg$=()=>{}; window.$RefreshSig$=()=>type=>type; window.__vite_plugin_react_preamble_installed__=true;
      await import('/account-refresh-module');</script></body></html>` }))
    await page.route('**/src/hooks/useSettings.ts*', route => route.fulfill({ contentType: 'application/javascript', body: 'export function useSettings(){return {settings:{refresh_minutes:5,island_clones:[]}}}' }))
    await page.route('**/account-refresh-module', route => route.fulfill({ contentType: 'application/javascript', body: `
      import React from '/node_modules/.vite/deps/react.js';
      import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
      import {AccountsPanel} from '/src/components/providers/AccountsPanel.tsx';
      import {TipProvider} from '/src/components/ui/Tip.tsx';
      import '/src/index.css';
      const connections=['first','second'].map(id=>({id,platformId:'codex',kind:'auth',name:id,baseName:id,accountEmail:id+'@example.test',label:'',masked:'',status:'active',lastSyncText:'刚刚同步',baseUrl:null}));
      const noop=async()=>{};
      ReactDOM.createRoot(document.getElementById('root')).render(React.createElement(TipProvider,null,React.createElement('div',{style:{height:'100vh'},className:'flex min-h-0 flex-col gap-3 p-8'},React.createElement(AccountsPanel,{conn:{connections,loading:false,error:null,remove:noop,reload:noop},onConnect:noop,onBrowseProviders:noop}))));
    ` }))
    await page.goto(`${origin}/account-refresh-check`)
    await page.waitForLoadState('networkidle')
    const cards = page.locator('article')
    await cards.first().waitFor({ timeout: 5000 }).catch(error => { throw new Error(`Fixture failed to render: ${errors.join('; ')}`, { cause: error }) })
    assert.equal(await cards.count(), 2)
    await cards.nth(1).getByText('可用 1 次', { exact: true }).waitFor({ timeout: 5000 }).catch(async error => { throw new Error(JSON.stringify({ calls: await page.evaluate(() => window.fixture.calls), content: await cards.nth(1).innerText(), errors }), { cause: error }) })
    const before = await cards.nth(1).innerHTML()
    const query = cards.first().getByRole('button', { name: '查询重置卡', exact: true })
    await query.click()
    assert.equal(await query.isDisabled(), true)
    assert.equal(await cards.nth(1).getByRole('button', { name: '查询重置卡', exact: true }).isEnabled(), true)
    assert.equal(await cards.nth(1).innerHTML(), before)
    const calls = await page.evaluate(() => window.fixture.calls)
    assert.equal(calls.length, 3)
    assert.deepEqual(calls.slice(0, 2).sort((a, b) => a.id.localeCompare(b.id)), [{ id: 'first', force: false }, { id: 'second', force: false }])
    assert.deepEqual(calls[2], { id: 'first', force: true })
    await page.evaluate(() => window.fixture.resolve())
    await cards.first().getByText('可用 2 次', { exact: true }).waitFor()
    assert.equal(await cards.nth(1).innerHTML(), before)
    await query.click()
    await page.waitForFunction(() => window.fixture.calls.length === 4)
    await page.evaluate(() => window.fixture.reject())
    await cards.first().getByText(/刷新失败/).first().waitFor({ timeout: 5000 }).catch(async error => { throw new Error(JSON.stringify({ calls: await page.evaluate(() => window.fixture.calls), content: await cards.first().innerText(), errors }), { cause: error }) })
    assert.equal(await cards.first().getByText('可用 2 次', { exact: true }).count(), 1)
    assert.equal(await query.isEnabled(), true)
    assert.equal(await cards.nth(1).innerHTML(), before)
    assert.deepEqual(errors, [])
    assert.deepEqual(await page.evaluate(() => window.fixture.unexpected), [])
    console.log('PASS: refresh is local to one account; success updates reset credits; failure retains marked stale data; no pageerror')
  } finally {
    await browser.close()
  }
})().catch(error => { console.error(error); process.exitCode = 1 })
