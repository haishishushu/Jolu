// 独立浏览器与合成 Promise；不连接真实桌面或修改用户文件。
import assert from "node:assert/strict"
import { createRequire } from "node:module"
import { mkdir } from "node:fs/promises"
import { resolve } from "node:path"

const require = createRequire(import.meta.url)
const { chromium } = require(require.resolve("playwright-core", { paths: [resolve("output/acceptance/node")] }))
const browser = await chromium.launch({ channel: "msedge", headless: true })
const base = process.env.JOLU_PREVIEW_URL || "http://127.0.0.1:5197"
assert.ok(["localhost", "127.0.0.1", "[::1]"].includes(new URL(base).hostname))
const output = resolve("output/acceptance/cc-switch")
await mkdir(output, { recursive: true })
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } })
  const errors = []
  page.on("pageerror", e => { errors.push(e.message); console.error("PAGE ERROR", e.message) })

  await page.route("**/src/hooks/useSettings.ts*", r => r.fulfill({ contentType: "application/javascript", body: `const settings={island_platform:'claude',island_kind:'auth',island_connection_id:null,island_connection_name:null,island_source_id:null,dock:{edge:null,offset:0,monitor:null},island_clones:[{id:'fixture-clone',name:'合成分身'}],island_visible:true,always_on_top:false}; export function useSettings(){return {settings,ready:true}}` }))
  await page.route("**/confirmation-check", route => route.fulfill({ contentType: "text/html", body: `<!doctype html><html><head><meta charset="UTF-8"></head><body><div id="root"></div><script type="module">
    import RefreshRuntime from '/@react-refresh';
    RefreshRuntime.injectIntoGlobalHook(window); window.$RefreshReg$=()=>{}; window.$RefreshSig$=()=>type=>type;
    window.__vite_plugin_react_preamble_installed__=true;
    await import('/confirmation-fixture');</script></body></html>` }))
  await page.route("**/confirmation-fixture", route => route.fulfill({ contentType: "application/javascript", body: `
import React from '/node_modules/.vite/deps/react.js';
import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
import { ConfirmDialog } from '/src/components/ui/ConfirmDialog.tsx';
import { SkillEditor } from '/src/components/skills/SkillEditor.tsx';
import { ContextMenuWindow } from '/src/views/ContextMenuWindow.tsx';
import { api } from '/src/lib/api.ts';
api.menuRefreshing=async()=>false;api.menuAction=()=>{count++;return new Promise((resolve,reject)=>{resolveAction=resolve;rejectAction=reject})};
import { ToastProvider } from '/src/components/ui/Toast.tsx';
import { TipProvider } from '/src/components/ui/Tip.tsx';
import '/src/index.css';
let count=0, resolveAction, rejectAction;
function Fixture(){const [open,setOpen]=React.useState(false);return React.createElement(React.Fragment,null,
React.createElement('button',{onClick:()=>setOpen(true)},'打开合成确认'),
open&&React.createElement(ConfirmDialog,{title:'合成删除确认',description:'仅测试，不修改真实文件',confirmLabel:'确认删除',danger:true,onClose:()=>setOpen(false),onConfirm:()=>{count++;return new Promise((resolve,reject)=>{resolveAction=resolve;rejectAction=reject})}}))}
const root=ReactDOM.createRoot(document.getElementById('root'));
function render(child){root.render(React.createElement(TipProvider,null,React.createElement(ToastProvider,null,child)))}
render(React.createElement(Fixture));
window.fixture={count:()=>count,success:()=>resolveAction(),fail:()=>rejectAction(new Error('合成失败，请重试')),showMenu:()=>{history.replaceState(null,'','?source=island');render(React.createElement(ContextMenuWindow))},showEditor:()=>render(React.createElement(SkillEditor,{target:{mode:'new',platform:'claude'},items:[],platforms:[],projects:[],onCancel:()=>{},onSaved:()=>{}}))};
` }))
  await page.goto(`${base}/confirmation-check`)
  await page.getByRole("button", { name: "打开合成确认" }).waitFor({ timeout: 15000 })
  const open = () => page.getByRole("button", { name: "打开合成确认" }).click()
  await open()
  await page.getByRole("dialog").waitFor()
  assert.equal(await page.evaluate(() => document.activeElement?.textContent), "取消")
  await page.keyboard.press("Escape")
  await page.getByRole("dialog").waitFor({ state: "detached" })
  assert.equal(await page.evaluate(() => window.fixture.count()), 0)
  assert.equal(await page.evaluate(() => document.activeElement?.textContent), "打开合成确认")
  await open()
  await page.mouse.click(20, 100)
  assert.equal(await page.getByRole("dialog").count(), 1, "遮罩不能误关危险确认")
  await page.getByRole("button", { name: "确认删除", exact: true }).evaluate(button => { button.click(); button.click() })
  assert.equal(await page.evaluate(() => window.fixture.count()), 1)
  await page.keyboard.press("Escape")
  assert.equal(await page.getByRole("dialog").count(), 1)
  assert.ok(await page.getByRole("button", { name: "取消", exact: true }).isDisabled())
  await page.evaluate(() => window.fixture.fail())
  await page.getByRole("alert").filter({ hasText: "合成失败" }).waitFor()
  await page.getByRole("button", { name: "确认删除", exact: true }).click()
  assert.equal(await page.evaluate(() => window.fixture.count()), 2)
  await page.evaluate(() => document.documentElement.classList.add("dark"))
  await page.screenshot({ path: `${output}/confirm-dark.png` })
  await page.evaluate(() => window.fixture.success())
  await page.getByRole("dialog").waitFor({ state: "detached" })
  await page.evaluate(() => { document.documentElement.classList.remove("dark"); window.fixture.showEditor() })
  const editor = page.getByRole("textbox", { name: "SKILL.md 内容", exact: true })
  await editor.fill("草稿内容，取消替换后必须保留")
  const template = async () => { await page.getByRole("button", { name: "插入模板" }).click(); await page.getByRole("menuitem", { name: /基础模板/ }).click() }
  await template()
  await page.getByRole("dialog").waitFor()
  await page.getByRole("button", { name: "取消", exact: true }).last().click()
  await page.getByRole("dialog").waitFor({ state: "detached" })
  assert.equal(await editor.inputValue(), "草稿内容，取消替换后必须保留")
  assert.equal(await page.evaluate(() => document.activeElement?.textContent), "插入模板", "从菜单打开的确认关闭后应回到菜单触发按钮")
  await template()
  await page.getByRole("button", { name: "替换模板", exact: true }).click()
  await page.getByRole("dialog").waitFor({ state: "detached" })
  assert.notEqual(await editor.inputValue(), "草稿内容，取消替换后必须保留")
  await page.setViewportSize({ width: 340, height: 460 })
  await page.evaluate(() => window.fixture.showMenu())
  await page.getByRole("menuitem", { name: "销毁分身", exact: true }).click()
  await page.getByRole("dialog").waitFor()
  const bounds = await page.getByRole("dialog").boundingBox()
  assert.ok(bounds.x >= 0 && bounds.y >= 0 && bounds.x + bounds.width <= 340 && bounds.y + bounds.height <= 460, "紧凑菜单确认框应位于视口内")
  const beforeDestroy = await page.evaluate(() => window.fixture.count())
  await page.getByRole("button", { name: "取消", exact: true }).click()
  await page.getByRole("dialog").waitFor({ state: "detached" })
  assert.equal(await page.evaluate(() => window.fixture.count()), beforeDestroy)
  await page.getByRole("menuitem", { name: "销毁分身", exact: true }).click()
  await page.getByRole("button", { name: "销毁灵动岛", exact: true }).click()
  assert.equal(await page.evaluate(() => window.fixture.count()), beforeDestroy + 1)
  await page.keyboard.press("Escape")
  assert.equal(await page.getByRole("dialog").count(), 1)
  await page.evaluate(() => window.fixture.fail())
  await page.getByRole("dialog").getByRole("alert").waitFor()
  await page.screenshot({ path: `${output}/clone-confirm-compact.png` })
  await page.getByRole("button", { name: "取消", exact: true }).click()
  await page.getByRole("dialog").waitFor({ state: "detached" })
  assert.deepEqual(errors, [])
  console.log("PASS 确认取消不执行、焦点恢复、遮罩保护、异步互斥、失败保留与重试、模板覆盖确认")
} finally { await browser.close() }
