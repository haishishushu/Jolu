// 主窗口冒烟验收：通过 WebView2 远程调试口驱动隔离运行的开发版应用，依次打开侧栏八个页面。
// 每页检查：没有「读取失败」「扫描失败」「刷新失败」；「正在读取」在 10 秒内消失；期间控制台没有 error、页面没有未捕获异常。
// 截图保存到 output/acceptance/main-window/<页面>.png。
// 前置：按 README「隔离桌面流程」启动应用，并在能解析 `playwright` 或 `playwright-core` 的目录下运行。
// 用法：node <仓库>/scripts/acceptance/main-window-smoke.e2e.mjs（JOLU_CDP / JOLU_DEV_URL 见 cdp.mjs）
import { mkdirSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { DEV_URL, connect, invoke, isolatedIsland, log, sleep, waitFor } from "./cdp.mjs"

const OUT = resolve(dirname(fileURLToPath(import.meta.url)), "../../output/acceptance/main-window")
const LOADING_TIMEOUT = 10_000
// 读取结束后再观察一会儿，接住稍晚到达的失败提示与控制台报错
const SETTLE_MS = 1_500
// 顺序与文案取自 lib/mainNavigation.ts（Jolu.pen 画板 01）
const PAGES = [
  ["island", "灵动岛"],
  ["providers", "供应商"],
  ["checkin", "一键签到"],
  ["usage", "用量统计"],
  ["sessions", "会话线程"],
  ["automation", "自动化"],
  ["skills", "Skills 管理"],
  ["settings", "设置"],
]

// 主窗口：开发地址下、不带 window= 参数（灵动岛 window=island、菜单 window=menu），也不是启动页
const isMain = (p) => p.url().startsWith(DEV_URL) && !p.url().includes("window=") && !p.url().includes("startup")

// 失败提示：页面级「读取失败」，Skills「扫描失败」，用量 / 额度「刷新失败」（含 toast）
const FAILURE_WORDS = ["读取失败", "扫描失败", "刷新失败"]

const { ctx, pages } = await connect()
const results = []
let exitCode = 0
try {
  const { island, database } = await isolatedIsland(pages)

  // 在打开主窗口之前就监听所有页面，覆盖主窗口首次加载时的报错；只记主窗口的
  let errors = []
  const watched = new WeakSet()
  const watch = (page) => {
    if (watched.has(page)) return
    watched.add(page)
    page.on("console", (m) => { if (m.type() === "error" && isMain(page)) errors.push("console: " + m.text().slice(0, 300)) })
    page.on("pageerror", (e) => { if (isMain(page)) errors.push("pageerror: " + e.message.slice(0, 300)) })
  }
  pages().forEach(watch)
  ctx.on("page", watch)

  // 与托盘「打开主窗口」同一路径：显示并聚焦主窗口（窗口不存在时创建）
  await invoke(island, "menu_action", { id: "open_main" })
  const main = await waitFor("主窗口页面", async () => pages().find(isMain), 60000)
  await main.waitForSelector('nav[aria-label="主导航"]', { timeout: 60000 })
  log("主窗口：", main.url())

  mkdirSync(OUT, { recursive: true })
  const nav = main.locator('nav[aria-label="主导航"]')
  const bodyText = () => main.evaluate(() => document.body.innerText)

  for (const [id, label] of PAGES) {
    const problems = []
    let loadingMs = null
    try {
      const started = Date.now()
      const button = nav.getByRole("button", { name: label, exact: true })
      await button.click()
      await waitFor(label + " 成为当前页", async () => (await button.getAttribute("aria-current")) === "page", 5000)
      try {
        await waitFor(label + "「正在读取」消失", async () => !(await bodyText()).includes("正在读取"), LOADING_TIMEOUT)
        loadingMs = Date.now() - started
      } catch {
        const line = (await bodyText()).split("\n").find((l) => l.includes("正在读取"))
        problems.push("「正在读取」超过 10 秒未消失：" + line?.trim())
      }
      await sleep(SETTLE_MS)
      const text = await bodyText()
      for (const line of text.split("\n").filter((l) => FAILURE_WORDS.some((w) => l.includes(w)))) problems.push("出现失败提示：" + line.trim().slice(0, 200))
      if (loadingMs !== null && text.includes("正在读取")) problems.push("「正在读取」再次出现")
    } catch (error) {
      problems.push(error.message)
    }
    // 第一页同时带上主窗口首次加载期间的报错；记完再清空，留给下一页
    problems.push(...errors)
    errors = []
    await main.screenshot({ path: resolve(OUT, id + ".png") }).catch((error) => problems.push("截图失败：" + error.message))
    const pass = problems.length === 0
    if (!pass) exitCode = 1
    results.push({ page: id, label, pass, loadingMs, problems })
    log(pass ? "通过" : "失败", label, loadingMs === null ? "" : `读取 ${loadingMs}ms`, pass ? "" : JSON.stringify(problems))
  }
  console.log("RESULT " + JSON.stringify({ pass: exitCode === 0, database, screenshots: OUT, pages: results }))
} catch (error) {
  exitCode = 1
  console.error("FAILED", error.message)
  console.error("pages:", pages().map((p) => p.url()))
}
process.exit(exitCode)
