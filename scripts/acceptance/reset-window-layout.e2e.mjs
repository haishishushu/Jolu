// 仅连接带 .acceptance-profile 标记的隔离桌面实例，验证真实原生窗口重置不会死锁。
import assert from "node:assert/strict"
import { connect, invoke, isolatedIsland, waitFor } from "./cdp.mjs"

function deadline(promise, label, ms = 8000) {
  let timer
  return Promise.race([
    promise,
    new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`${label}超时，原生窗口可能死锁`)), ms) }),
  ]).finally(() => clearTimeout(timer))
}

const watchdog = setTimeout(() => { console.error("FAIL: 窗口重置验收超时"); process.exit(1) }, 60000)
const { browser, pages } = await connect()
try {
  const { island } = await isolatedIsland(pages)
  const main = await waitFor("主窗口", async () => pages().find(page => !new URL(page.url()).searchParams.has("window") && !page.url().includes("startup.html")))
  await main.locator('nav[aria-label="主导航"]').waitFor()
  await main.getByRole("button", { name: "灵动岛", exact: true }).click()
  const reset = main.getByRole("button", { name: "重置位置", exact: true })
  await reset.waitFor()

  // 经已授权的窗口操作触发 Resized / Moved，不扩大产品的窗口权限。
  await main.evaluate(async () => {
    const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js")
    const window = getCurrentWindow()
    await window.toggleMaximize()
    await window.toggleMaximize()
  })
  await reset.click()
  await deadline(main.waitForFunction(() => ![...document.querySelectorAll("button")].find(button => button.textContent.trim() === "重置位置")?.disabled), "主面板重置")
  await deadline(invoke(main, "get_settings"), "重置后设置查询")

  // 重复重置、菜单入口、隐藏主面板、分身停靠都复用同一后端路径。
  await deadline(invoke(island, "island_action", { id: "clone_create", island: null }), "创建分身")
  await deadline(invoke(island, "menu_action", { id: "pos_top" }), "停靠本体")
  const before = await invoke(island, "get_settings")
  const clone = before.island_clones[0]
  assert.ok(clone)
  await deadline(invoke(island, "island_position", { edge: "left", island: clone.id }), "停靠分身")
  for (const command of ["island_action", "menu_action", "island_action"]) {
    await deadline(invoke(island, command, { id: "reset_layout", island: null }), command)
    const settings = await deadline(invoke(main, "get_settings"), "重置后查询")
    assert.equal(settings.dock.edge, null)
    assert.ok(settings.island_clones.every(item => item.dock.edge === null))
    assert.equal(settings.island_visible, true)
  }
  await main.evaluate(async () => { const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js"); await getCurrentWindow().close() })
  await deadline(invoke(island, "menu_action", { id: "reset_layout" }), "隐藏主面板后重置")
  await deadline(invoke(island, "menu_action", { id: "open_main" }), "重新打开主面板")
  const geometry = await deadline(main.evaluate(async () => {
    const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js")
    const window = getCurrentWindow(), scale = await window.scaleFactor(), size = await window.innerSize()
    return { width: size.width / scale, height: size.height / scale, visible: await window.isVisible() }
  }), "重置后窗口几何查询")
  assert.deepEqual(geometry, { width: 1280, height: 900, visible: true })
  console.log("PASS: 主面板按钮、菜单、重复重置、分身解除停靠、隐藏后重置均完成，窗口和 IPC 持续响应")
} finally {
  clearTimeout(watchdog)
  await deadline(browser.close(), "断开隔离浏览器", 3000).catch(() => {})
}
