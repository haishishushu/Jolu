import assert from "node:assert/strict"
import test from "node:test"
import { lineDiff } from "./textDiff.ts"

test("大块改写按总比较预算降级，保留公共首尾且不分配平方级矩阵", () => {
  const left = ["prefix", ...Array.from({ length: 1500 }, (_, i) => `old ${i}`), "shared middle", ...Array.from({ length: 1500 }, (_, i) => `old tail ${i}`), "suffix"]
  const right = ["prefix", ...Array.from({ length: 1500 }, (_, i) => `new ${i}`), "shared middle", ...Array.from({ length: 1500 }, (_, i) => `new tail ${i}`), "suffix"]
  const diff = lineDiff(left.join("\n"), right.join("\n"))
  assert.deepEqual(diff.filter((line) => line.kind === "same").map((line) => line.text), ["prefix", "suffix"])
  assert.equal(diff.filter((line) => line.kind === "del").length, 3001)
  assert.equal(diff.filter((line) => line.kind === "add").length, 3001)
})

const short = (lines) => lines.map((l) => `${l.kind} ${l.text}`)

test("按行对比：中间一行被替换", () => {
  assert.deepEqual(short(lineDiff("a\nb\nc", "a\nx\nc")), ["same a", "del b", "add x", "same c"])
})

test("两段相同时全部是 same", () => {
  const text = "---\nname: api-review\n---\n# 标题"
  assert.ok(lineDiff(text, text).every((l) => l.kind === "same"))
  assert.equal(lineDiff(text, text).length, 4)
})

test("空串对 a 只有一行新增", () => {
  assert.deepEqual(short(lineDiff("", "a")), ["add a"])
  assert.deepEqual(short(lineDiff("a", "")), ["del a"])
  assert.deepEqual(lineDiff("", ""), [])
})

test("统一换行符，尾部插入与删除", () => {
  assert.deepEqual(short(lineDiff("a\r\nb", "a\nb\nc")), ["same a", "same b", "add c"])
  assert.deepEqual(short(lineDiff("x\na\nb", "a\nb")), ["del x", "same a", "same b"])
})

test("裁掉公共前后缀后中间仍超过 5000 行时退化为整段替换", () => {
  const left = Array.from({ length: 5001 }, (_, i) => `old ${i}`)
  const right = Array.from({ length: 5001 }, (_, i) => `new ${i}`)
  const result = lineDiff(["头", ...left, "尾"].join("\n"), ["头", ...right, "尾"].join("\n"))
  assert.deepEqual(short([result[0], result[1], result.at(-1)]), ["same 头", "del old 0", "same 尾"])
  assert.equal(result.filter((l) => l.kind === "del").length, 5001)
  assert.equal(result.filter((l) => l.kind === "add").length, 5001)
  assert.equal(result[5002].kind, "add", "先全部删除再全部新增")
})

test("6000 行只改一行时不退化", () => {
  const lines = Array.from({ length: 6000 }, (_, i) => `line ${i}`)
  const changed = [...lines]
  changed[3000] = "改过的一行"
  const result = lineDiff(lines.join("\n"), changed.join("\n"))
  assert.equal(result.length, 6001)
  assert.equal(result.filter((l) => l.kind === "same").length, 5999)
  assert.deepEqual(short(result.filter((l) => l.kind !== "same")), ["del line 3000", "add 改过的一行"])
})

test("末尾单个换行不产生多余的空行", () => {
  assert.deepEqual(short(lineDiff("a\nb\n", "a\nb\n")), ["same a", "same b"])
  assert.deepEqual(short(lineDiff("", "a\n")), ["add a"])
  assert.deepEqual(short(lineDiff("a\r\n", "a\r\nb\r\n")), ["same a", "add b"])
  // 只差末尾空行时仍能看出来
  assert.deepEqual(short(lineDiff("a\n", "a\n\n")), ["same a", "add "])
})
