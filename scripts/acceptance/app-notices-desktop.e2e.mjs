// 只在 .acceptance-profile 隔离桌面运行：使用本机合成网关，不修改真实账号或阈值。
import assert from "node:assert/strict"
import { mkdirSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { connect, isolatedIsland, invoke, waitFor } from "./cdp.mjs"

const output = resolve(dirname(fileURLToPath(import.meta.url)), "../../output/acceptance/app-notices")
const { browser, pages } = await connect()
let main, connectionId
try {
  const { island } = await isolatedIsland(pages)
  await invoke(island, "menu_action", { id: "open_main" })
  main = await waitFor("主窗口", async () => pages().find(page => !new URL(page.url()).searchParams.has("window") && !page.url().includes("startup.html")))
  await main.locator('nav[aria-label="主导航"]').waitFor()
  const call = (method, ...args) => main.evaluate(async ({ method, args }) => (await import("/src/lib/api.ts")).api[method](...args), { method, args })
  await call("setBalanceAlert", null, "USD")
  connectionId = await call("addConnection", { platform: "custom", kind: "api", name: "通知验收示例", secret: "test-notification-fixture", base_url: "http://127.0.0.1:18765", protocol: "openai", model: "synthetic-model" })
  await call("setBalanceAlert", 20, "USD")
  await invoke(island, "menu_action", { id: "open_main" })
  const balance = await call("connectionBalance", connectionId, true)
  assert.equal(balance.state, "ok")
  assert.equal(balance.balance, 12.5)
  const card = main.locator('[data-balance-notice="true"]')
  await card.waitFor({ timeout: 10000 })
  await card.hover()
  assert.match(await card.locator('[data-balance-amount]').innerText(), /\$12\.50/)
  assert.match(await card.innerText(), /\$20\.00 USD/)
  await call("connectionBalance", connectionId, true)
  assert.equal(await card.count(), 1, "持续低余额不重复弹卡片")
  mkdirSync(output, { recursive: true })
  await main.screenshot({ path: resolve(output, "balance-card-desktop.png") })
  await card.getByRole("button", { name: "查看账号", exact: true }).click()
  await main.locator(`[data-account-id="${connectionId}"][data-account-located="true"]`).waitFor()
  assert.equal(await main.getByRole("tab", { name: "账号管理", exact: true }).getAttribute("aria-selected"), "true")
  await assert.rejects(invoke(island, "take_app_notices"), /仅主窗口/)
  await main.waitForFunction(() => !document.querySelector('[data-balance-notice="true"]'))
  await call("setDnd", true)
  await call("setBalanceAlert", 30, "USD")
  await call("connectionBalance", connectionId, true)
  assert.deepEqual(await call("takeAppNotices"), [])
  assert.equal(await card.count(), 0, "免打扰不弹前台提醒")
  await call("setDnd", false)
  await call("connectionBalance", connectionId, true)
  assert.deepEqual(await call("takeAppNotices"), [], "免打扰期间已发生的跌破不补弹")
  console.log("PASS desktop: real balance query → amount card → account target; repeat suppression, main-only reads and DND")
} finally {
  if (main && connectionId) await invoke(main, "remove_connection", { id: connectionId }).catch(() => {})
  await browser.close()
}
