import test from "node:test"
import assert from "node:assert/strict"
import { createQueryCache } from "./queryCache.ts"

test("返回页面复用快照，过期后重新显示首次加载", () => {
  const cache = createQueryCache(2, 30_000)
  cache.set("stats", { requests: 0 }, 100)
  assert.deepEqual(cache.get("stats", 30_099), { requests: 0 })
  assert.equal(cache.get("stats", 30_100), null)
})
test("不同页面查询互不污染；缓存有界且覆盖写入更新时效", () => {
  const cache = createQueryCache(2, 30_000)
  cache.set("stats", 1, 0)
  cache.set("automation", 2, 0)
  cache.set("stats", 3, 100)
  cache.set("sessions", 4, 200)
  assert.equal(cache.get("automation", 200), null)
  assert.equal(cache.get("stats", 200), 3)
  assert.equal(cache.get("sessions", 200), 4)
})
