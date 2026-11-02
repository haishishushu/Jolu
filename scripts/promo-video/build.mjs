/**
 * 生成 Jolu 宣传片（1920×1080 · H.264 + AAC），片长与帧率读 promo.html 的 promo-duration / promo-fps。
 *
 * 流程：promo.html 填入素材路径 → 无头 Edge 暂停全部 CSS 动画、逐帧拨到对应时刻截图
 *       → JPEG 帧流经管道送入 ffmpeg → 与配乐合成 MP4，写到 output/promo-video/。
 *
 * 运行：node scripts/promo-video/build.mjs --music <音乐或带音轨的视频> [选项]
 *   --music <文件>     配乐；fit-music.py 会无缝剪短或循环延长到片长，保留原曲开头与自然结尾
 *   --still <秒,秒…>   只导出这些时刻的 PNG 关键帧到 output/promo-video/stills/，不编码视频（不需要 --music）
 *   --out <文件>       成片路径，默认 output/promo-video/jolu-promo.mp4
 *
 * 素材只用浏览器预览截图（演示数据）：docs/assets/readme/ 与 shoot.mjs 补拍的 output/promo-video/shots/。
 * 依赖：playwright-core（与 README 截图脚本一样从 output/acceptance/node 解析）、本机 Microsoft Edge、
 *       Python + numpy/scipy（适配配乐）、ffmpeg（JOLU_FFMPEG → PATH → Python 包 imageio-ffmpeg 自带的可执行文件）。
 */
import { spawn, spawnSync } from "node:child_process"
import fs from "node:fs"
import { createRequire } from "node:module"
import path from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

const here = path.dirname(fileURLToPath(import.meta.url))
const repo = path.resolve(here, "..", "..")
const outDir = path.join(repo, "output", "promo-video")
const WIDTH = 1920
const HEIGHT = 1080

function option(name) {
  const i = process.argv.indexOf(name)
  return i === -1 ? null : process.argv[i + 1]
}
const musicArg = option("--music")
const stillArg = option("--still")
const outFile = path.resolve(option("--out") ?? path.join(outDir, "jolu-promo.mp4"))
fs.mkdirSync(outDir, { recursive: true })

// ---------- 素材 ----------
const file = (...parts) => {
  const full = path.join(repo, ...parts)
  if (!fs.existsSync(full)) throw new Error(`缺少素材：${path.relative(repo, full)}${parts.includes("shots") ? "（先启动预览并运行 shoot.mjs）" : ""}`)
  return pathToFileURL(full).href
}
const readme = (name) => file("docs", "assets", "readme", name)

// 平台标识路径直接取 PlatformLogo.tsx，与应用内一致
const logoSource = fs.readFileSync(path.join(repo, "frontend", "src", "components", "brand", "PlatformLogo.tsx"), "utf8")
const logoPath = (name) => {
  const hit = logoSource.match(new RegExp(`const ${name}\\s*=\\s*"([^"]+)"`))
  if (!hit) throw new Error(`PlatformLogo.tsx 里找不到 ${name}`)
  return hit[1]
}

const shot = (name) => file("output", "promo-video", "shots", `${name}.png`)

// 中文标题用思源黑体（Noto Sans SC 可变字体，含 900 字重）；找不到时 CSS 回退到微软雅黑
const cjkFont = [
  process.env.JOLU_CJK_FONT,
  path.join(process.env.WINDIR ?? "C:\\Windows", "Fonts", "NotoSansSC-VF.ttf"),
  path.join(process.env.LOCALAPPDATA ?? "", "Microsoft", "Windows", "Fonts", "NotoSansSC-VF.ttf"),
].find((f) => f && fs.existsSync(f))
if (!cjkFont) console.warn("未找到 NotoSansSC-VF.ttf，中文标题回退到微软雅黑（可用 JOLU_CJK_FONT 指定）")

const tokens = {
  interFont: file("frontend", "public", "fonts", "Inter-Variable.woff2"),
  monoFont: file("frontend", "public", "fonts", "JetBrainsMono-Variable.woff2"),
  cjkFont: cjkFont ? pathToFileURL(cjkFont).href : "",
  icon: file("frontend", "public", "app-icon.png"),
  islandCollapsedDark: readme("island-collapsed-dark.png"),
  islandExpandedDark: readme("island-expanded-dark.png"),
  automationDark: shot("automation-wait-dark"),
  checkinDark: shot("panel-checkin-dark"),
  usageDark: shot("panel-usage-dark"),
  sessionsDark: shot("panel-sessions-dark"),
  skillsDark: shot("panel-skills-dark"),
  settingsDark: shot("panel-settings-dark"),
  qoder: file("frontend", "src", "assets", "platforms", "qoder.svg"),
  workbuddy: file("frontend", "src", "assets", "platforms", "workbuddy.svg"),
  zcode: file("frontend", "src", "assets", "platforms", "zcode.png"),
  claudeD: logoPath("CLAUDE_D"),
  codexD: logoPath("CODEX_D"),
  geminiD: logoPath("GEMINI_D"),
  traeD: logoPath("TRAE_D"),
  grokD1: logoPath("GROK_D1"),
  grokD2: logoPath("GROK_D2"),
}

let html = fs.readFileSync(path.join(here, "promo.html"), "utf8")
for (const [key, value] of Object.entries(tokens)) html = html.replaceAll(`{{${key}}}`, value)
const leftover = html.match(/\{\{\w+\}\}/)
if (leftover) throw new Error(`promo.html 里有未替换的占位符：${leftover[0]}`)
const meta = (name) => Number(html.match(new RegExp(`name="${name}" content="([\\d.]+)"`))?.[1])
const DURATION = meta("promo-duration")
const FPS = meta("promo-fps")
if (!DURATION || !FPS) throw new Error("promo.html 缺少 promo-duration / promo-fps")
const pagePath = path.join(outDir, "promo.build.html")
fs.writeFileSync(pagePath, html, "utf8")

// ---------- 工具 ----------
function findFfmpeg() {
  if (process.env.JOLU_FFMPEG) return process.env.JOLU_FFMPEG
  if (spawnSync("ffmpeg", ["-version"], { stdio: "ignore" }).status === 0) return "ffmpeg"
  const py = spawnSync("python", ["-c", "import imageio_ffmpeg; print(imageio_ffmpeg.get_ffmpeg_exe())"], { encoding: "utf8" })
  if (py.status === 0 && py.stdout.trim()) return py.stdout.trim()
  throw new Error("找不到 ffmpeg：设置 JOLU_FFMPEG，或安装 ffmpeg / pip install imageio-ffmpeg")
}

function loadChromium() {
  const require = createRequire(import.meta.url)
  const resolved = require.resolve("playwright-core", { paths: [path.join(repo, "output", "acceptance", "node"), repo] })
  return require(resolved).chromium
}

/** 配乐经 fit-music.py 无缝剪短或循环延长到片长（保留原曲开头与自然结尾） */
function music() {
  if (!musicArg) throw new Error("请用 --music 指定配乐（音乐文件或带音轨的视频）")
  if (!fs.existsSync(musicArg)) throw new Error(`找不到音乐文件：${musicArg}`)
  const wav = path.join(outDir, "music-fitted.wav")
  const run = spawnSync("python", [path.join(here, "fit-music.py"), path.resolve(musicArg), wav, String(DURATION)], {
    stdio: "inherit",
    env: { ...process.env, PYTHONIOENCODING: "utf-8" },
  })
  if (run.status !== 0) throw new Error("配乐适配失败（需要 Python + numpy + scipy + ffmpeg）")
  return wav
}

// ---------- 渲染 ----------
const chromium = loadChromium()
const browser = await chromium.launch({ channel: "msedge", headless: true, args: ["--allow-file-access-from-files"] })
const page = await browser.newPage({ viewport: { width: WIDTH, height: HEIGHT }, deviceScaleFactor: 1 })
await page.goto(pathToFileURL(pagePath).href)
await page.evaluate(async () => {
  await document.fonts.ready
  await Promise.all([...document.images].map((img) => img.decode().catch(() => {})))
})
const broken = await page.evaluate(() => [...document.images].filter((img) => !img.naturalWidth).map((img) => img.src))
if (broken.length) throw new Error(`图片加载失败：\n${broken.join("\n")}`)

async function frameAt(t, type = "jpeg") {
  await page.evaluate((s) => new Promise((done) => { window.__seek(s); requestAnimationFrame(() => done()) }), t)
  return page.screenshot(type === "jpeg" ? { type, quality: 92 } : { type })
}

try {
  if (stillArg) {
    const dir = path.join(outDir, "stills")
    fs.mkdirSync(dir, { recursive: true })
    for (const t of stillArg.split(",").map(Number)) {
      const target = path.join(dir, `t${String(t).padStart(5, "0")}.png`)
      fs.writeFileSync(target, await frameAt(t, "png"))
      console.log("关键帧", path.relative(repo, target))
    }
  } else {
    const audio = music()
    const ffmpeg = findFfmpeg()
    const args = [
      "-y", "-loglevel", "error",
      "-f", "image2pipe", "-framerate", String(FPS), "-c:v", "mjpeg", "-i", "-",
      "-i", audio,
      "-map", "0:v", "-map", "1:a", "-t", String(DURATION),
      // 截图是全范围 JPEG，转成 BT.709 有限范围，避免播放器偏色
      "-vf", "scale=in_range=pc:out_range=tv:out_color_matrix=bt709,format=yuv420p",
      "-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv",
      "-c:v", "libx264", "-preset", "slow", "-crf", "18",
      "-c:a", "aac", "-b:a", "192k",
      "-movflags", "+faststart", outFile,
    ]
    const encoder = spawn(ffmpeg, args, { stdio: ["pipe", "inherit", "inherit"] })
    const finished = new Promise((resolve, reject) => {
      encoder.on("error", reject)
      encoder.on("close", (code) => (code === 0 ? resolve() : reject(new Error(`ffmpeg 退出码 ${code}`))))
    })
    const total = Math.round(FPS * DURATION)
    const started = Date.now()
    for (let i = 0; i < total; i++) {
      const jpeg = await frameAt(i / FPS)
      if (!encoder.stdin.write(jpeg)) await new Promise((r) => encoder.stdin.once("drain", r))
      if (i % (FPS * 5) === 0) console.log(`渲染 ${(i / FPS).toFixed(0)}s / ${DURATION}s（${((Date.now() - started) / 1000).toFixed(0)} 秒）`)
    }
    encoder.stdin.end()
    await finished
    console.log(`成片：${path.relative(repo, outFile)}（${DURATION} 秒 · ${FPS}fps）`)
  }
} finally {
  await browser.close()
}
