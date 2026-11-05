import { useEffect, useRef, type RefObject } from "react"
import { gazeSmooth, gazeStep, gazeTarget, type Vec } from "@/lib/botGaze"
import { isTauri, listenEvent } from "@/lib/api"
import { reducedMotion } from "@/lib/motion"

/**
 * 视线跟随：所有 bot 共用一个指针状态，每帧插值逼近目标，直接写 transform，不触发 React 重渲染。
 *
 * 指针来源（设计说明 §6.2）：
 * - 窗口内 `pointermove`：延迟最低；
 * - 后端 `island-pointer`：桌面物理坐标，鼠标在灵动岛窗口之外也能跟随。
 *   换算：减去窗口内容区左上角的物理坐标，再除以缩放比，得到窗口内的 CSS 坐标。
 */
const pointer = { x: 0, y: 0, on: false }
const listeners = new Set<() => void>()
const notify = () => listeners.forEach((listener) => listener())
let installed = false
/** 最近一次收到后端推送的时刻；推送在跑时，鼠标离开窗口不算「指针消失」 */
let lastGlobal = 0

function install() {
  if (installed) return
  installed = true
  window.addEventListener("pointermove", (event) => {
    pointer.x = event.clientX; pointer.y = event.clientY; pointer.on = true
    notify()
  }, true)
  const lost = () => {
    if (performance.now() - lastGlobal < 500) return
    pointer.on = false
    notify()
  }
  document.documentElement.addEventListener("mouseleave", lost)
  window.addEventListener("blur", lost)
  if (isTauri) void installGlobal()
}

async function installGlobal() {
  const { getCurrentWindow } = await import("@tauri-apps/api/window")
  const win = getCurrentWindow()
  let origin = { x: 0, y: 0 }
  let scale = window.devicePixelRatio || 1
  const refresh = async () => {
    try {
      const [position, factor] = await Promise.all([win.innerPosition(), win.scaleFactor()])
      origin = { x: position.x, y: position.y }
      scale = factor
    } catch { /* 窗口正在销毁 */ }
  }
  await refresh()
  // 拖动、停靠与缩放都会改变窗口原点
  void win.onMoved(() => void refresh())
  void win.onScaleChanged(() => void refresh())
  window.addEventListener("resize", () => void refresh())
  void listenEvent<{ x: number; y: number }>("island-pointer", (physical) => {
    lastGlobal = performance.now()
    pointer.x = (physical.x - origin.x) / scale
    pointer.y = (physical.y - origin.y) / scale
    pointer.on = true
    notify()
  })
}

export function useGaze(
  faceRef: RefObject<SVGCircleElement | null>,
  eyesRef: RefObject<SVGGElement | null>,
  input: { base: Vec; amp: number },
  faceSize: number,
) {
  // 视线状态跨渲染保留：表情切换只改目标，眼睛从当前位置平滑过去，不跳回中位
  const latest = useRef(input)
  latest.current = input
  const current = useRef<Vec>({ x: 0, y: 0 })
  const kickRef = useRef<() => void>(() => {})

  useEffect(() => {
    install()
    let frame = 0
    const step = () => {
      frame = 0
      const face = faceRef.current, eyes = eyesRef.current
      if (!face || !eyes) return
      const rect = face.getBoundingClientRect()
      const reduced = reducedMotion()
      const target = gazeTarget(
        { cx: rect.left + rect.width / 2, cy: rect.top + rect.height / 2, width: rect.width },
        pointer.on && !reduced ? { x: pointer.x, y: pointer.y } : null,
        latest.current,
      )
      const result = gazeStep(current.current, target, gazeSmooth(reduced))
      current.current = result.next
      eyes.style.transform = `translate(${result.next.x * faceSize}px, ${result.next.y * faceSize}px)`
      if (!result.settled) frame = requestAnimationFrame(step)
    }
    const kick = () => { if (!frame) frame = requestAnimationFrame(step) }
    kickRef.current = kick
    listeners.add(kick)
    kick()
    return () => { listeners.delete(kick); cancelAnimationFrame(frame); kickRef.current = () => {} }
  }, [faceRef, eyesRef, faceSize])

  // 每次渲染后重新对准：表情偏移、胶囊尺寸或位置可能变了
  useEffect(() => { kickRef.current() })
}
