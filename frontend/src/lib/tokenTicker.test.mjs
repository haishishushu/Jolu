import assert from "node:assert/strict"
import test from "node:test"
import { formatShort, tickerNext, tickerStart } from "./tokenTicker.ts"

test("节拍：先眼睛 2 秒，之后 Token 3 秒、眼睛 4 秒循环", () => {
  assert.deepEqual(tickerStart(), { face: "eyes", delay: 2000 })
  assert.deepEqual(tickerNext("eyes"), { face: "tick", delay: 3000 })
  assert.deepEqual(tickerNext("tick"), { face: "eyes", delay: 4000 })
})

test("短格式最多 4 个字符", () => {
  const cases = [[0, "0"], [860, "860"], [999, "999"], [1000, "1.0k"], [8200, "8.2k"], [9960, "10k"], [17000, "17k"],
    [170000, "170k"], [999400, "999k"], [999600, "1.0M"], [1_200_000, "1.2M"], [12_000_000, "12M"], [999_600_000, "1.0B"]]
  for (const [n, text] of cases) {
    assert.equal(formatShort(n), text, String(n))
    assert.ok(formatShort(n).length <= 4, `${n} → ${formatShort(n)}`)
  }
  assert.equal(formatShort(-5), "0")
})
