/**
 * 从 Jolu 人物母版生成 Windows 应用、任务栏及托盘图标（Jolu.pen 画板 107 / 108）。
 * 运行 node scripts/icons/generate.mjs；构建直接使用已生成的产物。
 *
 * 清晰度要点：
 * - 每个尺寸都从母版一次性 Lanczos 缩放并轻微锐化（resample.mjs），不在小图之间二次缩放。
 * - ICO 第一张放 256px：锁定版本的 tauri-codegen 取 ICO 第一张作默认窗口图标，
 *   放 16px 会被任务栏放大成一团。
 * - ICO 额外带 30 / 36px，对应 125% / 150% 缩放下的任务栏尺寸。
 * - 任务栏与托盘另存按系统缩放挑选的 RGBA 底图，运行时原尺寸交给系统，不再由系统缩放。
 */
import fs from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { encodePNG, encodeICO } from "./encode.mjs"
import { decodePNG, resize } from "./resample.mjs"

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..")
const iconsDir = path.join(root, "backend/icons")
const source = path.join(root, "docs/logo/jolu-desktop-icon.png")
const ICO_SIZES = [256, 16, 20, 24, 30, 32, 36, 40, 48, 64, 128]
/** 托盘：16 × 缩放（SM_CXSMICON） */
const TRAY_SIZES = [16, 20, 24, 28, 32, 36, 40]
/** 任务栏：24 × 缩放（100% / 125% / 150% / 175% / 200%） */
const TASKBAR_SIZES = [24, 30, 36, 42, 48]
const PNG_FILES = [["32x32.png", 32], ["128x128.png", 128], ["128x128@2x.png", 256], ["icon.png", 512]]

const master = decodePNG(fs.readFileSync(source))
const cache = new Map()
const rgba = (size) => {
  if (!cache.has(size)) cache.set(size, resize(master, size))
  return cache.get(size)
}

fs.mkdirSync(iconsDir, { recursive: true })
for (const [name, size] of PNG_FILES) fs.writeFileSync(path.join(iconsDir, name), encodePNG(size, rgba(size)))
const ico = encodeICO(ICO_SIZES.map((size) => ({ size, png: encodePNG(size, rgba(size)) })))
fs.writeFileSync(path.join(iconsDir, "icon.ico"), ico)
fs.writeFileSync(path.join(root, "docs/logo/jolu-desktop-icon.ico"), ico)
for (const size of TRAY_SIZES) fs.writeFileSync(path.join(iconsDir, `tray-base-${size}.rgba`), rgba(size))
for (const size of TASKBAR_SIZES) fs.writeFileSync(path.join(iconsDir, `taskbar-${size}.rgba`), rgba(size))
fs.writeFileSync(path.join(root, "frontend/public/favicon.png"), encodePNG(64, rgba(64)))
fs.writeFileSync(path.join(root, "frontend/public/app-icon.png"), encodePNG(256, rgba(256)))
console.log(`Jolu 人物图标已生成：应用 PNG、${ICO_SIZES.length} 档 ICO、${TRAY_SIZES.length} 档托盘底图、${TASKBAR_SIZES.length} 档任务栏底图、前端图标。`)
