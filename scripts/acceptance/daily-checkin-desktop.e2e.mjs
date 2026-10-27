// 真实 Tauri IPC + 带标记隔离数据库：不读取真实登录、不向官方领取。
import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { mkdirSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { DEV_URL, connect, invoke, isolatedIsland, waitFor } from "./cdp.mjs"

const out = resolve(dirname(fileURLToPath(import.meta.url)), "../../output/acceptance/daily-checkin")
const { browser, pages } = await connect()
try {
  const { island, database } = await isolatedIsland(pages)
  await invoke(island, "menu_action", { id: "open_main" })
  const main = await waitFor("主窗口", async () => pages().find(p => p.url().startsWith(DEV_URL) && !p.url().includes("window=") && !p.url().includes("startup")))
  await main.waitForSelector('nav[aria-label="主导航"]')
  await assert.rejects(invoke(island, "daily_checkin_overview"), /主窗口/)
  execFileSync("python", ["-c", `import sqlite3,sys
db=sqlite3.connect(sys.argv[1])
rows=[('workbuddy','main'),('workbuddy','ai'),('qoder','cn'),('qoder','global'),('trae','cn'),('trae','ide')]
for platform,edition in rows:
 id='monitor:native:'+platform+':'+edition
 db.execute("INSERT INTO connections(id,platform,kind,name,secret,masked,status,created_ms) VALUES(?,?,'auth',?,'synthetic-no-credential','****','connected',1)",(id,platform,'isolated-'+platform+'-'+edition))
db.commit()
`, database])
  const overview = await invoke(main, "daily_checkin_overview")
  assert.equal(overview.accounts.length, 6)
  assert.equal(overview.records.length, 0)
  assert.equal(JSON.stringify(overview).includes("synthetic-no-credential"), false)
  assert.deepEqual(overview.accounts.map(a => a.edition).sort(), ["ai", "cn", "cn", "cn", "global", "global"])
  for (const account of overview.accounts) {
    const status = await invoke(main, "daily_checkin_status", { id: account.id })
    assert.equal(status.outcome.state, "unauthorized")
    assert.match(status.outcome.message, /隔离验收/)
    assert.equal(status.account_key, null)
    const claim = await invoke(main, "daily_checkin_claim", { id: account.id, expectedAccountKey: "synthetic-key" })
    assert.equal(claim.outcome.state, "unauthorized")
  }
  assert.equal((await invoke(main, "daily_checkin_overview")).records.length, 0)
  const nav = main.locator('nav[aria-label="主导航"]')
  await nav.getByRole("button", { name: "一键签到", exact: true }).click()
  await main.getByRole("heading", { name: "每日福利，一次领取" }).waitFor()
  await waitFor("隔离状态读取完成", async () => !(await main.getByRole("button", { name: "刷新状态", exact: true }).isDisabled()))
  assert.equal(await main.locator('[data-checkin-edition]').count(), 6)
  assert.equal(await main.getByRole("button", { name: "一键签到", exact: true }).last().isDisabled(), true)
  await invoke(main, "set_default_page", { page: "checkin" })
  await invoke(main, "remember_page", { page: "checkin" })
  assert.equal((await invoke(main, "get_settings")).default_page, "checkin")
  mkdirSync(out, { recursive: true })
  await main.screenshot({ path: resolve(out, "desktop-isolated.png") })
  console.log("PASS: 真实IPC主窗口限制/六版本映射/凭证脱敏/隔离禁止真实读写与联网/零领取记录/默认页保存/真实WebView页面")
} finally { await browser.close() }
