import { useCallback, useRef } from "react"
import { CapsuleIsland } from "@/components/island/CapsuleIsland"
import { LazyConnectionSwitcher } from "@/components/island/LazyConnectionSwitcher"
import { DockControls, FreeControls } from "@/components/island/PreviewControls"
import { useIslandWindowModel } from "@/hooks/useIslandWindowModel"
import { useIslandWindowShell } from "@/hooks/useIslandWindowShell"
import { useIslandWindowSize } from "@/hooks/useIslandWindowSize"
import { api, isTauri } from "@/lib/api"
import { LiveCountsContext } from "@/lib/liveCounts"

/**
 * 胶囊灵动岛窗口（A3）。
 *
 * 停靠、探出、展开始终是同一个 CapsuleIsland 实例，外壳自己做形变，
 * 不会因为换分支而重新挂载、硬切成另一个组件。
 *
 * 布局按停靠边锚定：窗口先长、外壳后长。内容框贴住停靠的那条边，
 * 窗口变大时外壳不会被窗口左上角带着跑（经典样式的居中布局正是停靠动效生硬的来源）。
 *
 * 探出节奏是本样式独有的：只有左右贴边缩成迷你胶囊，悬停 140ms 恢复完整胶囊，
 * 指针落到窗口空白处立即收回。
 */
export function CapsuleWindow() {
  const zoomRef = useRef(1)
  const shellRef = useIslandWindowSize(zoomRef)
  // 右键菜单的“正在刷新”标记要在打开那一刻读，用 ref 传入共享形态机
  const menuBusy = useRef(false)
  // 胶囊：指针是否停在外壳上（探出与卡片收回后的落点都按它判断）
  const capsuleHoverRef = useRef(false)
  const shell = useIslandWindowShell(menuBusy)
  const {
    mode,
    edge,
    peek,
    dragging,
    connectionMenuOpen,
    setMode,
    setEdge,
    setPeek,
    setConnectionMenuOpen,
    openDockEdge,
    draggingRef,
    dockedRef,
    expandedRef,
    edgeRef,
    peekRef,
    peekTimer,
    snapIndicator,
    onPointerDown,
    contextMenu,
    undock,
    collapseExpanded,
  } = shell

  // 形态是数据装配的入参：胶囊同样只在卡片可见时才查余额与用量
  const model = useIslandWindowModel({ capsule: true, mode, peek, dragging })
  const {
    data,
    settings,
    profile,
    cloneId,
    connections,
    connectionsLoading,
    active,
    refreshActive,
    quota,
    balance,
    gateway,
    live,
  } = model
  menuBusy.current = model.refreshingRef.current || quota.loading || balance.loading

  /**
   * 胶囊的指针进出：和原型一样挂在岛本体上，不是整个窗口。
   * 窗口在形变时会先变大，把悬停判定放在壳上，才不会「指针还在窗口空白里就探出」。
   */
  const onCapsuleHover = useCallback((over: boolean) => {
    capsuleHoverRef.current = over
    if (peekTimer.current) { window.clearTimeout(peekTimer.current); peekTimer.current = null }
    if (!over) {
      if (peekRef.current) setPeek(false)
      return
    }
    if (draggingRef.current || !dockedRef.current || expandedRef.current) return
    // 只有左右贴边会缩成迷你胶囊（原型 .dock.docked-left / docked-right）；
    // 顶部、底部贴边保持完整胶囊，没有需要探出的迷你态
    if (edgeRef.current === "top" || edgeRef.current === "bottom") return
    // 原型是 400ms，实机偏慢：改成 140ms 的意图延迟——鼠标路过不会误弹出，
    // 真正停在迷你说胶囊上时几乎立刻展开。
    peekTimer.current = window.setTimeout(() => setPeek(true), 140)
  }, [capsuleHoverRef, peekTimer, peekRef, draggingRef, dockedRef, expandedRef, edgeRef, setPeek])

  /** 指针落在窗口空白处（形变后壳已经不在指针下）：立即收回，避免停靠时留着一个完整胶囊 */
  const onCapsulePointerMove = useCallback((event: React.PointerEvent) => {
    if (!dockedRef.current || !peekRef.current || expandedRef.current) return
    const target = event.target as HTMLElement | null
    if (target?.closest("[data-capsule-island]")) return
    setPeek(false)
  }, [dockedRef, peekRef, expandedRef, setPeek])

  // 停靠是持久状态：卡片打开时仍贴在同一条边上（原型 .dock.docked-* 不随 open 取消）
  const dockEdge = mode === "docked" ? edge : mode === "expanded" ? openDockEdge.current : null
  // 只有左右贴边缩成迷你胶囊（原型只有 .docked-left / .docked-right）；
  // 顶部、底部贴边保持完整胶囊，不是「停靠条」形态
  const mini = (dockEdge === "left" || dockEdge === "right") && !peek
  const scale = settings.island_scale / 100
  zoomRef.current = scale
  // 胶囊本体不投影，窗口就直接贴着胶囊长：窗口 == 胶囊。
  // 之前固定留 20px，视觉上像「胶囊外面套了一圈方框」，而且那一圈是置顶窗口的死区，会吃掉桌面点击。
  // 只有展开卡片要画 0 4px 16px 的投影，这时才留 20px（偏移 4 + 模糊 16）。
  const pad = mode === "expanded" ? "p-5" : "p-0"
  const frame = !dockEdge || dockEdge === "top" ? "top-0 left-1/2 -translate-x-1/2"
    : dockEdge === "bottom" ? "bottom-0 left-1/2 -translate-x-1/2"
      : dockEdge === "left" ? "left-0 top-1/2 -translate-y-1/2"
        : "right-0 top-1/2 -translate-y-1/2"

  return (
    <LiveCountsContext.Provider value={live.counts}>
      <div
        style={{ opacity: settings.island_opacity / 100 }}
        className={`relative h-full w-full ${dragging ? "island-dragging" : ""}`}
        onContextMenu={contextMenu}
        onPointerDown={onPointerDown}
        onPointerMove={onCapsulePointerMove}
        // 拖动整窗时别让浏览器把里面的 <img>/<svg> 当成可拖拽素材：
        // 一旦原生 HTML5 拖拽抢到 pointer，窗口拖动会变成「把图拖出来」。
        onDragStart={(e) => e.preventDefault()}
      >
        {snapIndicator}
        <div className="absolute inset-0" style={{ zoom: scale }}>
          <div ref={shellRef} className={`absolute w-max ${pad} ${frame}`}>
            <CapsuleIsland
              data={data}
              open={mode === "expanded"}
              mini={mini}
              connectionMenuOpen={connectionMenuOpen}
              onOpenChange={(next) => {
                if (next) {
                  openDockEdge.current = mode === "docked" ? edge : profile.dock.edge ?? null
                  setMode("expanded")
                  setConnectionMenuOpen(false)
                } else collapseExpanded(capsuleHoverRef.current)
              }}
              dragging={dragging}
              refreshing={refreshActive}
              dockEdge={dockEdge}
              gateway={gateway.relay ? gateway : null}
              zoom={scale}
              onHoverChange={onCapsuleHover}
              connectionSwitcher={
                <LazyConnectionSwitcher
                  connections={connections}
                  selectedId={active?.id}
                  loading={connectionsLoading}
                  onOpenChange={setConnectionMenuOpen}
                  onSelect={isTauri ? (id) => api.setIslandConnection(id, cloneId) : undefined}
                />
              }
            />
          </div>
        </div>
        {/* 浏览器预览的演示按钮不参与测量，否则会把内容框撑宽、让停靠位置失真 */}
        {!isTauri && (
          <div className="fixed bottom-4 left-1/2 z-50 -translate-x-1/2 rounded-[10px] border bg-surface px-3 py-2 text-[11px] shadow">
            {dockEdge
              ? <DockControls edge={edge} onEdge={(e) => { setEdge(e); setMode("docked") }} onUndock={undock} />
              : <FreeControls mode={mode} onMode={setMode} onDock={(e) => { setEdge(e); setMode("docked") }} />}
          </div>
        )}
      </div>
    </LiveCountsContext.Provider>
  )
}
