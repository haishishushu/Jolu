import assert from "node:assert/strict"
import test from "node:test"
import * as accountActions from "./accountActions.ts"
const { removeAccountWithReplacement } = accountActions

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

test("选择只保留当前列表中的唯一连接，并保留选择顺序", () => {
  assert.equal(typeof accountActions.retainAccountSelection, "function")
  assert.deepEqual(accountActions.retainAccountSelection(["b", "gone", "a", "b"], ["a", "b", "c"]), ["b", "a"])
  const filtered = accountActions.retainAccountSelection(["a", "b"], ["b"])
  assert.deepEqual(accountActions.retainAccountSelection(filtered, ["a", "b"]), ["b"])
})

test("批量删除去重后顺序处理所选连接，失败不阻断后续且不会自动重试", async () => {
  assert.equal(typeof accountActions.removeAccounts, "function")
  const calls = []
  let inFlight = 0
  const result = await accountActions.removeAccounts(["a", "b", "a", "c"], async id => {
    assert.equal(inFlight++, 0)
    calls.push(id)
    await Promise.resolve()
    inFlight--
    if (id === "b") throw new Error("被自动化引用")
  })
  assert.deepEqual(calls, ["a", "b", "c"])
  assert.deepEqual(result, { deletedIds: ["a", "c"], failed: [{ id: "b", message: "被自动化引用" }] })
  calls.length = 0
  assert.deepEqual(await accountActions.removeAccounts(result.failed.map(item => item.id), async id => { calls.push(id) }), { deletedIds: ["b"], failed: [] })
  assert.deepEqual(calls, ["b"])
})

test("空批量选择不会调用删除，非Error拒绝仍保留原因", async () => {
  assert.equal(typeof accountActions.removeAccounts, "function")
  assert.deepEqual(await accountActions.removeAccounts([], async () => { assert.fail("空列表不应调用删除") }), { deletedIds: [], failed: [] })
  assert.deepEqual(await accountActions.removeAccounts(["a"], async () => { throw "连接不存在" }), { deletedIds: [], failed: [{ id: "a", message: "连接不存在" }] })
})
