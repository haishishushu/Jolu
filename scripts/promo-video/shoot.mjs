// 宣传片补拍：只截浏览器预览（示例数据），不连接桌面或真实账号。
// 用法：先在另一个终端启动预览（pnpm dev），再 node scripts/promo-video/shoot.mjs [要拍的画面...]
import { createRequire } from "node:module"
import { mkdir } from "node:fs/promises"
import { join, resolve, dirname } from "node:path"
import { fileURLToPath } from "node:url"

const require = createRequire(import.meta.url)
const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const { chromium } = require(require.resolve("playwright-core", { paths: [join(repo, "output/acceptance/node"), repo] }))
const url = process.env.JOLU_PREVIEW_URL ?? "http://localhost:5173"
const out = join(repo, "output/promo-video/shots")
await mkdir(out, { recursive: true })

const only = process.argv.slice(2)
const want = (k) => only.length === 0 || only.includes(k)
const browser = await chromium.launch({ channel: "msedge", headless: true })
const ctx = await browser.newContext({ viewport: { width: 1440, height: 1000 }, deviceScaleFactor: 2, reducedMotion: "reduce" })

async function panel(name, file, after) {
  const page = await ctx.newPage()
  await page.goto(url)
  await page.waitForLoadState("networkidle")
  if (await page.evaluate(() => "__TAURI_INTERNALS__" in window)) throw new Error("拒绝连接真实桌面接口")
  await page.getByRole("navigation", { name: "主导航" }).getByRole("button", { name, exact: true }).click()
  await page.waitForLoadState("networkidle")
  await page.waitForTimeout(1200)
  if (after) await after(page)
  await page.locator("main > div").first().screenshot({ path: join(out, `${file}.png`) })
  console.log("ok", file)
  await page.close()
}

const dark = async (page) => { await page.getByRole("button", { name: "切换主题" }).click(); await page.waitForTimeout(800) }

try {
  if (want("accounts")) await panel("供应商", "panel-accounts", async (page) => {
    await page.getByRole("tab", { name: "账号管理" }).click()
    await page.waitForTimeout(1200)
  })
  if (want("compress")) await panel("会话线程", "panel-compress", async (page) => {
    await page.getByRole("button", { name: "压缩会话" }).first().click()
    await page.waitForTimeout(1200)
  })
  // 深色主题：与宣传片的深色背景统一
  if (want("usage-dark")) await panel("用量统计", "panel-usage-dark", dark)
  if (want("checkin-dark")) await panel("一键签到", "panel-checkin-dark", dark)
  if (want("sessions-dark")) await panel("会话线程", "panel-sessions-dark", dark)
  if (want("skills-dark")) await panel("Skills 管理", "panel-skills-dark", dark)
  if (want("providers-dark")) await panel("供应商", "panel-providers-dark", dark)
  if (want("island-dark")) await panel("灵动岛", "panel-island-dark", dark)
  if (want("settings-dark")) await panel("设置", "panel-settings-dark", dark)
  if (want("automation-dark")) await panel("自动化", "panel-automation-dark", dark)
  if (want("automation-wait-dark")) await panel("自动化", "automation-wait-dark", async (page) => {
    await dark(page)
    await page.getByRole("button", { name: /查看等待/ }).first().click()
    await page.waitForTimeout(1200)
  })
  if (want("compress-dark")) await panel("会话线程", "panel-compress-dark", async (page) => {
    await dark(page)
    await page.getByRole("button", { name: "压缩会话" }).first().click()
    await page.waitForTimeout(1200)
  })
} finally {
  await browser.close()
}
