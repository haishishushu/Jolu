import assert from "node:assert/strict"
import test from "node:test"
import { buildManifest, planAssets } from "./release-assets.mjs"
import { verifyRelease } from "./verify-release.mjs"

const SIGNATURE = Buffer.from([
  "untrusted comment: signature from tauri secret key",
  Buffer.alloc(74, 1).toString("base64"),
  "trusted comment: timestamp:1\tfile:x",
  Buffer.alloc(64, 2).toString("base64"),
].join("\n")).toString("base64")

const dirs = (tree) => (dir) => tree[dir] ?? []

test("planAssets：Windows 安装包与签名按约定重命名", () => {
  const plan = planAssets({ version: "0.1.0", platform: "windows-x86_64", listDir: dirs({ nsis: ["Jolu_0.1.0_x64-setup.exe", "Jolu_0.1.0_x64-setup.exe.sig"] }) })
  assert.deepEqual(plan, [
    { from: "nsis/Jolu_0.1.0_x64-setup.exe", to: "Jolu-v0.1.0-Windows-x86_64-Setup.exe" },
    { from: "nsis/Jolu_0.1.0_x64-setup.exe.sig", to: "Jolu-v0.1.0-Windows-x86_64-Setup.exe.sig" },
  ])
})

test("planAssets：macOS 与 Linux 同时收集安装包和更新包", () => {
  const mac = planAssets({ version: "0.1.0", platform: "macos-arm64", listDir: dirs({ dmg: ["Jolu_0.1.0_aarch64.dmg"], macos: ["Jolu.app", "Jolu.app.tar.gz", "Jolu.app.tar.gz.sig"] }) })
  assert.deepEqual(mac.map((item) => item.to), ["Jolu-v0.1.0-macOS-arm64.dmg", "Jolu-v0.1.0-macOS-arm64.app.tar.gz", "Jolu-v0.1.0-macOS-arm64.app.tar.gz.sig"])
  const linux = planAssets({ version: "0.1.0", platform: "linux-x86_64", listDir: dirs({ appimage: ["Jolu_0.1.0_amd64.AppImage", "Jolu_0.1.0_amd64.AppImage.sig"], deb: ["Jolu_0.1.0_amd64.deb"] }) })
  assert.deepEqual(linux.map((item) => item.to), ["Jolu-v0.1.0-Linux-x86_64.AppImage", "Jolu-v0.1.0-Linux-x86_64.AppImage.sig", "Jolu-v0.1.0-Linux-x86_64.deb"])
})

test("planAssets：缺包、多包、缺签名或未知平台都失败", () => {
  assert.throws(() => planAssets({ version: "0.1.0", platform: "windows-x86_64", listDir: dirs({}) }), /恰好 1 个/)
  assert.throws(() => planAssets({ version: "0.1.0", platform: "windows-x86_64", listDir: dirs({ nsis: ["a-setup.exe", "b-setup.exe"] }) }), /恰好 1 个/)
  assert.throws(() => planAssets({ version: "0.1.0", platform: "windows-x86_64", listDir: dirs({ nsis: ["Jolu_0.1.0_x64-setup.exe"] }) }), /缺少更新签名/)
  assert.throws(() => planAssets({ version: "0.1.0", platform: "freebsd", listDir: dirs({}) }), /未知构建平台/)
})

test("buildManifest：四个更新平台指向本次发行资源，且能通过发布校验", () => {
  const manifest = buildManifest({ version: "0.1.0", tag: "v0.1.0", notes: "n", pubDate: "2026-10-27T00:00:00Z", readSignature: () => SIGNATURE })
  assert.deepEqual(Object.keys(manifest.platforms).sort(), ["darwin-aarch64", "darwin-x86_64", "linux-x86_64", "windows-x86_64"])
  assert.equal(manifest.platforms["windows-x86_64"].url, "https://github.com/haishishushu/Jolu/releases/download/v0.1.0/Jolu-v0.1.0-Windows-x86_64-Setup.exe")
  const files = [
    "Windows-x86_64-Setup.exe", "macOS-arm64.dmg", "macOS-x86_64.dmg", "Linux-x86_64.AppImage", "Linux-x86_64.deb",
    "macOS-arm64.app.tar.gz", "macOS-x86_64.app.tar.gz",
  ].map((suffix) => `Jolu-v0.1.0-${suffix}`)
  const sigs = ["Windows-x86_64-Setup.exe", "macOS-arm64.app.tar.gz", "macOS-x86_64.app.tar.gz", "Linux-x86_64.AppImage"].map((suffix) => `Jolu-v0.1.0-${suffix}.sig`)
  const assets = [...files, ...sigs, "latest.json"].map((name) => ({ name }))
  assert.doesNotThrow(() => verifyRelease({ version: "0.1.0", tag: "v0.1.0", manifest, assets }))
})

test("buildManifest：任一平台缺签名不生成清单", () => {
  assert.throws(() => buildManifest({ version: "0.1.0", tag: "v0.1.0", notes: "n", pubDate: "x", readSignature: (name) => (name.includes("Linux") ? null : SIGNATURE) }), /Linux-x86_64\.AppImage\.sig/)
})
