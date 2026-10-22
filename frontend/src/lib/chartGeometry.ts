/**
 * 面积图的几何计算。
 *
 * 全部是纯函数：输入数值、输出坐标与字符串，不碰 DOM、不读主题、不依赖 React。
 * 图表最容易出错的地方是刻度取整、空数据和"未知值"的断线处理，这些都在这里，
 * 由 chartGeometry.test.mjs 覆盖；组件只负责把算好的 path 画出来。
 */

/** 单条系列：`values` 与 X 轴刻度一一对应，null 表示该点未知（断线，不画成 0） */
export interface Series {
  key: string
  label: string
  /** CSS 变量名，如 "--chart-fresh-input"；取色由组件负责，几何计算不关心具体色值 */
  colorVar: string
  values: Array<number | null>
}

export interface Plot {
  width: number
  height: number
}

/**
 * "好看的"刻度上界：把最大值向上取整到 1/2/2.5/5×10ⁿ。
 *
 * 直接用最大值当上界会让峰值顶到画布边缘，且刻度标签变成 25.74M 这种读不出量级的数；
 * 取整后刻度是 10M / 20M / 30M，一眼能比。
 */
export function niceCeil(value: number): number {
  if (!Number.isFinite(value) || value <= 0) return 0
  const magnitude = 10 ** Math.floor(Math.log10(value))
  const normalized = value / magnitude
  const step = normalized <= 1 ? 1 : normalized <= 2 ? 2 : normalized <= 2.5 ? 2.5 : normalized <= 5 ? 5 : 10
  return step * magnitude
}

/**
 * 轴刻度：从 0 到 `niceCeil(max)` 等分。返回值由大到小，与自上而下的渲染顺序一致。
 * 全零或空数据时只给一条 0 —— 画四条都是 0 的刻度线是在假装有量纲。
 */
export function axisTicks(max: number, count = 4): number[] {
  const top = niceCeil(max)
  if (top <= 0) return [0]
  return Array.from({ length: count }, (_, index) => (top * (count - 1 - index)) / (count - 1))
}

/** 多条系列里的最大值；全部为 null 或空时返回 0 */
export function seriesMax(series: Series[]): number {
  let max = 0
  for (const item of series) {
    for (const value of item.values) {
      if (value !== null && Number.isFinite(value) && value > max) max = value
    }
  }
  return max
}

/** 第 index 个点的 X 坐标。单点时居中，避免宽度 0 的除零 */
export function xAt(index: number, count: number, width: number): number {
  if (count <= 1) return width / 2
  return (index * width) / (count - 1)
}

/** 数值到 Y 坐标；`top` 为 0 时一律贴底，不做除零 */
export function yAt(value: number, top: number, height: number): number {
  if (top <= 0) return height
  const ratio = Math.min(1, Math.max(0, value / top))
  return height - ratio * height
}

/**
 * 鼠标横坐标落在第几个点上。返回 -1 表示没有点可命中。
 * 按最近邻取点而不是按区间归属：光标停在两点中间时，跳到更近的那个更符合直觉。
 */
export function hitIndex(offsetX: number, count: number, width: number): number {
  if (count <= 0) return -1
  if (count === 1) return 0
  const ratio = Math.min(1, Math.max(0, offsetX / width))
  return Math.round(ratio * (count - 1))
}

/**
 * X 轴标签的抽稀间隔：点数多时每隔若干个标一次，保证最多 `maxLabels` 个标签。
 * 标签重叠比标签稀疏更难读，所以宁可少标。
 */
export function labelEvery(count: number, maxLabels = 8): number {
  if (count <= maxLabels) return 1
  return Math.ceil(count / maxLabels)
}

/**
 * 平滑折线（画板 30「平滑折线趋势」）：每个连续段按 Catmull-Rom 转三次贝塞尔。
 * 控制点纵向夹在相邻两点之间，曲线不会冲出数据范围（不会画出负值）；null 照样断线。
 */
export function smoothPath(values: Array<number | null>, top: number, plot: Plot): string {
  const parts: string[] = []
  let segment: Array<{ x: number; y: number }> = []
  const f = (n: number) => n.toFixed(2)
  const flush = () => {
    if (!segment.length) return
    const p = segment
    parts.push(`M${f(p[0].x)},${f(p[0].y)}`)
    for (let i = 0; i < p.length - 1; i++) {
      const p0 = p[i - 1] ?? p[i]
      const p1 = p[i]
      const p2 = p[i + 1]
      const p3 = p[i + 2] ?? p2
      const lo = Math.min(p1.y, p2.y)
      const hi = Math.max(p1.y, p2.y)
      const clamp = (y: number) => Math.min(hi, Math.max(lo, y))
      const c1 = { x: p1.x + (p2.x - p0.x) / 6, y: clamp(p1.y + (p2.y - p0.y) / 6) }
      const c2 = { x: p2.x - (p3.x - p1.x) / 6, y: clamp(p2.y - (p3.y - p1.y) / 6) }
      parts.push(`C${f(c1.x)},${f(c1.y)} ${f(c2.x)},${f(c2.y)} ${f(p2.x)},${f(p2.y)}`)
    }
    segment = []
  }
  values.forEach((value, index) => {
    if (value === null || !Number.isFinite(value)) return flush()
    segment.push({ x: xAt(index, values.length, plot.width), y: yAt(value, top, plot.height) })
  })
  flush()
  return parts.join(" ")
}
