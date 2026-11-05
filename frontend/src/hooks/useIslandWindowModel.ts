import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"
import type { DeltaPhase } from "@/components/island/TokenDelta"
import type { IslandData } from "@/components/island/UsageIsland"
import { CLAUDE_QUOTAS, LOCAL_SOURCE, RUNNING_SESSION_TEXT, SESSIONS, TOTAL_DELTA_TEXT } from "@/mock/data"
import { useSettings } from "@/hooks/useSettings"
import { useConnectionKind, useLiveUsage } from "@/hooks/useLiveUsage"
import { useConnections } from "@/hooks/useConnections"
import { shouldRefreshConnection } from "@/lib/refreshScope"
import { useApiUsage, useBalance, useCostEstimate, useQuota } from "@/hooks/useQuota"
import { formatQuotaCountdown, toQuotaWindows } from "@/lib/quotaMap"
import { api, isTauri, listenEvent } from "@/lib/api"
import { islandIdFromSearch, islandProfile } from "@/lib/islandProfile"
import { authQuotaRows } from "@/lib/authQuotaRows"
import type { IslandMode, PlatformId } from "@/types"
import { platformConfig, isFetchOnlyPlatform, platformSupports } from "@/lib/platforms"
import { remainingDisplay } from "@/lib/remainingDisplay"
import { useSourceMetrics } from "@/hooks/useSourceMetrics"
import { useCollectionStatus } from "@/hooks/useCollectionStatus"
import { balanceSnapshotTime, connectionQueries, islandActiveConnection, islandConnectionStatus, islandCost } from "@/lib/islandPresentation"
import { dockQuotaView } from "@/lib/dockQuotaView"
import { carriedDeltaCountKey, carriedDeltaView, latchSessionDelta } from "@/lib/carriedDelta"
import { onUsageChanged } from "@/lib/usageChanges"
import { useGatewayActivity } from "@/hooks/useGatewayActivity"

/** 平台展示名。未知平台原样显示，不臆测 */
const PLATFORM_NAMES: Record<string, string> = {
  claude: "Claude",
  codex: "Codex",
  gemini: "Gemini",
  grok: "Grok",
}

export interface IslandWindowModelInput {
  /** 胶囊样式：模型入口只在胶囊下订阅，经典样式不查询、不监听 */
  capsule: boolean
  /** 当前形态：停靠且未探出时卡片不可见，余额 / 组织用量 / 本机费用暂停查询 */
  mode: IslandMode
  peek: boolean
  /** 原生拖动中：暂停逐帧追数 */
  dragging: boolean
}

/**
 * 经典与胶囊两个容器共用的数据装配。
 *
 * 两种样式读同一份设置、同一批查询；差别只在“何时需要这些数字”：
 * 形态与拖动由容器传进来（`mode` / `peek` / `dragging`），这里据此决定要不要查。
 * 数据表达式与拆分前逐字一致——拆容器不能顺带改数据语义。
 */
export function useIslandWindowModel({ capsule, mode, peek, dragging }: IslandWindowModelInput) {
  const [quotaNow, setQuotaNow] = useState(() => Date.now())

  useEffect(() => {
    const timer = window.setInterval(() => setQuotaNow(Date.now()), 30_000)
    return () => window.clearInterval(timer)
  }, [])

  // 本岛身份：URL 带 clone 参数的是分身，否则是本体；连接与停靠按身份从设置里取
  const { settings } = useSettings()
  const cloneId = useMemo(() => islandIdFromSearch(window.location.search), [])
  const profile = islandProfile(settings, cloneId)
  const islandPlatform = profile.platform
  const live = useLiveUsage(islandPlatform, true, settings.dnd, dragging)
  const collection = useCollectionStatus()
  // 接入方式来自本机真实配置，不写死
  const conn = useConnectionKind(islandPlatform)

  // 额度按连接查；灵动岛只看当前配置平台的那一个连接（§7.3）
  const { connections, loading: connectionsLoading } = useConnections()
  // 灵动岛生效连接：明确选择优先；未选择时自动启用当前平台第一个连接成功的
  // 连接（2026-09-19 鼠鼠定版）。仍不回退到本机账号或唯一候选。
  const active = islandActiveConnection(profile.connection_id, connections, islandPlatform)
  const islandKind = active?.kind ?? profile.kind
  const localQuotaPlatform = (islandPlatform === "grok" || islandPlatform === "zcode") && islandKind === "auth"
  const queries = localQuotaPlatform
    ? { quota: null, balance: null, usage: null }
    : connectionQueries(active)
  // 停靠且未探出时，停靠条只画额度水位：额度照常轮询；余额、组织用量、本机费用与积分
  // 只在卡片里显示，此时暂停查询，探出或展开时立即补查（定时刷新走后端缓存，不额外打上游）。
  const cardsVisible = mode !== "docked" || peek
  const quota = useQuota(queries.quota)
  const grokLocalQuota = useQuota(null, "grok", islandPlatform === "grok" && islandKind === "auth")
  const zcodeLocalQuota = useQuota(null, "zcode", islandPlatform === "zcode" && islandKind === "auth")
  const balance = useBalance(queries.balance, cardsVisible)
  const officialApiUsage = useApiUsage(queries.usage, cardsVisible)
  const localCost = useCostEstimate(islandPlatform, "today", undefined, undefined, undefined, cardsVisible)
  useEffect(() => setQuotaNow(Date.now()), [islandPlatform])
  const nativeMonitor = isFetchOnlyPlatform(islandPlatform as PlatformId) && islandPlatform !== "grok"
  // 本机积分只有原生来源平台在卡片里展示；其他平台不查
  const sourceMetrics = useSourceMetrics(islandPlatform, "today", null, null, quotaNow, nativeMonitor && cardsVisible)
  const remainingPlatform = isFetchOnlyPlatform(islandPlatform as PlatformId)
  const remainingQuota = islandPlatform === "grok" && islandKind === "auth" ? grokLocalQuota
    : islandPlatform === "zcode" && islandKind === "auth" ? zcodeLocalQuota : null
  const remaining = remainingPlatform
    ? remainingDisplay(islandPlatform as PlatformId, islandKind, remainingQuota?.state ?? null)
    : null
  const refreshing = useRef(false)
  const selectionRefreshKey = useRef<string | null>(null)
  const refreshGeneration = useRef(0)
  // 刷新反馈动画的序号：任一入口触发刷新都递增，岛的三种形态据此重放动画
  const [refreshSerial, setRefreshSerial] = useState(0)
  // 刷新是否进行中：驱动绿色流光循环，直到全部查询完成再收尾
  const [refreshActive, setRefreshActive] = useState(false)
  useEffect(() => {
    let disposed = false
    const offs: Array<() => void> = []
    const listen = (promise: Promise<() => void>) =>
      void promise.then((un) => { if (disposed) un(); else offs.push(un) }).catch(console.error)
    // 本平台统计真有变化才更新查询时刻（驱动本机积分重查），续期心跳与连续变化不逐条触发
    offs.push(onUsageChanged(islandPlatform, () => setQuotaNow(Date.now())))
    // 网络额度/余额/用量刷新，多个刷新事件共用同一防重入闸门。
    const refreshNetwork = (scanSessions: boolean) => {
      if (refreshing.current) return
      refreshing.current = true
      const generation = refreshGeneration.current
      setRefreshSerial((value) => value + 1)
      setRefreshActive(true)
      const jobs: Array<Promise<unknown>> = [quota.refresh(), balance.refresh(), officialApiUsage.refresh()]
      if (islandPlatform === "grok" && islandKind === "auth") jobs.push(grokLocalQuota.refresh())
      if (islandPlatform === "zcode" && islandKind === "auth") jobs.push(zcodeLocalQuota.refresh())
      if (scanSessions) jobs.push(api.scanLocalSessions())
      void Promise.all(jobs).catch(console.error).finally(() => {
        if (generation !== refreshGeneration.current) return
        refreshing.current = false
        setRefreshActive(false)
      })
    }
    // 右键“立即刷新”：连同本机会话一起重扫。
    listen(listenEvent("island-refresh", () => refreshNetwork(true)))
    // 连接管理“更新”凭证/网关、平台切换、监听器发现凭证变化：只需重查网络额度，
    // 会话由采集器/watcher 单独维护，不必在此重扫。
    listen(listenEvent<string | null>("refresh-requested", (connectionId) => {
      if (shouldRefreshConnection(connectionId, active?.id)) refreshNetwork(false)
    }))
    // 主面板「启用」完成后 settings-changed 与连接列表可能先后到达。
    // 等新连接实际成为灵动岛生效连接，再用它强制刷新额度与本机会话。
    const selectedKey = active?.id && profile.connection_id === active.id
      ? `${active.id}:${active.status}` : null
    if (selectionRefreshKey.current !== selectedKey) {
      selectionRefreshKey.current = selectedKey
      refreshGeneration.current += 1
      refreshing.current = false
      if (selectedKey) refreshNetwork(true)
      else setRefreshActive(false)
    }
    return () => { disposed = true; offs.forEach((off) => off()) }
  }, [active?.id, active?.status, profile.connection_id, islandPlatform, islandKind, queries.quota, quota.refresh, grokLocalQuota.refresh, zcodeLocalQuota.refresh, balance.refresh, officialApiUsage.refresh])
  const displayQuota = remainingQuota ?? quota
  const quotaWindows =
    displayQuota.state?.state === "ok" ? toQuotaWindows(displayQuota.state.windows, quotaNow) : null

  // 停靠条的额度形态：5h/7d 真实窗口 / 已确认无双窗口的彩虹双轨 / 未知空轨道。
  // 岛卡片与停靠条共用同一份查询结果，判定收敛在 dockQuotaView 一处。
  const quotaView = dockQuotaView({
    kind: islandKind,
    connected: localQuotaPlatform || Boolean(active && active.status !== "paused"),
    quotaQueried: localQuotaPlatform || Boolean(queries.quota),
    quotaLoading: displayQuota.loading,
    quotaState: displayQuota.state?.state ?? null,
    windows: quotaWindows,
  })
  const dockQuotas = quotaView.type === "windows" ? authQuotaRows(quotaView.windows) : authQuotaRows(undefined)

  // 采集异常按平台定界：只有当前灵动岛平台的来源出错时，会话状态才不可判定。
  // 其他平台的文件错误照常在来源说明里展示，但不把本平台会话打成「状态未知」。
  const sourceFailed = Boolean(
    collection.status && !collection.status.ok && collection.status.failed_sources.includes(islandPlatform),
  )

  // 胶囊的「正在写入」与「本轮增量」：来源提供可信运行状态（running 会话）才算工作中；
  // 采集源出错时状态不可判定，不当成工作中。增量取后端 running_tokens（只含 running 会话），
  // 拿不到就是 null——胶囊显示未知，不补 0、不拿示例数字。
  const islandSessions = sourceFailed
    ? live.sessions.map((session) => ({ ...session, state: "unknown" as const }))
    : live.sessions
  const working = !sourceFailed && live.sessions.some((session) => session.state === "running")
  const liveDelta = sourceFailed ? null : live.runningDelta
  /* 胶囊收缩态尾部增量段的 A + B 组合（§5.7）：锁存值 ?? 今日累计。
     A 本轮终值锁存：进行中记下 running_tokens 的最新非零读数，结束后停在这个读数上，不跳变、
       不重新起跳；新一轮从 0 / 较小值往上走时被新值覆盖。锁存只存在这一处。
     B 今日累计兜底：应用重启（内存态锁存丢失）后、或当天第一笔还没落库时，显示 today_tokens。
     清空时机：今日累计归 0 / null（后端跨天重置、切换平台、重启后的初始快照）与手动刷新。
     不在前端判日期，也不在前端累加 Token。 */
  // 判定源放 ref：同一帧里 React 可能多次执行渲染体（严格模式、并发重放），
  // 按 ref 递推才能让每一步都拿到上一次的锁存值，不会互相覆盖成旧值。
  const latchedRef = useRef<number | null>(null)
  // 渲染用镜像是 state：锁存值变化要能自己触发一次更新，不依赖别处的状态碰巧跟着变。
  // 用 layout effect 同步，提交后、绘制前落定，不会出现一帧旧读数。
  const [latchedDelta, setLatchedDelta] = useState<number | null>(null)
  // 手动刷新（右键「立即刷新」、连接更新、平台切换）都走现有刷新入口：锁存一起清，
  // 本轮仍在写的会话会在下一次推送重新锁存；清掉后没有本轮值就回落到今日累计。
  const resetMark = useRef(refreshSerial)
  if (resetMark.current !== refreshSerial) {
    resetMark.current = refreshSerial
    latchedRef.current = null
  }
  const nextLatched = latchSessionDelta({
    previous: latchedRef.current,
    working,
    runningDelta: live.runningDelta,
    todayTokens: live.todayTokens,
  })
  if (nextLatched !== latchedRef.current) latchedRef.current = nextLatched
  useLayoutEffect(() => {
    setLatchedDelta((current) => (current === nextLatched ? current : nextLatched))
  }, [nextLatched])
  const carried = carriedDeltaView({
    latched: latchedDelta,
    todayTokens: live.todayTokens,
    sourceFailed,
  })
  const carriedDelta = carried.value
  const carriedSource = carried.source
  const carriedCountKey = carriedDeltaCountKey(carried.source, working)

  const data: IslandData = isTauri || live.live
    ? {
        platform: islandPlatform as PlatformId,
        platformName: platformConfig(islandPlatform as PlatformId).name,
        localMetrics: nativeMonitor ? { token: live.todayTokenText, credits: sourceMetrics.data?.credits?.toLocaleString("zh-CN", { maximumFractionDigits: 4 }) ?? null, message: "本机留存记录；上报积分不等于余额。" } : undefined,
        remainingMetrics: remaining ? {
          rows: remaining.rows.map((row) => ({ label: row.label, value: row.value, reset: formatQuotaCountdown(row.resetsAt, quotaNow) })),
          message: remainingQuota?.stale ? "额度可能已过期，请刷新" : remaining.reason,
          source: remaining.source,
          updated: remainingQuota?.fetchedAt?.toLocaleTimeString("zh-CN", { hour12: false }) ?? null,
          localToken: nativeMonitor ? live.todayTokenText : null,
          localCredits: nativeMonitor && platformSupports(islandPlatform as PlatformId, "credits")
            ? sourceMetrics.data?.credits?.toLocaleString("zh-CN", { maximumFractionDigits: 4 }) ?? null
            : null,
        } : undefined,
        kind: islandKind,
        // Auth 保持额度布局；未知值不回退成 API 用量。
        quotas: islandKind === "auth" && islandPlatform === "codex" && quotaWindows
          ? authQuotaRows(quotaWindows, "账号额度接口未返回该窗口")
          : quotaWindows ?? undefined,
        quotaUnavailable: !quotaWindows,
        connected: localQuotaPlatform || Boolean(active && active.status !== "paused"),
        connectionId: active?.id ?? null,
        // 只有真正发起了额度查询、且结果是失败类状态时才算查询失败；不支持与加载中不算
        queryFailed: (localQuotaPlatform || Boolean(queries.quota)) && !displayQuota.loading
          && Boolean(displayQuota.state) && !["ok", "unsupported"].includes(displayQuota.state!.state),
        quotaMessage: active?.status === "paused" ? "连接已断开，请在设置中重新连接"
          : quota.stale ? "额度可能已过期，请刷新"
          : quota.loading && !quotaWindows ? "正在查询额度…"
          : quota.state && quota.state.state !== "ok" ? quota.state.reason
          : islandPlatform !== "grok" && quotaWindows && !quotaWindows.some(w => w.key === "5h") ? "当前连接未提供 5h 额度"
          : !quotaWindows && islandKind === "auth" ? "暂无额度信息" : undefined,
        apiToday: {
          tokenLabel: officialApiUsage.state?.state === "ok" ? "组织今日 Token" : "本机今日 Token",
          token: officialApiUsage.state?.state === "ok"
            ? `${officialApiUsage.state.total_tokens.toLocaleString("en-US")} Token`
            : live.todayTokenText,
          ...islandCost(officialApiUsage.state?.state === "ok" ? officialApiUsage.state.cost_usd : null, localCost),
        },
        balanceText: queries.balance && balance.state?.state === "ok" ? `${balance.state.balance.toFixed(2)} ${balance.state.currency}` : undefined,
        balanceLevel: queries.balance && balance.state?.state === "ok" ? (balance.state.balance <= 0 ? "empty" : "healthy") : undefined,
        balanceTimeText: queries.balance && balance.state?.state === "ok" ? balanceSnapshotTime(balance.fetchedAt) ?? undefined : undefined,
        apiMessage: islandKind !== "api" ? undefined : active?.status === "paused" ? "连接已断开，当前仅显示本机统计"
          : !active ? "请先在连接管理中启用连接，当前仅显示本机统计"
          : queries.balance ? balance.stale ? "余额可能已过期，请刷新" : balance.state && balance.state.state !== "ok" ? balance.state.reason : undefined
          : officialApiUsage.state?.state === "ok" ? officialApiUsage.state.cost_reason ?? undefined
          : officialApiUsage.state?.reason ?? (officialApiUsage.loading ? "正在查询组织用量，暂显示本机统计" : undefined),
        todayTokenText: live.todayTokenText,
        connectionLabel: active?.name ?? (profile.connection_id
          ? `${profile.connection_name ?? "所选连接"}（不可用）`
          : conn.label ?? undefined),
        sessions: islandSessions,
        sessionStatusUnknown: sourceFailed,
        sessionCountText: sourceFailed && live.sessions.length > 0
          ? `${live.sessions.length} 个会话状态未知`
          : live.activityCountText,
        sessionSectionTitle: sourceFailed && live.sessions.length > 0
          ? `${live.sessions.length} 个会话状态未知`
          : live.activityCountText ?? undefined,
        // 逐帧追数由叶子组件经 LiveCountsContext 订阅，岛体本身不随每帧重渲染
        deltaCountKey: "total",
        deltaPhase: live.deltaPhase,
        // 胶囊收缩态用：是否有会话正在写入、本轮已写入的 Token 增量（未知为 null）。
        // 经典样式不读这两个字段。
        working,
        liveDelta,
        // 胶囊尾部增量段：锁存值 ?? 今日累计（§5.7），以及它来自哪一边（决定文案）。
        carriedDelta,
        carriedSource,
        carriedCountKey,
        status: nativeMonitor && active?.status === "connected" ? { tone: "success", label: "本机监控" } : islandConnectionStatus(active?.status ?? null, false, Boolean(profile.connection_id)),
        sourceText: `${collection.status && !collection.status.ok ? `采集异常：${collection.status.errors.join("；")} · ` : ""}${officialApiUsage.state?.state === "ok" ? `API 用量来源：${officialApiUsage.state.source} · ` : ""}统计来源：本机 ${PLATFORM_NAMES[islandPlatform] ?? islandPlatform} 会话记录（无法按账号区分）· ${live.activityKind === "running" ? "按本轮开始／结束事件及会话存活状态判断；思考、等待输入及压缩期间保留会话与 Token" : islandPlatform === "zcode" ? `${live.activeWindowSeconds} 秒内有新记录视为最近活跃；完成 / 失败 / 运行中按本机库的请求状态标注` : `${live.activeWindowSeconds} 秒内有新记录视为最近活跃`}`,
      }
    : {
        platform: "claude",
        platformName: "Claude",
        kind: "auth",
        quotas: CLAUDE_QUOTAS,
        sessions: SESSIONS,
        sessionCountText: RUNNING_SESSION_TEXT,
        deltaText: TOTAL_DELTA_TEXT,
        deltaPhase: "hold" as DeltaPhase,
        todayTokenText: "12.84M",
        status: { tone: "success", label: "已连接" },
        sourceText: `统计来源：${LOCAL_SOURCE.name}（${LOCAL_SOURCE.scope}）· 设计示例`,
      }

  // 模型入口只在胶囊样式下订阅；经典样式不查询、不监听
  const gateway = useGatewayActivity(capsule)

  return {
    data,
    settings,
    profile,
    cloneId,
    islandPlatform,
    connections,
    connectionsLoading,
    active,
    refreshActive,
    refreshSerial,
    /** 刷新防重入闸门：右键菜单据此显示“正在刷新” */
    refreshingRef: refreshing,
    quota,
    balance,
    quotaView,
    dockQuotas,
    quotaWindows,
    live,
    gateway,
  }
}
