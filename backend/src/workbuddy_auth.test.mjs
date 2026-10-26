import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import vm from "node:vm"
import { createCipheriv, createHash } from "node:crypto"
import { EventEmitter } from "node:events"
import test from "node:test"

const source = () => readFileSync(new URL("./workbuddy_auth.cjs", import.meta.url), "utf8")
const secret = Buffer.alloc(32, 7).toString("base64")
const key = createHash("sha256").update(secret).digest()
const keyId = createHash("sha256").update(key).digest("hex").slice(0, 16)
const lp = value => { const bytes = Buffer.from(value); const size = Buffer.alloc(4); size.writeUInt32BE(bytes.length); return Buffer.concat([size, bytes]) }
function envelope() {
  const nonce = Buffer.alloc(12, 3)
  const cipher = createCipheriv("aes-256-gcm", key, nonce)
  cipher.setAAD(Buffer.concat([Buffer.from("WB-AAD\0"), Buffer.from([1]), lp("WBEV1"), lp("sym-v1"), Buffer.from([0, 0, 0, 1]), lp(keyId), Buffer.from([2, 0, 0])]))
  const ciphertext = Buffer.concat([cipher.update("synthetic.access-token"), cipher.final()])
  return { $wbEncrypted: 1, envelope: Buffer.from(JSON.stringify({ suite: 1, keyId, nonce: nonce.toString("base64"), authTag: cipher.getAuthTag().toString("base64"), ciphertext: ciphertext.toString("base64") })).toString("base64") }
}
async function run(value, payload = { version: 1, atRestSecretKey: secret }) {
  const stdin = new EventEmitter()
  let output = ""
  const context = { Buffer, require: name => { assert.equal(name, "node:crypto"); return crypto }, process: { stdin, stdout: { write: text => { output += text } }, _linkedBinding: name => { assert.equal(name, "electron_browser_workbuddy_storage"); return { loggerGet: () => JSON.stringify(payload) } } } }
  vm.runInNewContext(source(), context)
  stdin.emit("data", Buffer.from(JSON.stringify(value)))
  stdin.emit("end")
  return JSON.parse(output)
}
const crypto = await import("node:crypto")

test("只在内存中解码官方 sym-v1 凭证，输出不含密钥", async () => {
  const result = await run(envelope())
  assert.deepEqual(result, { token: "synthetic.access-token" })
  assert.ok(!JSON.stringify(result).includes(secret))
})
test("篡改、未知格式和不同客户端密钥均拒绝，不回显敏感字段", async () => {
  const valid = envelope()
  const raw = JSON.parse(Buffer.from(valid.envelope, "base64"))
  const bad = { ...raw, authTag: Buffer.alloc(16).toString("base64") }
  for (const [value, payload] of [[{ $wbEncrypted: 2, envelope: valid.envelope }], [{ ...valid, scheme: "asym-v1" }], [{ $wbEncrypted: 1, envelope: "invalid" }], [{ ...valid, envelope: Buffer.from(JSON.stringify(bad)).toString("base64") }], [valid, { version: 1, atRestSecretKey: Buffer.alloc(32, 9).toString("base64") }]]) {
    const result = await run(value, payload)
    assert.equal(typeof result.error, "string")
    assert.equal(result.token, undefined)
    assert.ok(!JSON.stringify(result).includes(secret))
  }
})
