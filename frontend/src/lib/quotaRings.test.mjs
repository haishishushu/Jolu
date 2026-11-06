import test from "node:test"
import assert from "node:assert/strict"
import {
  MAX_RING_COUNT,
  TICK_COUNT,
  quotaRings,
  tickRingMarker,
  tickRingSegments,
  tickedSegments,
} from "./quotaRings.ts"

const w = (key, used_percent, window_name = key) => ({
  key, windowName: window_name, usedPercent: used_percent, usedText: null, resetCountdown: null, badgeTone: "neutral",
})

const keys = (rings) => rings.map((ring) => ring.window.key)
const shapes = (rings) => rings.map((ring) => `${ring.window.key}:${ring.geometry}`)

test("几何按窗口声明：订阅窗口弧段环，日额度与积分桶刻度环", () => {
  assert.deepEqual(shapes(quotaRings("claude", [w("5h", 10), w("7d", 20)])), ["7d:arc", "5h:arc"])
  // 自建网关的日消费窗口 / 总配额是刻度环
  assert.deepEqual(shapes(quotaRings("custom", [w("1d", 30)])), ["1d:ticks"])
  assert.deepEqual(shapes(quotaRings("custom", [w("quota", 30)])), ["quota:ticks"])
  assert.deepEqual(shapes(quotaRings("grok", [w("credits", 40)])), ["credits:ticks"])
})

test("未声明的窗口 key 用平台缺省：ZCode 的 bucket_id 是刻度环", () => {
  // ZCode 本机登录返回的是按 bucket_id 的日额度桶，注册表里不可能逐个声明
  assert.deepEqual(shapes(quotaRings("zcode", [w("bkt-1", 62, "GLM-5.3")])), ["bkt-1:ticks"])
  // 平台没写进注册表时回到改动前的弧段环
  assert.deepEqual(shapes(quotaRings("custom-unknown", [w("whatever", 20)])), ["whatever:arc"])
  assert.deepEqual(shapes(quotaRings(undefined, [w("whatever", 20)])), ["whatever:arc"])
})

test("声明为不画环的窗口不参与（7d-opus / 7d-sonnet 只是卡片里的额外子窗口）", () => {
  assert.deepEqual(quotaRings("claude", [w("7d-opus", 95), w("7d-sonnet", 96)]), [])
  // 它们不会按水位把 5h / 7d 挤出环
  assert.deepEqual(keys(quotaRings("claude", [w("7d-opus", 95), w("5h", 10), w("7d", 20)])), ["7d", "5h"])
})

test("未知（没有已用百分比）不画环，也不画成 0%", () => {
  assert.deepEqual(quotaRings("claude", [w("5h", null), w("7d", null)]), [])
  assert.deepEqual(quotaRings("zcode", [w("bkt-1", null)]), [])
  // 未知的那些不进环，已知的照常画
  assert.deepEqual(keys(quotaRings("claude", [w("5h", null), w("7d", 12)])), ["7d"])
  // 真实 0% 是有效读数：画成全轨道，不能被当成未知丢掉
  assert.deepEqual(shapes(quotaRings("claude", [w("5h", 0)])), ["5h:arc"])
})

test("两环共存的顺序：订阅窗口在外，日额度在后（内圈刻度环）", () => {
  assert.deepEqual(
    shapes(quotaRings("zcode", [w("bkt-1", 95, "GLM-5.3"), w("5h", 40), w("7d", 88)])),
    ["7d:arc", "bkt-1:ticks"],
  )
  // 只有订阅窗口时保持改造前的内外圈：7 天在外、5 小时在内
  assert.deepEqual(shapes(quotaRings("claude", [w("5h", 40), w("7d", 88)])), ["7d:arc", "5h:arc"])
  // 5h 在、7d 不在：5h 独占外圈
  assert.deepEqual(shapes(quotaRings("zcode", [w("5h", 40), w("bkt-1", 95)])), ["5h:arc", "bkt-1:ticks"])
})

test("超过两个候选按最紧张优先取（未知排后），最多两个环", () => {
  assert.equal(MAX_RING_COUNT, 2)
  // 7d 100% 比 5h 10% 紧张：被挤掉的是 5h，不是日额度
  assert.deepEqual(keys(quotaRings("zcode", [w("5h", 10), w("7d", 100), w("bkt-1", 20)])), ["7d", "bkt-1"])
  // 四个日额度桶：只留最紧张的两个，组内也是紧张的在先
  assert.deepEqual(keys(quotaRings("zcode", [w("bkt-1", 30), w("bkt-2", 40), w("bkt-3", 5), w("bkt-4", 12)])), ["bkt-2", "bkt-1"])
  assert.equal(quotaRings("custom", [w("5h", 10), w("7d", 20), w("1d", 30), w("30d", 40)]).length, 2)
})

test("同一个窗口 key 只画一个环（去重后仍保持顺序）", () => {
  assert.deepEqual(keys(quotaRings("zcode", [w("bkt-1", 10), w("bkt-1", 90)])), ["bkt-1"])
  assert.equal(quotaRings("zcode", [w("bkt-1", 10), w("bkt-1", 90)])[0].window.usedPercent, 10)
})

test("点亮段数换算：0 / 62 / 100 与超限收口", () => {
  assert.equal(TICK_COUNT, 24)
  assert.equal(tickedSegments(0), 0)
  assert.equal(tickedSegments(62), 15) // 0.62 × 24 = 14.88 → 15，与画板「点亮 15 段」一致
  assert.equal(tickedSegments(100), 24)
  // 超过 100 按 100 收口，负数按 0 收口
  assert.equal(tickedSegments(150), 24)
  assert.equal(tickedSegments(100.4), 24)
  assert.equal(tickedSegments(-5), 0)
  // 未知不点亮
  assert.equal(tickedSegments(null), 0)
  assert.equal(tickedSegments(undefined), 0)
  assert.equal(tickedSegments(Number.NaN), 0)
})

test("刻度环几何与画板 A3-13 逐项一致（32px 外径、半径 15、环宽 2）", () => {
  const args = { size: 32, radius: 15, ringWidth: 2, percent: 62 }
  const segments = tickRingSegments(args)
  assert.equal(segments.length, 24)
  assert.equal(segments.filter((segment) => segment.on).length, 15)
  // 12 点那一整段：圆心正上方，段长径向、竖直（不旋转）
  const top = segments[0]
  assert.equal(top.cx, 16)
  assert.equal(top.cy, 1)
  assert.equal(top.width, 1.44) // 弧段环 2 × 0.72
  assert.equal(top.height, 2.6)
  assert.equal(top.angle, 0)
  // 每段 15°，第 6 段落在 3 点方向，长轴转到水平
  const right = segments[6]
  assert.equal(Math.round(right.cx * 1000) / 1000, 31)
  assert.equal(Math.round(right.cy * 1000) / 1000, 16)
  assert.equal(right.angle, 90)
  // 起点标记 1.6（半径 0.8）落在 12 点方向、半径 14.8 处
  assert.deepEqual(tickRingMarker({ size: 32, radius: 15, ringWidth: 2 }), { cx: 16, cy: 1.2, r: 0.8 })
})

test("刻度环 0% 是全轨道、100% 整圈点亮", () => {
  const zero = tickRingSegments({ size: 32, radius: 15, ringWidth: 2, percent: 0 })
  assert.equal(zero.filter((segment) => segment.on).length, 0)
  const full = tickRingSegments({ size: 32, radius: 15, ringWidth: 2, percent: 100 })
  assert.equal(full.filter((segment) => segment.on).length, 24)
  // 未知/超限：未知不点亮，超过 100 收口到满圈
  assert.equal(tickRingSegments({ size: 32, radius: 15, ringWidth: 2, percent: null }).filter((s) => s.on).length, 0)
  assert.equal(tickRingSegments({ size: 32, radius: 15, ringWidth: 2, percent: 250 }).filter((s) => s.on).length, 24)
})

test("刻度环按所在环半径缩放：内圈半径小、刻度跟着缩进", () => {
  const outer = tickRingSegments({ size: 32, radius: 15, ringWidth: 2, percent: 50 })
  const inner = tickRingSegments({ size: 32, radius: 11.5, ringWidth: 2, percent: 50 })
  assert.equal(outer[0].cy, 1)
  assert.equal(inner[0].cy, 4.5) // 16 − 11.5
  // 段长/线宽只随环宽变，不随半径变
  assert.equal(outer[0].height, inner[0].height)
})

test("每段都落在半径上、长轴沿半径（放射状，与画板的 180 − θ 同一根长轴）", () => {
  const segments = tickRingSegments({ size: 32, radius: 15, ringWidth: 2, percent: 50 })
  for (const [index, segment] of segments.entries()) {
    const t = (index * 2 * Math.PI) / 24
    // 段中心在半径 15 的圆上
    assert.ok(Math.abs(Math.hypot(segment.cx - 16, segment.cy - 16) - 15) < 1e-9, `段 ${index} 不在半径上`)
    // SVG 里绕段中心顺时针旋转 angle，段的长轴方向是 (−sin angle, cos angle)；
    // 与径向 (sin t, −cos t) 平行的充要条件是叉积为 0
    const a = (segment.angle * Math.PI) / 180
    const cross = -Math.sin(a) * -Math.cos(t) - Math.cos(a) * Math.sin(t)
    assert.ok(Math.abs(cross) < 1e-9, `段 ${index} 的长轴不是径向`)
  }
})
