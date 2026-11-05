import { useCallback, useContext, useEffect, useRef, useState } from "react"
import { LiveCountsContext } from "@/lib/liveCounts"
import { quotaJustReset, SHOT_MS, SLEEP_AFTER_MS, SURPRISE_TOKENS, type BotShot } from "@/lib/botExpression"
import type { DeltaPhase } from "@/components/island/TokenDelta"

/**
 * 胶囊灵动岛 bot 的「一次性表情」与「睡着」判定。
 * 一次性表情只在真实事件发生的那一刻触发，错过了不回放：
 *   完成   增量提示收起（本段输出结束）
 *   惊讶   单段 Token 从 0 平滑增长并越过 50k（快照一步跳到高位不算新增）
 *   打招呼 展开
 *   额度重置  已用比例从高位回到低位
 */
export function useBotShots(input: {
  deltaPhase: DeltaPhase | undefined
  open: boolean
  /** 各额度窗口的已用百分比，按窗口标识索引 */
  quotaUsed: Record<string, number | null>
  /** 有会话在写入，或增量提示在显示：用于判断 30 分钟无活动 */
  active: boolean
  /** 平台 + 连接：切换连接时额度读数换了来源，只更新基线，不算「额度重置」 */
  identity: string
}) {
  const { deltaPhase, open, quotaUsed, active, identity } = input
  const store = useContext(LiveCountsContext)
  const [shot, setShot] = useState<BotShot | null>(null)
  const [sleepy, setSleepy] = useState(false)
  const shotTimer = useRef(0)

  const fire = useCallback((next: BotShot) => {
    window.clearTimeout(shotTimer.current)
    setShot(next)
    shotTimer.current = window.setTimeout(() => setShot(null), SHOT_MS[next])
  }, [])
  useEffect(() => () => window.clearTimeout(shotTimer.current), [])

  // 展开 → 打招呼
  const wasOpen = useRef(open)
  useEffect(() => {
    if (open && !wasOpen.current) fire("greet")
    wasOpen.current = open
  }, [open, fire])

  // 增量提示收起 → 完成
  const lastPhase = useRef(deltaPhase)
  useEffect(() => {
    if (deltaPhase === "leave" && lastPhase.current !== "leave") fire("happy")
    lastPhase.current = deltaPhase
  }, [deltaPhase, fire])

  // 单段越过 50k → 惊讶。直接订阅追数存储，未越线时不触发任何渲染
  const crossed = useRef(false)
  useEffect(() => {
    if (!store) return
    let previous = store.get("total") ?? 0
    return store.subscribe(() => {
      const value = store.get("total") ?? 0
      if (value === 0) crossed.current = false
      else if (!crossed.current && previous > 0 && previous < SURPRISE_TOKENS && value >= SURPRISE_TOKENS) {
        crossed.current = true
        fire("surprise")
      }
      previous = value
    })
  }, [store, fire])

  // 额度重置：同一个连接的已用比例从高位回到低位
  const lastQuota = useRef({ identity, quotaUsed })
  const quotaSignature = JSON.stringify(quotaUsed)
  useEffect(() => {
    const previous = lastQuota.current
    if (previous.identity === identity && quotaJustReset(previous.quotaUsed, quotaUsed)) fire("reset")
    lastQuota.current = { identity, quotaUsed }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [quotaSignature, identity, fire])

  // 30 分钟没有会话写入就睡着；任何活动重新计时
  useEffect(() => {
    setSleepy(false)
    if (active) return
    const timer = window.setTimeout(() => setSleepy(true), SLEEP_AFTER_MS)
    return () => window.clearTimeout(timer)
  }, [active])

  return { shot, inactiveMs: sleepy ? SLEEP_AFTER_MS : 0 }
}
