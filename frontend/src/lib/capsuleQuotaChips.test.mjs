import test from "node:test"
import assert from "node:assert/strict"
import {
  capsuleQuotaChips,
  chipTone,
  compactQuotaNumber,
  MAX_WINDOW_NAME,
  referenceWindowName,
  shortWindowName,
  usedOfTotal,
} from "./capsuleQuotaChips.ts"

const render = (chips) => chips.map((chip) => `${chip.name} ${chip.value}`).join(" ")

test("有已用百分比时优先显示百分比，不再出现整句", () => {
  const { chips, more } = capsuleQuotaChips([
    { key: "bkt-1", name: "GLM-5.3", usedPercent: 0, amountText: "0 / 3000000 token", remainingText: "剩余 3000000 token" },
    { key: "bkt-2", name: "GLM-5.3-Flash", usedPercent: 62, amountText: "3100000 / 5000000 token" },
  ])
  assert.equal(more, 0)
  assert.equal(render(chips), "GLM-5.3 0% Flash 62%")
  // 实测故障串里的整句不得再出现
  const text = render(chips)
  assert.doesNotMatch(text, /剩余额度/)
  assert.doesNotMatch(text, /剩余 \d+ token/)
  // 剩余原文与重置倒计时仍在悬停说明里
  assert.match(chips[0].title, /GLM-5\.3/)
  assert.match(chips[0].title, /剩余 3000000 token/)
})

test("没有百分比时回落成压缩的 已用/总量", () => {
  const { chips } = capsuleQuotaChips([
    { key: "bkt-1", name: "GLM-5.3", usedPercent: null, amountText: "0 / 3000000 token" },
  ])
  assert.equal(chips[0].value, "0/3.0M")
  assert.equal(render(chips), "GLM-5.3 0/3.0M")
  assert.equal(usedOfTotal("250000 / 1000000 token"), "250.0K/1.0M")
  assert.equal(usedOfTotal("3M / 3M token"), "3M/3M", "来源已压缩的写法不二次换算")
  assert.equal(usedOfTotal("没有斜杠"), null)
})

test("窗口名超过 6 字符时用同一胶囊内一致的短形式", () => {
  const { chips } = capsuleQuotaChips([
    { key: "bkt-1", name: "GLM-5.3", usedPercent: 0 },
    { key: "bkt-2", name: "GLM-5.3-Flash", usedPercent: 62 },
  ])
  assert.deepEqual(chips.map((chip) => chip.name), ["GLM-5.3", "Flash"])
  // 基准名（别的窗口名的前缀）保留，变体去公共前缀
  assert.equal(referenceWindowName(["GLM-5.3", "GLM-5.3-Flash"]), "GLM-5.3")
  assert.equal(shortWindowName("GLM-5.3", "GLM-5.3"), "GLM-5.3")
  assert.equal(shortWindowName("GLM-5.3-Flash", "GLM-5.3"), "Flash")
  // 单个窗口时它自己就是基准名：GLM-5.3 保持完整，不被截成 GLM-5…
  assert.equal(referenceWindowName(["GLM-5.3"]), "GLM-5.3")
  assert.equal(shortWindowName("GLM-5.3", referenceWindowName(["GLM-5.3"])), "GLM-5.3")
  assert.deepEqual(capsuleQuotaChips([{ key: "only", name: "GLM-5.3", usedPercent: 0 }]).chips.map((c) => c.name), ["GLM-5.3"])
  // 没有基准关系（互相不是前缀）的长名字：截到 5 字符 + 省略号，名字总长不超过上限
  assert.equal(shortWindowName("some-very-long-window"), "some…")
  assert.ok(shortWindowName("some-very-long-window").length <= MAX_WINDOW_NAME, "截断后名字不超过上限")
  assert.equal(shortWindowName("7d"), "7d")
  const unrelated = capsuleQuotaChips([
    { key: "a", name: "Qwen3-Coder-Plus", usedPercent: 10 },
    { key: "b", name: "Kimi-K2-Thinking", usedPercent: 20 },
  ])
  assert.deepEqual(unrelated.chips.map((chip) => chip.name), ["Qwen3…", "Kimi…"])
})

test("数字压缩边界：0 / 999 / 1000 / 1e6 / 1e9", () => {
  assert.equal(compactQuotaNumber(0), "0")
  assert.equal(compactQuotaNumber(999), "999")
  assert.equal(compactQuotaNumber(1000), "1.0K")
  assert.equal(compactQuotaNumber(12_500), "12.5K")
  assert.equal(compactQuotaNumber(1_000_000), "1.0M")
  assert.equal(compactQuotaNumber(3_000_000), "3.0M")
  assert.equal(compactQuotaNumber(20_250_000), "20.3M", "增量段 +20.25M 只保留一位小数")
  assert.equal(compactQuotaNumber(1_000_000_000), "1.0B")
  assert.equal(compactQuotaNumber(1_200_000_000), "1.2B")
  // 保留一位小数：不会再出现 3.00M 这种多余零
  assert.equal(compactQuotaNumber(2_999_999), "3.0M")
  assert.equal(compactQuotaNumber(Number.NaN), "-")
})

test("超过 2 个窗口时只显示最紧的一个 + 被折叠的数量", () => {
  const { chips, more } = capsuleQuotaChips([
    { key: "a", name: "GLM-5.3", usedPercent: 12 },
    { key: "b", name: "GLM-5.3-Flash", usedPercent: 92 },
    { key: "c", name: "GLM-5.3-Air", usedPercent: 4 },
  ])
  assert.equal(more, 2)
  assert.equal(chips.length, 1)
  assert.equal(render(chips), "Flash 92%")
  assert.equal(chips[0].tone, "warn")
  // 全是未知时保留第一个，并照实显示 -
  const unknown = capsuleQuotaChips([
    { key: "a", name: "GLM-5.3", usedPercent: null },
    { key: "b", name: "Flash", usedPercent: null },
    { key: "c", name: "Air", usedPercent: null },
  ])
  assert.equal(unknown.more, 2)
  assert.equal(unknown.chips[0].value, "-")
})

test("未知 - 与真实 0 各自保持原样", () => {
  const unknown = capsuleQuotaChips([{ key: "quota", name: "套餐额度", usedPercent: null }])
  assert.equal(unknown.chips[0].value, "-")
  assert.equal(unknown.chips[0].tone, null, "未知不着色，不画成告警")
  assert.equal(unknown.chips[0].name, "套餐额度", "中文短名不截断")
  const zero = capsuleQuotaChips([{ key: "quota", name: "GLM-5.3", usedPercent: 0 }])
  assert.equal(zero.chips[0].value, "0%")
  assert.notEqual(zero.chips[0].value, "-", "真实的 0 不等于未知")
  // 只有剩余量时也不回到整句：压成 余0
  const remaining = capsuleQuotaChips([{ key: "quota", name: "GLM-5.3", usedPercent: null, remainingText: "剩余 0 token" }])
  assert.equal(remaining.chips[0].value, "余0")
})

test("水温着色：≥80 橙、100 红", () => {
  assert.equal(chipTone(0), null)
  assert.equal(chipTone(79.9), null)
  assert.equal(chipTone(80), "warn")
  assert.equal(chipTone(99), "warn")
  assert.equal(chipTone(100), "danger")
  assert.equal(chipTone(null), null)
  const { chips } = capsuleQuotaChips([{ key: "q", name: "GLM-5.3", usedPercent: 100 }])
  assert.equal(chips[0].value, "100%")
  assert.equal(chips[0].tone, "danger")
})
