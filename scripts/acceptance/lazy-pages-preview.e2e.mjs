// Production browser preview only: synthetic data, no desktop/CDP or real account access.
import { createRequire } from 'node:module'
import { mkdir, writeFile } from 'node:fs/promises'
import assert from 'node:assert/strict'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
const require = createRequire(import.meta.url)
const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const { chromium } = require(require.resolve('playwright-core', { paths: [join(repo, 'output/acceptance/node')] }))
const out = join(repo, 'output/acceptance/lazy-pages')
const previewUrl = process.env.JOLU_PREVIEW_URL ?? 'http://localhost:5307'
assert.ok(['localhost', '127.0.0.1', '[::1]'].includes(new URL(previewUrl).hostname), '只允许本机浏览器预览地址')
await mkdir(out, { recursive: true })
const browser = await chromium.launch({ channel: 'msedge', headless: true })
let release
try {
  const page = await browser.newPage({ viewport: { width: 1600, height: 1100 } })
  const errors = []
  page.on('pageerror', error => errors.push(error.message))
  const requests = []
  page.on('request', req => requests.push(req.url()))
  await page.goto(previewUrl)
  await page.waitForLoadState('networkidle')
  await page.getByText('前端预览 · 数据均为设计示例', { exact: true }).waitFor()
  assert.equal(await page.evaluate(() => '__TAURI_INTERNALS__' in window), false, '拒绝连接真实桌面接口')
  assert.ok(requests.some(url => /IslandView-.*\.js/.test(url)), '请使用生产构建的 vite preview，不要使用 dev server')
  assert.equal(requests.some(url => /SessionsView-|UsageView-|ProvidersView-|SkillsView-|SettingsPage-|AutomationView-/.test(url)), false)
  const nav = page.getByRole('navigation', { name: '主导航' })
  const ready = {
    island: () => page.getByRole('heading',{name:'连接与分身',exact:true}),
    providers: () => page.getByRole('heading',{name:'连接你的 AI 工具',exact:true}),
    usage: () => page.getByRole('tab',{name:'概览',exact:true}),
    sessions: () => page.getByRole('button',{name:'更多会话操作',exact:true}),
    automation: () => page.getByRole('heading',{name:'把重复工作交给 AI',exact:true}),
    skills: () => page.getByRole('heading',{name:'我的 Skills',exact:true}),
    settings: () => page.getByRole('heading',{name:'常规',exact:true}),
  }
  for (const [key, name] of [['island','灵动岛'],['providers','供应商'],['usage','用量统计'],['sessions','会话线程'],['automation','自动化'],['skills','Skills 管理'],['settings','设置']]) {
    await nav.getByRole('button', { name, exact: true }).click()
    await page.waitForLoadState('networkidle')
    await page.getByRole('status', { name: `正在加载${name}`, exact: true }).waitFor({ state: 'hidden' })
    await ready[key]().waitFor()
    await page.screenshot({ path: `${out}/${key}.png`, fullPage: true })
    assert.equal(await page.getByRole('alert').filter({ hasText: '暂时无法显示' }).count(), 0)
    console.log('page passed', name)
  }
  assert.deepEqual(errors, [])
  // Hold a new page's actual production chunk to prove local skeleton and live navigation.
  const delayed = await browser.newPage({ viewport: { width:1600, height:1100 } })
  const barrier = new Promise(resolve => { release = resolve })
  await delayed.route('**/SkillsView-*.js', async route => { await barrier; await route.continue() })
  await delayed.goto(previewUrl)
  await delayed.waitForLoadState('networkidle')
  const dn = delayed.getByRole('navigation', { name:'主导航' })
  await dn.getByRole('button', {name:'Skills 管理', exact:true}).click()
  await delayed.getByRole('status', {name:'正在加载Skills 管理',exact:true}).waitFor()
  await delayed.screenshot({path:`${out}/skills-skeleton.png`,fullPage:true})
  await dn.getByRole('button', {name:'用量统计',exact:true}).click()
  await delayed.getByRole('tab', {name:'概览',exact:true}).waitFor()
  release()
  await delayed.waitForLoadState('networkidle')
  assert.equal(await dn.getByRole('button',{name:'用量统计',exact:true}).getAttribute('aria-current'),'page')
  console.log('delayed chunk: local skeleton and switch-away passed')
  // Fail the first chunk request, then allow subsequent requests and retry.
  const fault = await browser.newPage({ viewport:{width:1600,height:1100} })
  let failures = 0
  await fault.route('**/SkillsView-*.js', async route => {
    failures++
    if(failures===1) await route.abort('failed'); else await route.continue()
  })
  await fault.goto(previewUrl)
  await fault.waitForLoadState('networkidle')
  await fault.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'Skills 管理',exact:true}).click()
  await fault.getByRole('alert').filter({hasText:'暂时无法显示'}).waitFor()
  await fault.screenshot({path:`${out}/skills-error.png`,fullPage:true})
  // Resource failures truthfully offer explicit app reload, not a cached local retry.
  const fn = fault.getByRole('navigation',{name:'主导航'})
  await fn.getByRole('button',{name:'用量统计',exact:true}).click()
  await fault.getByRole('tab',{name:'概览',exact:true}).waitFor()
  assert.equal(await fn.getByRole('button',{name:'用量统计',exact:true}).getAttribute('aria-current'),'page')
  await fn.getByRole('button',{name:'Skills 管理',exact:true}).click()
  await fault.getByRole('button',{name:'重新加载应用',exact:true}).click()
  await fault.waitForLoadState('networkidle')
  await fault.getByRole('navigation',{name:'主导航'}).getByRole('button',{name:'Skills 管理',exact:true}).click()
  await fault.getByRole('button',{name:'扫描目录',exact:true}).waitFor({timeout:5000})
  await fault.waitForLoadState('networkidle')
  assert.equal(await fault.getByRole('alert').filter({hasText:'暂时无法显示'}).count(),0)
  await fault.screenshot({path:`${out}/skills-retry.png`,fullPage:true})
  assert.equal(failures,2)
  console.log('explicit reload recovery passed')
  await writeFile(`${out}/result.json`, JSON.stringify({errors,initialPageOnly:true,pages:7,delayedSwitch:true,retryRequests:failures},null,2))
} finally {
  release?.()
  await browser.close()
}
