import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react"
import { BadgeCheck, ChevronDown, Clock3, KeyRound, RotateCw, TriangleAlert } from "lucide-react"
import { cn } from "@/lib/utils"
import {
  credentialLabel, defaultWindowSize, projectName, relativeTime, sessionLines, sessionStatus, sessionTitle, sourceSummary, tokenDisplay, toolLabel,
  type SessionView, type SourceLive, type SourceTone, type TokenFreshness,
} from "@/lib/gatewaySessions"
import type { Connection } from "@/types"
import { agentLabel } from "@/lib/gatewayPage"
import { StatusCapsule } from "./StatusCapsule"

/**
 * V6 · 来源与最近会话共用滚动区（Jolu.pen `FL4Xi` / J6-02 `qGotz`，动效 J6-03 `vRih5`）。
 *
 * - 两栏与连线在同一个滚动容器、同一坐标系里，标题固定在顶部；
 * - 运行中的关联：2.4px 主线 + 柔光 + 从来源流向会话的光点（2.8s 一轮）；结束 220ms 淡为 1.4px 静态细线；
 * - 会话重排 280ms 平滑位移，卡片与曲线端点同步；查看下方内容时保持首个可见会话的位置；
 * - 只有可见的运行中关联播放流光；离屏、窗口隐藏或减少动效时停止。
 */

export interface GraphSource {
  connection: Connection
  tone: SourceTone
}

const TONES: Record<SourceTone, { color: string; soft: string; glow: string }> = {
  green: { color: "var(--jolu-accent)", soft: "var(--jolu-selected)", glow: "var(--jolu-gateway-glow-green)" },
  blue: { color: "var(--jolu-gateway-blue)", soft: "var(--jolu-gateway-blue-soft)", glow: "var(--jolu-gateway-glow-blue)" },
  purple: { color: "var(--jolu-gateway-purple)", soft: "var(--jolu-gateway-purple-soft)", glow: "var(--jolu-gateway-glow-purple)" },
  amber: { color: "var(--jolu-gateway-amber)", soft: "var(--jolu-gateway-amber-soft)", glow: "var(--jolu-gateway-glow-amber)" },
}

/** 端口圆心相对卡片顶部的位置（画板：来源卡 52px、会话卡 44px） */
const SOURCE_PORT_Y = 52
const SESSION_PORT_Y = 44
const REORDER_MS = 280
const TOP_SLACK = 24
const EXPAND_MS = 240

type Line = { key: string; sessionKey: string; connectionId: string; d: string; active: boolean; tone: SourceTone; y1: number; y2: number }
type Hover = { kind: "source"; id: string } | { kind: "session"; key: string } | null

function reducedMotion(): boolean {
  return document.documentElement.classList.contains("reduce-motion") || window.matchMedia("(prefers-reduced-motion: reduce)").matches
}

export function SessionGraph({ sources, views, sourceLive, modelCount, loading, loaded, error, freshness, desktop, busy, onRetry, onToggleServe }: {
  sources: GraphSource[]
  views: SessionView[]
  /** 没有会话标识的在途请求（如 Gemini CLI），按来源 id 汇总 */
  sourceLive: Map<string, SourceLive>
  modelCount: number
  loading: boolean
  loaded: boolean
  error: string | null
  freshness: TokenFreshness
  desktop: boolean
  busy: boolean
  onRetry: () => void
  /** 来源卡上的「已接入 / 未接入」胶囊：加入 / 移出模型池（调用方负责确认） */
  onToggleServe: (connection: Connection) => void
}) {
  const [expanded, setExpanded] = useState(false)
  const [hover, setHover] = useState<Hover>(null)
  const [lines, setLines] = useState<Line[]>([])
  const [viewport, setViewport] = useState({ top: 0, bottom: Number.POSITIVE_INFINITY })
  const [onScreen, setOnScreen] = useState(true)
  const [pageVisible, setPageVisible] = useState(() => !document.hidden)
  const [now, setNow] = useState(() => Date.now())
  const scrollRef = useRef<HTMLDivElement | null>(null)
  const contentRef = useRef<HTMLDivElement | null>(null)
  const expandRef = useRef<HTMLDivElement | null>(null)
  const sourceEls = useRef(new Map<string, HTMLDivElement>())
  const sessionEls = useRef(new Map<string, HTMLDivElement>())
  const prevTops = useRef(new Map<string, number>())
  const linesSig = useRef("")
  const frame = useRef<number | null>(null)

  const toneOf = new Map(sources.map((source) => [source.connection.id, source.tone]))
  const windowSize = defaultWindowSize(views)
  const first = views.slice(0, windowSize)
  const rest = views.slice(windowSize)
  const shownKeys = new Set((expanded ? views : first).map((view) => view.key))

  /* ── 连线几何：与卡片使用同一份布局结果（含位移中的 transform） ── */
  const measureRef = useRef<() => void>(() => {})
  measureRef.current = () => {
    const content = contentRef.current
    if (!content) return
    const base = content.getBoundingClientRect()
    const clip = expandRef.current?.getBoundingClientRect()
    const next: Line[] = []
    for (const view of views) {
      const sessionEl = sessionEls.current.get(view.key)
      if (!sessionEl) continue
      const t = sessionEl.getBoundingClientRect()
      // 收起区域里被裁掉的卡片不画线
      if (!shownKeys.has(view.key) || (clip && sessionEl.dataset.rest === "1" && t.top + SESSION_PORT_Y > clip.bottom)) continue
      for (const line of sessionLines(view)) {
        const sourceEl = sourceEls.current.get(line.connectionId)
        const tone = toneOf.get(line.connectionId)
        if (!sourceEl || !tone) continue
        const s = sourceEl.getBoundingClientRect()
        const x1 = s.right - base.left
        const y1 = s.top - base.top + SOURCE_PORT_Y
        const x2 = t.left - base.left
        const y2 = t.top - base.top + SESSION_PORT_Y
        const dx = Math.max(24, (x2 - x1) * 0.5)
        next.push({
          key: `${view.key}->${line.connectionId}`, sessionKey: view.key, connectionId: line.connectionId, active: line.active, tone, y1, y2,
          d: `M ${x1.toFixed(1)} ${y1.toFixed(1)} C ${(x1 + dx).toFixed(1)} ${y1.toFixed(1)}, ${(x2 - dx).toFixed(1)} ${y2.toFixed(1)}, ${x2.toFixed(1)} ${y2.toFixed(1)}`,
        })
      }
    }
    const sig = next.map((line) => `${line.key}|${line.d}|${line.active ? 1 : 0}|${line.tone}`).join(";")
    if (sig !== linesSig.current) {
      linesSig.current = sig
      setLines(next)
    }
  }
  const measure = useCallback(() => measureRef.current(), [])

  const recordTops = useCallback(() => {
    const content = contentRef.current
    if (!content) return
    const base = content.getBoundingClientRect().top
    const tops = new Map<string, number>()
    sessionEls.current.forEach((el, key) => tops.set(`s:${key}`, el.getBoundingClientRect().top - base))
    sourceEls.current.forEach((el, id) => tops.set(`c:${id}`, el.getBoundingClientRect().top - base))
    prevTops.current = tops
  }, [])

  /** 动画期间逐帧跟随：曲线端点与位移中的卡片同步，同时记下可接续的当前位置 */
  const follow = useCallback((ms: number) => {
    if (frame.current) cancelAnimationFrame(frame.current)
    const until = performance.now() + ms + 40
    const step = () => {
      measure()
      recordTops()
      frame.current = performance.now() < until ? requestAnimationFrame(step) : null
    }
    frame.current = requestAnimationFrame(step)
  }, [measure, recordTops])

  useLayoutEffect(() => { measure() })

  /* ── 重排：FLIP 位移 + 阅读位置保持（只在顺序或展开状态变化时） ── */
  const orderSig = views.map((view) => view.key).join("|") + `#${sources.map((s) => s.connection.id).join("|")}`
  useLayoutEffect(() => {
    const content = contentRef.current
    const scroller = scrollRef.current
    if (!content || !scroller) return
    const reduced = reducedMotion()
    const items: Array<[string, HTMLDivElement]> = [
      ...[...sessionEls.current].map(([key, el]) => [`s:${key}`, el] as [string, HTMLDivElement]),
      ...[...sourceEls.current].map(([id, el]) => [`c:${id}`, el] as [string, HTMLDivElement]),
    ]
    // 先去掉进行中的位移，读出新的布局位置；旧的视觉位置来自上一帧记录
    for (const [, el] of items) { el.style.transition = "none"; el.style.transform = "" }
    const base = content.getBoundingClientRect().top
    const layout = new Map(items.map(([key, el]) => [key, el.getBoundingClientRect().top - base]))
    const prev = prevTops.current
    const oldScroll = scroller.scrollTop
    let shift = 0
    // 离顶部不到 24px 视为在顶部：新运行的会话直接出现在顶部可见区域，不做位置保持
    if (oldScroll > TOP_SLACK && prev.size > 0) {
      // 首个可见会话：保持它在视口里的相对位置，不因新请求跳回顶部
      const anchor = [...sessionEls.current.entries()]
        .map(([key, el]) => ({ key: `s:${key}`, top: prev.get(`s:${key}`), height: el.offsetHeight }))
        .filter((item): item is { key: string; top: number; height: number } => item.top !== undefined)
        .sort((a, b) => a.top - b.top)
        .find((item) => item.top + item.height > oldScroll)
      const target = anchor && layout.get(anchor.key)
      if (anchor && target !== undefined) {
        scroller.scrollTop = oldScroll + (target - anchor.top)
        shift = scroller.scrollTop - oldScroll
      }
    }
    const moving: HTMLDivElement[] = []
    if (!reduced && prev.size > 0) {
      for (const [key, el] of items) {
        const from = prev.get(key)
        const to = layout.get(key)
        if (from === undefined || to === undefined) continue
        const delta = from - to + shift
        if (Math.abs(delta) < 0.5) continue
        el.style.transform = `translateY(${delta}px)`
        moving.push(el)
      }
    }
    if (moving.length > 0) {
      void content.offsetHeight
      for (const el of moving) {
        el.style.transition = `transform ${REORDER_MS}ms var(--motion-ease)`
        el.style.transform = ""
      }
      follow(REORDER_MS)
    } else {
      prevTops.current = new Map([...layout.entries()])
      measure()
    }
  }, [orderSig])

  useEffect(() => { follow(EXPAND_MS) }, [expanded, follow])

  /* ── 尺寸变化（窗口、侧栏收起、缩放）后端口重新对齐 ── */
  useEffect(() => {
    const content = contentRef.current
    if (!content) return
    const observer = new ResizeObserver(() => { measure(); recordTops() })
    observer.observe(content)
    return () => observer.disconnect()
  }, [measure, recordTops])

  /* ── 流光只在可见时播放 ── */
  useEffect(() => {
    const scroller = scrollRef.current
    if (!scroller) return
    let raf: number | null = null
    const update = () => {
      raf = null
      setViewport({ top: scroller.scrollTop, bottom: scroller.scrollTop + scroller.clientHeight })
    }
    const onScroll = () => { if (raf === null) raf = requestAnimationFrame(update) }
    update()
    scroller.addEventListener("scroll", onScroll, { passive: true })
    const io = new IntersectionObserver(([entry]) => setOnScreen(entry.isIntersecting))
    io.observe(scroller)
    const onVisibility = () => setPageVisible(!document.hidden)
    document.addEventListener("visibilitychange", onVisibility)
    return () => {
      scroller.removeEventListener("scroll", onScroll)
      if (raf !== null) cancelAnimationFrame(raf)
      io.disconnect()
      document.removeEventListener("visibilitychange", onVisibility)
    }
  }, [])

  useEffect(() => () => { if (frame.current) cancelAnimationFrame(frame.current) }, [])
  // 「刚刚 / N 分钟前」每 30 秒更新一次文案，不触发重排
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 30_000)
    return () => window.clearInterval(timer)
  }, [])

  const related = (line: Line) => hover === null ? true
    : hover.kind === "source" ? line.connectionId === hover.id : line.sessionKey === hover.key
  const sourceHighlighted = (id: string) => hover?.kind === "source" ? hover.id === id
    : hover?.kind === "session" ? lines.some((line) => line.sessionKey === hover.key && line.connectionId === id) : false
  const sessionHighlighted = (key: string) => hover?.kind === "session" ? hover.key === key
    : hover?.kind === "source" ? lines.some((line) => line.connectionId === hover.id && line.sessionKey === key) : false
  const flowing = onScreen && pageVisible

  const sessionTone = (view: SessionView): SourceTone | null => {
    const lineSet = sessionLines(view)
    const active = lineSet.find((line) => line.active && toneOf.has(line.connectionId)) ?? lineSet.find((line) => toneOf.has(line.connectionId))
    return active ? toneOf.get(active.connectionId)! : null
  }

  const renderSession = (view: SessionView, isRest: boolean) => {
    const tone = sessionTone(view)
    return <SessionCard
      key={view.key}
      innerRef={(el) => { if (el) sessionEls.current.set(view.key, el); else sessionEls.current.delete(view.key) }}
      view={view}
      rest={isRest}
      tone={tone}
      now={now}
      freshness={freshness}
      highlighted={sessionHighlighted(view.key)}
      onHover={(on) => setHover(on ? { kind: "session", key: view.key } : null)}
    />
  }

  const empty = !loading && loaded && views.length === 0
  return (
    <section aria-label="模型来源与最近会话" className="flex shrink-0 flex-col overflow-hidden rounded-[14px] border border-jolu-cc-border bg-jolu-bg">
      <header className="grid grid-cols-[minmax(240px,330px)_minmax(80px,1fr)_minmax(300px,494px)] px-8 pb-3 pt-[22px] pr-[46px]">
        <div className="flex flex-col gap-[7px]">
          <h2 className="text-[16px] font-semibold text-jolu-text">模型来源</h2>
          <p className="text-[11px] text-jolu-muted">{sources.length} 个来源 · {modelCount} 个模型</p>
        </div>
        <span />
        <div className="flex flex-col gap-[7px]">
          <h2 className="text-[16px] font-semibold text-jolu-text">最近会话</h2>
          <p className="text-[11px] text-jolu-muted">{expanded ? "运行中优先 · 已展开更多最近会话" : "运行中优先 · 其余按最近活动排序"}</p>
        </div>
      </header>

      <div ref={scrollRef} className="gw6-scroll relative max-h-[570px] min-h-[200px] overflow-y-auto pl-8 pr-3">
        <div ref={contentRef} className="relative grid grid-cols-[minmax(240px,330px)_minmax(80px,1fr)_minmax(300px,494px)] pb-4 pr-[30px]"
          style={{ ["--gw6-hover-ms" as string]: hover ? "140ms" : "100ms" }}>
          <svg aria-hidden className="pointer-events-none absolute inset-0 h-full w-full overflow-visible">
            <defs>
              <filter id="gw6-glow" x="-20%" y="-50%" width="140%" height="200%"><feGaussianBlur stdDeviation="3.5" /></filter>
            </defs>
            {lines.map((line) => {
              const color = TONES[line.tone].color
              const dim = !related(line)
              const visible = Math.max(line.y1, line.y2) >= viewport.top && Math.min(line.y1, line.y2) <= viewport.bottom
              return (
                <g key={line.key}>
                  <path d={line.d} fill="none" stroke={color} strokeWidth={6} filter="url(#gw6-glow)" className="gw6-glow"
                    style={{ opacity: line.active ? (dim ? 0.12 : 0.35) : 0 }} />
                  <path d={line.d} fill="none" stroke={color} strokeLinecap="round" className="gw6-line"
                    style={{ strokeWidth: line.active ? 2.4 : 1.4, opacity: dim ? 0.3 : line.active ? 1 : 0.5 }} />
                  {line.active && flowing && visible && (
                    <circle r="3" fill={color} className="gw6-flow" style={{ filter: `drop-shadow(0 0 5px ${TONES[line.tone].glow})`, opacity: dim ? 0.35 : 1 }}>
                      <animateMotion dur="2.8s" repeatCount="indefinite" path={line.d} />
                    </circle>
                  )}
                </g>
              )
            })}
          </svg>

          <div className="relative z-10 flex min-w-0 flex-col gap-[26px] pt-1">
            {sources.map((source) => {
              const summary = sourceSummary(views, source.connection.id)
              return <SourceCard key={source.connection.id}
                innerRef={(el) => { if (el) sourceEls.current.set(source.connection.id, el); else sourceEls.current.delete(source.connection.id) }}
                source={source} summary={summary} live={sourceLive.get(source.connection.id)} highlighted={sourceHighlighted(source.connection.id)}
                toggleDisabled={busy || !desktop} onToggleServe={() => onToggleServe(source.connection)}
                onHover={(on) => setHover(on ? { kind: "source", id: source.connection.id } : null)} />
            })}
            {sources.length === 0 && !loading && (
              <p className="rounded-xl border border-dashed border-jolu-border px-4 py-6 text-center text-[11px] text-jolu-muted">
                还没有来源加入模型池。可在「管理来源」里把订阅或 API Key 加入。
              </p>
            )}
          </div>

          <div aria-hidden className="gw6-dots mx-3" />

          <div className="relative z-10 flex min-w-0 flex-col pt-1">
            {!desktop ? (
              <Placeholder>浏览器预览不读取网关会话；请在桌面端查看真实数据。</Placeholder>
            ) : error ? (
              <Placeholder>
                <span className="text-jolu-danger">最近会话读取失败：{error}</span>
                <button type="button" onClick={onRetry} className="motion-button mt-2 inline-flex h-8 items-center gap-1.5 rounded-lg border border-jolu-border bg-jolu-bg px-3 text-[11px] text-jolu-text outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent">
                  <RotateCw aria-hidden className="size-3.5" />重试
                </button>
              </Placeholder>
            ) : loading && !loaded ? (
              <Placeholder>正在读取最近会话…</Placeholder>
            ) : empty ? (
              <Placeholder>还没有经网关的会话。工具接入网关并发出请求后，这里会显示会话与来源的关联。</Placeholder>
            ) : (
              <>
                <div className="flex flex-col gap-[18px]">{first.map((view) => renderSession(view, false))}</div>
                {rest.length > 0 && (
                  <>
                    <div ref={expandRef} className={cn("gw6-expand", expanded && "open")} aria-hidden={!expanded}>
                      <div>
                        <div className="flex flex-col gap-[18px] pt-[18px]" inert={!expanded}>
                          {rest.map((view) => renderSession(view, true))}
                        </div>
                      </div>
                    </div>
                    <button type="button" aria-expanded={expanded} onClick={() => setExpanded((value) => !value)}
                      aria-label={expanded ? "收起更多会话" : `展开更多最近会话，还有 ${rest.length} 条`}
                      className="motion-button mt-[14px] flex h-11 shrink-0 items-center justify-center gap-2 rounded-[10px] bg-jolu-stage text-[12px] text-jolu-muted outline-none hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent">
                      {expanded ? "收起更多会话" : "展开更多最近会话"}
                      <ChevronDown aria-hidden className={cn("gw6-chevron size-3.5", expanded && "rotate-180")} />
                    </button>
                  </>
                )}
              </>
            )}
          </div>
        </div>
      </div>

      <footer className="grid grid-cols-[minmax(240px,330px)_minmax(80px,1fr)_minmax(300px,494px)] px-8 pb-[22px] pt-3 pr-[46px] text-[11px] text-jolu-muted">
        <span>{expanded ? "两栏共用滚动区域 · 展开后保持当前阅读位置" : "模型来源与最近会话一起滚动"}</span>
        <span />
        <span className="flex gap-5">
          <span>亮线 · AI 运行中</span>
          <span>淡线 · 最近关联</span>
        </span>
      </footer>
    </section>
  )
}

function Placeholder({ children }: { children: React.ReactNode }) {
  return <div className="flex flex-col items-center rounded-[14px] border border-dashed border-jolu-border px-5 py-8 text-center text-[11px] text-jolu-muted">{children}</div>
}

function SourceCard({ innerRef, source, summary, live, highlighted, toggleDisabled, onToggleServe, onHover }: {
  innerRef: (el: HTMLDivElement | null) => void
  source: GraphSource
  summary: { sessions: number; model: string | null }
  live: SourceLive | undefined
  highlighted: boolean
  toggleDisabled: boolean
  onToggleServe: () => void
  onHover: (on: boolean) => void
}) {
  const tone = TONES[source.tone]
  const { connection } = source
  const Icon = connection.kind === "auth" ? BadgeCheck : KeyRound
  return (
    <div ref={innerRef} onMouseEnter={() => onHover(true)} onMouseLeave={() => onHover(false)}
      className="gw6-card relative flex h-[104px] flex-col justify-between rounded-xl border border-jolu-border bg-jolu-bg px-4 pb-[17px] pt-4"
      style={{ boxShadow: highlighted || live ? `0 3px 14px ${tone.glow}` : "0 3px 9px var(--jolu-cc-control-shadow)", borderColor: highlighted || live ? tone.color : undefined }}>
      <div className="flex min-w-0 items-start gap-3">
        <span className="grid size-[34px] shrink-0 place-items-center rounded-[9px]" style={{ backgroundColor: tone.soft }}>
          <Icon aria-hidden className="size-[18px]" style={{ color: tone.color }} />
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-[6px]">
          <span className="truncate text-[13px] font-semibold text-jolu-text" title={connection.baseName}>{connection.baseName}</span>
          <span className="truncate text-[10px] text-jolu-muted">{credentialLabel(connection.kind, connection.platformId)}</span>
        </div>
        <StatusCapsule size="sm" tone={connection.serve_enabled ? "on" : "off"} label={connection.serve_enabled ? "已接入" : "未接入"}
          title={connection.serve_enabled ? "在模型池中，点击移出" : "不在模型池中，点击加入"} disabled={toggleDisabled} onClick={onToggleServe} />
      </div>
      <div className="flex min-w-0 items-baseline justify-between gap-3">
        {summary.model
          ? <span className="truncate text-[12px] font-semibold" style={{ color: tone.color }} title={summary.model}>{summary.model}</span>
          : <span className="truncate text-[11px] text-jolu-muted">暂无经网关的请求</span>}
        {live ? (
          // 没有会话标识的请求（如 Gemini CLI）：只在来源卡上显示运行中，不连到会话（J6-04 05）
          <span className="flex shrink-0 items-center gap-[5px] text-[10px] font-semibold" style={{ color: tone.color }}
            title="这些请求没有带会话标识，无法关联到具体会话" aria-live="polite">
            <span className="size-1.5 rounded-full" style={{ backgroundColor: tone.color }} />
            运行中 · {live.agents.length ? live.agents.map(agentLabel).join("、") : "未识别工具"}{live.count > 1 ? ` × ${live.count}` : ""}
          </span>
        ) : (
          <span className="shrink-0 text-[10px] text-jolu-muted">{summary.sessions} 条最近会话</span>
        )}
      </div>
      <span aria-hidden className="absolute -right-1 top-12 size-2 rounded-full border-[1.5px] bg-jolu-bg" style={{ borderColor: tone.color }} />
    </div>
  )
}

function SessionCard({ innerRef, view, rest, tone, now, freshness, highlighted, onHover }: {
  innerRef: (el: HTMLDivElement | null) => void
  view: SessionView
  rest: boolean
  tone: SourceTone | null
  now: number
  freshness: TokenFreshness
  highlighted: boolean
  onHover: (on: boolean) => void
}) {
  const { session, running } = view
  const status = sessionStatus(running, session.thread_status)
  const colors = tone ? TONES[tone] : null
  const accent = running && colors ? colors : null
  const token = tokenDisplay(session.tokens, freshness)
  const project = projectName(session.cwd)
  return (
    <div ref={innerRef} data-rest={rest ? "1" : "0"} onMouseEnter={() => onHover(true)} onMouseLeave={() => onHover(false)}
      className="gw6-card relative flex h-[88px] items-stretch justify-between gap-4 rounded-[14px] border border-jolu-border bg-jolu-bg py-[15px] pl-5 pr-5"
      style={{
        borderColor: accent ? accent.color : highlighted && colors ? colors.color : undefined,
        boxShadow: accent ? `0 3px 14px ${accent.glow}` : highlighted && colors ? `0 3px 14px ${colors.glow}` : "none",
      }}>
      <div className="flex min-w-0 flex-col justify-between">
        <span className="truncate text-[14px] font-semibold text-jolu-text" title={session.title ?? session.session_id}>{sessionTitle(session)}</span>
        <span className="truncate text-[11px] text-jolu-muted">{toolLabel(session.platform)} · {project ?? "未识别项目"}</span>
        <span className="truncate text-[10px] text-jolu-muted">{relativeTime(view.activity, now)}</span>
      </div>
      <div className="flex shrink-0 flex-col items-end gap-[9px]">
        <span className="flex items-center gap-[7px]" aria-live="polite">
          <span className="size-1.5 rounded-full" style={{ backgroundColor: accent ? accent.color : "var(--jolu-muted)" }} />
          <span className={cn("text-[10.5px]", accent ? "font-semibold" : status.tone === "failed" ? "text-jolu-danger" : "text-jolu-muted")}
            style={accent ? { color: accent.color } : undefined}>{status.label}</span>
        </span>
        <span title={token.title}
          className="flex h-[26px] w-40 items-center justify-end gap-[5px] rounded-md pl-1 pr-[9px]"
          style={{ backgroundColor: accent ? accent.soft : "var(--jolu-stage)" }}>
          {token.badge && <TokenBadge kind={token.badge.kind} label={token.badge.label} />}
          <span className={cn("font-mono text-[12px] font-semibold tabular-nums", !token.known || token.flag ? "text-jolu-muted" : !accent && "text-jolu-text")}
            style={accent && token.known && !token.flag ? { color: accent.color } : undefined}>{token.text}</span>
          <span className="text-[10.5px] text-jolu-muted">Tokens</span>
          {token.flag && <span className="sr-only">（{token.flag}）</span>}
        </span>
      </div>
      {colors && <span aria-hidden className="absolute -left-1 top-10 size-2 rounded-full border-[1.5px] bg-jolu-bg" style={{ borderColor: colors.color }} />}
    </div>
  )
}

/**
 * Token 角标（鼠鼠 2026-11-06 要求专门设计；补稿见 Jolu.pen「J6-04 · 统一网关 · 状态胶囊与 Token 角标」）：
 * 贴在 Token 胶囊左侧；刷新失败 = 红色三角 +「刷新失败」，过期 = 琥珀色时钟 +「已过期」；数值保留上次结果并转灰。
 */
function TokenBadge({ kind, label }: { kind: "failed" | "stale"; label: string }) {
  const Icon = kind === "failed" ? TriangleAlert : Clock3
  return (
    <span className={cn("mr-auto inline-flex h-[18px] items-center gap-[3px] rounded-[5px] px-[5px] text-[9.5px] font-semibold",
      kind === "failed" ? "bg-jolu-status-error-bg text-jolu-danger" : "bg-jolu-gateway-amber-soft text-jolu-gateway-amber")}>
      <Icon aria-hidden className="size-[11px]" />{label}
    </span>
  )
}
