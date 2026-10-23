// 浏览器生产预览的合成数据回归；不连接桌面接口，不创建或执行任务。
import { createRequire } from "node:module"
import assert from "node:assert/strict"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const require = createRequire(import.meta.url)
const { chromium } = require(require.resolve("playwright-core", { paths: [join(repo, "output/acceptance/node")] }))
const previewUrl = process.env.JOLU_PREVIEW_URL ?? "http://localhost:5307"
assert.ok(["localhost", "127.0.0.1", "[::1]"].includes(new URL(previewUrl).hostname), "只允许本机浏览器预览地址")

const browser = await chromium.launch({ channel: "msedge", headless: true })
try {
  const page = await browser.newPage({ viewport: { width: 1600, height: 1100 } })
  const errors = []
  page.on("pageerror", (error) => errors.push(error.message))
  await page.goto(previewUrl)
  await page.getByText("前端预览 · 数据均为设计示例", { exact: true }).waitFor()
  assert.equal(await page.evaluate(() => "__TAURI_INTERNALS__" in window), false, "拒绝连接真实桌面接口")
  const nav = page.getByRole("navigation", { name: "主导航" })
  await nav.getByRole("button", { name: "会话线程", exact: true }).click()
  await page.getByRole("button", { name: "更多会话操作" }).click()
  await page.getByRole("menuitem", { name: "查看用量明细" }).click()
  await page.getByRole("textbox").first().waitFor()
  assert.equal(await nav.getByRole("button", { name: "用量统计", exact: true }).getAttribute("aria-current"), "page")
  assert.ok(await page.getByRole("textbox").first().inputValue() === "session_demo_71fa0c1d2c08", "跨页后应保留原会话 ID 筛选")
  console.log("session to usage filter passed")

  await nav.getByRole("button", { name: "会话线程", exact: true }).click()
  await page.getByRole("button", { name: "创建自动化", exact: true }).click()
  await page.getByRole("button", { name: "继续新建", exact: true }).click()
  const dialog = page.getByRole("dialog", { name: "新建任务 · 来自会话" })
  await dialog.waitFor()
  await dialog.getByText("Codex", { exact: true }).waitFor()
  await dialog.getByText("优化 Jolu 的会话界面", { exact: true }).waitFor()
  await dialog.getByText("已带入当前会话", { exact: true }).waitFor()
  assert.ok(await dialog.getByPlaceholder("选择项目目录").inputValue() === "D:/develop/jolu", "任务应带入原项目目录")
  assert.ok(await dialog.getByPlaceholder("例如：完善登录页面").inputValue() === "继续开发：优化 Jolu 的会话界面", "任务名称应来自原会话")
  assert.equal(errors.length, 0, "页面不应出现未捕获异常")
  console.log("session to automation prefill passed")
} finally {
  await browser.close()
}
