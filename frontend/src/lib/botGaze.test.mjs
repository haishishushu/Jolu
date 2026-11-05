import assert from "node:assert/strict"
import test from "node:test"
import { gazeAmp, gazeBase, gazeSmooth, gazeStep, gazeTarget, GAZE_MAX_X, GAZE_MAX_Y } from "./botGaze.ts"

const face = { cx: 100, cy: 100, width: 28 }
const opt = { base: { x: 0, y: 0 }, amp: 1 }

test("方向与指针一致", () => {
  const t = gazeTarget(face, { x: 200, y: 140 }, opt)
  const cos = (t.x * 100 + t.y * 40) / (Math.hypot(t.x, t.y) * Math.hypot(100, 40))
  assert.ok(cos > 0.9)
})

test("偏移不超过水平 13% 垂直 10%", () => {
  const t = gazeTarget(face, { x: 5000, y: 5000 }, opt)
  assert.ok(Math.abs(t.x) <= GAZE_MAX_X + 1e-9 && Math.abs(t.y) <= GAZE_MAX_Y + 1e-9)
  const far = gazeTarget(face, { x: -5000, y: 100 }, opt)
  assert.ok(Math.abs(far.x - -GAZE_MAX_X) < 1e-6)
})

test("距离越近偏移越小，指针在脸上不偏移", () => {
  const near = gazeTarget(face, { x: 110, y: 100 }, opt)
  const far = gazeTarget(face, { x: 180, y: 100 }, opt)
  assert.ok(near.x < far.x)
  assert.deepEqual(gazeTarget(face, { x: 100, y: 100 }, opt), { x: 0, y: 0 })
})

test("没有指针时只剩表情偏移，幅度为 0 时不跟随", () => {
  assert.deepEqual(gazeTarget(face, null, { base: { x: 0.2, y: 0 }, amp: 1 }), { x: 0.2, y: 0 })
  assert.deepEqual(gazeTarget(face, { x: 500, y: 500 }, { base: { x: 0, y: 0 }, amp: 0 }), { x: 0, y: 0 })
})

test("睡着、偷看、被拖动按幅度减弱", () => {
  assert.equal(gazeAmp("sleep"), 0.3)
  assert.equal(gazeAmp("peek"), 0.4)
  assert.equal(gazeAmp("drag"), 0.6)
  assert.equal(gazeAmp("idle"), 1)
  const full = gazeTarget(face, { x: 5000, y: 100 }, opt)
  const sleepy = gazeTarget(face, { x: 5000, y: 100 }, { base: { x: 0, y: 0 }, amp: gazeAmp("sleep") })
  assert.ok(Math.abs(sleepy.x - full.x * 0.3) < 1e-9)
})

test("贴边偷看的基础偏移朝屏幕内侧", () => {
  assert.ok(gazeBase("peek", "left").x > 0)
  assert.ok(gazeBase("peek", "right").x < 0)
  assert.deepEqual(gazeBase("idle", "left"), { x: 0, y: 0 })
})

test("插值逼近目标并最终落稳；减少动效一步到位", () => {
  let cur = { x: 0, y: 0 }
  const target = { x: 0.1, y: 0.05 }
  let settled = false
  for (let i = 0; i < 60 && !settled; i++) ({ next: cur, settled } = gazeStep(cur, target, gazeSmooth(false)))
  assert.equal(settled, true)
  assert.deepEqual(cur, target)
  assert.equal(gazeStep({ x: 0, y: 0 }, target, gazeSmooth(true)).settled, true)
})
