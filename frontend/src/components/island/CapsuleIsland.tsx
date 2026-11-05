import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { Bot, type BotRing } from "./Bot"
import { AnimatedTokens } from "./TokenDelta"
import { SessionList } from "./SessionList"
import { SourceFooter, type IslandData } from "./UsageIsland"
import { LazyConnectionSwitcher } from "./LazyConnectionSwitcher"
import { AgentStack, GatewaySection, RelayRoute, wiredAgents } from "./CapsuleGateway"
import { useBotShots } from "@/hooks/useBotShots"
import type { GatewayActivity } from "@/hooks/useGatewayActivity"
import { botExpression, BOT_NAMES } from "@/lib/botExpression"
import { authQuotaRows } from "@/lib/authQuotaRows"
import { listenEvent } from "@/lib/api"
import { islandIdFromSearch } from "@/lib/islandProfile"
import { reducedMotion } from "@/lib/motion"
import { cn } from "@/lib/utils"
import type { QuotaWindow } from "@/types"
import "./capsule.css"

/**
 * 胶囊灵动岛（Jolu.pen 模块 A3，设计说明 docs/specs/2026-11-04-capsule-island-design.md）。
 * 与经典样式共用同一份 IslandData，只换展示；样式由设置里的 island_style 决定。
 *
 * 形变完全按 output/island-prototype 的 index.html + style.css：
 * 外壳自己插值宽、高、圆角（.42s 弹簧），内容分层淡入并逐段错开；
 * 迷你胶囊、完整胶囊、展开卡片是同一个外壳的三种尺寸，不换组件、不等窗口。
 * 原生窗口只是跟着外壳的目标尺寸走：变大立即申请（窗口总比外壳先到位），
 * 变小等外壳缩完再收，避免形变被窗口边缘裁掉。
 */
const CAP_H = 40
const EXPANDED_W = 380
/** 与 .cap-shell 的 .42s 弹簧同长；缩小时窗口要等形变结束才收 */
const SHRINK_HOLD_MS = 460
const LABEL: Record<string, string> = { "5h": "5 小时", "7d": "7 天" }

type CapsuleView = "card" | "capsule" | "mini"

const windowLabel = (q: QuotaWindow) => LABEL[q.key] ?? q.windowName
const resetText = (q: QuotaWindow) => q.resetCountdown?.replace(/^还剩\s*/, "") ?? null

/** 环色：达到 80% 转橙、用尽转红；其余 5h 青、7d 绿（与现有额度判定同一阈值） */
function ringColor(q: QuotaWindow): string {
  const p = q.usedPercent
  if (p !== null && p >= 100) return "var(--pn-capsule-red)"
  if (p !== null && p >= 80) return "var(--pn-capsule-orange)"
  return q.key === "5h" ? "var(--pn-capsule-cyan)" : "var(--pn-capsule-green)"
}

export function CapsuleIsland({
  data, open, mini = false, onOpenChange, connectionSwitcher, connectionMenuOpen = false, dragging = false, refreshing = false, dockEdge = null, gateway = null, zoom = 1, onHoverChange,
}: {
  data: IslandData
  /** 模型入口：只有转发模式且监听已启动时才有值 */
  gateway?: GatewayActivity | null
  /** 贴边迷你态：同一个外壳缩到 bot 大小，悬停 400ms 形变回完整胶囊（原型 ui.js 的 miniHover） */
  mini?: boolean
  open: boolean
  onOpenChange: (open: boolean) => void
  connectionSwitcher?: ReactNode
  /** 连接下拉菜单展开中：卡片至少 240 高，让原生窗口给菜单留出空间 */
  connectionMenuOpen?: boolean
  dragging?: boolean
  refreshing?: boolean
  dockEdge?: "top" | "bottom" | "left" | "right" | null
  /** 外层容器的 CSS zoom，用于把停靠可用高度换算回内容像素 */
  zoom?: number
  /** 指针进出外壳：胶囊自己决定 400ms 探出，停靠时机不受经典样式影响 */
  onHoverChange?: (over: boolean) => void
}) {
  // ---------- 由 IslandData 推出展示内容 ----------
  // 原型里只有订阅（Auth）才画额度环；API Key 不画环，显示今日 Token 与费用。
  // API 连接也可能返回套餐窗口，但胶囊样式按原型不把它们画成环（否则会出现只有轨道的「白环」）。
  const showQuotas = data.kind === "auth" && !data.unavailable && !data.remainingMetrics && !data.localMetrics
  const quotaRows = useMemo(() => (data.platform === "grok" ? data.quotas ?? [] : authQuotaRows(data.quotas)).slice(0, 2), [data.platform, data.quotas])
  // 外圈在前：7 天在外、5 小时在内
  const ringWindows = useMemo(() => (showQuotas ? [...quotaRows].reverse() : []), [showQuotas, quotaRows])
  const ringsFor = (aria: boolean): BotRing[] => ringWindows.map((q) => ({
    p: q.usedPercent, color: ringColor(q), warn: q.usedPercent !== null && q.usedPercent >= 80,
    aria: aria ? `${windowLabel(q)}额度${q.usedPercent === null ? "未知" : `已用 ${Math.round(q.usedPercent)}%`}${resetText(q) ? `，${resetText(q)} 后重置` : ""}。按 Enter 固定显示` : undefined,
  }))
  const quotaUsed = useMemo(() => (showQuotas ? Object.fromEntries(quotaRows.map((q) => [q.key, q.usedPercent])) : {}), [showQuotas, quotaRows])

  const running = data.sessions?.filter((s) => s.state === "running").length ?? 0
  // 只有来源提供可信运行状态（running）时才算「工作中」，不把最近活跃当成正在写入
  const working = !data.sessionStatusUnknown && running > 0
  const showDelta = data.deltaPhase !== undefined && data.deltaPhase !== "idle"
  const { shot, inactiveMs } = useBotShots({
    deltaPhase: data.deltaPhase, open, quotaUsed, active: working || showDelta,
    identity: `${data.platform}:${data.kind}:${data.connectionId ?? ""}`,
  })

  // ---------- 模型入口 ----------
  const relayOn = Boolean(gateway?.relay)
  const relay = relayOn ? gateway!.activity.shown : null
  const agents = useMemo(() => (relayOn ? wiredAgents(gateway!.status, gateway!.activity) : []), [relayOn, gateway])
  const drifted = agents.filter((w) => w.drift).length
  // 绿点、「n 分钟前」需要随时间刷新；只在有转发数据时每 30 秒刷新一次
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!relayOn) return
    setNow(Date.now())
    const timer = window.setInterval(() => setNow(Date.now()), 30_000)
    // 转发成功后头像右下角的绿点只留 10 秒（与原型一致），到点再刷新一次
    const dot = relay?.phase === "ok" ? window.setTimeout(() => setNow(Date.now()), 10_100) : 0
    return () => { window.clearInterval(timer); window.clearTimeout(dot) }
  }, [relayOn, relay?.requestId, relay?.phase])

  const [hover, setHover] = useState(false)
  const [focusRing, setFocusRing] = useState<number | null>(null)
  const [pinRing, setPinRing] = useState<number | null>(null)
  const { expr, reason } = botExpression({
    connected: data.connected ?? data.status?.tone === "success",
    queryFailed: showQuotas && Boolean(data.queryFailed),
    quotaUsed: quotaRows.map((q) => q.usedPercent),
    dragging, refreshing, working, shot, inactiveMs: relay ? 0 : inactiveMs, docked: Boolean(dockEdge), hover,
    relay: relay ? (relay.phase === "failed" ? "rejected" : relay.phase) : null,
  })
  // 逐帧数值来自追数存储；没有追数键（浏览器预览的示例数据）时不轮播，避免显示假的 0
  const ticker = working && showDelta && data.deltaCountKey ? { countKey: data.deltaCountKey, label: "本段 Token", sub: `${running} 个会话写入中` } : null

  const focused = pinRing ?? focusRing
  const focusWindow = focused !== null ? ringWindows[focused] : undefined
  const display: [string, string, string?] | null = focusWindow && focusWindow.usedPercent !== null
    ? [`${Math.round(focusWindow.usedPercent)}%`, windowLabel(focusWindow), resetText(focusWindow) ? `${resetText(focusWindow)} 后重置` : undefined]
    : null

  // ---------- 三种形态的尺寸（与原型 .island / .island.mini / .island.open 完全一致） ----------
  const view: CapsuleView = open ? "card" : mini ? "mini" : "capsule"
  const mainRef = useRef<HTMLSpanElement>(null)
  const exRef = useRef<HTMLDivElement>(null)
  const shellRef = useRef<HTMLDivElement>(null)
  const reserveRef = useRef<HTMLDivElement>(null)
  const [mainW, setMainW] = useState(0)
  const [exH, setExH] = useState(0)
  const [ready, setReady] = useState(false)
  // 首帧量完之前不改预留盒子，避免刚挂载时排队一次无意义的「缩小等待」
  const measured = useRef(false)

  useLayoutEffect(() => {
    const main = mainRef.current, ex = exRef.current
    if (!main || !ex) return
    const measure = () => {
      // 收缩层在迷你态被移出布局，但 getBoundingClientRect 仍是内容的自然宽度
      setMainW(Math.ceil(main.getBoundingClientRect().width))
      setExH(Math.ceil(ex.offsetHeight))
    }
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(main)
    observer.observe(ex)
    const frame = requestAnimationFrame(() => { measured.current = true; setReady(true) })
    return () => { observer.disconnect(); cancelAnimationFrame(frame) }
  }, [])

  // 窗口高度上限（停靠在屏幕边缘时由后端给出工作区高度）：卡片超出时整卡改为内部滚动
  const [maxH, setMaxH] = useState<number | null>(null)
  useEffect(() => {
    const onLimit = (event: Event) => {
      const limit = (event as CustomEvent<number | null>).detail
      const shell = shellRef.current
      if (limit === null || !shell) { setMaxH(null); return }
      const bounds = shell.getBoundingClientRect()
      const scale = zoom || 1
      setMaxH(Math.max(CAP_H, Math.floor((limit - bounds.top) / scale) - 16))
    }
    window.addEventListener("island-size-limit", onLimit)
    return () => window.removeEventListener("island-size-limit", onLimit)
  }, [zoom])
  // 自由态也不超过屏幕可用高度（约等于 screen.availHeight），避免卡片伸出屏幕底部
  const cap = Math.min(maxH ?? Infinity, Math.max(CAP_H, ((window.screen?.availHeight ?? 900) - 120) / (zoom || 1)))
  const fullCardH = exH + 2
  const cardH = Math.max(CAP_H, Math.min(fullCardH, cap))
  const scroll = view === "card" && fullCardH > cap

  // 完整胶囊宽 = 左内边距 3 + bot 32 + 间距 10 + 内容 + 右内边距 12，再加 1px 边框 ×2
  const capsuleW = Math.max(38, mainW + 59)
  // 迷你胶囊：2 + bot 26 + 8（再加工作中的脉冲点 6+6），再加边框
  const miniW = (working ? 48 : 36) + 2
  const shellW = view === "card" ? EXPANDED_W : view === "mini" ? miniW : capsuleW
  const shellH = view === "card" ? cardH : view === "mini" ? 32 : CAP_H
  const radius = view === "card" ? 28 : view === "mini" ? 16 : 20

  // 预留盒子决定原生窗口的大小。变大立刻跟上；变小等外壳缩完再收，
  // 这样缩回时外壳不会被窗口边缘切掉，长出来时窗口总比外壳先到位。
  const [reserve, setReserve] = useState(() => ({
    w: open ? EXPANDED_W : mini ? miniW : capsuleW,
    h: open ? Math.max(CAP_H, fullCardH || CAP_H) : mini ? 32 : CAP_H,
  }))
  const previousTarget = useRef({ w: shellW, h: shellH })
  // 形变期间关掉外壳的 backdrop-filter：窗口每一帧都在改尺寸，
  // blur(30px) saturate(1.6) 每帧都要把整块重栅格化一遍，贴边弹出就掉帧。
  // 外壳本身是 90% 白底，关掉模糊几乎看不出差别。
  const [shapeAnim, setShapeAnim] = useState(false)
  useEffect(() => {
    setShapeAnim(true)
    const timer = window.setTimeout(() => setShapeAnim(false), 380)
    return () => window.clearTimeout(timer)
  }, [shellW, shellH])
  useLayoutEffect(() => {
    const previous = previousTarget.current
    previousTarget.current = { w: shellW, h: shellH }
    const apply = () => setReserve((current) => (current.w === shellW && current.h === shellH ? current : { w: shellW, h: shellH }))
    if (!measured.current || (shellW >= previous.w && shellH >= previous.h)) {
      apply()
      return
    }
    const timer = window.setTimeout(apply, reducedMotion() ? 0 : SHRINK_HOLD_MS)
    return () => window.clearTimeout(timer)
  }, [shellW, shellH])

  // 停靠时外壳贴住停靠的那条边：窗口先变大、后形变时内容不会被窗口边缘带着跑
  const anchor = dockEdge === "bottom" ? "items-end justify-center"
    : dockEdge === "left" ? "items-center justify-start"
    : dockEdge === "right" ? "items-center justify-end"
    : "items-start justify-center"

  // ---------- 交互 ----------
  const collapse = useCallback(() => { setPinRing(null); setFocusRing(null); onOpenChange(false) }, [onOpenChange])
  useEffect(() => { if (!open) { setPinRing(null); setFocusRing(null) } }, [open])
  const leaveTimer = useRef(0)
  const onRingHover = (index: number | null) => {
    window.clearTimeout(leaveTimer.current)
    if (index !== null) setFocusRing(index)
    else leaveTimer.current = window.setTimeout(() => setFocusRing(null), 300)
  }
  useEffect(() => () => window.clearTimeout(leaveTimer.current), [])
  const togglePin = (index: number) => setPinRing((current) => (current === index ? null : index))

  // Esc：先取消固定，再收回。灵动岛窗口点击后才有焦点，挂到 window 上才不会因为焦点在卡片里而失效。
  // 连接下拉打开时先让下拉自己处理 Esc，再按一次才收回。
  useEffect(() => {
    if (!open) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || connectionMenuOpen) return
      if (pinRing !== null) setPinRing(null)
      else collapse()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [open, pinRing, collapse, connectionMenuOpen])

  // 点卡片外收回（对应原型的 document pointerdown）：桌面端点别的窗口由下面的失焦处理
  useEffect(() => {
    if (!open || connectionMenuOpen) return
    const onDown = (event: PointerEvent) => {
      const target = event.target as HTMLElement | null
      if (!target) return
      if (target.closest("[data-capsule-island]")) return
      // 下拉菜单、弹层与浏览器预览的控制面板不算「卡片外」
      if (target.closest("[role=menu], [role=listbox], [role=dialog], [data-radix-popper-content-wrapper], .panel")) return
      collapse()
    }
    document.addEventListener("pointerdown", onDown, true)
    return () => document.removeEventListener("pointerdown", onDown, true)
  }, [open, connectionMenuOpen, collapse])

  // 点到别的窗口：桌面端拿不到窗口外的事件，用失焦代替。
  // 右键菜单是单独的窗口，打开时灵动岛也会失焦，不能因此把卡片收起来。
  // 拖动整窗期间窗口可能短暂失焦（原生拖动会改变前台焦点），那不是「点到卡片外」，
  // 不能因此把展开的卡片收回，否则一拖就变成先收回再拖。
  const dragRef = useRef(dragging)
  dragRef.current = dragging

  useEffect(() => {
    if (!open || connectionMenuOpen) return
    let menuAt = 0
    let disposed = false
    let off: (() => void) | null = null
    const onMenu = () => { menuAt = Date.now() }
    const onBlur = () => { if (!dragRef.current && Date.now() - menuAt > 800) collapse() }
    window.addEventListener("island-menu-open", onMenu)
    window.addEventListener("blur", onBlur)
    // 原生窗口失焦：点主窗口、托盘或别的应用
    const self = islandIdFromSearch(window.location.search)
    void listenEvent<string | null>("island-blur", (id) => { if ((id ?? null) === self) onBlur() }).then((un) => { if (disposed) un(); else off = un }).catch(() => {})
    return () => {
      disposed = true
      off?.()
      window.removeEventListener("island-menu-open", onMenu)
      window.removeEventListener("blur", onBlur)
    }
  }, [open, connectionMenuOpen, collapse])

  const bubbleVisible = view !== "card" && view !== "mini" && !data.sessionStatusUnknown && running >= 2
  const driftVisible = view !== "card" && view !== "mini" && drifted > 0

  // ---------- 内容 ----------
  const plat = (
    <span className="cap-plat">
      <PlatformLogo platform={data.platform} size={13} />
      {data.platformName}
    </span>
  )
  // 收缩层内容严格按原型 app.js 的 collapsedHTML() / quotaHTML() 拼：
  // 平台 + 额度窗口芯片（5h 10% / 7d 59%，≥80% 转橙、满格换红字「x 后重置」）+ 绿色增量。
  // 拿不到额度时只给破折号芯片，绝不写「剩余额度 —」这种卡片句式 —— 那会把胶囊读成压扁的卡片。
  const fullQuota = quotaRows.find((q) => q.usedPercent !== null && q.usedPercent >= 100)
  const quotaChips = fullQuota
    ? <span className="cap-r cap-num cap-mono text-xs">{resetText(fullQuota) ?? "额度已用尽"} 后重置</span>
    : quotaRows.map((q) => (
      <span key={q.key} className="cap-q">
        <i>{q.key}</i>
        <b className={cn("cap-num", q.usedPercent !== null && q.usedPercent >= 80 && "cap-o")} title={q.usedText ?? undefined}>
          {q.usedPercent === null ? "—" : `${Math.round(q.usedPercent)}%`}
        </b>
      </span>
    ))
  // 没有任何额度窗口时的最窄形态：一个破折号，13px text-3（原型 inside-note 的写法）
  const dashOnly = <span className="cap-num cap-t3 text-[13px]">—</span>

  const collapsedMain = relay ? (
    <RelayRoute relay={relay} />
  ) : data.unavailable ? (
    <>
      <span className="cap-plat cap-dim"><PlatformLogo platform={data.platform} size={13} />{data.platformName}</span>
      <span className="cap-wdot" /><span className="cap-t2">{data.unavailable.title} · 双击查看</span>
    </>
  ) : data.queryFailed && showQuotas ? (
    <>{plat}<span className="cap-r text-xs font-semibold">查询失败 · 双击查看</span></>
  ) : data.remainingMetrics ? (
    <>
      {plat}
      {data.remainingMetrics.rows.length > 0
        ? data.remainingMetrics.rows.slice(0, 2).map((row, i) => <span key={`${row.label}:${i}`} className="cap-q"><i>{row.label}</i><b className="cap-num">{row.value}</b></span>)
        : data.quotas?.length ? quotaChips : dashOnly}
    </>
  ) : data.localMetrics ? (
    <>{plat}<span className="cap-q"><i>本机今日</i><b className="cap-num">{data.localMetrics.token ?? "—"}</b></span></>
  ) : showQuotas ? (
    <>{plat}{quotaRows.length > 0 ? quotaChips : dashOnly}</>
  ) : (
    <>
      {plat}
      <span className="cap-chip cap-b">API</span>
      <b className="cap-num cap-mono">{data.apiToday?.token ?? "—"}</b>
      <span className="cap-t3">Token</span>
      {data.apiToday?.cost && <><span className="cap-hair" /><b className={cn("cap-num", data.apiToday.costEstimated && "cap-o")}>{data.apiToday.cost}</b></>}
    </>
  )

  const statusColor = data.unavailable ? "var(--pn-capsule-orange)" : showQuotas && data.queryFailed ? "var(--pn-capsule-red)"
    : refreshing ? "var(--pn-capsule-cyan)" : data.status?.tone === "danger" ? "var(--pn-capsule-red)"
    : data.status?.tone === "warn" ? "var(--pn-capsule-orange)" : "var(--pn-capsule-green)"
  const statusText = data.unavailable ? data.unavailable.title : showQuotas && data.queryFailed ? "额度查询失败 · 请刷新重试"
    : refreshing ? "正在刷新…" : data.status?.label ?? "—"

  const legend = data.unavailable ? (
    <p className="cap-note">{data.unavailable.hint}</p>
  ) : showQuotas ? (
    [...quotaRows].map((q) => {
      const index = ringWindows.findIndex((w) => w.key === q.key)
      const on = focused === index
      const reset = resetText(q)
      return (
        <button
          key={q.key}
          type="button"
          className={cn("cap-lg", on && "on", pinRing === index && "pin")}
          aria-pressed={pinRing === index}
          data-cap-ring={q.key}
          onPointerEnter={() => onRingHover(index)}
          onPointerLeave={() => onRingHover(null)}
          onFocus={() => onRingHover(index)}
          onBlur={() => onRingHover(null)}
          onClick={() => togglePin(index)}
          onDoubleClick={(event) => event.stopPropagation()}
        >
          <span className="cap-sw" style={{ background: ringColor(q) }} />
          <span className="cap-t">
            <span className="cap-w">{windowLabel(q)} · {index === 0 ? "外圈" : "内圈"}</span>
            <span className="cap-p cap-num">{q.usedPercent === null ? "—" : Math.round(q.usedPercent)}<small>{q.usedPercent === null ? "" : "%"}</small></span>
            <span className="cap-r2">{reset ? `${reset} 后重置` : "未提供重置时间"}</span>
          </span>
        </button>
      )
    })
  ) : data.remainingMetrics ? (
    data.remainingMetrics.rows.length > 0
      ? data.remainingMetrics.rows.map((row, i) => (
        <div key={`${row.label}:${i}`} className="cap-met"><span>{row.label}{row.reset ? ` · ${row.reset} 后重置` : ""}</span><b className="cap-num">{row.value}</b></div>
      ))
      : <div className="cap-met"><span>剩余额度</span><b>—</b></div>
  ) : data.localMetrics ? (
    <>
      <div className="cap-met"><span>本机今日 Token</span><b className="cap-num">{data.localMetrics.token ?? "—"}</b></div>
      <div className="cap-met"><span>今日上报积分</span><b className="cap-num">{data.localMetrics.credits ?? "—"}</b></div>
    </>
  ) : (
    <>
      <div className="cap-met"><span>{data.apiToday?.tokenLabel ?? "今日 Token"}</span><b className="cap-num">{data.apiToday?.token ?? "—"}</b></div>
      <div className="cap-met"><span>{data.apiToday?.costEstimated ? "本机费用（估算）" : "今日费用"}</span><b className={cn("cap-num", data.apiToday?.costEstimated && "cap-o")}>{data.apiToday?.cost ?? "—"}</b></div>
    </>
  )

  const notes = [
    showQuotas && data.quotaMessage,
    data.remainingMetrics?.message,
    data.localMetrics?.message,
    !showQuotas && !data.remainingMetrics && !data.localMetrics && data.apiMessage,
    !showQuotas && !data.remainingMetrics && !data.localMetrics && data.connectionLabel && `连接：${data.connectionLabel}`,
    data.balanceText && `网关余额 ${data.balanceText}${data.balanceTimeText ? `（${data.balanceTimeText}）` : ""}`,
  ].filter(Boolean) as string[]

  const sessionTotal = data.sessions?.length ?? 0
  const sectionTitle = sessionTotal > 0 ? (data.sessionSectionTitle ?? `运行中的会话 · ${sessionTotal}`) : "本轮 Token"

  // 原生窗口按中心对齐（停靠贴边居中 + 自由态中心缩放），所以气泡这类侧挂内容
  // 必须左右对称：三栏网格把岛固定在正中，两侧等宽，气泡出现时岛不位移。
  // 上下停靠时贴住的是屏幕上下边，只有左右需要对称。
  const aside = bubbleVisible || driftVisible
  const verticalDock = dockEdge === "left" || dockEdge === "right"
  const symmetric = aside && !verticalDock

  return (
    <div
      className={cn(
        "items-center gap-2",
        verticalDock ? cn("flex", dockEdge === "right" ? "flex-row-reverse" : "flex-row") : symmetric ? "grid grid-cols-[1fr_auto_1fr]" : "flex",
      )}
      data-capsule-island
    >
      {symmetric && <span aria-hidden className="col-start-1" />}
      {/* 预留盒子：原生窗口按它定尺寸；外壳在盒子里按停靠边贴住，形变期间内容不跟着窗口跑 */}
      <div ref={reserveRef} className={cn("relative shrink-0", symmetric && "col-start-2")} style={{ width: reserve.w, height: reserve.h }}>
        <div className={cn("absolute inset-0 flex", anchor)}>
          <div
            ref={shellRef}
            role="group"
            aria-label={`${data.platformName} 灵动岛`}
            title={reason}
            data-cap-expr={expr}
            data-open={open}
            data-mini={view === "mini"}
            data-docked={Boolean(dockEdge)}
            className={cn("cap-shell", ready && "cap-ready", shapeAnim && "cap-anim", dragging && "!transition-none", scroll && "cap-scroll")}
            style={{ width: shellW, height: shellH, borderRadius: radius, opacity: 1 }}
            onDoubleClick={(event) => {
              // 收缩态整层就是开关（它自己是 role=button），不能再被排除掉；
              // 展开态只有交互控件之外的双击才收回，和原型一致
              if (!open) { onOpenChange(true); return }
              if ((event.target as HTMLElement).closest("button, [role=button], [role=combobox], input, a")) return
              collapse()
            }}
            onPointerEnter={() => { setHover(true); onHoverChange?.(true) }}
            onPointerLeave={() => { setHover(false); onHoverChange?.(false) }}
          >
            <div
              className="cap-layer cap-cl cap-toggle"
              role="button"
              tabIndex={open ? -1 : 0}
              aria-expanded={false}
              aria-label={`${data.platformName}，${BOT_NAMES[expr]}。${view === "mini" ? "悬停展开，双击打开卡片" : "按 Enter 展开"}`}
              inert={open}
              onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); onOpenChange(true) } }}
            >
              <Bot
                size={view === "mini" ? 26 : 32}
                ringWidth={view === "mini" ? 1.8 : 2}
                ringGap={view === "mini" ? 1.2 : 1.5}
                inset={view === "mini" ? 2 : 3}
                rings={ringsFor(false)} expr={expr} dockEdge={dockEdge}
                ticker={open || view === "mini" || relay ? null : ticker}
              />
              {/* 迷你胶囊只留 bot 与工作中的脉冲点（原型 collapsedHTML 的 mini 分支） */}
              {view === "mini" && working && <span className="cap-pulse" />}
              <span ref={mainRef} className="cap-main">
                {collapsedMain}
                {showDelta && !data.unavailable && !relay && (
                  <>
                    <span className="cap-hair" />
                    <span className="cap-live cap-num cap-mono"><AnimatedTokens tokens={data.deltaTokens} text={data.deltaText} countKey={data.deltaCountKey} className="!text-[12px] !font-semibold" /></span>
                  </>
                )}
                {relayOn && agents.length > 0 && <>{!relay && <span className="cap-hair" />}<AgentStack agents={agents} activity={gateway!.activity} now={now} /></>}
              </span>
              {relay?.phase === "in" && <span className="cap-flow" />}
            </div>

            <div ref={exRef} className="cap-layer cap-ex" inert={!open} style={connectionMenuOpen ? { minHeight: 240 } : undefined}>
              <div className="cap-sec cap-hd">
                <div className="cap-id">
                  <div className="cap-nm">
                    <PlatformLogo platform={data.platform} size={15} />
                    {data.platformName}
                    <span className={cn("cap-chip", data.localMetrics ? "" : data.kind === "auth" ? "cap-g" : "cap-b")}>
                      {data.localMetrics ? "本机" : data.kind === "auth" ? "Auth" : "API"}
                    </span>
                  </div>
                  <div className="cap-st"><i style={{ background: statusColor }} /><span className="truncate">{statusText}</span></div>
                </div>
                {open && (connectionSwitcher ?? <LazyConnectionSwitcher />)}
              </div>

              <div className="cap-sec cap-hero">
                <div className="cap-orb-l">
                  <Bot
                    size={140} ringWidth={9} ringGap={5} inset={28} rings={ringsFor(true)} expr={expr} dockEdge={dockEdge}
                    ticker={open ? ticker : null} focus={focused} display={display} interactive={showQuotas}
                    onRingHover={onRingHover} onRingClick={togglePin} pinnedRing={pinRing}
                  />
                </div>
                <div className="cap-legend">{legend}</div>
              </div>

              {(sessionTotal > 0 || showDelta) && (
                <div className="cap-sec cap-col">
                  <div className="cap-sh">
                    <span>{sectionTitle}</span>
                    <em><AnimatedTokens tokens={data.deltaTokens} text={data.deltaText} countKey={data.deltaCountKey} className="!text-[11px] !text-[var(--pn-capsule-green)]" /></em>
                  </div>
                  {sessionTotal > 0 && <SessionList sessions={data.sessions!} showUnknownTag={data.sessionStatusUnknown} />}
                </div>
              )}

              {relayOn && <GatewaySection status={gateway!.status!} activity={gateway!.activity} agents={agents} now={now} />}

              <div className="cap-sec cap-foot">
                {data.todayTokenText && showQuotas && !data.remainingMetrics && (
                  <div className="cap-row"><span className="cap-l">本地今日 Token</span><span className="cap-v cap-num">{data.todayTokenText}</span></div>
                )}
                {notes.map((text) => <p key={text} className="cap-note">{text}</p>)}
                {data.sourceText && <SourceFooter text={data.sourceText} />}
              </div>
            </div>
          </div>
        </div>
      </div>

      {aside && (
        <div className={cn("flex items-center gap-2", symmetric && "col-start-3 justify-self-start")}>
          {bubbleVisible && (
            <div className="cap-bubble" role="status" aria-label={`${running} 个会话运行中`}>
              <span className="cap-spinner" />
              <b className="cap-num">{running}</b>
            </div>
          )}
          {driftVisible && (
            <div className="cap-bubble" role="status"><span className="cap-wdot" />{drifted} 个接线需处理</div>
          )}
        </div>
      )}
    </div>
  )
}
