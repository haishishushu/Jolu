import { useCallback, useRef } from "react"
import { isTauri } from "@/lib/api"
import { createWindowSizeSync } from "@/lib/windowSizeSync"

/**
 * 桌面端：让 Tauri 窗口尺寸跟随岛的实际内容。
 * 收缩 / 展开 / 停靠三态尺寸差别很大，写死窗口高度会把内容截断或留大片空白。
 *
 * 经典与胶囊两个容器共用这一份测量：形变方式不同，但“量内容、改窗口”是同一件事。
 */
export function useIslandWindowSize(zoomRef: { current: number }) {
  const roRef = useRef<ResizeObserver | null>(null)
  const cleanupRef = useRef<(() => void) | null>(null)
  const syncRef = useRef<ReturnType<typeof createWindowSizeSync> | null>(null)
  if (!syncRef.current) {
    syncRef.current = createWindowSizeSync(async (size) => {
      const { invoke } = await import("@tauri-apps/api/core")
      const maxHeight = await invoke<number | null>("resize_island", size)
      window.dispatchEvent(new CustomEvent("island-size-limit", { detail: maxHeight }))
    })
  }

  // 用 callback ref：收缩/展开/停靠切换时会换整棵 JSX 树，
  // useRef + useLayoutEffect 只在挂载时跑一次，会盯住已卸载的节点。
  return useCallback((el: HTMLDivElement | null) => {
    cleanupRef.current?.()
    cleanupRef.current = null
    roRef.current?.disconnect()
    roRef.current = null
    syncRef.current?.cancelPending()
    if (!isTauri || !el) return

    const apply = (reconcile = false) => {
      if (!el.isConnected) return
      const r = el.getBoundingClientRect()
      // zoom 挂在外层容器上，被量的节点自己不是 zoom 的元素
      const zoom = zoomRef.current || 1
      const w = Math.ceil(Math.max(r.width, el.scrollWidth * zoom))
      const h = Math.ceil(Math.max(r.height, el.scrollHeight * zoom))
      if (w < 8 || h < 8) return
      // Rust 停靠/重置也会改窗口尺寸；内容没变时 ResizeObserver 不会通知。
      // 允许一个 CSS 像素的 DPI 取整误差，防止反复校正正常的分数像素。
      const mismatch = Math.abs(window.innerWidth - w) > 1 || Math.abs(window.innerHeight - h) > 1
      syncRef.current?.request({ width: w, height: h }, reconcile && mismatch)
    }
    let frame = 0
    const reconcile = () => {
      cancelAnimationFrame(frame)
      frame = requestAnimationFrame(() => apply(true))
    }
    window.addEventListener("resize", reconcile)
    cleanupRef.current = () => {
      window.removeEventListener("resize", reconcile)
      cancelAnimationFrame(frame)
    }
    const ro = new ResizeObserver(() => void apply())
    ro.observe(el)
    roRef.current = ro
    // 新节点挂载时原生窗口可能仍是上一种形态，必须校正而非命中旧尺寸缓存。
    void apply(true)
    reconcile()
  }, [])
}
