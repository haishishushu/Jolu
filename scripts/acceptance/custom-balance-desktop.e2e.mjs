// 仅在带标记的隔离桌面中运行；使用本机合成网关，不读取或切换真实登录。
import assert from "node:assert/strict"
import { DatabaseSync } from "node:sqlite"
import { connect, isolatedIsland } from "./cdp.mjs"

const { pages } = await connect()
const created = []
let db
let call
let exitCode = 0
try {
  const { island, database } = await isolatedIsland(pages)
  call = (method, ...args) => island.evaluate(async ({ method, args }) => (await import("/src/lib/api.ts")).api[method](...args), { method, args })
  for (const [secret, expected] of [["test-zero", "ok"], ["test-forbidden", "forbidden"], ["test-unauthorized", "unauthorized"], ["test-malformed", "failed"], ["test-missing", "unsupported"]]) {
    const id = await call("addConnection", { platform: "custom", kind: "api", name: `Custom balance ${expected}`, secret, base_url: "http://127.0.0.1:18765", protocol: "openai", model: "synthetic-model" })
    created.push(id)
    const balance = await call("connectionBalance", id, true)
    assert.equal(balance.state, expected)
    if (expected === "ok") { assert.equal(balance.balance, 0); assert.equal(balance.currency, "USD") }
    assert.equal((await call("connectionQuota", id, true)).state, "unsupported", "余额授权不扩大为自定义订阅额度")
    const connection = (await call("listConnections")).find(item => item.id === id)
    assert.ok(connection)
    assert.equal("secret" in connection, false)
    assert.ok(!JSON.stringify(connection).includes(secret))
  }
  db = new DatabaseSync(database)
  db.exec("PRAGMA busy_timeout = 5000")
  for (const [id, platform] of [["native:trae:ide", "trae"], ["native:trae:cn", "trae"], ["native:zcode:account", "zcode"], ["native:workbuddy:ai", "workbuddy"], ["native:qoder:global", "qoder"]]) {
    const key = `monitor:${id}`
    db.prepare("INSERT INTO connections(id,platform,kind,name,label,masked,status,created_ms) VALUES(?,?,'auth','Synthetic native source','fixture','','connected',?)").run(key, platform, Date.now())
    created.push(key)
    const quota = await call("connectionQuota", key, true)
    assert.equal(quota.state, "unauthorized")
    assert.match(quota.reason, /隔离/)
  }
  console.log("PASS desktop: custom zero, permission, auth, malformed, unsupported; no credential DTO; isolated native sources never read real logins")
} catch (error) {
  exitCode = 1
  console.error("FAILED", error.message)
} finally {
  db?.close()
  if (call) {
    for (const id of created) {
      try { await call("removeConnection", id) } catch { exitCode = 1; console.error("FAILED fixture cleanup") }
    }
  }
}
process.exit(exitCode)
