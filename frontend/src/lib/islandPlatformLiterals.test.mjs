import test from "node:test"
import assert from "node:assert/strict"
import { readFileSync, readdirSync } from "node:fs"
import { join, relative } from "node:path"
import { fileURLToPath } from "node:url"

/**
 * 架构守卫：灵动岛组件只允许查 `lib/providers/registry.ts`，不允许再写平台字面量。
 *
 * 平台判断散落在 `components/island/` 里是「接一家改多处」的根源（
 * `docs/specs/2026-11-05-provider-quota-descriptor.md` §6.4）。这里做静态扫描：
 * 去掉注释后，组件代码里不允许再出现 `grok` / `zcode` / `bigmodel`。
 *
 * 注释里的平台名保留（解释「ZCode 的 bucket 列表」这类事实），所以先剥注释再扫；
 * 剥注释会正确跳过字符串和模板字符串里的 `//`。
 */

const LIB_DIR = fileURLToPath(new URL(".", import.meta.url))
const SRC_DIR = join(LIB_DIR, "..")
const ISLAND_DIR = join(SRC_DIR, "components", "island")
const REGISTRY = join(LIB_DIR, "providers", "registry.ts")
const WATCHED = ["grok", "zcode", "bigmodel"]
const CODE_FILE = /\.(ts|tsx)$/

/** 递归列出灵动岛目录下的源码与样式 */
function listFiles(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) return listFiles(path)
    return entry.isFile() && /\.(ts|tsx|css)$/.test(entry.name) ? [path] : []
  })
}

/**
 * 剥掉注释，保留字符串内容（字符串里的平台名同样算违规）。
 * `.css` 只有块注释；`.ts` / `.tsx` 两种都剥。
 */
function stripComments(source, lineComments) {
  let out = ""
  let quote = null
  let index = 0
  while (index < source.length) {
    const char = source[index]
    const next = source[index + 1]
    if (quote) {
      out += char
      if (char === "\\") { out += next ?? ""; index += 2; continue }
      if (char === quote) quote = null
      index += 1
      continue
    }
    if (char === "\"" || char === "'" || char === "`") { quote = char; out += char; index += 1; continue }
    if (char === "/" && next === "*") {
      const end = source.indexOf("*/", index + 2)
      index = end === -1 ? source.length : end + 2
      continue
    }
    if (lineComments && char === "/" && next === "/") {
      const end = source.indexOf("\n", index + 2)
      index = end === -1 ? source.length : end
      continue
    }
    out += char
    index += 1
  }
  return out
}

/** 某个文件里出现的平台字面量 */
function violationsIn(source, file) {
  const code = stripComments(source, CODE_FILE.test(file))
  return WATCHED.filter((name) => new RegExp(`\\b${name}\\b`, "i").test(code))
}

test("灵动岛组件里不再出现平台字面量", () => {
  const files = listFiles(ISLAND_DIR)
  // 改动前这三处是平台分支的所在；必须真的扫到它们，否则这个守卫等于没跑
  for (const name of ["CapsuleIsland.tsx", "UsageIsland.tsx", "CapsuleGateway.tsx"]) {
    assert.ok(files.some((file) => file.endsWith(name)), `扫描范围里缺少 ${name}`)
  }
  const offenders = files.flatMap((file) =>
    violationsIn(readFileSync(file, "utf8"), file).map((name) => `${relative(SRC_DIR, file)}: ${name}`),
  )
  assert.deepEqual(offenders, [], "平台判断必须查 lib/providers/registry.ts，不许写在 components/island/")
})

test("平台字面量搬进了 lib/providers/registry.ts，不是被删掉", () => {
  const registry = readFileSync(REGISTRY, "utf8")
  for (const name of WATCHED) {
    assert.match(registry, new RegExp(name, "i"), `${name} 的平台分支应当落在注册表里`)
  }
})

test("守卫本身能识别字面量，且不误报注释", () => {
  assert.deepEqual(violationsIn('const rows = data.platform === "grok" ? [] : authQuotaRows(data.quotas)', "x.tsx"), ["grok"])
  assert.deepEqual(violationsIn('const label = "bigmodel"', "x.ts"), ["bigmodel"])
  assert.deepEqual(violationsIn('const anchor = "https://zcode.z.ai"', "x.tsx"), ["zcode"])
  // 注释里写平台名是允许的：它解释事实，不是分支
  assert.deepEqual(violationsIn("// 其余（ZCode 的 bucket 列表，key 是 bucket_id）", "x.tsx"), [])
  assert.deepEqual(violationsIn("/* grok / zcode */ const x = 1", "x.tsx"), [])
  assert.deepEqual(violationsIn("/* 5h → 5 小时 */", "x.css"), [])
})
