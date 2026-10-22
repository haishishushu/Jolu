/**
 * 主窗口导航（Jolu.pen 模块 A：侧栏 + 内容页头）。
 *
 * 侧栏顺序与文案取自 Jolu.pen 画板 01；托盘、灵动岛菜单发来的跳转意图在这里换算成页面，
 * 便于 node --test 直接验证。
 */

export type MainPage = "island" | "providers" | "usage" | "sessions" | "automation" | "skills" | "settings"

export interface MainPageItem {
  id: MainPage
  label: string
}

/** 侧栏上部的六个模块；「设置」固定在侧栏底部，单独列出 */
export const PRIMARY_PAGES: readonly MainPageItem[] = [
  { id: "island", label: "灵动岛" },
  { id: "providers", label: "供应商" },
  { id: "usage", label: "用量统计" },
  { id: "sessions", label: "会话线程" },
  { id: "automation", label: "自动化" },
  { id: "skills", label: "Skills 管理" },
]

export const SETTINGS_PAGE: MainPageItem = { id: "settings", label: "设置" }

export function pageLabel(page: MainPage): string {
  return page === "settings" ? SETTINGS_PAGE.label : PRIMARY_PAGES.find((item) => item.id === page)?.label ?? ""
}

/** 托盘 / 菜单的跳转意图；结构与后端 MainIntent 一致（关于弹窗不切页面，不在这里处理） */
export type NavigationIntent =
  | { kind: "overview" }
  | { kind: "settings"; section: string }

export function pageForIntent(intent: NavigationIntent): MainPage {
  if (intent.kind === "overview") return "usage"
  if (intent.section === "island") return "island"
  if (intent.section === "connections") return "providers"
  return "settings"
}

export const SIDEBAR_KEY = "jolu:main:sidebarCollapsed"

type StorageLike = Pick<Storage, "getItem" | "setItem">

/** 侧栏收起状态只是本机偏好：读不到就按展开处理 */
export function readSidebarCollapsed(storage: StorageLike = localStorage): boolean {
  try {
    return storage.getItem(SIDEBAR_KEY) === "1"
  } catch {
    return false
  }
}

export function writeSidebarCollapsed(collapsed: boolean, storage: StorageLike = localStorage): void {
  try {
    storage.setItem(SIDEBAR_KEY, collapsed ? "1" : "0")
  } catch {
    // 存不进去只影响下次打开时的默认形态
  }
}
