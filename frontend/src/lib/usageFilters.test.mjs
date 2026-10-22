import assert from "node:assert/strict"
import test from "node:test"
import { UNATTRIBUTED_ACCOUNT, accountsTriggerLabel, keepApplicable } from "./usageFilters.ts"

const accounts = [
  { id: "a", platformId: "claude", baseName: "Momo" },
  { id: "b", platformId: "codex", baseName: "Alex" },
]

test("账号筛选按钮文案", () => {
  assert.equal(accountsTriggerLabel([], accounts), "全部账号")
  assert.equal(accountsTriggerLabel(["a"], accounts), "Momo · Claude Code")
  assert.equal(accountsTriggerLabel([UNATTRIBUTED_ACCOUNT], accounts), "未归属账号")
  assert.equal(accountsTriggerLabel(["a", "b"], accounts), "2 个账号")
})

test("切换供应商后清除不适用的账号，保留未归属", () => {
  assert.deepEqual(keepApplicable(["a", "b", UNATTRIBUTED_ACCOUNT], accounts, "codex"), ["b", UNATTRIBUTED_ACCOUNT])
  assert.deepEqual(keepApplicable(["a", "b"], accounts, null), ["a", "b"])
})
