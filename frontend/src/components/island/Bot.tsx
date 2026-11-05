import { useEffect, useRef, useState, type ReactNode } from "react"
import { cn } from "@/lib/utils"
import { compactTokens } from "@/lib/api"
import { useLiveCount } from "@/lib/liveCounts"
import { useGaze } from "@/hooks/useGaze"
import { gazeAmp, gazeBase } from "@/lib/botGaze"
import { formatShort, tickerNext, tickerStart, type TickerFace } from "@/lib/tokenTicker"
import type { BotExpr } from "@/lib/botExpression"

/**
 * 胶囊灵动岛的 bot：脸、眼睛、额度环（Jolu.pen A3-5 / A3-6，设计说明 §6）。
 * props 只收数据，不读全局状态；眼睛跟随与 Token 轮播都在内部直接改 DOM / 定时器，
 * 不让灵动岛每帧重渲染。
 */
const W = "#FFFFFF"
interface EyeStyle { k: string; c: string; g: string; R?: string; h?: number; s?: number; dy?: number; cls?: string }
const EYES: Record<BotExpr, EyeStyle> = {
  idle: { k: "pill", c: W, g: "#FFFFFF66" },
  blinkShot: { k: "line", c: W, g: "#FFFFFF66" },
  look: { k: "pill", c: W, g: "#FFFFFF66" },
  // 工作中的眼睛用主题绿：fill 由 capsule.css 的 `.cbot[data-expr="work"] .cbot-eyes-inner > *`
  // 覆盖成 --pn-capsule-green（浅深色各一套），这里的 c 只是 CSS 没命中时的兜底；
  // 光晕也换成同一个变量，免得浅色主题下深绿瞳孔外面套一圈亮薄荷色。
  work: { k: "pill", h: 0.85, c: "#5BF08A", g: "color-mix(in srgb, var(--pn-capsule-green) 55%, transparent)", cls: "cbot-scan" },
  refresh: { k: "spin", c: "#7DD8FF", g: "#5AC8FA99", cls: "cbot-spin" },
  happy: { k: "happy", c: W, g: "#FFFFFF66" },
  surprise: { k: "round", s: 1.15, c: W, g: "#FFFFFF66", cls: "cbot-hop" },
  greet: { k: "pill", R: "happy", c: W, g: "#FFFFFF66" },
  drag: { k: "gt", c: W, g: "#FFFFFF66" },
  peek: { k: "pill", h: 0.9, c: W, g: "#FFFFFF66" },
  worry: { k: "lid", c: "#FFB340", g: "#FF9F0A99", cls: "cbot-breathe" },
  sad: { k: "sad", dy: 0.05, c: "#FF6961", g: "#FF453A99" },
  error: { k: "x", c: "#FF6961", g: "#FF453A99", cls: "cbot-shake" },
  sleep: { k: "line", dy: 0.06, c: "#8E8E93", g: "transparent", cls: "cbot-sleep" },
  reset: { k: "star", c: "#5BF08A", g: "#30D15899" },
  wait: { k: "pill", h: 0.55, dy: 0.04, c: "#FFB340", g: "#FF9F0A99" },
  relay: { k: "pill", h: 0.85, c: "#7DD8FF", g: "#5AC8FA99" },
}

const n = (value: number) => Math.round(value * 100) / 100

/** 单只眼睛。F 为脸的直径，(cx, cy) 为眼睛中心，flip 用于左右镜像的形状。 */
function Eye({ kind, cx, cy, F, e, flip }: { kind: string; cx: number; cy: number; F: number; e: EyeStyle; flip: boolean }) {
  const sw = n(Math.max(1.5, F * 0.055))
  const stroke = { fill: "none", stroke: e.c, strokeWidth: sw, strokeLinecap: "round" as const, strokeLinejoin: "round" as const }
  switch (kind) {
    case "pill": {
      const w = F * 0.14, h = F * 0.3 * (e.h ?? 1)
      return <rect x={n(cx - w / 2)} y={n(cy - h / 2)} width={n(w)} height={n(h)} rx={n(w / 2)} fill={e.c} />
    }
    case "line": {
      const w = F * 0.224
      return <rect x={n(cx - w / 2)} y={n(cy - sw / 2)} width={n(w)} height={sw} rx={n(sw / 2)} fill={e.c} />
    }
    case "round":
      return <circle cx={n(cx)} cy={n(cy)} r={n(F * 0.085 * (e.s ?? 1))} fill={e.c} />
    case "happy":
      return <path d={`M${n(cx - F * 0.1)} ${n(cy + F * 0.045)} Q${n(cx)} ${n(cy - F * 0.1)} ${n(cx + F * 0.1)} ${n(cy + F * 0.045)}`} {...stroke} />
    case "sad":
      return <path d={`M${n(cx - F * 0.1)} ${n(cy - F * 0.045)} Q${n(cx)} ${n(cy + F * 0.1)} ${n(cx + F * 0.1)} ${n(cy - F * 0.045)}`} {...stroke} />
    case "x": {
      const a = F * 0.07
      return <path d={`M${n(cx - a)} ${n(cy - a)} L${n(cx + a)} ${n(cy + a)} M${n(cx + a)} ${n(cy - a)} L${n(cx - a)} ${n(cy + a)}`} {...stroke} />
    }
    case "gt": {
      const a = F * 0.06, b = F * 0.075, s = flip ? -1 : 1
      return <path d={`M${n(cx - a * s)} ${n(cy - b)} L${n(cx + a * s)} ${n(cy)} L${n(cx - a * s)} ${n(cy + b)}`} {...stroke} />
    }
    case "spin": {
      const r = F * 0.08, c = 2 * Math.PI * r
      return (
        <g transform={`rotate(${flip ? 180 : 0} ${n(cx)} ${n(cy)})`}>
          <circle className="cbot-spin-eye" style={{ ["--o" as string]: `${n(cx)}px ${n(cy)}px` }} cx={n(cx)} cy={n(cy)} r={n(r)}
            fill="none" stroke={e.c} strokeWidth={sw} strokeLinecap="round" strokeDasharray={`${n(c * 0.68)} ${n(c)}`} />
        </g>
      )
    }
    case "lid": {
      // 内侧上眼睑抬高，形成担心的眉形
      const w = F * 0.14, h = F * 0.234, x0 = cx - w / 2, y0 = cy - h * 0.46, u = w / 10, v = h / 18
      const p = (x: number, y: number) => `${n(x0 + x * u)} ${n(y0 + y * v)}`
      const top = flip ? `M${p(0, 0)} L${p(10, 3)}` : `M${p(0, 3)} L${p(10, 0)}`
      return <path d={`${top} L${p(10, 14)} Q${p(10, 18)} ${p(5, 18)} Q${p(0, 18)} ${p(0, 14)} Z`} fill={e.c} />
    }
    case "star": {
      const r = F * 0.11, k = r * 0.12
      return <path d={`M${n(cx)} ${n(cy - r)} Q${n(cx + k)} ${n(cy - k)} ${n(cx + r)} ${n(cy)} Q${n(cx + k)} ${n(cy + k)} ${n(cx)} ${n(cy + r)} Q${n(cx - k)} ${n(cy + k)} ${n(cx - r)} ${n(cy)} Q${n(cx - k)} ${n(cy - k)} ${n(cx)} ${n(cy - r)} Z`} fill={e.c} />
    }
  }
  return null
}

/** 逐帧 Token 数：叶子组件订阅，追数时只有这里重渲染 */
function LiveText({ countKey, short, ...rest }: { countKey: string; short: boolean } & React.SVGProps<SVGTextElement>) {
  const value = useLiveCount(countKey) ?? 0
  return <text {...rest}>{short ? formatShort(value) : `+${compactTokens(value)}`}</text>
}

export interface BotRing {
  /** 已用百分比；未知为 null，只画空轨道 */
  p: number | null
  /** CSS 颜色，如 var(--pn-capsule-green) */
  color: string
  warn?: boolean
  aria?: string
}

export interface BotProps {
  size: number
  /** 环宽、环间距、无环时的内边距 */
  ringWidth?: number
  ringGap?: number
  inset?: number
  rings?: BotRing[]
  expr: BotExpr
  dockEdge?: "top" | "bottom" | "left" | "right" | null
  /** 运行中 Token 轮播：countKey 指向 LiveCountsContext 的键 */
  ticker?: { countKey: string; label: string; sub?: string } | null
  /** 聚焦的环序号：其余环变淡，屏幕改显数值 */
  focus?: number | null
  display?: [string, string, string?] | null
  interactive?: boolean
  onRingHover?: (index: number | null) => void
  onRingClick?: (index: number) => void
  pinnedRing?: number | null
  className?: string
}

const SMALL_FACE = 40
const MIN_TICK_FACE = 16

export function Bot({
  size, ringWidth = 0, ringGap = 0, inset = 0, rings = [], expr, dockEdge = null, ticker = null,
  focus = null, display = null, interactive = false, onRingHover, onRingClick, pinnedRing = null, className,
}: BotProps) {
  const count = rings.length
  const offset = count ? count * (ringWidth + ringGap) : inset
  const F = size - 2 * offset
  const small = F < SMALL_FACE
  const e = EYES[expr] ?? EYES.idle
  const c = size / 2

  const svgRef = useRef<SVGSVGElement>(null)
  const faceRef = useRef<SVGCircleElement>(null)
  const eyesRef = useRef<SVGGElement>(null)
  useGaze(faceRef, eyesRef, { base: gazeBase(expr, dockEdge), amp: gazeAmp(expr) }, F)

  // Token 轮播：首轮先看眼睛 2 秒，之后眼睛 4 秒、Token 3 秒；迷你尺寸不显示数字
  const tickerOn = Boolean(ticker) && F >= MIN_TICK_FACE && !display
  const [face, setFace] = useState<TickerFace>("eyes")
  useEffect(() => {
    if (!tickerOn) { setFace("eyes"); return }
    let timer = 0
    let step = tickerStart()
    setFace(step.face)
    const advance = () => {
      step = tickerNext(step.face)
      setFace(step.face)
      timer = window.setTimeout(advance, step.delay)
    }
    timer = window.setTimeout(advance, step.delay)
    return () => window.clearTimeout(timer)
  }, [tickerOn])
  const ticking = tickerOn && face === "tick"

  // 空闲类表情每 4–6 秒眨一次眼
  useEffect(() => {
    if (!["idle", "look", "peek"].includes(expr)) return
    let timer = 0, reset = 0
    const schedule = () => {
      timer = window.setTimeout(() => {
        const svg = svgRef.current
        if (svg) { svg.classList.add("cbot-blink"); reset = window.setTimeout(() => svg.classList.remove("cbot-blink"), 160) }
        schedule()
      }, 4000 + Math.random() * 2000)
    }
    schedule()
    return () => { window.clearTimeout(timer); window.clearTimeout(reset) }
  }, [expr])

  // 完成：外圈走一圈高光
  const previous = useRef<BotExpr>(expr)
  useEffect(() => {
    const svg = svgRef.current
    if (expr === "happy" && previous.current !== "happy" && svg && count) {
      svg.classList.remove("cbot-sweeping"); void svg.getBoundingClientRect(); svg.classList.add("cbot-sweeping")
      const timer = window.setTimeout(() => svg.classList.remove("cbot-sweeping"), 1150)
      previous.current = expr
      return () => window.clearTimeout(timer)
    }
    previous.current = expr
  }, [expr, count])

  const g = F * 0.135, cy = F / 2 + (e.dy ?? 0) * F
  const ringEls: ReactNode[] = rings.map((ring, i) => {
    const r = size / 2 - i * (ringWidth + ringGap) - ringWidth / 2
    const C = 2 * Math.PI * r
    const p = ring.p === null ? 0 : Math.max(0, Math.min(100, ring.p)) / 100
    const dimmed = focus != null && focus !== i
    const w = focus === i ? ringWidth * 1.35 : ringWidth
    return (
      <g
        key={i}
        className="cbot-ring"
        {...(interactive ? {
          tabIndex: 0, role: "button", "aria-label": ring.aria, "aria-pressed": pinnedRing === i,
          onPointerEnter: () => onRingHover?.(i), onPointerLeave: () => onRingHover?.(null),
          onFocus: () => onRingHover?.(i), onBlur: () => onRingHover?.(null),
          onClick: () => onRingClick?.(i),
          onDoubleClick: (event: React.MouseEvent) => event.stopPropagation(),
          onKeyDown: (event: React.KeyboardEvent) => {
            if (event.key === "Enter" || event.key === " ") { event.preventDefault(); event.stopPropagation(); onRingClick?.(i) }
          },
        } : {})}
      >
        <circle className="cbot-ring-track" cx={c} cy={c} r={n(r)} fill="none" stroke="var(--pn-capsule-track)" strokeWidth={w} style={{ opacity: dimmed ? 0.5 : 1 }} />
        <circle className={cn("cbot-ring-arc", ring.warn && "cbot-warn-ring")} cx={c} cy={c} r={n(r)} fill="none" stroke={ring.color} strokeWidth={w}
          strokeLinecap="round" strokeDasharray={n(C)} strokeDashoffset={n(C * (1 - p))} transform={`rotate(-90 ${c} ${c})`}
          style={{ opacity: p <= 0 ? 0 : dimmed ? 0.3 : 1 }} />
        {i === 0 && (
          <circle className="cbot-sweep" cx={c} cy={c} r={n(r)} fill="none" stroke="#FFFFFF" strokeWidth={ringWidth} strokeLinecap="round"
            strokeDasharray={`${n(C * 0.1)} ${n(C)}`} transform={`rotate(-90 ${c} ${c})`} style={{ ["--c" as string]: n(C) }} />
        )}
        {interactive && <circle cx={c} cy={c} r={n(r)} fill="none" stroke="transparent" strokeWidth={ringWidth + ringGap + 2} pointerEvents="stroke" />}
      </g>
    )
  })

  return (
    // data-expr 给 capsule.css 用：只有「工作中」把眼睛染成 --pn-capsule-green，
    // 其他表达式（含表情库预览）保持原色；blink / scan 的 keyframes 不受影响。
    <svg
      ref={svgRef}
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      aria-hidden={interactive ? undefined : true}
      className={cn("cbot", e.cls, ticking && "cbot-ticking", display && "cbot-showing", className)}
      data-expr={expr}
      data-small={small && count ? 1 : 0}
      style={{ ["--F" as string]: n(F), ["--z" as string]: n(size / F), ["--glow" as string]: e.g }}
    >
      {ringEls}
      <g transform={`translate(${n(offset)} ${n(offset)})`}>
        <g className="cbot-zoom">
          <circle ref={faceRef} cx={n(F / 2)} cy={n(F / 2)} r={n(F / 2 - 0.5)} fill="var(--pn-capsule-bot)" stroke="var(--pn-capsule-bot-stroke)" strokeWidth={1} />
          <g ref={eyesRef} className="cbot-eyes" style={{ filter: `drop-shadow(0 0 ${n(Math.max(1.5, F * 0.1))}px var(--glow))` }}>
            <g className="cbot-eyes-slide">
              <g className="cbot-eyes-inner" key={expr}>
                <Eye kind={e.k} cx={F / 2 - g} cy={cy} F={F} e={e} flip={false} />
                <Eye kind={e.R ?? e.k} cx={F / 2 + g} cy={cy} F={F} e={e} flip />
              </g>
            </g>
          </g>
          {ticker && (
            <g className="cbot-tick" textAnchor="middle" fontFamily="var(--font-mono, ui-monospace)" style={{ filter: `drop-shadow(0 0 ${n(Math.max(1.5, F * 0.06))}px #30D15880)` }}>
              {small ? (
                <LiveText countKey={ticker.countKey} short x={n(F / 2)} y={n(F / 2 + F * 0.13)} fontSize={n(F * 0.36)} fontWeight={700} fill="#5BF08A" letterSpacing={-0.3} />
              ) : (
                <>
                  <LiveText countKey={ticker.countKey} short={false} x={n(F / 2)} y={n(F * 0.53)} fontSize={n(F * 0.2)} fontWeight={700} fill="#5BF08A" letterSpacing={-0.6} />
                  <text x={n(F / 2)} y={n(F * 0.67)} fontSize={n(F * 0.095)} fontWeight={600} fill="#FFFFFFB3">{ticker.label}</text>
                  {ticker.sub && <text x={n(F / 2)} y={n(F * 0.79)} fontSize={n(F * 0.08)} fontWeight={500} fill="#FFFFFF80">{ticker.sub}</text>}
                </>
              )}
            </g>
          )}
          {display && (
            <g className="cbot-disp" textAnchor="middle">
              <text x={n(F / 2)} y={n(F * 0.52)} fontSize={n(F * 0.24)} fontWeight={700} fill="#FFFFFF" letterSpacing={-0.5}>{display[0]}</text>
              <text x={n(F / 2)} y={n(F * 0.66)} fontSize={n(F * 0.095)} fontWeight={600} fill="#FFFFFFB3">{display[1]}</text>
              {display[2] && <text x={n(F / 2)} y={n(F * 0.78)} fontSize={n(F * 0.08)} fontWeight={500} fill="#FFFFFF80">{display[2]}</text>}
            </g>
          )}
          <g className="cbot-zz" fontWeight={700}>
            <text x={n(F * 0.68)} y={n(F * 0.34)} fontSize={n(F * 0.17)} fill="#FFFFFF80">z</text>
            <text x={n(F * 0.8)} y={n(F * 0.22)} fontSize={n(F * 0.12)} fill="#FFFFFF55">z</text>
          </g>
        </g>
      </g>
    </svg>
  )
}
