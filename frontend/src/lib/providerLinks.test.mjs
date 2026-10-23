import assert from "node:assert/strict"
import test from "node:test"
import { PROVIDER_LINKS, linkFor } from "./providerLinks.ts"
import { PROVIDERS } from "./providerCatalog.ts"

test("收录的链接都是 HTTPS，且每条都写了核实日期", () => {
  const entries = Object.entries(PROVIDER_LINKS)
  assert.ok(entries.length > 0)
  for (const [platform, links] of entries) {
    assert.match(links.verified, /^\d{4}-\d{2}-\d{2}$/, `${platform} 缺少核实日期`)
    assert.ok(links.loginGuide || links.switchAccount, `${platform} 没有任何链接却占了一条记录`)
    for (const url of [links.loginGuide, links.switchAccount]) {
      if (url === undefined) continue
      assert.ok(url.startsWith("https://"), `${platform}：${url}`)
      assert.doesNotThrow(() => new URL(url))
    }
  }
})

test("只给提供订阅连接的平台收录链接", () => {
  const subscription = new Set(PROVIDERS.filter((p) => p.subscription === "ready").map((p) => p.id))
  for (const platform of Object.keys(PROVIDER_LINKS)) {
    assert.ok(subscription.has(platform), `${platform} 没有订阅连接`)
  }
})

test("linkFor：有链接返回地址，未收录的平台或种类返回 null", () => {
  assert.equal(linkFor("claude", "loginGuide"), PROVIDER_LINKS.claude.loginGuide)
  assert.equal(linkFor("custom", "loginGuide"), null)
  assert.equal(linkFor("opencode", "switchAccount"), null)
  // Gemini 官方文档没有切换账号的说明，只收录登录指引
  assert.equal(typeof linkFor("gemini", "loginGuide"), "string")
  assert.equal(linkFor("gemini", "switchAccount"), null)
})
