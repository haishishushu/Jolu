import { useEffect, useState } from "react"
import { isTauri, listenEvent } from "@/lib/api"
import { gatewayApi, type GatewayStatusDto } from "@/lib/gatewayApi"
import {
  activityFromLog, dayKey, emptyActivity, mergeSnapshot, nextTickAt, onEnd, onStart, onTick,
  type ActivityState, type RouteEndEvent, type RouteStartEvent,
} from "@/lib/gatewayActivity"

export interface GatewayActivity {
  /** 网关处于转发模式且监听已启动；为 false 时界面不显示任何模型入口元素 */
  relay: boolean
  status: GatewayStatusDto | null
  activity: ActivityState
}

/**
 * 胶囊灵动岛的模型入口数据（设计说明 §8）：
 * 首次读取 gateway_status 与路由流水，之后订阅请求开始 / 结束与接线变化事件。
 * 只在 `enabled`（胶囊样式）时工作；经典样式不订阅、不查询。
 */
export function useGatewayActivity(enabled: boolean): GatewayActivity {
  const [status, setStatus] = useState<GatewayStatusDto | null>(null)
  const [activity, setActivity] = useState<ActivityState>(() => emptyActivity(Date.now()))
  const relay = Boolean(status && status.mode === "relay" && status.enabled)

  // 网关状态：启动时读一次；设置或接线变化时重读
  useEffect(() => {
    if (!enabled || !isTauri) { setStatus(null); return }
    let stopped = false
    const load = () => void gatewayApi.status().then((next) => { if (!stopped) setStatus(next) }).catch(() => { if (!stopped) setStatus(null) })
    load()
    const offs: Array<() => void> = []
    const listen = (promise: Promise<() => void>) => void promise.then((off) => { if (stopped) off(); else offs.push(off) }).catch(() => {})
    listen(listenEvent("settings-changed", load))
    listen(listenEvent("gateway-wiring-changed", load))
    return () => { stopped = true; offs.forEach((off) => off()) }
  }, [enabled])

  // 转发模式开启后：先订阅实时事件，再读流水与今日计数作为快照
  useEffect(() => {
    if (!enabled || !isTauri || !relay) { setActivity(emptyActivity(Date.now())); return }
    let stopped = false
    // 快照返回前到达的结束事件先记下来：后端计数时刻（at）之后结束的才需要补加
    let early: RouteEndEvent[] | null = []
    const offs: Array<() => void> = []
    const listen = (promise: Promise<() => void>) => void promise.then((off) => { if (stopped) off(); else offs.push(off) }).catch(() => {})
    listen(listenEvent<RouteStartEvent>("gateway-route-start", (event) => setActivity((s) => onStart(s, event, Date.now()))))
    listen(listenEvent<RouteEndEvent>("gateway-route", (event) => {
      early?.push(event)
      setActivity((s) => onEnd(s, event, Date.now()))
    }))
    void Promise.all([gatewayApi.routeLog(), gatewayApi.todayCounts().catch(() => null)]).then(([rows, counts]) => {
      if (stopped) return
      const now = Date.now()
      const restored = activityFromLog(rows, now)
      const after = { ok: 0, rejected: 0, failed: 0 }
      // 今日计数以后端统计为准；取不到时退回最近 20 条流水里的今日部分（只是近似）
      let today = restored.today
      if (counts) {
        today = { day: dayKey(now), ok: counts.ok, rejected: counts.rejected, failed: counts.failed }
        for (const event of early ?? []) {
          if (event.ts <= counts.at) continue
          if (event.result === "ok") after.ok += 1
          else if (event.result === "rejected") after.rejected += 1
          else after.failed += 1
        }
      }
      early = null
      setActivity((current) => mergeSnapshot(current, { lastByAgent: restored.lastByAgent, today }, after))
    }).catch(() => { early = null })
    return () => { stopped = true; offs.forEach((off) => off()) }
  }, [enabled, relay])

  // 结果停留与超时收尾：按下一次到期时刻精确定时，不常驻轮询
  const due = nextTickAt(activity)
  useEffect(() => {
    if (due === null) return
    const timer = window.setTimeout(() => setActivity((s) => onTick(s, Date.now())), Math.max(0, due - Date.now()) + 16)
    return () => window.clearTimeout(timer)
  }, [due])

  // 跨零点清空今日统计
  useEffect(() => {
    if (!relay) return
    const now = new Date()
    const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1).getTime()
    const timer = window.setTimeout(() => setActivity((s) => onTick(s, Date.now())), midnight - now.getTime() + 50)
    return () => window.clearTimeout(timer)
  }, [relay, activity.today.day])

  return { relay, status, activity }
}
