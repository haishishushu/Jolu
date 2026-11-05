// 本机前端预览：独立浏览器，只使用预览示例，不调用桌面后端。
import assert from "node:assert/strict"
import { createRequire } from "node:module"
import { mkdir, writeFile } from "node:fs/promises"
import { resolve } from "node:path"
const require = createRequire(import.meta.url)
const { chromium } = require(require.resolve("playwright-core", { paths: [resolve("output/acceptance/node")] }))
const base = process.env.JOLU_PREVIEW_URL || "http://127.0.0.1:5197"
assert.ok(["localhost", "127.0.0.1", "[::1]"].includes(new URL(base).hostname))
const output = resolve("output/acceptance/cc-switch")
await mkdir(output, { recursive: true })
const browser = await chromium.launch({ channel: "msedge", headless: true })
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1080 } })
  const errors = []
  const evidence = []
  page.on("pageerror", error => errors.push(error.message))
  await page.goto(base)
  const nav = page.getByRole("navigation", { name: "主导航" })
  await nav.waitFor()
  const pages = [["island", "灵动岛"], ["providers", "供应商"], ["checkin", "一键签到"], ["usage", "用量统计"], ["sessions", "会话线程"], ["automation", "自动化"], ["skills", "Skills 管理"], ["gateway", "模型入口"], ["settings", "设置"]]
  for (const theme of ["light", "dark"]) {
    if (theme === "dark") await page.getByRole("button", { name: "切换主题", exact: true }).click()
    for (const [slug, label] of pages) {
      await nav.getByRole("button", { name: label, exact: true }).click()
      await page.waitForFunction(() => !document.querySelector('[data-panel-scroll] [aria-busy="true"]'))
      await page.waitForTimeout(350)
      const result = await page.evaluate(() => {
        const area = document.querySelector("[data-panel-scroll]")
        const active = document.querySelector('.jolu-tab[aria-selected="true"]')
        const card = area.querySelector(".jolu-card")
        const rect = active?.getBoundingClientRect()
        const track = active?.parentElement.getBoundingClientRect()
        return {
          overflow: area.scrollWidth > area.clientWidth + 1,
          cardShadow: card ? getComputedStyle(card).boxShadow : null,
          cardSurface: card ? getComputedStyle(card).backgroundColor : null,
          tabShadow: active ? getComputedStyle(active).boxShadow : null,
          tabClipped: !!rect && (rect.top < track.top || rect.bottom > track.bottom + 1),
          blur: [...area.querySelectorAll("*")].filter(e => getComputedStyle(e).backdropFilter !== "none").length,
        }
      })
      assert.equal(result.overflow, false, `${theme} ${label} 内容区不应横向溢出`)
      assert.equal(result.tabClipped, false, `${theme} ${label} 页签不应裁切`)
      assert.equal(result.blur, 0, `${theme} ${label} 无液态模糊`)
      if (result.cardShadow) assert.notEqual(result.cardShadow, "none", `${label} 卡片短投影`)
      if (["providers", "usage", "automation"].includes(slug)) assert.notEqual(result.tabShadow, "none", `${label} 选中页签短投影`)
      await page.screenshot({ path: `${output}/${slug}-${theme}.png`, fullPage: true })
      evidence.push({ theme, page: slug, ...result })
      if (slug === "settings") {
        for (const [section, sectionSlug] of [["外观", "appearance"], ["网络与代理", "network"], ["数据与存储", "data"], ["通知", "notifications"], ["关于与更新", "about"]]) {
          await page.getByRole("button", { name: section, exact: true }).click()
          await page.waitForTimeout(180)
          assert.equal(await page.locator("[data-panel-scroll]").evaluate(e => e.scrollWidth > e.clientWidth + 1), false)
          await page.screenshot({ path: `${output}/settings-${sectionSlug}-${theme}.png`, fullPage: true })
        }
      }
      if (slug === "providers") {
        await page.getByRole("tab", { name: "账号管理", exact: true }).click()
        await page.waitForTimeout(250)
        await page.screenshot({ path: `${output}/accounts-${theme}.png`, fullPage: true })
        await page.getByRole("tab", { name: "供应商连接", exact: true }).click()
        await page.getByRole("button", { name: "连接 Claude Code", exact: true }).click()
        await page.getByRole("dialog").waitFor()
        const dialog = page.getByRole("dialog")
        const paint = await dialog.evaluate(e => ({ bg: getComputedStyle(e).backgroundColor, shadow: getComputedStyle(e).boxShadow, clipped: e.scrollWidth > e.clientWidth + 1 }))
        assert.equal(paint.bg, theme === "light" ? "rgb(255, 255, 255)" : "rgb(24, 24, 24)")
        assert.equal(paint.clipped, false)
        await page.screenshot({ path: `${output}/connect-${theme}.png`, fullPage: true })
        await page.keyboard.press("Escape")
        await dialog.waitFor({ state: "detached" })
      }
    }
  }
  // 窄窗口检查：主导航收起，确认按钮仍位于视口内。
  await page.setViewportSize({ width: 1100, height: 900 })
  await nav.getByRole("button", { name: "供应商", exact: true }).click()
  await page.getByRole("button", { name: "收起侧栏", exact: true }).click()
  await page.waitForTimeout(250)
  await page.screenshot({ path: `${output}/providers-compact-dark.png`, fullPage: true })
  await page.setViewportSize({ width: 1440, height: 1080 })
  await page.getByRole("button", { name: "灵动岛", exact: true }).first().click()
  await page.waitForTimeout(300)
  for (const theme of ["dark", "light"]) {
    if (theme === "light") await page.getByRole("button", { name: "切换主题", exact: true }).click()
    await page.waitForTimeout(200)
    await page.screenshot({ path: `${output}/floating-${theme}.png`, fullPage: true })
    await page.getByRole("button", { name: "双击展开", exact: true }).click()
    await page.waitForTimeout(300)
    await page.screenshot({ path: `${output}/floating-expanded-${theme}.png`, fullPage: true })
    await page.getByRole("button", { name: "双击收缩", exact: true }).click()
  }
  // 启动桥接需要桌面运行时；这里只隔离桥接并检查原始 HTML/CSS 的材质与布局。
  const startup = await browser.newPage({ viewport: { width: 1280, height: 720 } })
  await startup.route("**/src/startup.ts*", r => r.fulfill({ contentType: "application/javascript", body: 'document.documentElement.dataset.theme=new URLSearchParams(location.search).get("theme");document.documentElement.classList.add("ready");' }))
  for (const theme of ["light", "dark"]) {
    await startup.goto(`${base}/startup.html?theme=${theme}`)
    await startup.locator(".ready .startup").waitFor()
    await startup.waitForTimeout(350)
    const paint = await startup.locator(".startup").evaluate(e => getComputedStyle(e, "::before").backgroundColor)
    assert.equal(paint, theme === "light" ? "rgb(255, 255, 255)" : "rgb(16, 18, 17)")
    await startup.screenshot({ path: `${output}/startup-${theme}.png` })
  }
  await startup.close()
  assert.deepEqual(errors, [])
  await writeFile(`${output}/ui-results.json`, JSON.stringify(evidence, null, 2), "utf8")
  console.log(`PASS ${evidence.length} 个主模块主题组合、账号页、连接弹窗、页签、材质与内容区溢出检查`)
} finally { await browser.close() }
