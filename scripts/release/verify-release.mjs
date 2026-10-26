import { readFileSync } from "node:fs"
import { resolve } from "node:path"
import { pathToFileURL } from "node:url"

const INSTALLERS = [
  "Windows-x86_64-Setup.exe", "macOS-arm64.dmg", "macOS-x86_64.dmg",
  "Linux-x86_64.AppImage", "Linux-x86_64.deb",
]

/** 发布 latest.json 前确认发行版、安装包及更新地址都属于同一版本。 */
export function verifyRelease({ version, tag, manifest, assets }) {
  if (!/^\d+\.\d+\.\d+$/.test(version) || tag !== `v${version}`) {
    throw new Error(`发行标签与版本不一致：${tag} / ${version}`)
  }
  if (manifest?.version !== version) {
    throw new Error(`更新清单版本不一致：${manifest?.version} / ${version}`)
  }

  const prefix = `Jolu-v${version}-`
  const names = new Set(assets.map((asset) => asset.name))
  for (const name of names) {
    if (name !== "latest.json" && !name.startsWith(prefix)) {
      throw new Error(`发行版混入其他版本资源：${name}`)
    }
  }
  for (const suffix of INSTALLERS) {
    if (!names.has(`${prefix}${suffix}`)) throw new Error(`缺少安装包：${prefix}${suffix}`)
  }
  if (!names.has("latest.json")) throw new Error("缺少 latest.json")

  const platforms = Object.entries(manifest.platforms ?? {})
  if (platforms.length === 0) throw new Error("更新清单缺少平台")
  if (!manifest.platforms["windows-x86_64"]) throw new Error("更新清单缺少 Windows 平台：windows-x86_64")
  for (const [target, platform] of platforms) {
    const url = platform?.url
    if (typeof url !== "string") throw new Error("更新清单缺少下载地址")
    const parsed = new URL(url)
    if (parsed.origin !== "https://github.com" || parsed.username || parsed.password || parsed.search || parsed.hash
      || !parsed.pathname.startsWith("/haishishushu/Jolu/releases/download/")) {
      throw new Error(`更新清单下载地址必须来自官方仓库的 HTTPS 发行资源：${url}`)
    }
    const marker = `/haishishushu/Jolu/releases/download/${tag}/`
    if (!parsed.pathname.startsWith(marker)) throw new Error(`更新清单下载地址指向了其他发行版：${url}`)
    const name = decodeURIComponent(parsed.pathname.slice(marker.length))
    if (!name.startsWith(prefix) || !names.has(name)) {
      throw new Error(`更新清单下载地址对应的安装包不存在：${url}`)
    }
    if (target === "windows-x86_64" && name !== `${prefix}Windows-x86_64-Setup.exe`) {
      throw new Error(`Windows 安装包与更新平台不匹配：${name}`)
    }
    const signature = platform.signature
    if (typeof signature !== "string" || !signature.trim()) throw new Error(`更新清单缺少签名：${name}`)
    const lines = Buffer.from(signature.trim(), "base64").toString("utf8").trim().split(/\r?\n/)
    // 此处检查发布清单的签名封装；密码学验签仍由 updater 使用内置公钥完成。
    if (!lines[0]?.startsWith("untrusted comment:") || Buffer.from(lines[1] ?? "", "base64").length !== 74
      || !lines[2]?.startsWith("trusted comment:") || Buffer.from(lines[3] ?? "", "base64").length !== 64) {
      throw new Error(`更新清单签名格式不正确：${name}`)
    }
    if (!names.has(`${name}.sig`)) throw new Error(`缺少安装包签名文件：${name}.sig`)
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [, , version, tag, manifestPath, releasePath] = process.argv
  if (!version || !tag || !manifestPath || !releasePath) {
    throw new Error("用法：node scripts/release/verify-release.mjs <version> <tag> <latest.json> <release.json>")
  }
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"))
  const release = JSON.parse(readFileSync(releasePath, "utf8"))
  verifyRelease({ version, tag, manifest, assets: release.assets ?? [] })
  console.log(`已验证 ${tag}：安装包及更新清单均属于 ${version}`)
}
