import assert from "node:assert/strict"
import test from "node:test"
import { nextReleaseVersion } from "./next-release-version.mjs"

test("持续构建按已发布安装包版本递增，不受工作流运行次数影响", () => {
  const assets = [
    { name: "Jolu-v0.1.11-Windows-x86_64-Setup.exe" },
    { name: "Jolu-v0.1.9-Linux-x86_64.AppImage" },
    { name: "latest.json" },
  ]
  assert.equal(nextReleaseVersion("0.1.0", assets), "0.1.12")
})

test("不认识其他产品名的安装包", () => {
  const assets = [
    { name: "Other-App-v0.1.20-Windows-x86_64-Setup.exe" },
    { name: "Jolu-v0.1.5-Windows-x86_64-Setup.exe" },
  ]
  assert.equal(nextReleaseVersion("0.1.0", assets), "0.1.6")
})

test("补丁号 99 后到 100，100 后进位到下一次要版本", () => {
  assert.equal(nextReleaseVersion("0.1.99", []), "0.1.100")
  assert.equal(nextReleaseVersion("0.1.100", []), "0.2.0")
  assert.equal(nextReleaseVersion("0.2.0", []), "0.2.1")
})

test("首次持续构建从配置版本递增，忽略无关资源", () => {
  assert.equal(nextReleaseVersion("0.1.0", [{ name: "latest.json" }, { name: "Jolu-vx.y.z-debug.txt" }]), "0.1.1")
})

test("独立发行标签与旧 continuous 资源共同决定下一版", () => {
  const assets = [{ name: "Jolu-v0.1.13-Windows-x86_64-Setup.exe" }]
  assert.equal(nextReleaseVersion("0.1.0", assets, ["v0.1.0"]), "0.1.14")
  assert.equal(nextReleaseVersion("0.1.0", assets, ["v0.1.14", "v0.1.3"]), "0.1.15")
  assert.equal(nextReleaseVersion("0.1.0", [], ["v0.1.100", "continuous", "vbroken"]), "0.2.0")
})
