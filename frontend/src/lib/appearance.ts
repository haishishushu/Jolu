/**
 * 主窗口外观偏好（画板 88，D-21 / D-22 / D-23）：界面缩放、列表密度、减少动效。
 *
 * 缩放由后端对 `main` 窗口调用 `set_zoom`；密度与减少动效是 `<html>` 上的两个 class。
 * 灵动岛、菜单等 `island-window` 类窗口三项都不生效。
 */

export const ZOOM_STEPS = [80, 90, 100, 110, 125, 150] as const
export type ZoomStep = (typeof ZOOM_STEPS)[number]
export type Density = "comfortable" | "compact"

export const DENSITY_LABEL: Record<Density, string> = { comfortable: "舒适", compact: "紧凑" }
export const DENSITY_CLASS = "density-compact"
export const REDUCE_MOTION_CLASS = "reduce-motion"
const ISLAND_WINDOW_CLASS = "island-window"

/** applyAppearance 只用到 classList，便于在测试里传假的根节点 */
type ClassRoot = { classList: Pick<DOMTokenList, "contains" | "toggle"> }

export function normalizeZoom(value: number): ZoomStep {
  return (ZOOM_STEPS as readonly number[]).includes(value) ? (value as ZoomStep) : 100
}

export function normalizeDensity(value: string): Density {
  return value === "compact" ? "compact" : "comfortable"
}

/** 在 `<html>` 上切换密度与减少动效；灵动岛类窗口直接跳过，保持现状。 */
export function applyAppearance(root: HTMLElement | ClassRoot, density: Density, reduceMotion: boolean): void {
  if (root.classList.contains(ISLAND_WINDOW_CLASS)) return
  root.classList.toggle(DENSITY_CLASS, density === "compact")
  root.classList.toggle(REDUCE_MOTION_CLASS, reduceMotion)
}

/** 系统开启「减少动画」或主窗口开关打开，任一即减少动效（D-23） */
export function effectiveReduceMotion(systemReduce: boolean, setting: boolean): boolean {
  return systemReduce || setting
}

export function shouldReduceMotion(systemReduce: boolean, root: HTMLElement | ClassRoot): boolean {
  return effectiveReduceMotion(systemReduce, root.classList.contains(REDUCE_MOTION_CLASS))
}

/**
 * 开关下方的次要说明：系统已开启「减少动画」时开关即使是关，主窗口也会减少动效，避免误解。
 * 待确认：画板 88 未画此状态。
 */
export function reduceMotionHint(systemReduce: boolean): string | null {
  return systemReduce ? "系统已开启减少动画，主窗口会同时生效。" : null
}

/** 设置页预览区右侧的说明，如「100% · 舒适」 */
export function appearancePreview(zoom: number, density: Density): string {
  return `${zoom}% · ${DENSITY_LABEL[density]}`
}
