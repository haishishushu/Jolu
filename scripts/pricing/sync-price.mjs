import { readFile, mkdir, writeFile } from "node:fs/promises"
import { createHash } from "node:crypto"
import { resolve, dirname } from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const UPSTREAM = "https://raw.githubusercontent.com/Wei-Shaw/model-price-repo/main/model_prices_and_context_window.json"
const WARNING = "候选价目须逐项对官方来源人工复核；阶梯、峰谷、缓存时长规则保持本地原值，不能直接覆盖生产价目。"

function rate(entry, key, optional = false) {
  if (entry[key] === undefined && optional) return "None"
  const value = entry[key]
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) throw new Error(`无效或缺少价格字段：${key}`)
  const perMillion = value * 1_000_000
  if (!Number.isFinite(perMillion)) throw new Error(`价格溢出：${key}`)
  const number = Number(perMillion.toPrecision(12)).toString()
  const literal = /[.e]/i.test(number) ? number : `${number}.0`
  return optional ? `Some(${literal})` : literal
}

/** 只更新明确选择且精确匹配的现有模型；保留平台和复杂规则，不猜别名或缺失价格。 */
export function buildCandidate(table, upstream, requested) {
  if (!requested.length) throw new Error("请用 --models 指定模型，逗号分隔")
  if (!upstream || typeof upstream !== "object" || Array.isArray(upstream)) throw new Error("上游必须是模型对象")
  const models = []
  const selected = [...new Set(requested)]
  for (const model of selected) {
    const entries = [...table.matchAll(/Price \{[^\n]+\}/g)].filter(([line]) => line.includes(`model: ${JSON.stringify(model)},`))
    if (!entries.length) throw new Error(`本地价目不存在模型：${model}；新增模型需先明确平台和计价规则`)
    if (!Object.hasOwn(upstream, model)) throw new Error(`上游缺少模型：${model}，不猜别名`)
    const entry = upstream[model]
    if (!entry || typeof entry !== "object") throw new Error(`模型数据无效：${model}`)
    if (typeof entry.source !== "string") throw new Error(`缺少来源：${model}`)
    const source = new URL(entry.source)
    if (source.protocol !== "https:" || source.username || source.password) throw new Error(`来源必须是无凭据的 HTTPS URL：${model}`)
    const values = {
      input: rate(entry, "input_cost_per_token"),
      cached_input: rate(entry, "cache_read_input_token_cost"),
      output: rate(entry, "output_cost_per_token"),
      cache_write: rate(entry, "cache_creation_input_token_cost", true),
    }
    for (const [old] of entries) {
      let next = old
      for (const [key, value] of Object.entries(values)) {
        next = next.replace(new RegExp(`\\b${key}: (?:Some\\([^)]*\\)|None|[^,]+),`), `${key}: ${value},`)
      }
      next = next.replace(/\bsource: "[^"]*"/, `source: ${JSON.stringify(source.href)}`)
      table = table.replace(old, next)
    }
    // 保留完整上游条目，便于核对基础价之外的阶梯、促销、缓存规则。
    models.push({ model, upstream: entry })
  }
  return { table, models, warning: WARNING }
}

async function main(args) {
  const options = new Map()
  for (let i = 0; i < args.length; i += 2) {
    if (!["--models", "--input"].includes(args[i]) || !args[i + 1] || options.has(args[i])) {
      throw new Error("用法：node scripts/pricing/sync-price.mjs --models model1,model2 [--input 本地上游JSON]")
    }
    options.set(args[i], args[i + 1])
  }
  const selected = (options.get("--models") ?? "").split(",").map(v => v.trim()).filter(Boolean)
  if (!selected.length) throw new Error("请用 --models 指定模型，逗号分隔")
  let raw
  if (options.has("--input")) raw = await readFile(resolve(options.get("--input")), "utf8")
  else {
    const response = await fetch(UPSTREAM, { signal: AbortSignal.timeout(30_000), redirect: "error" })
    if (!response.ok) throw new Error(`上游下载失败：HTTP ${response.status}`)
    raw = await response.text()
  }
  const current = await readFile(resolve(ROOT, "backend/src/pricing/table.rs"), "utf8")
  // 全部校验完成后才写文件；失败不生成半份表，也不修改生产文件。
  const result = buildCandidate(current, JSON.parse(raw), selected)
  const output = resolve(ROOT, "output/pricing", new Date().toISOString().replace(/[:.]/g, "-"))
  await mkdir(output, { recursive: true })
  await writeFile(resolve(output, "table.candidate.rs"), `//! 待人工复核的候选，尚未应用。\n${result.table}`, "utf8")
  await writeFile(resolve(output, "review.json"), JSON.stringify({
    source: options.has("--input") ? "local-file" : UPSTREAM,
    upstream_sha256: createHash("sha256").update(raw).digest("hex"),
    local_table_sha256: createHash("sha256").update(current).digest("hex"),
    warning: result.warning, models: result.models,
  }, null, 2) + "\n", "utf8")
  console.log(`已生成 ${result.models.length} 个模型的候选与复核记录：${output}\n${WARNING}`)
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main(process.argv.slice(2)).catch(error => { console.error(error.message); process.exitCode = 1 })
}
