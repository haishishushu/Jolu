// 直连 CDP 的验收脚本（*.e2e.mjs）共用部分：解析 Playwright、连接隔离应用、核验隔离目录。
import { existsSync } from "node:fs"
import { createRequire } from "node:module"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const normalize = (path) => resolve(path).replaceAll("\\", "/").toLowerCase()

export const CDP = process.env.JOLU_CDP ?? "http://127.0.0.1:9337"
/** 前端开发地址；端口被占用、改用其他 devUrl 编译应用时用 JOLU_DEV_URL 覆盖 */
export const DEV_URL = (process.env.JOLU_DEV_URL ?? "http://localhost:5173").replace(/\/+$/, "") + "/"

export const log = (...args) => console.log(new Date().toISOString().slice(11, 19), ...args)
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

export async function waitFor(label, fn, timeout = 20000) {
  const started = Date.now()
  for (;;) {
    const value = await fn()
    if (value) return value
    if (Date.now() - started > timeout) throw new Error("超时：" + label)
    await sleep(200)
  }
}

/** 从当前工作目录解析 playwright 或 playwright-core（仓库本身不依赖它们） */
function loadChromium() {
  const require = createRequire(pathToFileURL(resolve(process.cwd(), "noop.js")))
  for (const name of ["playwright", "playwright-core"]) {
    try { return require(name).chromium } catch {}
  }
  throw new Error("当前目录解析不到 playwright / playwright-core，请在装有它的目录下运行")
}

/** 连接隔离应用；只断开、不关闭（应用由 processes.json 记录的 PID 统一停止） */
export async function connect() {
  const chromium = loadChromium()
  const browser = await waitFor("连接 CDP", async () => {
    try { return await chromium.connectOverCDP(CDP, { timeout: 3000 }) } catch { return null }
  }, 60000)
  const ctx = browser.contexts()[0]
  return { browser, ctx, pages: () => ctx.pages().filter((p) => !p.isClosed()) }
}

export const invoke = (page, cmd, args = {}) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args])

/**
 * 隔离目录核验：数据库必须位于 <仓库>/output/acceptance/profile/ 下（规范化斜杠与大小写后比较），
 * 且所在目录有 start.ps1 写入的 .acceptance-profile 标记；否则抛错，调用方不得继续操作或写库。
 */
export function assertIsolatedDatabase(database) {
  const root = normalize(join(REPO, "output/acceptance/profile")) + "/"
  if (!normalize(database).startsWith(root) || !existsSync(join(dirname(database), ".acceptance-profile"))) {
    throw new Error("拒绝在非隔离数据目录运行：" + database)
  }
}

/** 等本体灵动岛加载完成并核验隔离目录，返回 { island, database } */
export async function isolatedIsland(pages) {
  const island = await waitFor("灵动岛页面", async () => pages().find((p) => {
    const url = p.url()
    return url.startsWith(DEV_URL) && url.includes("window=island") && !url.includes("clone=")
  }), 120000)
  await island.waitForSelector("#root > *", { timeout: 60000 })
  const info = await waitFor("数据目录就绪", async () => { try { return await invoke(island, "data_info") } catch { return null } }, 60000)
  const database = String(info.path)
  assertIsolatedDatabase(database)
  return { island, database }
}
