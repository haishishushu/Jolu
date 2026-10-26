import assert from "node:assert/strict"
import test from "node:test"
import { verifyRelease } from "./verify-release.mjs"

const version = "0.1.14"
const tag = `v${version}`
const prefix = `Jolu-v${version}-`
const assets = [
  `${prefix}Windows-x86_64-Setup.exe`, `${prefix}Windows-x86_64-Setup.exe.sig`,
  `${prefix}macOS-arm64.dmg`, `${prefix}macOS-x86_64.dmg`,
  `${prefix}Linux-x86_64.AppImage`, `${prefix}Linux-x86_64.AppImage.sig`,
  `${prefix}Linux-x86_64.deb`, `${prefix}Linux-x86_64.deb.sig`, "latest.json",
].map((name) => ({ name }))
const signature = Buffer.from("untrusted comment: signature from test key\n" + Buffer.alloc(74).toString("base64") + "\ntrusted comment: test\n" + Buffer.alloc(64).toString("base64")).toString("base64")
const manifest = {
  version,
  platforms: {
    "windows-x86_64": { signature, url: `https://github.com/haishishushu/Jolu/releases/download/${tag}/${prefix}Windows-x86_64-Setup.exe` },
    "linux-x86_64": { signature, url: `https://github.com/haishishushu/Jolu/releases/download/${tag}/${prefix}Linux-x86_64.AppImage` },
  },
}

test("独立发行版只包含自身版本的安装包与更新清单", () => {
  assert.doesNotThrow(() => verifyRelease({ version, tag, manifest, assets }))
})

test("混入旧版本安装包时拒绝标为最新", () => {
  assert.throws(() => verifyRelease({ version, tag, manifest, assets: [...assets, { name: "Jolu-v0.1.13-Windows-x86_64-Setup.exe" }] }), /其他版本/)
})

test("清单仍指向 continuous 时拒绝标为最新", () => {
  const stale = structuredClone(manifest)
  stale.platforms["windows-x86_64"].url = stale.platforms["windows-x86_64"].url.replace(tag, "continuous")
  assert.throws(() => verifyRelease({ version, tag, manifest: stale, assets }), /下载地址/)
})

test("缺少系统安装包时拒绝发布清单", () => {
  assert.throws(() => verifyRelease({ version, tag, manifest, assets: assets.filter((asset) => !asset.name.endsWith("macOS-arm64.dmg")) }), /缺少/)
})

test("更新清单缺少签名时拒绝发布", () => {
  const unsigned = structuredClone(manifest)
  delete unsigned.platforms["windows-x86_64"].signature
  assert.throws(() => verifyRelease({ version, tag, manifest: unsigned, assets }), /签名/)
})

test("更新清单不接受伪装同路径的其他主机", () => {
  const malicious = structuredClone(manifest)
  malicious.platforms["windows-x86_64"].url = malicious.platforms["windows-x86_64"].url.replace("github.com", "example.com")
  assert.throws(() => verifyRelease({ version, tag, manifest: malicious, assets }), /官方仓库/)
})

test("更新清单不接受 HTTP 降级下载", () => {
  const insecure = structuredClone(manifest)
  insecure.platforms["windows-x86_64"].url = insecure.platforms["windows-x86_64"].url.replace("https:", "http:")
  assert.throws(() => verifyRelease({ version, tag, manifest: insecure, assets }), /官方仓库/)
})

test("Windows 应用的清单缺少 Windows 平台时拒绝发布", () => {
  const missing = structuredClone(manifest)
  delete missing.platforms["windows-x86_64"]
  assert.throws(() => verifyRelease({ version, tag, manifest: missing, assets }), /Windows 平台/)
})

test("Windows 更新条目不能指向同版 Linux 安装包", () => {
  const mismatch = structuredClone(manifest)
  mismatch.platforms["windows-x86_64"] = mismatch.platforms["linux-x86_64"]
  assert.throws(() => verifyRelease({ version, tag, manifest: mismatch, assets }), /Windows 安装包/)
})
