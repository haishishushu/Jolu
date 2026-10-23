import assert from "node:assert/strict"
import test from "node:test"
import { ZOOM_STEPS, appearancePreview, applyAppearance, effectiveReduceMotion, normalizeDensity, normalizeZoom, reduceMotionHint, shouldReduceMotion } from "./appearance.ts"

/** 只实现 applyAppearance 用到的 classList 方法 */
function fakeRoot(...initial) {
  const set = new Set(initial)
  return {
    classes: set,
    classList: {
      contains: (name) => set.has(name),
      toggle: (name, force) => {
        const on = force ?? !set.has(name)
        if (on) set.add(name)
        else set.delete(name)
        return on
      },
    },
  }
}

test("缩放档位与画板 88 / D-21 一致", () => {
  assert.ok(ZOOM_STEPS.includes(100))
  assert.deepEqual([...ZOOM_STEPS], [80, 90, 100, 110, 125, 150])
  assert.equal(normalizeZoom(125), 125)
  assert.equal(normalizeZoom(115), 100, "不在档位里的旧值回到 100%")
  assert.equal(normalizeDensity("compact"), "compact")
  assert.equal(normalizeDensity("bogus"), "comfortable")
})

test("applyAppearance 在 <html> 上加、去密度与减少动效两个 class", () => {
  const root = fakeRoot("dark")
  applyAppearance(root, "compact", true)
  assert.deepEqual([...root.classes].sort(), ["dark", "density-compact", "reduce-motion"])
  applyAppearance(root, "comfortable", false)
  assert.deepEqual([...root.classes], ["dark"], "关闭时只去掉自己的 class，不碰主题")
  applyAppearance(root, "comfortable", true)
  assert.deepEqual([...root.classes].sort(), ["dark", "reduce-motion"])
})

test("灵动岛类窗口不受密度与减少动效影响", () => {
  const root = fakeRoot("island-window")
  applyAppearance(root, "compact", true)
  assert.deepEqual([...root.classes], ["island-window"])
})

test("减少动效：系统偏好或 <html> 上的开关任一生效", () => {
  assert.equal(shouldReduceMotion(false, fakeRoot()), false)
  assert.equal(shouldReduceMotion(true, fakeRoot()), true)
  assert.equal(shouldReduceMotion(false, fakeRoot("reduce-motion")), true)
})

test("实际是否减少动效：系统偏好或开关任一开启（D-23）", () => {
  assert.equal(effectiveReduceMotion(false, false), false)
  assert.equal(effectiveReduceMotion(true, false), true)
  assert.equal(effectiveReduceMotion(false, true), true)
  assert.equal(effectiveReduceMotion(true, true), true)
})

test("系统开启减少动画时才在开关下方显示说明", () => {
  assert.equal(reduceMotionHint(true), "系统已开启减少动画，主窗口会同时生效。")
  assert.equal(reduceMotionHint(false), null)
})

test("预览区右侧显示「<缩放>% · 舒适/紧凑」", () => {
  assert.equal(appearancePreview(100, "comfortable"), "100% · 舒适")
  assert.equal(appearancePreview(125, "compact"), "125% · 紧凑")
})
