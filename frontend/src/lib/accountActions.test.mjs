import assert from "node:assert/strict"
import test from "node:test"
import { removeAccountWithReplacement } from "./accountActions.ts"

test("替代账号启用失败时保留原账号", async () => {
  const calls = []
  await assert.rejects(removeAccountWithReplacement("original", "replacement", {
    enable: async id => { calls.push(`enable:${id}`); throw new Error("授权不匹配") },
    remove: async id => { calls.push(`remove:${id}`) },
  }), /授权不匹配/)
  assert.deepEqual(calls, ["enable:replacement"])
})

test("切换成功后才删除原连接，无替代时不修改CLI", async () => {
  const calls = []
  const actions = { enable: async id => { calls.push(`enable:${id}`) }, remove: async id => { calls.push(`remove:${id}`) } }
  await removeAccountWithReplacement("original", "replacement", actions)
  assert.deepEqual(calls, ["enable:replacement", "remove:original"])
  calls.length = 0
  await removeAccountWithReplacement("original", null, actions)
  assert.deepEqual(calls, ["remove:original"])
})
