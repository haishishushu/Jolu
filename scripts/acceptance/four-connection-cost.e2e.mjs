// 灵动岛费用刷新与断开连接验收（原 four-connection-cost.cjs）。
// 旧脚本靠已删除的 api.importData 写入统计；现有的 importDataFile 会弹系统文件框，不适合自动化。
// 改为直接向隔离数据库的 requests 表写入合成记录，再「恢复连接」（set_connection_paused false）
// 触发一次全局 refresh-requested——与导入完成后发出的是同一个事件。核对：
//   1. 费用在同一个 DOM 节点上从「基线 + $3.00」变为「基线 + $6.00」（不重挂载）；
//   2. 断开连接后显示「连接已断开，当前仅显示本机统计」，灵动岛仍保留所选连接。
// 前置：按 README「隔离桌面流程」启动应用与 gateway.py（18765），在能解析 playwright / playwright-core 的目录下运行。
// 用法：node <仓库>/scripts/acceptance/four-connection-cost.e2e.mjs（JOLU_CDP / JOLU_DEV_URL 见 cdp.mjs）
import { DatabaseSync } from "node:sqlite"
import { connect, isolatedIsland, waitFor } from "./cdp.mjs"

const { pages } = await connect()
let exitCode = 0
let db = null
try {
  const { island, database } = await isolatedIsland(pages)
  const call = (method, ...args) => island.evaluate(async ({ method, args }) => (await import("/src/lib/api.ts")).api[method](...args), { method, args })
  const id = await call("addConnection", { platform: "claude", kind: "api", name: "Cost refresh fixture", secret: "test-valid", base_url: "http://127.0.0.1:18765" })
  await call("setIslandConnection", id)
  // 新隔离目录默认停靠在顶部，停靠未探出时不显示卡片、也不查询费用；切到自由位置让卡片常显
  await call("islandPosition", null, null)

  // 同一隔离目录可能跑过别的脚本：以写入前的今日估算为基线
  const base = await call("costEstimate", "claude", "today", null, null, Date.now(), null)
  if (!base.complete) throw new Error("今日已有记录无法估算，请换新的隔离目录：" + JSON.stringify(base))
  const label = (extra) => `≈ $${(base.amount + extra).toFixed(2)}`

  // 每条 claude-sonnet-4 输入 100 万 Token，按价目表估算 $3.00
  db = new DatabaseSync(database)
  db.exec("PRAGMA busy_timeout = 5000")
  const insertRow = db.prepare(`INSERT INTO requests (platform, source, dedup_key, session_id, ts, model,
      input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, total_tokens)
    VALUES ('claude', 'local', ?, 'cost-refresh', ?, 'claude-sonnet-4', 1000000, 0, 0, 0, 1000000)`)
  const insert = async (key) => {
    insertRow.run(`${key}-${Date.now()}`, Date.now())
    await call("setConnectionPaused", id, false)
  }

  await insert("cost-first")
  const first = island.getByText(label(3), { exact: true })
  await first.waitFor({ timeout: 12000 })
  const node = await first.elementHandle()
  await insert("cost-second")
  await island.getByText(label(6), { exact: true }).waitFor({ timeout: 12000 })
  const sameNode = await node.evaluate((el, text) => el.isConnected && el.textContent === text, label(6))
  if (!sameNode) throw new Error("费用更新时节点被替换（发生了重挂载）")

  await call("setConnectionPaused", id, true)
  await island.getByText("连接已断开，当前仅显示本机统计", { exact: true }).waitFor({ timeout: 12000 })
  if ((await call("getSettings")).island_connection_id !== id) throw new Error("断开连接后丢失了灵动岛的连接选择")
  console.log("RESULT " + JSON.stringify({ pass: true, costRefresh: `${label(3)} → ${label(6)}，同一节点`, pause: "保留连接选择", database }))
} catch (error) {
  exitCode = 1
  console.error("FAILED", error.message)
} finally {
  db?.close()
}
process.exit(exitCode)
