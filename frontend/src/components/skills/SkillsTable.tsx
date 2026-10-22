import * as Menu from "@radix-ui/react-dropdown-menu"
import { ChevronRight, Copy, Ellipsis, FolderOpen, Pencil, Square, SquareCheck, SquareMinus, Trash2 } from "lucide-react"
import { cn } from "@/lib/utils"
import { scopeLabel, sourceLabel, statusLabel, toggleBlocked } from "@/lib/skillsManager"
import type { SkillItemDto } from "@/lib/skillsApi"
import { providerName } from "@/components/usage/usageControls"
import { SkillIcon, SkillSwitch, Tag } from "./skillParts"

export function skillKey(item: Pick<SkillItemDto, "platform" | "path">): string {
  return `${item.platform}:${item.path}`
}

const COLS = "grid grid-cols-[32px_minmax(0,330fr)_minmax(0,120fr)_minmax(0,120fr)_minmax(0,126fr)_90px_102px] items-center gap-2.5"
const MENU_ITEM = "flex h-8 cursor-default select-none items-center gap-2 rounded-md px-2 text-xs text-jolu-text outline-none data-[highlighted]:bg-jolu-side data-[disabled]:opacity-40"

/** Skills 表格（画板 74）：勾选、名称描述、作用范围、来源、状态、启停与操作 */
export function SkillsTable({ items, total, selected, busy, showPlatform, onSelect, onSelectAll, onToggle, onOpen, onEdit, onCopy, onRemove, onOpenDir }: {
  items: SkillItemDto[]
  total: number
  selected: Set<string>
  busy: Set<string>
  showPlatform: boolean
  onSelect: (item: SkillItemDto) => void
  onSelectAll: (all: boolean) => void
  onToggle: (item: SkillItemDto, enabled: boolean) => void
  onOpen: (item: SkillItemDto) => void
  onEdit: (item: SkillItemDto) => void
  onCopy: (item: SkillItemDto) => void
  onRemove: (item: SkillItemDto) => void
  onOpenDir: (item: SkillItemDto) => void
}) {
  const picked = items.filter((i) => selected.has(skillKey(i))).length
  const HeadBox = picked === 0 ? Square : picked === items.length ? SquareCheck : SquareMinus
  return (
    <section className="overflow-hidden rounded-xl border border-jolu-border bg-jolu-bg">
      <div className={cn(COLS, "h-10 bg-jolu-side px-3.5 text-[11px] font-medium text-jolu-muted")}>
        <button type="button" aria-label={picked ? "取消全选" : "全选当前结果"} onClick={() => onSelectAll(picked === 0)} className="grid size-5 place-items-center rounded outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent">
          <HeadBox aria-hidden className={cn("size-3.5", picked ? "text-jolu-on" : "text-jolu-muted")} />
        </button>
        <span>Skill / 描述</span>
        <span>作用范围</span>
        <span>来源</span>
        <span>状态</span>
        <span>启停</span>
        <span>操作</span>
      </div>
      {items.map((item) => {
        const key = skillKey(item)
        const checked = selected.has(key)
        const blocked = toggleBlocked(item)
        return (
          <div
            key={key}
            className={cn(COLS, "h-[55px] border-t border-jolu-border px-3.5 transition-colors hover:bg-jolu-stage", checked && "bg-jolu-selected hover:bg-jolu-selected")}
          >
            <button type="button" role="checkbox" aria-checked={checked} aria-label={`选择 ${item.name}`} onClick={() => onSelect(item)} className="grid size-5 place-items-center rounded outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent">
              {checked ? <SquareCheck aria-hidden className="size-3.5 text-jolu-on" /> : <Square aria-hidden className="size-3.5 text-jolu-muted" />}
            </button>
            <button type="button" onClick={() => onOpen(item)} className="flex min-w-0 items-center gap-2.5 rounded-md text-left outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent">
              <SkillIcon item={item} />
              <span className="flex min-w-0 flex-col gap-1">
                <span className="truncate text-xs font-semibold text-jolu-text">{item.name}</span>
                <span className="truncate text-[10px] text-jolu-muted" title={item.issue ?? item.description ?? undefined}>
                  {showPlatform && `${providerName(item.platform)} · `}{item.issue ?? item.description ?? "没有描述"}
                </span>
              </span>
            </button>
            <span className="min-w-0"><Tag title={item.project ?? undefined}>{scopeLabel(item)}</Tag></span>
            <span className="truncate text-[11px] text-jolu-muted">{sourceLabel(item.source)}</span>
            <span className="min-w-0">
              <Tag tone={item.status === "enabled" ? "on" : item.status === "disabled" ? "muted" : "danger"} title={item.issue ?? undefined}>{statusLabel(item.status)}</Tag>
            </span>
            <span title={blocked ?? (item.disabled_by === "parked" ? "已移到 Jolu 停用区，启用时移回原位置" : undefined)}>
              <SkillSwitch
                on={item.enabled && item.status !== "invalid"}
                disabled={!!blocked || busy.has(key)}
                label={`${item.enabled ? "停用" : "启用"} ${item.name}`}
                onChange={(next) => onToggle(item, next)}
              />
            </span>
            <span className="flex items-center gap-1.5">
              <button type="button" onClick={() => onOpen(item)} className="inline-flex h-7 items-center gap-1 rounded-lg bg-jolu-selected px-2 text-xs font-medium text-jolu-on outline-none hover:bg-jolu-pressed focus-visible:outline-2 focus-visible:outline-jolu-accent">
                详情
                <ChevronRight aria-hidden className="size-4" />
              </button>
              <Menu.Root modal={false}>
                <Menu.Trigger aria-label={`${item.name} 更多操作`} className="grid h-7 w-6 place-items-center rounded-lg border border-jolu-border bg-jolu-bg text-jolu-text outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent data-[state=open]:bg-jolu-stage">
                  <Ellipsis aria-hidden className="size-4" />
                </Menu.Trigger>
                <Menu.Portal>
                  <Menu.Content align="end" sideOffset={6} collisionPadding={12} className="motion-popover z-[100] w-[176px] rounded-xl border border-jolu-border bg-jolu-bg p-1.5 shadow-popover">
                    <Menu.Item className={MENU_ITEM} onSelect={() => onOpenDir(item)}><FolderOpen aria-hidden className="size-4 text-jolu-muted" />打开目录</Menu.Item>
                    <Menu.Item className={MENU_ITEM} disabled={item.readonly} onSelect={() => onEdit(item)}><Pencil aria-hidden className="size-4 text-jolu-muted" />编辑 Skill</Menu.Item>
                    <Menu.Item className={MENU_ITEM} onSelect={() => onCopy(item)}><Copy aria-hidden className="size-4 text-jolu-muted" />复制到供应商</Menu.Item>
                    <Menu.Separator className="my-1 h-px bg-jolu-border" />
                    <Menu.Item className={cn(MENU_ITEM, "text-jolu-danger")} disabled={item.readonly} onSelect={() => onRemove(item)}><Trash2 aria-hidden className="size-4" />移除 Skill</Menu.Item>
                  </Menu.Content>
                </Menu.Portal>
              </Menu.Root>
            </span>
          </div>
        )
      })}
      <div className="flex h-11 items-center justify-between border-t border-jolu-border px-4 text-[11px] text-jolu-muted">
        <span>共 {total} 个安装项{items.length !== total && ` · 当前显示 ${items.length} 个`} · 按目录区分同名 Skill</span>
        <span>{picked ? `已选择 ${picked} 个` : "勾选后可批量管理"}</span>
      </div>
    </section>
  )
}
