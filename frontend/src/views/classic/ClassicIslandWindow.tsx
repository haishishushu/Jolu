import { useCallback, useRef } from "react"
import { DockedIsland } from "@/components/island/DockedIsland"
import { IslandMotion } from "@/components/island/IslandMotion"
import { LazyConnectionSwitcher } from "@/components/island/LazyConnectionSwitcher"
import { DockControls, FreeControls } from "@/components/island/PreviewControls"
import { IslandCollapsed, IslandExpanded } from "@/components/island/UsageIsland"
import { useIslandWindowModel } from "@/hooks/useIslandWindowModel"
import { useIslandWindowShell } from "@/hooks/useIslandWindowShell"
import { useIslandWindowSize } from "@/hooks/useIslandWindowSize"
import { api, isTauri } from "@/lib/api"
import { dockPulseActive } from "@/lib/dockPulse"
import { LiveCountsContext } from "@/lib/liveCounts"
import { CLAUDE_QUOTAS } from "@/mock/data"

/**
 * 经典灵动岛窗口（A2）。
 *
 * 行为与拆分前逐字一致：停靠条悬停 150ms 探出、离开 500ms 收回，
 * 缩放分 `island_scale` / `island_shrink_scale` 两档，`p-3.5` 只在非停靠或探出时留白。
 * 这些节奏只属于经典样式，所以写在视图里，不放进共享形态机。
 */
export function ClassicIslandWindow() {
  const zoomRef = useRef(1)
  const shellRef = useIslandWindowSize(zoomRef)
  // 右键菜单的“正在刷新”标记要在打开那一刻读，用 ref 传入共享形态机
  const menuBusy = useRef(false)
  const shell = useIslandWindowShell(menuBusy)
  const {
    mode,
    peek,
    edge,
    dragging,
    connectionMenuOpen,
    setMode,
    setEdge,
    setPeek,
    setConnectionMenuOpen,
    draggingRef,
    dockedRef,
    peekTimer,
    snapIndicator,
    onPointerDown,
    contextMenu,
    undock,
    collapseExpanded,
  } = shell

  // 形态是数据装配的入参：停靠未探出时卡片不可见，余额/用量查询会暂停
  const model = useIslandWindowModel({ capsule: false, mode, peek, dragging })
  const {
    data,
    settings,
    cloneId,
    connections,
    connectionsLoading,
    active,
    refreshActive,
    refreshSerial,
    quota,
    balance,
    quotaView,
    dockQuotas,
    quotaWindows,
    live,
  } = model
  menuBusy.current = model.refreshingRef.current || quota.loading || balance.loading

  const onPeekEnter = useCallback(() => {
    if (draggingRef.current || !dockedRef.current) return
    if (peekTimer.current) window.clearTimeout(peekTimer.current)
    // 经典停靠条 150ms 探出
    peekTimer.current = window.setTimeout(() => setPeek(true), 150)
  }, [draggingRef, dockedRef, peekTimer, setPeek])

  // 经典停靠条沿用 500ms 防抖
  const onPeekLeave = useCallback(() => {
    if (draggingRef.current || !dockedRef.current) return
    if (peekTimer.current) window.clearTimeout(peekTimer.current)
    peekTimer.current = window.setTimeout(() => setPeek(false), 500)
  }, [draggingRef, dockedRef, peekTimer, setPeek])

  // 阴影 0 2px 10px：窗口需为岛留出四周空白，否则被窗口边缘切掉
  const shellPad = isTauri && (mode !== "docked" || peek) ? "p-3.5" : ""

  if (mode === "docked") {
    zoomRef.current = (peek ? settings.island_scale : settings.island_shrink_scale) / 100
    return (
      <LiveCountsContext.Provider value={live.counts}>
      <div
        ref={shellRef}
        style={{ opacity: settings.island_opacity / 100, zoom: peek ? settings.island_scale / 100 : settings.island_shrink_scale / 100 }}
        className={`flex w-fit flex-col items-center ${shellPad} ${dragging ? "island-dragging" : ""}`}
        onContextMenu={contextMenu}
        onPointerDown={onPointerDown}
        onMouseEnter={onPeekEnter}
        onMouseLeave={onPeekLeave}
      >
        {snapIndicator}
        {peek ? (
          /* 探出后双击才进入展开态（§2.1.3） */
          <IslandCollapsed data={data} onExpand={() => setMode("expanded")} refreshKey={refreshSerial} refreshing={refreshActive} />
        ) : (
          /* 对停靠条双击 = 解除停靠、回到自由态收缩态。
             处理器只挂在停靠条上，避免探出时与「双击展开」同时触发 */
          <div
            onDoubleClick={undock}
            className={edge === "top" || edge === "bottom" ? "dock-contract-horizontal" : "dock-contract-vertical"}
          >
            {/* 查到 5h/7d 画真实水位；已确认没有这两个窗口时画两条满格彩虹色；
                其余查不到的只留空轨道，不着色不满格。
                浏览器预览仍用设计示例展示三档水位 */}
            <DockedIsland
              edge={edge}
              quotas={isTauri ? dockQuotas : CLAUDE_QUOTAS}
              unlimited={isTauri && quotaView.type === "unlimited"}
              unavailable={isTauri && !quotaWindows && quotaView.type !== "unlimited"}
              pulse={dockPulseActive(live.deltaPositive, live.deltaPhase, settings.dnd, dragging)}
              pulseKey={live.pulseSerial}
              refreshing={refreshActive}
              refreshKey={refreshSerial}
            />
          </div>
        )}
        {!isTauri && <DockControls edge={edge} onEdge={setEdge} onUndock={undock} />}
      </div>
      </LiveCountsContext.Provider>
    )
  }

  zoomRef.current = settings.island_scale / 100
  return (
    <LiveCountsContext.Provider value={live.counts}>
    <div
      ref={shellRef}
      style={{ opacity: settings.island_opacity / 100, zoom: settings.island_scale / 100 }}
      className={`flex w-fit flex-col items-center gap-3 ${shellPad} ${dragging ? "island-dragging" : ""}`}
      onContextMenu={contextMenu}
      onPointerDown={onPointerDown}
    >
      {snapIndicator}
      <IslandMotion expanded={mode === "expanded"} dragging={dragging}>
      {mode === "collapsed" ? (
        <IslandCollapsed data={data} onExpand={() => setMode("expanded")} refreshKey={refreshSerial} refreshing={refreshActive} />
      ) : (
        <IslandExpanded
          data={data}
          connectionMenuOpen={connectionMenuOpen}
          connectionSwitcher={
            <LazyConnectionSwitcher
              connections={connections}
              selectedId={active?.id}
              loading={connectionsLoading}
              onOpenChange={setConnectionMenuOpen}
              onSelect={isTauri ? (id) => api.setIslandConnection(id, cloneId) : undefined}
            />
          }
          onCollapse={collapseExpanded}
          refreshKey={refreshSerial}
          refreshing={refreshActive}
        />
      )}
      </IslandMotion>
      {!isTauri && (
        <FreeControls mode={mode} onMode={setMode} onDock={(e) => { setEdge(e); setMode("docked") }} />
      )}
    </div>
    </LiveCountsContext.Provider>
  )
}
