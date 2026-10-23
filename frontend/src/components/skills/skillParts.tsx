import * as Dialog from "@radix-ui/react-dialog"
import * as Menu from "@radix-ui/react-dropdown-menu"
import { useState, type ReactNode } from "react"
import {
  Blocks, Check, ChevronDown, Code, FilePenLine, Files, FileX, FlaskConical, FolderCode, GitCommitHorizontal, Hexagon,
  LayoutDashboard, NotebookPen, Palette, ScanText, Search, X,
} from "lucide-react"
import { cn } from "@/lib/utils"
import { PENDING_SKILL_PROVIDERS, SKILL_PLATFORMS, inlineParts, markdownBlocks, splitFrontmatter } from "@/lib/skillsManager"
import type { SkillItemDto, SkillPlatformDto } from "@/lib/skillsApi"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { providerName } from "@/components/usage/usageControls"
import type { PlatformId } from "@/types"

/* ───────────── 小部件 ───────────── */

/** 标签：作用范围 / 状态（画板 74，高 24、圆角 6） */
export function Tag({ tone = "muted", children, title }: { tone?: "muted" | "on" | "danger"; children: ReactNode; title?: string }) {
  return (
    <span
      title={title}
      className={cn(
        "inline-flex h-6 max-w-full shrink-0 items-center truncate rounded-md px-2 text-[10px]",
        tone === "on" && "bg-jolu-selected text-jolu-on",
        tone === "muted" && "bg-jolu-side text-jolu-muted",
        tone === "danger" && "bg-jolu-danger-soft text-jolu-danger",
      )}
    >
      <span className="truncate">{children}</span>
    </span>
  )
}

/** 启停开关：30×18，开为品牌绿、关为轨道色（画板 74） */
export function SkillSwitch({ on, disabled, label, onChange }: { on: boolean; disabled?: boolean; label: string; onChange: (next: boolean) => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!on)}
      className={cn(
        "motion-toggle flex h-[18px] w-[30px] shrink-0 items-center rounded-full p-0.5 outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-45",
        on ? "bg-jolu-accent" : "bg-jolu-track",
      )}
    >
      <span className="motion-toggle-thumb size-[14px] rounded-full bg-white" style={{ transform: `translateX(${on ? 12 : 0}px)` }} />
    </button>
  )
}

const ICONS = [Palette, ScanText, FolderCode, FlaskConical, GitCommitHorizontal, FilePenLine, NotebookPen, LayoutDashboard]

/** 列表图标：异常项用固定图标；其余按名称稳定取一个，只作区分（画板 74 的示例图标） */
export function SkillIcon({ item, size = 30 }: { item: Pick<SkillItemDto, "name" | "status">; size?: 30 | 48 }) {
  let Icon = ICONS[[...item.name].reduce((sum, ch) => sum + ch.charCodeAt(0), 0) % ICONS.length]
  if (item.status === "invalid") Icon = FileX
  if (item.status === "conflict") Icon = Files
  return (
    <span className={cn("grid shrink-0 place-items-center", size === 30 ? "size-[30px] rounded-lg bg-jolu-side" : "size-12 rounded-xl bg-jolu-selected")}>
      <Icon aria-hidden className={cn("text-jolu-on", size === 30 ? "size-4" : "size-[22px]")} />
    </span>
  )
}

export function ProviderMark({ platform, size = 18 }: { platform: string; size?: number }) {
  if (platform === "all") return <Blocks aria-hidden className="text-jolu-muted" style={{ width: size - 2, height: size - 2 }} />
  return <PlatformLogo platform={platform as PlatformId} size={size} />
}

/* ───────────── 供应商下拉（画板 76） ───────────── */

export function ProviderMenu({ value, platforms, onChange }: { value: string; platforms: SkillPlatformDto[]; onChange: (platform: string) => void }) {
  const [query, setQuery] = useState("")
  const total = platforms.reduce((sum, p) => sum + p.count, 0)
  const q = query.trim().toLowerCase()
  const show = (name: string) => !q || name.toLowerCase().includes(q)
  const row = (id: string, name: string, note: string, enabled: boolean, icon: ReactNode) => (
    <Menu.Item
      key={id}
      disabled={!enabled}
      onSelect={() => onChange(id)}
      className={cn(
        "flex h-[34px] cursor-default select-none items-center gap-2.5 rounded-[7px] px-2 text-xs outline-none data-[highlighted]:bg-jolu-side data-[disabled]:opacity-40",
        value === id ? "bg-jolu-selected text-jolu-on data-[highlighted]:bg-jolu-selected" : "text-jolu-text",
      )}
    >
      <span aria-hidden className="grid size-[18px] shrink-0 place-items-center">{icon}</span>
      <span className="min-w-0 flex-1 truncate">{name}</span>
      <span className="shrink-0 text-[10px] text-jolu-muted">{note}</span>
      {value === id && <Check aria-hidden className="size-3.5 shrink-0 text-jolu-on" />}
    </Menu.Item>
  )
  return (
    <Menu.Root modal={false} onOpenChange={(open) => !open && setQuery("")}>
      <Menu.Trigger
        aria-label="选择供应商"
        className="inline-flex h-[38px] w-[176px] shrink-0 items-center gap-2 rounded-lg border border-jolu-border bg-jolu-bg px-3 text-xs font-medium text-jolu-text outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent data-[state=open]:bg-jolu-stage"
      >
        <ProviderMark platform={value} />
        <span className="min-w-0 flex-1 truncate text-left">{value === "all" ? "全部供应商" : providerName(value)}</span>
        <ChevronDown aria-hidden className="size-4 text-jolu-muted" />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content align="start" sideOffset={6} collisionPadding={12} className="motion-popover z-[100] flex w-[336px] flex-col gap-2 rounded-xl border border-jolu-border bg-jolu-bg p-3 shadow-popover" style={{ maxHeight: "min(520px, var(--radix-dropdown-menu-content-available-height))" }}>
          <label className="flex h-[34px] shrink-0 items-center gap-2 rounded-[7px] bg-jolu-side px-2.5">
            <Search aria-hidden className="size-4 text-jolu-muted" />
            <input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => event.stopPropagation()}
              placeholder="搜索供应商"
              aria-label="搜索供应商"
              className="min-w-0 flex-1 bg-transparent text-xs text-jolu-text outline-none placeholder:text-jolu-muted"
            />
          </label>
          <div className="flex min-h-0 flex-col gap-0.5 overflow-y-auto">
            {show("全部供应商") && row("all", "全部供应商", `${total} 个安装项`, true, <Blocks className="size-4 text-jolu-muted" />)}
            {SKILL_PLATFORMS.filter((id) => show(providerName(id))).map((id) => {
              const p = platforms.find((x) => x.id === id)
              return row(id, providerName(id), p?.detected ? `${p.count} 个安装项` : "未检测到目录", true, <PlatformLogo platform={id} size={18} />)
            })}
            {PENDING_SKILL_PROVIDERS.filter((p) => show(p.name)).map((p) => row(p.id, p.name, "待接入", false, <PlatformLogo platform={p.id as PlatformId} size={18} />))}
            {show("自定义") && row("custom", "自定义", "待接入", false, (
              <span className="relative grid size-5 place-items-center rounded-md bg-jolu-selected text-jolu-on">
                <Hexagon className="absolute size-4" />
                <Code className="size-2.5" />
              </span>
            ))}
          </div>
          <span className="text-[10px] text-jolu-muted">安装项按供应商与实际目录区分；未核对目录规则的工具标「待接入」。</span>
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}

/* ───────────── 表单下拉 ───────────── */

export interface FieldOption {
  value: string
  label: string
  disabled?: boolean
  note?: string
}

/** 38px 高的表单下拉（画板 80 / 82 的「供应商 *」「作用范围 *」） */
export function FieldSelect({ value, options, onChange, label, disabled, placeholder }: {
  value: string
  options: FieldOption[]
  onChange: (value: string) => void
  label: string
  disabled?: boolean
  placeholder?: string
}) {
  const selected = options.find((o) => o.value === value)
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger
        aria-label={label}
        disabled={disabled}
        className="flex h-[38px] w-full min-w-0 items-center gap-2 rounded-lg border border-jolu-border bg-jolu-bg px-3 text-left text-xs text-jolu-text outline-none hover:bg-jolu-stage focus-visible:border-jolu-accent data-[state=open]:border-jolu-accent disabled:cursor-not-allowed disabled:bg-jolu-side disabled:text-jolu-muted"
      >
        <span className={cn("min-w-0 flex-1 truncate", !selected && "text-jolu-muted")}>{selected?.label ?? placeholder ?? "请选择"}</span>
        <ChevronDown aria-hidden className="size-4 shrink-0 text-jolu-muted" />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content align="start" sideOffset={6} collisionPadding={12} className="motion-popover z-[130] min-w-[var(--radix-dropdown-menu-trigger-width)] overflow-y-auto rounded-xl border border-jolu-border bg-jolu-bg p-1.5 shadow-popover" style={{ maxHeight: "min(300px, var(--radix-dropdown-menu-content-available-height))" }}>
          <Menu.RadioGroup value={value} onValueChange={onChange}>
            {options.map((o) => (
              <Menu.RadioItem key={o.value} value={o.value} disabled={o.disabled} className="relative flex h-8 cursor-default select-none items-center gap-2 rounded-md pl-2 pr-7 text-xs text-jolu-text outline-none data-[highlighted]:bg-jolu-side data-[state=checked]:text-jolu-on data-[disabled]:opacity-40">
                <span className="min-w-0 flex-1 truncate">{o.label}</span>
                {o.note && <span className="shrink-0 text-[10px] text-jolu-muted">{o.note}</span>}
                <Menu.ItemIndicator className="absolute right-2"><Check aria-hidden className="size-3.5" /></Menu.ItemIndicator>
              </Menu.RadioItem>
            ))}
          </Menu.RadioGroup>
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}

export function Field({ label, children, className }: { label: string; children: ReactNode; className?: string }) {
  return (
    <div className={cn("flex min-w-0 flex-1 flex-col gap-[7px]", className)}>
      <span className="text-xs font-medium text-jolu-text">{label}</span>
      {children}
    </div>
  )
}

export const INPUT_CLASS = "h-[38px] w-full min-w-0 rounded-lg border border-jolu-border bg-jolu-bg px-3 text-xs text-jolu-text outline-none placeholder:text-jolu-muted focus:border-jolu-accent disabled:bg-jolu-side disabled:text-jolu-muted"

/* ───────────── 弹窗外壳 ───────────── */

export function DialogShell({ open = true, width, title, onClose, busy, children, height }: {
  open?: boolean
  width: number
  title: ReactNode
  onClose: () => void
  busy?: boolean
  children: ReactNode
  height?: number
}) {
  return (
    <Dialog.Root open={open} onOpenChange={(open) => !open && !busy && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className="motion-overlay fixed inset-x-0 bottom-0 top-[30px] z-50 bg-black/40" />
        <Dialog.Content
          aria-describedby={undefined}
          className="motion-dialog fixed left-1/2 top-[calc(50%+15px)] z-50 flex max-h-[calc(100dvh-70px)] max-w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col gap-3.5 overflow-y-auto rounded-2xl border border-jolu-border bg-jolu-bg p-6 shadow-dialog outline-none"
          style={{ width, height }}
        >
          <div className="flex min-h-7 items-center gap-2.5">
            <Dialog.Title className="min-w-0 flex-1 truncate text-xl font-semibold text-jolu-text">{title}</Dialog.Title>
            <Dialog.Close aria-label="关闭" disabled={busy} className="grid size-7 place-items-center rounded-lg border border-jolu-border bg-jolu-bg text-jolu-text outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:opacity-45">
              <X aria-hidden className="size-4" />
            </Dialog.Close>
          </div>
          {children}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

/* ───────────── Markdown 预览 ───────────── */

function Inline({ text }: { text: string }) {
  return (
    <>
      {inlineParts(text).map((part, i) =>
        part.kind === "code" ? <code key={i} className="rounded bg-jolu-side px-1 font-mono text-[11px]">{part.text}</code>
          : part.kind === "strong" ? <strong key={i} className="font-semibold">{part.text}</strong>
            : <span key={i}>{part.text}</span>,
      )}
    </>
  )
}

/** SKILL.md 预览：忽略 frontmatter，覆盖标题、段落、列表、代码块（画板 78 / 80） */
export function MarkdownPreview({ source, stripFront = true }: { source: string; stripFront?: boolean }) {
  const body = stripFront ? splitFrontmatter(source).body : source
  const blocks = markdownBlocks(body)
  if (!blocks.length) return <p className="text-xs text-jolu-muted">没有正文内容</p>
  let first = true
  return (
    <div className="flex flex-col gap-[18px] break-words">
      {blocks.map((block, i) => {
        if (block.kind === "heading") {
          const top = first && block.level === 1
          first = false
          return (
            <h3 key={i} className={cn("font-semibold text-jolu-text", top ? "text-2xl" : block.level <= 2 ? "text-sm" : "text-[13px]")}>
              <Inline text={block.text} />
            </h3>
          )
        }
        first = false
        if (block.kind === "paragraph") return <p key={i} className="whitespace-pre-wrap text-xs leading-5 text-jolu-text"><Inline text={block.text} /></p>
        if (block.kind === "list") {
          const List = block.ordered ? "ol" : "ul"
          return (
            <List key={i} className={cn("flex flex-col gap-1 pl-5 text-xs leading-5 text-jolu-text", block.ordered ? "list-decimal" : "list-disc")}>
              {block.items.map((text, j) => <li key={j}><Inline text={text} /></li>)}
            </List>
          )
        }
        if (block.kind === "code") return <pre key={i} className="overflow-x-auto rounded-lg bg-jolu-side p-3 font-mono text-[11px] leading-5 text-jolu-text">{block.text}</pre>
        return <hr key={i} className="border-jolu-border" />
      })}
    </div>
  )
}
