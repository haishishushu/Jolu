/**
 * 生成 NSIS 安装向导的两张位图。
 *
 * 流程：HTML/CSS 源 → 本机 Chromium 无头 2 倍截图 → PNG → 24 位 BMP。
 * 用 HTML 画是为了能直接复用前端的设计变量、Inter 字体和圆角阴影；
 * 位图是产物但一并入库，因为 `tauri build` 不应依赖本机有没有 Chromium。
 *
 * 运行：node scripts/installer-art/build.mjs
 * 指定浏览器：设 JOLU_CHROME 环境变量为 chrome.exe 路径。
 */
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { findChrome, screenshot } from "./find-chrome.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, "..", "..");
const outDir = path.join(repoRoot, "backend", "windows", "installer");
const fontDir = path.join(repoRoot, "frontend", "public", "fonts");

const tauriConf = JSON.parse(
  fs.readFileSync(path.join(repoRoot, "backend", "tauri.conf.json"), "utf8"),
);

/** 尺寸是 Tauri NSIS 的硬性要求，不是随便定的：见 NsisConfig 的 schema 说明。 */
const TARGETS = [
  { source: "sidebar.html", output: "sidebar.bmp", width: 164, height: 314 },
  { source: "header.html", output: "header.bmp", width: 150, height: 57 },
];

/** 2 倍渲染再降采样，1 倍直出的小字和圆角会有锯齿。 */
const SCALE = 2;

function fontDataUrl(file) {
  const base64 = fs.readFileSync(path.join(fontDir, file)).toString("base64");
  return `data:font/woff2;base64,${base64}`;
}

function render(chrome, target, tmpDir, tokens) {
  const html = fs
    .readFileSync(path.join(here, target.source), "utf8")
    .replaceAll("{{productName}}", tokens.productName)
    .replaceAll("{{version}}", tokens.version)
    .replaceAll("{{interFontUrl}}", tokens.interFontUrl)
    .replaceAll("{{monoFontUrl}}", tokens.monoFontUrl)
    .replaceAll("{{iconUrl}}", tokens.iconUrl);

  const pagePath = path.join(tmpDir, target.source);
  fs.writeFileSync(pagePath, html, "utf8");

  const pngPath = path.join(tmpDir, `${path.parse(target.output).name}.png`);
  screenshot(chrome, {
    pageUrl: pathToFileURL(pagePath).href,
    pngPath,
    width: target.width,
    height: target.height,
    scale: SCALE,
    profileDir: path.join(tmpDir, "profile"),
  });

  const bmpPath = path.join(outDir, target.output);
  const output = execFileSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-NonInteractive",
      "-ExecutionPolicy", "Bypass",
      "-File", path.join(here, "png-to-bmp.ps1"),
      "-Source", pngPath,
      "-Target", bmpPath,
      "-Width", String(target.width),
      "-Height", String(target.height),
    ],
    { encoding: "utf8" },
  );
  if (!output.includes("ok")) throw new Error(`转 BMP 失败：${target.output}\n${output}`);

  return bmpPath;
}

/** 读 BMP 头自查，尺寸或位深不对宁可构建失败，也别等装到一半才发现是黑块。 */
function assertBmp(file, width, height) {
  const head = Buffer.alloc(30);
  const fd = fs.openSync(file, "r");
  try {
    fs.readSync(fd, head, 0, 30, 0);
  } finally {
    fs.closeSync(fd);
  }
  const actual = {
    signature: head.toString("latin1", 0, 2),
    width: head.readInt32LE(18),
    height: head.readInt32LE(22),
    bpp: head.readUInt16LE(28),
  };
  if (
    actual.signature !== "BM" ||
    actual.width !== width ||
    Math.abs(actual.height) !== height ||
    actual.bpp !== 24
  ) {
    throw new Error(
      `${path.basename(file)} 校验不通过：期望 ${width}x${height} 24bpp BM，` +
        `实际 ${actual.width}x${actual.height} ${actual.bpp}bpp ${actual.signature}`,
    );
  }
  return actual;
}

const chrome = findChrome();
const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "jolu-installer-art-"));
fs.mkdirSync(outDir, { recursive: true });

console.log(`Chromium: ${chrome}`);
const tokens = {
  productName: tauriConf.productName,
  version: tauriConf.version,
  interFontUrl: fontDataUrl("Inter-Variable.woff2"),
  monoFontUrl: fontDataUrl("JetBrainsMono-Variable.woff2"),
  // Jolu 人物图标（与桌面 / 任务栏图标同一母版，见 Jolu.pen 画板 107）
  iconUrl: `data:image/png;base64,${fs.readFileSync(path.join(repoRoot, "frontend", "public", "app-icon.png")).toString("base64")}`,
};

try {
  for (const target of TARGETS) {
    const bmpPath = render(chrome, target, tmpDir, tokens);
    const info = assertBmp(bmpPath, target.width, target.height);
    const size = fs.statSync(bmpPath).size;
    console.log(
      `${target.output}  ${info.width}x${Math.abs(info.height)}  ${info.bpp}bpp  ${size} bytes`,
    );
  }
} finally {
  fs.rmSync(tmpDir, { recursive: true, force: true });
}

console.log(`\n产物目录：${outDir}`);
