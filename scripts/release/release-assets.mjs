import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs"
import { join, resolve } from "node:path"
import { pathToFileURL } from "node:url"

const REPO_DOWNLOAD = "https://github.com/haishishushu/Jolu/releases/download"

/** 各构建平台在 bundle 目录下的产物：子目录、匹配规则、发行资源后缀、是否带更新签名 */
export const PLATFORM_ASSETS = {
  "windows-x86_64": [{ dir: "nsis", match: /-setup\.exe$/, suffix: "Windows-x86_64-Setup.exe", signed: true }],
  "macos-arm64": [
    { dir: "dmg", match: /\.dmg$/, suffix: "macOS-arm64.dmg" },
    { dir: "macos", match: /\.app\.tar\.gz$/, suffix: "macOS-arm64.app.tar.gz", signed: true },
  ],
  "macos-x86_64": [
    { dir: "dmg", match: /\.dmg$/, suffix: "macOS-x86_64.dmg" },
    { dir: "macos", match: /\.app\.tar\.gz$/, suffix: "macOS-x86_64.app.tar.gz", signed: true },
  ],
  "linux-x86_64": [
    { dir: "appimage", match: /\.AppImage$/, suffix: "Linux-x86_64.AppImage", signed: true },
    { dir: "deb", match: /\.deb$/, suffix: "Linux-x86_64.deb" },
  ],
}

/** 更新清单平台 → 发行资源后缀（必须是带签名的那一个） */
export const UPDATER_PLATFORMS = {
  "windows-x86_64": "Windows-x86_64-Setup.exe",
  "darwin-aarch64": "macOS-arm64.app.tar.gz",
  "darwin-x86_64": "macOS-x86_64.app.tar.gz",
  "linux-x86_64": "Linux-x86_64.AppImage",
}

export const assetPrefix = (version) => `Jolu-v${version}-`

/**
 * 找出某平台的构建产物并给出发行资源名。每类产物必须恰好一个，
 * 带签名的产物还必须有同名 .sig，避免把旧版本残留或缺签名的包传上去。
 */
export function planAssets({ version, platform, listDir }) {
  const rules = PLATFORM_ASSETS[platform]
  if (!rules) throw new Error(`未知构建平台：${platform}`)
  const plan = []
  for (const rule of rules) {
    const files = listDir(rule.dir)
    const found = files.filter((name) => rule.match.test(name) && !name.endsWith(".sig"))
    if (found.length !== 1) throw new Error(`${platform} 的 ${rule.dir} 产物应恰好 1 个，实际 ${found.length} 个：${found.join(", ")}`)
    const to = `${assetPrefix(version)}${rule.suffix}`
    plan.push({ from: `${rule.dir}/${found[0]}`, to })
    if (rule.signed) {
      if (!files.includes(`${found[0]}.sig`)) throw new Error(`缺少更新签名：${rule.dir}/${found[0]}.sig`)
      plan.push({ from: `${rule.dir}/${found[0]}.sig`, to: `${to}.sig` })
    }
  }
  return plan
}

/** 由各平台 .sig 生成 latest.json；任一平台缺签名即失败，不生成残缺清单 */
export function buildManifest({ version, tag, notes, pubDate, readSignature }) {
  const platforms = {}
  for (const [target, suffix] of Object.entries(UPDATER_PLATFORMS)) {
    const name = `${assetPrefix(version)}${suffix}`
    const signature = readSignature(`${name}.sig`)
    if (!signature?.trim()) throw new Error(`缺少更新签名：${name}.sig`)
    platforms[target] = { signature: signature.trim(), url: `${REPO_DOWNLOAD}/${tag}/${encodeURIComponent(name)}` }
  }
  return { version, notes, pub_date: pubDate, platforms }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [, , command, ...args] = process.argv
  if (command === "collect") {
    const [version, platform, bundleDir, outDir] = args
    if (!outDir) throw new Error("用法：node scripts/release/release-assets.mjs collect <version> <platform> <bundleDir> <outDir>")
    const listDir = (dir) => (existsSync(join(bundleDir, dir)) ? readdirSync(join(bundleDir, dir)) : [])
    mkdirSync(outDir, { recursive: true })
    for (const { from, to } of planAssets({ version, platform, listDir })) {
      copyFileSync(join(bundleDir, from), join(outDir, to))
      console.log(`${from} → ${to}`)
    }
  } else if (command === "manifest") {
    const [version, tag, sigDir, outFile, notes] = args
    if (!outFile) throw new Error("用法：node scripts/release/release-assets.mjs manifest <version> <tag> <sigDir> <latest.json> [notes]")
    const readSignature = (name) => (existsSync(join(sigDir, name)) ? readFileSync(join(sigDir, name), "utf8") : null)
    const manifest = buildManifest({ version, tag, notes: notes || `Jolu ${tag}`, pubDate: new Date().toISOString(), readSignature })
    writeFileSync(outFile, `${JSON.stringify(manifest, null, 2)}\n`)
    console.log(`已生成 ${outFile}：${Object.keys(manifest.platforms).join(", ")}`)
  } else {
    throw new Error("用法：node scripts/release/release-assets.mjs <collect|manifest> ...")
  }
}
