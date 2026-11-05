import { useSettings } from "@/hooks/useSettings"
import { CapsuleWindow } from "@/views/capsule/CapsuleWindow"
import { ClassicIslandWindow } from "@/views/classic/ClassicIslandWindow"

/**
 * 灵动岛窗口入口 —— 按设置分流到两套独立容器。
 *
 * 形态状态机（§2.1.3）：
 *
 *   自由态 collapsed ──双击──▶ expanded ──双击──▶ collapsed
 *        │  拖到边缘吸附
 *        ▼
 *   停靠态 docked ──悬停──▶ peek（完整收缩态）──双击──▶ expanded
 *        └──双击──▶ 回到自由态 collapsed
 *
 * 手势：桌面端按住直接拖动，不使用长按。
 *
 * 经典（A2）与胶囊（A3）共用数据与形态机（`hooks/useIslandWindowModel`、
 * `useIslandWindowShell`），但各自持有容器：形变锚定方式、探出节奏、缩放对象都不同，
 * 挤在同一个容器里会互相牵制。宁可让两份容器代码看起来相似，也不让两套样式互相影响。
 */
export function IslandWindow() {
  const { settings } = useSettings()
  // 分流只发生在最外层；换样式即换容器，形态会按当前停靠设置重新对齐
  return settings.island_style === "capsule" ? <CapsuleWindow /> : <ClassicIslandWindow />
}
