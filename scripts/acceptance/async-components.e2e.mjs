// 真实组件 + 内存中可控 Promise；没有真实桌面、Skills 文件或账号访问。
import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import { createRequire } from "node:module"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const here = dirname(fileURLToPath(import.meta.url))
const repo = resolve(here, "../..")
const require = createRequire(import.meta.url)
const { chromium } = require(require.resolve("playwright-core", { paths: [join(repo, "output/acceptance/node")] }))
const baseUrl = new URL(process.env.JOLU_COMPONENT_URL ?? "http://localhost:5311")
assert.ok(["localhost", "127.0.0.1", "[::1]"].includes(baseUrl.hostname), "只允许本机开发服务器")
const moduleText = await readFile(join(here, "async-components.fixture.mjs"), "utf8")
const browser = await chromium.launch({ channel: "msedge", headless: true })
try {
  const page = await browser.newPage({ viewport: { width: 1500, height: 1100 } })
  const errors = []
  page.on("pageerror", (error) => errors.push(error.message))
  // 使真实组件进入异步分支；所有已知 API 在 fixture 中替换，其余桥接一律拒绝。
  await page.addInitScript(() => {
    window.unexpectedNativeCalls = 0
    window.__TAURI_INTERNALS__ = { invoke: () => {
      window.unexpectedNativeCalls++
      return Promise.reject(new Error("合成验收禁止真实原生调用"))
    } }
  })
  await page.route("**/async-components-check", (route) => route.fulfill({ contentType: "text/html", body: `
    <!doctype html><html><head><meta charset="UTF-8"></head><body><div id="root"></div><script type="module">
    import RefreshRuntime from '/@react-refresh';
    RefreshRuntime.injectIntoGlobalHook(window); window.$RefreshReg$=()=>{}; window.$RefreshSig$=()=>type=>type;
    window.__vite_plugin_react_preamble_installed__=true;
    await import('/async-components-fixture');</script></body></html>` }))
  await page.route("**/async-components-fixture", (route) => route.fulfill({ contentType: "application/javascript", body: moduleText }))
  await page.goto(new URL("/async-components-check", baseUrl).href)
  await page.waitForFunction(() => !!window.fixture)
  const invoke = (method, ...args) => page.evaluate(([name, values]) => window.fixture[name](...values), [method, args])
  const waitRequest = (kind, count) => page.waitForFunction(([name, value]) => window.fixture.count(name) === value, [kind, count])
  // 等待 Promise 微任务和 React 提交完成，再确认迟到结果未改变可见界面。
  const settle = async (method, ...args) => {
    await invoke(method, ...args)
    await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  }
  const absent = async (text) => assert.equal(await page.getByText(text, { exact: true }).count(), 0)

  await invoke("showDetail", 0)
  await waitRequest("detail", 1)
  await settle("completeDetail", 0, 0)
  await page.getByRole("button", { name: "a.txt", exact: false }).click()
  await waitRequest("file", 1)
  await page.getByRole("button", { name: "b.txt", exact: false }).click()
  await waitRequest("file", 2)
  await settle("completeFile", 1, "最新文件 B")
  await settle("completeFile", 0, "迟到旧文件 A")
  await page.getByText("最新文件 B", { exact: true }).waitFor()
  await absent("迟到旧文件 A")
  await page.getByRole("button", { name: "a.txt", exact: false }).click()
  await waitRequest("file", 3)
  await page.getByRole("button", { name: "c.txt", exact: false }).click()
  await waitRequest("file", 4)
  await settle("completeFile", 3, "最新文件 C")
  await settle("fail", "file", 2, "迟到旧文件错误")
  await page.getByText("最新文件 C", { exact: true }).waitFor()
  assert.equal(await page.getByText(/迟到旧文件错误/).count(), 0)
  await page.getByRole("button", { name: "a.txt", exact: false }).click()
  await waitRequest("file", 5)
  await settle("fail", "file", 4, "当前文件错误")
  await page.getByText(/当前文件错误/).waitFor()
  // 同一组件切换 Skill 本身也必须忽略上一项的迟到错误。
  await invoke("showDetail", 1)
  await waitRequest("detail", 2)
  await invoke("showDetail", 2)
  await waitRequest("detail", 3)
  await settle("completeDetail", 2, 2)
  await settle("fail", "detail", 1, "迟到旧详情错误")
  await page.getByRole("heading", { name: "fixture-skill-2", exact: true }).first().waitFor()
  assert.equal(await page.getByRole("alert").count(), 0)
  console.log("PASS SkillDetail: latest file wins, stale errors ignored, current errors visible")

  await invoke("showEditor")
  await page.getByRole("textbox", { name: "名称", exact: true }).fill("fixture-review")
  await page.getByRole("textbox", { name: "描述", exact: true }).fill("合成保存测试")
  const draft = await page.getByRole("textbox", { name: "SKILL.md 内容", exact: true }).inputValue()
  await page.getByRole("button", { name: "保存 Skill", exact: true }).dblclick()
  await waitRequest("save", 1)
  for (const name of ["名称", "描述", "SKILL.md 内容"]) {
    assert.ok(await page.getByRole("textbox", { name, exact: true }).isDisabled(), `保存期间 ${name} 应禁用`)
  }
  for (const name of ["供应商", "作用范围", "插入模板", "取消", "保存中…"]) {
    assert.ok(await page.getByRole("button", { name, exact: true }).isDisabled(), `保存期间 ${name} 应禁用`)
  }
  assert.ok(await page.getByRole("checkbox", { name: /保存后启用/ }).isDisabled())
  // 用户连点和已禁用按钮都不能发出第二次请求。
  await page.getByRole("button", { name: "保存中…", exact: true }).evaluate((button) => { button.click(); button.click() })
  assert.equal(await invoke("count", "save"), 1)
  await settle("fail", "save", 0, "合成保存失败")
  await page.getByText("合成保存失败", { exact: true }).waitFor()
  assert.equal(await page.getByRole("textbox", { name: "描述", exact: true }).isDisabled(), false)
  assert.ok(await page.getByRole("textbox", { name: "SKILL.md 内容", exact: true }).inputValue() === draft, "失败后应保留原草稿")
  assert.equal(await invoke("savedCount"), 0)
  await page.getByRole("button", { name: "保存 Skill", exact: true }).click()
  await waitRequest("save", 2)
  await settle("completeSave", 1)
  assert.equal(await invoke("savedCount"), 1)
  console.log("PASS SkillEditor: controls frozen, duplicate save blocked, failure retains draft, retry succeeds")

  await invoke("showTask", "a", 1)
  await waitRequest("quota", 1)
  await settle("completeQuota", 0, 11)
  await page.getByText("11% · 可用", { exact: true }).waitFor()
  await invoke("showTask", "b", 1)
  await waitRequest("quota", 2)
  await absent("11% · 可用")
  await page.getByText("正在读取额度…", { exact: true }).waitFor()
  await invoke("showTask", "c", 1)
  await waitRequest("quota", 3)
  await settle("completeQuota", 2, 33)
  await settle("completeQuota", 1, 88)
  await page.getByText("33% · 可用", { exact: true }).waitFor()
  await absent("88% · 可用")
  await invoke("showTask", "d", 1)
  await waitRequest("quota", 4)
  await invoke("showTask", "e", 1)
  await waitRequest("quota", 5)
  await settle("completeQuota", 4, 44)
  await settle("fail", "quota", 3, "迟到旧账号错误")
  await page.getByText("44% · 可用", { exact: true }).waitFor()
  assert.equal(await page.getByText(/迟到旧账号错误/).count(), 0)
  // 同一账号的新检查失败时，旧成功额度也不能继续冒充当前状态。
  await invoke("showTask", "e", 2)
  await waitRequest("quota", 6)
  await settle("fail", "quota", 5, "当前账号查询失败")
  await page.getByText(/当前账号查询失败/).waitFor()
  await absent("44% · 可用")
  await invoke("showTask", "e", 3)
  await waitRequest("quota", 7)
  await settle("completeQuota", 6, 55)
  await page.getByText("55% · 可用", { exact: true }).waitFor()
  assert.equal(await page.getByText(/当前账号查询失败/).count(), 0)
  await invoke("showTask", "f", 1)
  await waitRequest("quota", 8)
  await invoke("showTask", "g", 1)
  await waitRequest("quota", 9)
  await settle("fail", "quota", 8, "最新账号失败")
  await settle("completeQuota", 7, 66)
  await page.getByText(/最新账号失败/).waitFor()
  await absent("66% · 可用")
  assert.equal(errors.length, 0, "不得出现未捕获浏览器异常")
  assert.equal(await page.evaluate(() => window.unexpectedNativeCalls), 0, "不得调用真实桥接")
  console.log("PASS TaskDetail: account switch clears old quota, stale replies ignored, failure and recovery replace state")
} finally {
  await browser.close()
}
