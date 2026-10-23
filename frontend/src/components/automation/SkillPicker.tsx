import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"
import { createPortal } from "react-dom"
import { Check, ChevronDown, ChevronUp, Info, Search, X } from "lucide-react"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import { MAX_SKILLS, projectOf, skillSummary, toolName, usableSkills } from "@/lib/automation"
import { automationApi, type AutomationPlatform, type SkillOptionDto } from "@/lib/automationApi"
import { demoSkills } from "@/mock/automation"

const TRIGGER = "flex h-[38px] w-full items-center justify-between gap-2.5 rounded-lg border bg-jolu-bg px-3 text-left text-xs outline-none focus-visible:border-jolu-accent"

/**
 * 「Skills（可选）」多选（画板 60、72；D-08）。字段文案、占位与「已选择 N 项 · a、b」按画板；
 * 弹出层画板没有画，以下为待确认的实现：搜索框 + 每项一个复选框，项目级的项后标项目名，最多选 10 个。
 * 候选只列目标工具已启用的全局 Skills 与当前工作目录下的项目 Skills；切换工具或工作目录后去掉已经无效的选择。
 * 编辑任务时，打开弹窗那一刻已失效的选择不会被悄悄删掉，而是在列表顶部标「已停用或不存在」（待确认）。
 */
export function SkillPicker({ platform, cwd, value, onChange, open, onOpenChange, root, scroller }: {
  platform: AutomationPlatform
  cwd: string
  value: string[]
  onChange: (names: string[]) => void
  open: boolean
  onOpenChange: (open: boolean) => void
  /** 弹窗根节点：下拉以它为定位容器，不被表单滚动区裁切 */
  root: HTMLElement | null
  scroller: HTMLElement | null
}) {
  const [options, setOptions] = useState<SkillOptionDto[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [query, setQuery] = useState("")
  const [place, setPlace] = useState<{ top: number; left: number; width: number; maxHeight: number } | null>(null)
  const ref = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const panelRef = useRef<HTMLDivElement>(null)
  const dir = cwd.trim()
  const key = `${platform}|${dir}`
  const initialKey = useRef(key)
  /** 工具或工作目录变过之后，加载到的候选用于清掉无效选择 */
  const changed = useRef(false)
  if (key !== initialKey.current) changed.current = true
  const latest = useRef({ value, onChange })
  latest.current = { value, onChange }

  useEffect(() => {
    let alive = true
    setError(null)
    // 手动输入目录时稍等再读取，避免每个字符都扫描一次
    const timer = window.setTimeout(() => {
      const load = isTauri ? automationApi.skills(platform, dir || null) : Promise.resolve(demoSkills(platform, dir || null))
      load.then((list) => {
        if (!alive) return
        setOptions(list)
        if (!changed.current) return
        const { value: selected, onChange: change } = latest.current
        const kept = usableSkills(selected, list)
        if (kept.length !== selected.length) change(kept)
      }, (e: unknown) => {
        if (!alive) return
        setOptions(null)
        setError(String(e))
      })
    }, dir ? 300 : 0)
    return () => { alive = false; window.clearTimeout(timer) }
  }, [platform, dir])

  useLayoutEffect(() => {
    if (!open || !root) return setPlace(null)
    const update = () => {
      const trigger = triggerRef.current
      if (!trigger) return
      const box = root.getBoundingClientRect()
      const r = trigger.getBoundingClientRect()
      const top = r.bottom - box.top + 6
      setPlace({ top, left: r.left - box.left, width: r.width, maxHeight: Math.max(160, box.height - top - 12) })
    }
    update()
    scroller?.addEventListener("scroll", update)
    window.addEventListener("resize", update)
    return () => {
      scroller?.removeEventListener("scroll", update)
      window.removeEventListener("resize", update)
    }
  }, [open, root, scroller])

  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node
      if (!ref.current?.contains(target) && !panelRef.current?.contains(target)) onOpenChange(false)
    }
    document.addEventListener("mousedown", onDown)
    return () => document.removeEventListener("mousedown", onDown)
  }, [open, onOpenChange])

  useEffect(() => { if (!open) setQuery("") }, [open])

  const missing = useMemo(() => (options ? value.filter((n) => !options.some((o) => o.name === n)) : []), [options, value])
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase()
    return (options ?? []).filter((o) => !q || `${o.name} ${o.description ?? ""} ${o.project ?? ""}`.toLowerCase().includes(q))
  }, [options, query])
  const full = value.length >= MAX_SKILLS
  const toggle = (name: string) => onChange(value.includes(name) ? value.filter((n) => n !== name) : full ? value : [...value, name])

  return (
    <div ref={ref} className="flex min-w-0 flex-col gap-[7px]">
      <span className="text-[11px] text-jolu-muted">Skills（可选）</span>
      <button
        ref={triggerRef}
        type="button"
        aria-label="选择 Skills"
        aria-expanded={open}
        onClick={() => onOpenChange(!open)}
        className={cn(TRIGGER, open ? "border-jolu-accent" : "border-jolu-border", missing.length && "border-jolu-danger")}
      >
        <span className={cn("truncate", value.length ? "text-jolu-text" : "text-jolu-muted")}>{value.length ? skillSummary(value) : "选择需要使用的 Skills（可选）"}</span>
        {open ? <ChevronUp aria-hidden className="size-[15px] shrink-0 text-jolu-muted" /> : <ChevronDown aria-hidden className="size-[15px] shrink-0 text-jolu-muted" />}
      </button>
      {missing.length > 0 && (
        <span role="alert" className="text-[10px] text-jolu-danger">{missing.map((n) => `「${n}」`).join("、")}已停用或不存在，执行时会转为需要处理；可在列表中取消选择。</span>
      )}

      {open && root && place && createPortal(
        <div
          ref={panelRef}
          role="dialog"
          aria-label="选择 Skills"
          style={{ top: place.top, left: place.left, width: place.width, maxHeight: place.maxHeight }}
          className="absolute z-10 flex flex-col gap-1.5 overflow-hidden rounded-xl border border-jolu-border bg-jolu-bg p-2 shadow-popover"
        >
          <label className="flex h-[38px] shrink-0 items-center gap-2 rounded-lg border border-jolu-accent px-2.5">
            <Search aria-hidden className="size-4 text-jolu-muted" />
            <input autoFocus value={query} onChange={(e) => setQuery(e.target.value)} placeholder="搜索 Skill 名称或说明" aria-label="搜索 Skills" className="min-w-0 flex-1 bg-transparent text-xs text-jolu-text outline-none placeholder:text-jolu-muted" />
            {query && <button type="button" aria-label="清除搜索" onClick={() => setQuery("")} className="text-jolu-muted hover:text-jolu-text"><X aria-hidden className="size-3.5" /></button>}
          </label>
          {error && <span role="alert" className="px-2.5 text-[11px] text-jolu-danger">Skills 读取失败：{error}</span>}
          {!error && options === null && <span className="px-2.5 text-[11px] text-jolu-muted">正在读取 Skills…</span>}
          {options !== null && !options.length && !missing.length && (
            <span className="px-2.5 text-[11px] text-jolu-muted">{toolName(platform)} 没有已启用的 Skills{dir ? "" : "；选择项目目录后会带出项目 Skills"}</span>
          )}
          {options !== null && !!options.length && !shown.length && <span className="px-2.5 text-[11px] text-jolu-muted">没有匹配的 Skills</span>}
          <div role="group" aria-label="Skills" className="flex max-h-[240px] min-h-0 shrink flex-col gap-0.5 overflow-y-auto">
            {missing.map((name) => (
              <SkillRow key={`missing-${name}`} name={name} note="已停用或不存在" checked danger onToggle={() => toggle(name)} />
            ))}
            {shown.map((o) => {
              const checked = value.includes(o.name)
              return (
                <SkillRow
                  key={o.name}
                  name={o.name}
                  note={o.description}
                  tag={o.scope === "project" ? projectOf(o.project) : undefined}
                  checked={checked}
                  disabled={!checked && full}
                  onToggle={() => toggle(o.name)}
                />
              )
            })}
          </div>
          <span className="border-t border-jolu-border px-2.5 pt-2 text-[10px] text-jolu-muted">
            {toolName(platform)}{dir ? ` · ${projectOf(dir)}` : ""} · 已选 {value.length} / {MAX_SKILLS}{full ? " · 已达上限" : ""}
          </span>
        </div>,
        root,
      )}
    </div>
  )
}

function SkillRow({ name, note, tag, checked, disabled, danger, onToggle }: {
  name: string
  note?: string | null
  tag?: string
  checked: boolean
  disabled?: boolean
  danger?: boolean
  onToggle: () => void
}) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      disabled={disabled}
      title={disabled ? `最多选择 ${MAX_SKILLS} 个 Skills` : undefined}
      onClick={onToggle}
      className={cn("flex min-h-[44px] shrink-0 items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left outline-none hover:bg-jolu-side focus-visible:bg-jolu-side disabled:cursor-not-allowed disabled:opacity-45 disabled:hover:bg-transparent", checked && "bg-jolu-selected hover:bg-jolu-selected")}
    >
      <span className={cn("grid size-4 shrink-0 place-items-center rounded-[4px]", checked ? "bg-jolu-accent" : "border border-jolu-border bg-jolu-bg")}>
        {checked && <Check aria-hidden className="size-3 text-[#071B10]" />}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex min-w-0 items-center gap-1.5">
          <span className={cn("truncate text-xs", danger ? "text-jolu-danger" : checked ? "text-jolu-on" : "text-jolu-text")}>{name}</span>
          {tag && <span className="shrink-0 rounded-[4px] bg-jolu-side px-1.5 text-[10px] text-jolu-muted">{tag}</span>}
        </span>
        {note && <span className={cn("truncate text-[10px]", danger ? "text-jolu-danger" : "text-jolu-muted")}>{note}</span>}
      </span>
      {danger && <Info aria-hidden className="size-4 shrink-0 text-jolu-danger" />}
    </button>
  )
}
