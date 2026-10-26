import assert from "node:assert/strict"
import test from "node:test"
import {
  axisTicks,
  hitIndex,
  labelEvery,
  niceCeil,
  seriesMax,
  xAt,
  yAt,
} from "./chartGeometry.ts"

test("刻度上界向上取整到易读的量级", () => {
  assert.equal(niceCeil(25_740_000), 50_000_000)
  assert.equal(niceCeil(8_000_000), 10_000_000)
  assert.equal(niceCeil(1_200), 2_000)
  assert.equal(niceCeil(2_400), 2_500)
  assert.equal(niceCeil(1), 1)
  // 非正数与非数字不产生量纲
  assert.equal(niceCeil(0), 0)
  assert.equal(niceCeil(-5), 0)
  assert.equal(niceCeil(Number.NaN), 0)
})

test("刻度自上而下等分，全零时只给一条 0", () => {
  const ticks = axisTicks(900)
  assert.equal(ticks.length, 4)
  assert.equal(ticks[0], 1000, "顶格是取整后的上界")
  assert.equal(ticks[3], 0)
  // 等分：相邻刻度间距一致
  assert.ok(Math.abs((ticks[0] - ticks[1]) - (ticks[1] - ticks[2])) < 1e-9)
  assert.ok(ticks[1] > ticks[2], "刻度必须由大到小")
  assert.equal(axisTicks(900, 3).length, 3)
  // 没有任何用量时不画四条都是 0 的线
  assert.deepEqual(axisTicks(0), [0])
})

test("系列最大值忽略未知点", () => {
  const series = [
    { key: "a", label: "A", colorVar: "--a", values: [1, null, 5] },
    { key: "b", label: "B", colorVar: "--b", values: [null, null, null] },
  ]
  assert.equal(seriesMax(series), 5)
  assert.equal(seriesMax([]), 0)
  assert.equal(seriesMax([{ key: "c", label: "C", colorVar: "--c", values: [] }]), 0)
})

test("单点时 X 居中，多点时首尾贴边", () => {
  assert.equal(xAt(0, 1, 100), 50)
  assert.equal(xAt(0, 3, 100), 0)
  assert.equal(xAt(2, 3, 100), 100)
  assert.equal(xAt(1, 3, 100), 50)
})

test("Y 坐标夹在绘图区内，上界为零时贴底", () => {
  assert.equal(yAt(0, 100, 176), 176)
  assert.equal(yAt(100, 100, 176), 0)
  assert.equal(yAt(50, 100, 176), 88)
  // 超出上界的值不溢出画布
  assert.equal(yAt(500, 100, 176), 0)
  // 上界为 0 时不做除零
  assert.equal(yAt(0, 0, 176), 176)
})

test("命中最近的点，边界不越界", () => {
  assert.equal(hitIndex(0, 5, 100), 0)
  assert.equal(hitIndex(100, 5, 100), 4)
  assert.equal(hitIndex(51, 5, 100), 2)
  // 超出两端时夹住，不返回 -1 或 5
  assert.equal(hitIndex(-20, 5, 100), 0)
  assert.equal(hitIndex(999, 5, 100), 4)
  assert.equal(hitIndex(30, 1, 100), 0)
  assert.equal(hitIndex(30, 0, 100), -1)
})

test("标签数量超限时按间隔抽稀", () => {
  assert.equal(labelEvery(5), 1)
  assert.equal(labelEvery(8), 1)
  assert.equal(labelEvery(24), 3)
  assert.equal(labelEvery(180), 23)
  assert.equal(labelEvery(0), 1)
})

test("平滑折线：跨过未知点连续连接，保留原始时间位置，不补零", async () => {
  const { smoothPath } = await import("./chartGeometry.ts")
  const path = smoothPath([0, 10, null, 5, 5], 10, { width: 40, height: 10 })
  assert.equal((path.match(/M/g) ?? []).length, 1)
  assert.equal((path.match(/C/g) ?? []).length, 3)
  assert.ok(path.includes('30.00,5.00'), "缺失点后仍处于原始时间位置")
  assert.equal(smoothPath([null, null], 10, {width:40,height:10}), "")
  assert.equal(smoothPath([null, 0, null], 10, {width:40,height:10}), "M20.00,10.00", "真实零保留，前后未知不延伸曲线")
  // 控制点不低于基线（y 不超过画布高度）
  const ys = [...path.matchAll(/,(-?\d+\.\d+)/g)].map((m) => Number(m[1]))
  assert.equal(ys.every((y) => y >= 0 && y <= 10), true)
})
