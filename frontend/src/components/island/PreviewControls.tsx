import type { DockEdge, IslandMode } from "@/types"

/** 停靠吸附区宽度：与后端 island_drag 判定的贴边距离保持一致 */
export const SNAP_ZONE = 24

/* 以下两个控制条只用于在浏览器里演示状态机；Tauri 中由真实窗口手势驱动，不出现在界面上 */
export function FreeControls({
  mode,
  onMode,
  onDock,
}: {
  mode: IslandMode
  onMode: (m: IslandMode) => void
  onDock: (e: DockEdge) => void
}) {
  return (
    <div className="flex items-center gap-2 text-[11px] text-text-tertiary">
      <span>演示：</span>
      <button
        type="button"
        className="rounded border px-2 py-0.5"
        onClick={() => onMode(mode === "collapsed" ? "expanded" : "collapsed")}
      >
        双击{mode === "collapsed" ? "展开" : "收缩"}
      </button>
      <span>吸附到</span>
      {(["top", "bottom", "left", "right"] as DockEdge[]).map((e) => (
        <button key={e} type="button" className="rounded border px-2 py-0.5" onClick={() => onDock(e)}>
          {{ top: "上", bottom: "下", left: "左", right: "右" }[e]}
        </button>
      ))}
      <span className="text-text-tertiary">（吸附区 {SNAP_ZONE}px）</span>
    </div>
  )
}

export function DockControls({
  edge,
  onEdge,
  onUndock,
}: {
  edge: DockEdge
  onEdge: (e: DockEdge) => void
  onUndock: () => void
}) {
  return (
    <div className="mt-3 flex items-center gap-2 text-[11px] text-text-tertiary">
      <span>已停靠（悬停探出 · 双击解除）：</span>
      {(["top", "bottom", "left", "right"] as DockEdge[]).map((e) => (
        <button
          key={e}
          type="button"
          className={`rounded border px-2 py-0.5 ${e === edge ? "text-accent-blue" : ""}`}
          onClick={() => onEdge(e)}
        >
          {{ top: "上", bottom: "下", left: "左", right: "右" }[e]}
        </button>
      ))}
      <button type="button" className="rounded border px-2 py-0.5" onClick={onUndock}>
        解除停靠
      </button>
    </div>
  )
}
