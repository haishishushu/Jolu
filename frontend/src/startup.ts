import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"

// 主题由后端经 ?theme= 传入（light / dark / system），设好后再显示，避免深色主题闪浅色
const theme = new URLSearchParams(location.search).get("theme")
document.documentElement.dataset.theme = theme === "light" || theme === "dark" ? theme : "system"
requestAnimationFrame(() => document.documentElement.classList.add("ready"))

const status = document.getElementById("status")!
const failure = document.getElementById("failure")!
const detail = document.getElementById("detail")!
let timeout: number | undefined

const showError = (message: string) => {
  window.clearTimeout(timeout)
  status.textContent = "Jolu 未能启动"
  detail.textContent = message || "请重试；若问题持续，请检查本地数据目录。"
  failure.hidden = false
}

void listen<string>("startup-error", (event) => showError(event.payload))
void invoke<string | null>("startup_error").then((error) => { if (error) showError(error) }).catch(() => {})

document.getElementById("retry")!.addEventListener("click", () => {
  failure.hidden = true
  status.textContent = "正在重新启动 Jolu"
  scheduleTimeout()
  void invoke<string>("retry_startup").catch((error) => showError(String(error || "重试失败，请退出后重新打开。")))
})

document.getElementById("quit")!.addEventListener("click", () => { void invoke("quit_startup") })

/** 就绪即进入主窗口，不设人为等待；长时间没有就绪时才给出重试入口 */
function scheduleTimeout() {
  window.clearTimeout(timeout)
  timeout = window.setTimeout(() => {
    if (failure.hidden) showError("启动时间较长，请点击重试。")
  }, 15000)
}

scheduleTimeout()
