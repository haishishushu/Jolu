import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

/**
 * 找 Chromium。优先 playwright 装好的 headless shell，其次完整 chrome，
 * 最后退回系统 Chrome / Edge。版本目录按数字倒序取最新。
 * 安装向导位图（installer-art）与社交预览图（social-preview）共用。
 */
export function findChrome() {
  if (process.env.JOLU_CHROME) return process.env.JOLU_CHROME;

  const pwRoot = path.join(
    process.env.LOCALAPPDATA ?? path.join(os.homedir(), "AppData", "Local"),
    "ms-playwright",
  );
  if (fs.existsSync(pwRoot)) {
    const version = (name) => Number(name.split("-").pop()) || 0;
    const dirs = fs
      .readdirSync(pwRoot)
      .filter((name) => name.startsWith("chromium"))
      .sort((a, b) => version(b) - version(a));
    // headless shell 必须排在完整版 chrome 前面：完整版加 --headless=new 截图会挂住不退出
    const byPreference = [
      ...dirs.map((dir) =>
        path.join(pwRoot, dir, "chrome-headless-shell-win64", "chrome-headless-shell.exe"),
      ),
      ...dirs.flatMap((dir) => [
        path.join(pwRoot, dir, "chrome-win64", "chrome.exe"),
        path.join(pwRoot, dir, "chrome-win", "chrome.exe"),
      ]),
    ];
    const hit = byPreference.find((file) => fs.existsSync(file));
    if (hit) return hit;
  }

  const system = [
    String.raw`C:\Program Files\Google\Chrome\Application\chrome.exe`,
    String.raw`C:\Program Files (x86)\Google\Chrome\Application\chrome.exe`,
    String.raw`C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe`,
    String.raw`C:\Program Files\Microsoft\Edge\Application\msedge.exe`,
  ].find((file) => fs.existsSync(file));
  if (system) return system;

  throw new Error(
    "找不到 Chromium。先跑一次 playwright 安装浏览器，或设置 JOLU_CHROME 指向 chrome.exe。",
  );
}

/** 用无头 Chromium 把本地 HTML 截成 PNG；挂死比失败更难查，给硬超时 */
export function screenshot(chrome, { pageUrl, pngPath, width, height, scale, profileDir }) {
  const args = [
    "--disable-gpu",
    "--hide-scrollbars",
    "--allow-file-access-from-files",
    "--no-first-run",
    "--no-default-browser-check",
    `--user-data-dir=${profileDir}`,
    `--force-device-scale-factor=${scale}`,
    `--window-size=${width},${height}`,
    // 让内联字体和图片加载完再截，否则会截到 fallback 字体
    "--virtual-time-budget=4000",
    `--screenshot=${pngPath}`,
    pageUrl,
  ];
  // chrome-headless-shell 本身就是无头的，再传 --headless 反而会被当成 URL 之外的噪声
  if (!path.basename(chrome).includes("headless-shell")) args.unshift("--headless=new");
  execFileSync(chrome, args, { stdio: "pipe", timeout: 60_000 });
  if (!fs.existsSync(pngPath)) throw new Error(`截图失败：${pageUrl}`);
}
