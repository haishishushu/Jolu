import assert from "node:assert/strict"
import test from "node:test"
import { buildCandidate } from "./sync-price.mjs"

const table = 'Price { platform: "claude", model: "claude-test", input: 1.0, cached_input: 0.1, output: 2.0, cache_write: None, source: "https://example.com/old", long_context: None, off_peak: None },'
const entry = { input_cost_per_token: 0.000002, output_cost_per_token: 0.000008, cache_read_input_token_cost: 0, source: "https://example.com/pricing" }

test("精确匹配模型并换算单位，缺缓存写入保持未知，规则不丢失", () => {
  const result = buildCandidate(table, { "claude-test": entry }, ["claude-test"])
  assert.match(result.table, /input: 2\.0, cached_input: 0\.0, output: 8\.0, cache_write: None/)
  assert.match(result.table, /long_context: None, off_peak: None/)
  assert.equal(result.models.length, 1)
})
test("缺模型、缺来源、非法价格或缺缓存读时整次失败", () => {
  assert.throws(() => buildCandidate(table, {}, ["claude-test"]), /缺少模型/)
  for (const invalid of [{ source: null }, { input_cost_per_token: -1 }, { output_cost_per_token: "0.1" }, { cache_read_input_token_cost: undefined }]) {
    assert.throws(() => buildCandidate(table, { "claude-test": { ...entry, ...invalid } }, ["claude-test"]))
  }
})
test("不猜私有别名，不自动新增平台归属", () => {
  assert.throws(() => buildCandidate(table, { "claude-test": entry }, ["claude-test-custom"]), /本地价目/)
})
test("保留并列出阶梯等上游字段供人工复核", () => {
  const result = buildCandidate(table, { "claude-test": { ...entry, input_cost_per_token_above_200k_tokens: 0.000004 } }, ["claude-test"])
  assert.equal(result.models[0].upstream.input_cost_per_token_above_200k_tokens, 0.000004)
  assert.match(result.warning, /人工复核/)
})
test("没有明确选择模型时不生成候选", () => {
  assert.throws(() => buildCandidate(table, { "claude-test": entry }, []), /指定模型/)
})
