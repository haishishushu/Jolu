/**
 * 生成 GitHub 仓库社交预览图（Settings → Social preview）。
 *
 * 流程：HTML/CSS 源 → 本机 Chromium 无头截图 → 1280×640 PNG。
 * 素材直接引用 README 截图与品牌图，改文案或重截 README 图后重跑即可；
 * 产物入库，上传到 GitHub 仍需仓库所有者在设置页手动完成。
 *
 * 运行：pnpm social-preview（或 node scripts/social-preview/build.mjs）
 * 指定浏览器：设 JOLU_CHROME 环境变量为 chrome.exe 路径。
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { findChrome, screenshot } from "../installer-art/find-chrome.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, "..", "..");
const output = path.join(repoRoot, "docs", "assets", "social-preview-1280x640.png");

/** GitHub 推荐尺寸；1 倍直出即为最终像素，素材本身都是 2 倍以上的大图 */
const WIDTH = 1280;
const HEIGHT = 640;

const fileUrl = (...parts) => pathToFileURL(path.join(repoRoot, ...parts)).href;
const fontData = fs.readFileSync(path.join(repoRoot, "frontend", "public", "fonts", "Inter-Variable.woff2")).toString("base64");

const tokens = {
  interFontUrl: `data:font/woff2;base64,${fontData}`,
  iconUrl: fileUrl("frontend", "public", "app-icon.png"),
  shotUrl: fileUrl("docs", "assets", "readme", "panel-usage.png"),
  islandUrl: fileUrl("docs", "assets", "readme", "island-expanded.png"),
  characterUrl: fileUrl("docs", "logo", "jolu-character-white-tights.png"),
};

/** 读 PNG 的 IHDR 自查尺寸，不对就让构建失败 */
function assertPng(file) {
  const head = fs.readFileSync(file).subarray(0, 24);
  const width = head.readUInt32BE(16);
  const height = head.readUInt32BE(20);
  if (head.toString("latin1", 1, 4) !== "PNG" || width !== WIDTH || height !== HEIGHT) {
    throw new Error(`社交预览图校验不通过：期望 ${WIDTH}x${HEIGHT} PNG，实际 ${width}x${height}`);
  }
}

const chrome = findChrome();
const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "jolu-social-preview-"));
console.log(`Chromium: ${chrome}`);

try {
  let html = fs.readFileSync(path.join(here, "social-preview.html"), "utf8");
  for (const [key, value] of Object.entries(tokens)) html = html.replaceAll(`{{${key}}}`, value);
  const pagePath = path.join(tmpDir, "social-preview.html");
  fs.writeFileSync(pagePath, html, "utf8");

  const pngPath = path.join(tmpDir, "social-preview.png");
  screenshot(chrome, {
    pageUrl: pathToFileURL(pagePath).href,
    pngPath,
    width: WIDTH,
    height: HEIGHT,
    scale: 1,
    profileDir: path.join(tmpDir, "profile"),
  });
  assertPng(pngPath);
  fs.copyFileSync(pngPath, output);
  console.log(`${path.relative(repoRoot, output)}  ${WIDTH}x${HEIGHT}  ${fs.statSync(output).size} bytes`);
} finally {
  fs.rmSync(tmpDir, { recursive: true, force: true });
}
