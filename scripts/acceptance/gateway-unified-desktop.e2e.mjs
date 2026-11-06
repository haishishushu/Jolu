// 统一网关真实桌面验收：隔离 Jolu 实例 + 真实 Claude Code / Codex / OpenCode / Gemini CLI + 合成模型上游。
//
// 前置（见 scripts/acceptance/README.md「统一网关真实桌面验收」）：
//   1. 按 5193 方式编译并用 start.ps1 -Executable 启动隔离实例（CDP 9337）；
//   2. JOLU_UPSTREAM_DELAY=4 python -I scripts/acceptance/model-upstream.py 18766；
//   3. 在 output/acceptance/node 里运行：node <仓库>/scripts/acceptance/gateway-unified-desktop.e2e.mjs
//
// 只在带标记的隔离目录运行；每个 CLI 用 output/acceptance/cli-home/<工具> 作为独立主目录，
// 不读写真实的 ~/.claude、~/.codex、~/.gemini 或 OpenCode 数据；所有模型请求只发到本机合成上游。
import assert from "node:assert/strict"
import { spawn, execFileSync } from "node:child_process"
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { connect, DEV_URL, invoke, isolatedIsland, log, sleep, waitFor } from "./cdp.mjs"

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const OUT = join(REPO, "output/acceptance/gateway-unified-desktop")
const HOMES = join(REPO, "output/acceptance/cli-home")
const UPSTREAM = process.env.JOLU_UPSTREAM ?? "http://127.0.0.1:18766"
const UPSTREAM_LOG = join(REPO, "output/acceptance/upstream-requests.jsonl")
mkdirSync(OUT, { recursive: true })

/** 递归找文件，返回路径列表 */
function walk(dir, out = []) {
  if (!existsSync(dir)) return out
  for (const name of readdirSync(dir)) {
    const path = join(dir, name)
    if (statSync(path).isDirectory()) walk(path, out)
    else out.push(path)
  }
  return out
}

function clipboard() {
  return execFileSync("powershell", ["-NoProfile", "-Command", "Get-Clipboard -Raw"], { encoding: "utf8" }).trim()
}
/** 恢复剪贴板文本；原来为空（或不是文本）时清空，不留下网关 Key */
function setClipboard(text) {
  if (text) execFileSync("powershell", ["-NoProfile", "-Command", "Set-Clipboard -Value ([Console]::In.ReadToEnd())"], { input: text })
  else execFileSync("powershell", ["-NoProfile", "-STA", "-Command", "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Clipboard]::Clear()"])
}

const { browser, pages } = await connect()
const report = { checks: [], sessions: [], notes: [] }
const check = (name, ok, detail = "") => { report.checks.push({ name, ok, detail }); log(ok ? "通过" : "失败", name, detail) }
try {
  const { island, database } = await isolatedIsland(pages)
  log("隔离数据库：", database)

  /* ── 准备：两个指向合成上游的自定义来源、加入模型池、开启转发 ── */
  const existing = await invoke(island, "list_connections")
  const ensure = async (name, protocol, slug) => {
    let row = existing.find((c) => c.name === name || c.baseName === name)
    if (!row) {
      await invoke(island, "add_connection", { input: { platform: "custom", kind: "api", name, secret: "acceptance-upstream-key", base_url: UPSTREAM, protocol, model: "test-model" } })
      row = (await invoke(island, "list_connections")).find((c) => c.name === name || c.baseName === name)
    }
    await invoke(island, "gateway_set_serve", { connectionId: row.id, enabled: true })
    if (row.slug !== slug) await invoke(island, "gateway_set_slug", { connectionId: row.id, slug })
    return row.id
  }
  const anthropicId = await ensure("验收 · Anthropic 合成上游", "anthropic", "acc-anthropic")
  const openaiId = await ensure("验收 · OpenAI 合成上游", "openai", "acc-openai")
  // 已在转发时不重复开启（重复开启会尝试再次绑定同一端口）
  const before = await invoke(island, "gateway_status")
  if (!(before.mode === "relay" && before.enabled)) await invoke(island, "gateway_set_mode", { mode: "relay", defaultConnection: null })
  const status = await waitFor("网关监听", async () => {
    const s = await invoke(island, "gateway_status")
    return s.mode === "relay" && s.enabled ? s : null
  }, 30000)
  const base = `http://127.0.0.1:${status.port}`
  log("网关地址：", base)

  // 手动接入 Key：后端写剪贴板，读完立即恢复原剪贴板内容
  let previous = ""
  try { previous = clipboard() } catch {}
  await invoke(island, "gateway_copy_key")
  const key = clipboard()
  setClipboard(previous)
  assert.ok(key.length > 16, "应读到手动接入 Key")

  /* ── 打开主窗口的统一网关页 ── */
  await invoke(island, "menu_action", { id: "open_main" })
  const main = await waitFor("主窗口", async () => pages().find((p) => p.url().startsWith(DEV_URL) && !p.url().includes("window=")), 60000)
  await main.setViewportSize({ width: 1600, height: 1800 }).catch(() => {})
  await main.reload()  // 每次从干净的页面状态开始（滚动位置、展开状态）
  await main.waitForSelector('nav[aria-label="主导航"]', { timeout: 60000 })
  await main.locator('nav[aria-label="主导航"]').getByRole("button", { name: "统一网关", exact: true }).click()
  await main.getByRole("heading", { name: "统一网关" }).last().waitFor()
  await main.screenshot({ path: join(OUT, "01-before.png"), fullPage: true })

  /* ── 真实 CLI：各自独立主目录，同时发请求（上游延迟 4 秒，便于观察运行中） ── */
  rmSync(UPSTREAM_LOG, { force: true })
  // 隔离主目录跨次复用（保留 OpenCode 首次下载的 provider 缓存）；配置文件每次重写
  const home = (name) => { const dir = join(HOMES, name); mkdirSync(dir, { recursive: true }); return dir }
  // 去掉本机 shell 里各工具自己的配置变量（如 ANTHROPIC_AUTH_TOKEN），确保只用下面给的隔离配置
  const toolVar = /^(ANTHROPIC_|CLAUDE_|OPENAI_|CODEX_|GEMINI_|GOOGLE_|OPENCODE_|AZURE_OPENAI)/i
  const baseEnv = { ...Object.fromEntries(Object.entries(process.env).filter(([k]) => !toolVar.test(k))), NO_COLOR: "1", CI: "1" }
  const isolatedEnv = (dir, extra) => ({ ...baseEnv, USERPROFILE: dir, HOME: dir, APPDATA: join(dir, "AppData/Roaming"), LOCALAPPDATA: join(dir, "AppData/Local"), XDG_CONFIG_HOME: join(dir, ".config"), XDG_DATA_HOME: join(dir, ".local/share"), XDG_CACHE_HOME: join(dir, ".cache"), XDG_STATE_HOME: join(dir, ".local/state"), ...extra })

  const claudeHome = home("claude")
  const codexHome = home("codex")
  mkdirSync(join(codexHome, ".codex"), { recursive: true })
  writeFileSync(join(codexHome, ".codex/config.toml"), [
    'model = "acc-openai/test-model"', 'model_provider = "jolu"', 'approval_policy = "never"', 'sandbox_mode = "read-only"', "",
    "[model_providers.jolu]", 'name = "Jolu acceptance"', `base_url = "${base}/v1"`, 'env_key = "JOLU_GATEWAY_KEY"', 'wire_api = "responses"', "",
  ].join("\n"))
  const opencodeHome = home("opencode")
  const opencodeConfig = join(opencodeHome, "opencode.json")
  writeFileSync(opencodeConfig, JSON.stringify({
    $schema: "https://opencode.ai/config.json",
    autoupdate: false,
    share: "disabled",
    provider: { jolu: { npm: "@ai-sdk/openai-compatible", name: "Jolu", options: { baseURL: `${base}/v1`, apiKey: key }, models: { "acc-openai/test-model": { name: "test-model" } } } },
    model: "jolu/acc-openai/test-model",
    small_model: "jolu/acc-openai/test-model",
  }, null, 2))
  const geminiHome = home("gemini")
  mkdirSync(join(geminiHome, ".gemini"), { recursive: true })
  writeFileSync(join(geminiHome, ".gemini/settings.json"), JSON.stringify({ security: { auth: { selectedType: "gemini-api-key" } }, general: { disableAutoUpdate: true }, privacy: { usageStatisticsEnabled: false } }))

  const runs = {
    claude: ["claude", ["-p", "say ok", "--model", "acc-anthropic/test-model"], isolatedEnv(claudeHome, {
      ANTHROPIC_BASE_URL: base, ANTHROPIC_API_KEY: key, ANTHROPIC_SMALL_FAST_MODEL: "acc-anthropic/test-model", ANTHROPIC_DEFAULT_HAIKU_MODEL: "acc-anthropic/test-model",
      CLAUDE_CONFIG_DIR: join(claudeHome, ".claude"), CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1", DISABLE_AUTOUPDATER: "1",
    })],
    codex: ["codex", ["exec", "--skip-git-repo-check", "say ok"], isolatedEnv(codexHome, { CODEX_HOME: join(codexHome, ".codex"), JOLU_GATEWAY_KEY: key })],
    opencode: ["opencode", ["run", "say ok", "-m", "jolu/acc-openai/test-model"], isolatedEnv(opencodeHome, { OPENCODE_CONFIG: opencodeConfig, OPENCODE_DISABLE_AUTOUPDATE: "1" })],
    gemini: ["gemini", ["-p", "say ok", "-m", "acc-openai/test-model"], isolatedEnv(geminiHome, { GEMINI_API_KEY: key, GOOGLE_GEMINI_BASE_URL: base, GEMINI_CLI_TRUST_WORKSPACE: "true" })],
  }
  const results = {}
  const children = Object.entries(runs).map(([name, [cmd, args, env]]) => new Promise((done) => {
    // Windows 上 npm 全局命令是 .cmd，需要经 shell 启动；参数自行加引号，避免「say ok」被拆成两个参数
    const quoted = args.map((a) => (/[\s"]/.test(a) ? `"${a.replaceAll('"', '\\"')}"` : a))
    const child = spawn([cmd, ...quoted].join(" "), { env, cwd: REPO, shell: true, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] })
    let output = ""
    child.stdout.on("data", (d) => { output += d })
    child.stderr.on("data", (d) => { output += d })
    const timer = setTimeout(() => child.kill(), 240000)
    child.on("close", (code) => { clearTimeout(timer); results[name] = { code, output: output.slice(-1200) }; done() })
  }))

  // 请求进行中：页面应出现运行中会话与流光
  const running = await waitFor("界面出现运行中会话", async () => (await main.getByText("运行中", { exact: true }).count()) > 0, 240000).catch(() => false)
  await main.screenshot({ path: join(OUT, "02-running.png"), fullPage: true })
  const runningOverview = await invoke(main, "gateway_sessions_overview")
  check("请求进行中界面显示运行中", !!running, `在途请求 ${runningOverview.in_flight.length} 个`)
  await Promise.all(children)
  for (const [name, r] of Object.entries(results)) report.notes.push({ tool: name, exit: r.code, output: r.output })
  for (const [name, r] of Object.entries(results)) check(`${name} 经网关拿到回复并正常退出`, r.code === 0 && /\bOK\b/.test(r.output), `退出码 ${r.code}`)

  await sleep(2500)
  const overview = await invoke(main, "gateway_sessions_overview")
  await main.screenshot({ path: join(OUT, "03-after.png"), fullPage: true })
  check("请求全部结束后没有残留在途请求", overview.in_flight.length === 0, `在途 ${overview.in_flight.length}`)

  /* ── 会话身份：网关识别的会话 ID 必须能在工具自己的会话文件里找到 ── */
  const ids = (platform) => overview.sessions.filter((s) => s.platform === platform).map((s) => s.session_id)
  const linkedTo = (platform, connectionId) => overview.sessions.filter((s) => s.platform === platform).every((s) => s.links.some((l) => l.connection_id === connectionId))
  const claudeFiles = walk(join(claudeHome, ".claude/projects")).filter((p) => p.endsWith(".jsonl")).map((p) => p.split(/[\\/]/).pop().replace(/\.jsonl$/, ""))
  const claudeIds = ids("claude")
  check("Claude Code 会话 ID 与 ~/.claude/projects 下的会话文件一致", claudeIds.length > 0 && claudeIds.every((id) => claudeFiles.includes(id)), `网关 ${claudeIds.join(",")} · 文件 ${claudeFiles.join(",")}`)
  check("Claude Code 会话连到 Anthropic 合成来源", claudeIds.length > 0 && linkedTo("claude", anthropicId))
  const codexFiles = walk(join(codexHome, ".codex/sessions")).filter((p) => /rollout-.*\.jsonl$/.test(p)).map((p) => p.split(/[\\/]/).pop())
  const codexIds = ids("codex")
  check("Codex 会话 ID 与 rollout 文件名一致", codexIds.length > 0 && codexIds.every((id) => codexFiles.some((f) => f.includes(id))), `网关 ${codexIds.join(",")} · 文件 ${codexFiles.join(",")}`)
  check("Codex 会话连到 OpenAI 合成来源", codexIds.length > 0 && linkedTo("codex", openaiId))
  const opencodeIds = ids("opencode")
  const opencodeText = walk(opencodeHome).filter((p) => statSync(p).size < 64 * 1024 * 1024).map((p) => { try { return readFileSync(p).toString("latin1") } catch { return "" } }).join("\n")
  check("OpenCode 会话 ID 能在 OpenCode 本地数据里找到", opencodeIds.length > 0 && opencodeIds.every((id) => opencodeText.includes(id.replace(/^opencode:/, ""))), `网关 ${opencodeIds.join(",")}`)
  const log2 = existsSync(UPSTREAM_LOG) ? readFileSync(UPSTREAM_LOG, "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l)) : []
  const page = await invoke(main, "gateway_route_log_page", { page: 1, pageSize: 50, upperId: null })
  const geminiRows = page.rows.filter((r) => r.agent === "gemini")
  check("Gemini 请求写入路由流水并识别为 Gemini", geminiRows.length > 0, `${geminiRows.length} 条`)
  check("Gemini 请求不带会话标识，不生成会话（不猜）", ids("gemini").length === 0 && geminiRows.every((r) => r.session_id === null))
  report.sessions = overview.sessions.map((s) => ({ platform: s.platform, session_id: s.session_id, agent: s.agent, links: s.links.map((l) => l.connection_id), tokens: s.tokens.state }))
  report.upstream = log2.map((r) => ({ path: r.path, ua: r.headers["user-agent"] ?? null, session_headers: Object.fromEntries(Object.entries(r.headers).filter(([k]) => /session|thread|opencode/.test(k))) }))

  // 界面：最近会话里出现上面三种工具的会话卡；流水分页有数据
  const graphText = await main.locator("section[aria-label='模型来源与最近会话']").innerText()
  check("最近会话显示 Claude Code / Codex / OpenCode 会话卡", ["Claude Code", "Codex", "OpenCode"].every((t) => graphText.includes(t)))
  const logText = await main.locator("section[aria-label='路由流水']").innerText()
  check("路由流水显示后端分页统计", /第 1–\d+ 条 · 共 \d+ 条/.test(logText))
} finally {
  writeFileSync(join(OUT, "report.json"), JSON.stringify(report, null, 2))
  await browser.close().catch(() => {})
}
const failed = report.checks.filter((c) => !c.ok)
console.log(failed.length ? `统一网关桌面验收：${failed.length} 项失败` : "统一网关桌面验收全部通过", "报告：", join(OUT, "report.json"))
process.exit(failed.length ? 1 : 0)
