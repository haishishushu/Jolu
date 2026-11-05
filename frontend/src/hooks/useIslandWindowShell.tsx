import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { SNAP_ZONE } from "@/components/island/PreviewControls"
import { api, isTauri, listenEvent, type DockChangedEvent, type DockHintEvent } from "@/lib/api"
import { isOwnIslandEvent, islandIdFromSearch, islandProfile } from "@/lib/islandProfile"
import { collapseExpandedView, snapIndicatorVisible } from "@/lib/islandViewState"
import { useSettings } from "@/hooks/useSettings"
import type { DockEdge, IslandMode } from "@/types"

/** 右键菜单打开时要告诉后端“现在是否在刷新”；每次渲染后由视图写入最新值 */
export interface IslandMenuBusy {
  current: boolean
}

/**
 * 灵动岛窗口的共享形态机与原生交互（§2.1.3）
 *
 *   自由态 collapsed ──双击──▶ expanded ──双击──▶ collapsed
 *        │  拖到边缘吸附
 *        ▼
 *   停靠态 docked ──悬停──▶ peek（完整收缩态）──双击──▶ expanded
 *        └──双击──▶ 回到自由态 collapsed
 *
 * 手势：桌面端按住直接拖动，不使用长按。
 *
 * 这里**不放探出计时器**：经典是 150ms / 500ms，胶囊是 400ms 且只对左右贴边生效，
 * 两套节奏写在各自的视图里，只共用 `setPeek` / `peekTimer` / `dockedRef` 这几件工具。
 */
export function useIslandWindowShell(menuBusy: IslandMenuBusy) {
  const [mode, setMode] = useState<IslandMode>("collapsed")
  const [edge, setEdge] = useState<DockEdge>("top")
  const [peek, setPeek] = useState(false)
  // 展开那一刻贴的是哪条边：卡片打开期间保持贴边锚定，收回或解除停靠后清空
  const openDockEdge = useRef<DockEdge | null>(null)
  const [dragging, setDragging] = useState(false)
  const draggingRef = useRef(false)
  const [connectionMenuOpen, setConnectionMenuOpen] = useState(false)
  const peekTimer = useRef<number | null>(null)
  // 形态的 ref：原生拖动、探出计时器等回调里读取最新的值
  const expandedRef = useRef(false)
  expandedRef.current = mode === "expanded"
  const dockedRef = useRef(false)
  dockedRef.current = mode === "docked"
  const edgeRef = useRef<DockEdge>("top")
  edgeRef.current = edge
  const peekRef = useRef(false)
  peekRef.current = peek
  // 后端报告的最新停靠边：拖动中被吞掉的 dock-changed 要用它收尾对齐形态
  const dockEdgeRef = useRef<DockEdge | null>(null)

  // 本岛身份：URL 带 clone 参数的是分身，否则是本体；停靠边与事件过滤都按身份取。
  // 视图拿不到停靠边时由这里自取：形态机必须知道“该贴哪条边”，否则启动时对不齐。
  const { settings } = useSettings()
  const cloneId = useMemo(() => islandIdFromSearch(window.location.search), [])
  const dockEdge = islandProfile(settings, cloneId).dock.edge

  const [snapHint, setSnapHint] = useState<{ edge: DockEdge; offset: number; centered: boolean } | null>(null)
  const snapEdge = snapHint?.edge
  useEffect(() => {
    let disposed = false
    let off: (() => void) | undefined
    void listenEvent<DockHintEvent>("dock-hint", (event) => {
      if (isOwnIslandEvent(event, cloneId)) setSnapHint(event.hint)
    }).then((un) => {
      if (disposed) un(); else off = un
    })
    return () => { disposed = true; off?.() }
  }, [cloneId])

  const onPointerDown = useCallback((e: React.PointerEvent) => {
    if (!isTauri || e.button !== 0 || e.detail > 1) return
    if ((e.target as HTMLElement).closest("button, input, a")) return
    const x = e.clientX, y = e.clientY
    const move = (event: PointerEvent) => {
      if (Math.hypot(event.clientX - x, event.clientY - y) < 4) return
      cleanup()
      // 清掉上一次拖动可能延迟到达的提示，等待本轮原生移动事件给出新位置。
      setSnapHint(null)
      draggingRef.current = true
      if (peekTimer.current) window.clearTimeout(peekTimer.current)
      peekTimer.current = null
      setDragging(true)
      void api.islandDrag().catch(console.error).finally(() => {
        draggingRef.current = false
        setDragging(false)
        setSnapHint(null)
        // 拖动中被吞掉的停靠变化在这里收尾：拖出屏幕边就回到自由胶囊，
        // 拖到边上就回到停靠态；展开的卡片保持展开，只跟随锚点。
        if (expandedRef.current) return
        setMode(dockEdgeRef.current ? "docked" : "collapsed")
      })
    }
    const cleanup = () => {
      window.removeEventListener("pointermove", move)
      window.removeEventListener("pointerup", cleanup)
    }
    window.addEventListener("pointermove", move)
    window.addEventListener("pointerup", cleanup, { once: true })
  }, [])

  const contextMenu = (e: React.MouseEvent) => {
    if (!isTauri) return
    e.preventDefault()
    if (peekTimer.current) window.clearTimeout(peekTimer.current)
    // 右键菜单会抢走焦点；先告诉胶囊别把这当成「点到卡片外」
    window.dispatchEvent(new Event("island-menu-open"))
    void api.islandMenu(menuBusy.current).catch(console.error)
  }

  // 启动时恢复上次停靠状态（几何已由 Rust 落位，这里只对齐形态）
  const resetDockView = useCallback((e: DockEdge | null) => {
    if (peekTimer.current) window.clearTimeout(peekTimer.current)
    peekTimer.current = null
    setPeek(false)
    if (e) setEdge(e)
    dockEdgeRef.current = e
    // 拖动时后端会先解除停靠、再按落点重新吸附，一路都在发 dock-changed；
    // 展开的卡片也能被拖动，这时候只能跟着边改锚点，不能把形态改回收缩态，
    // 否则一按住卡片拖动就会「先收回再拖」。
    if (expandedRef.current) {
      openDockEdge.current = e
      return
    }
    if (draggingRef.current) return
    openDockEdge.current = null
    setMode(e ? "docked" : "collapsed")
  }, [])

  useEffect(() => {
    resetDockView(dockEdge)
  }, [dockEdge, resetDockView])

  // 同一侧再次停靠时 edge 值没变，也必须退出探出/展开态并清掉旧定时器。
  useEffect(() => {
    let disposed = false
    let off: (() => void) | undefined
    void listenEvent<DockChangedEvent>("dock-changed", (event) => {
      if (isOwnIslandEvent(event, cloneId)) resetDockView(event.dock.edge)
    }).then((un) => { if (disposed) un(); else off = un })
    return () => {
      disposed = true
      off?.()
      if (peekTimer.current) window.clearTimeout(peekTimer.current)
    }
  }, [resetDockView, cloneId])

  const undock = useCallback(() => {
    setPeek(false)
    setMode("collapsed")
    if (isTauri) void api.dockUndock()
  }, [])

  /**
   * 收回到停靠 / 自由态。
   * `hoverAfterPeek`：胶囊卡片收回后停在完整胶囊还是迷你胶囊，取决于指针是否还在岛上
   * （原型 miniHover 的语义）；经典样式没有迷你态，收回即完整收缩态。
   */
  const collapseExpanded = useCallback((hoverAfterPeek = true) => {
    if (peekTimer.current) window.clearTimeout(peekTimer.current)
    peekTimer.current = null
    // 下拉若因收回被卸载，别让「菜单打开中」的状态留到下一次展开
    setConnectionMenuOpen(false)
    const next = collapseExpandedView(dockEdge ?? openDockEdge.current)
    openDockEdge.current = null
    if (next.edge) setEdge(next.edge)
    setPeek(next.edge ? hoverAfterPeek : false)
    setMode(next.mode)
  }, [dockEdge])

  const snapIndicator = snapIndicatorVisible(dragging, mode, Boolean(snapEdge)) && snapEdge && (
    <span
      aria-hidden
      className={`pointer-events-none fixed z-50 ${snapHint?.centered ? "bg-success" : "bg-accent-blue"} ${
        snapEdge === "top"
          ? "left-0 right-0 top-0 h-[3px]"
          : snapEdge === "bottom"
            ? "bottom-0 left-0 right-0 h-[3px]"
            : snapEdge === "left"
              ? "bottom-0 left-0 top-0 w-[3px]"
              : "bottom-0 right-0 top-0 w-[3px]"
      }`}
    />
  )

  return {
    mode,
    setMode,
    edge,
    setEdge,
    peek,
    setPeek,
    dragging,
    draggingRef,
    dockedRef,
    expandedRef,
    edgeRef,
    peekRef,
    openDockEdge,
    peekTimer,
    connectionMenuOpen,
    setConnectionMenuOpen,
    cloneId,
    snapHint,
    snapEdge,
    snapIndicator,
    snapZone: SNAP_ZONE,
    resetDockView,
    onPointerDown,
    contextMenu,
    undock,
    collapseExpanded,
  }
}
