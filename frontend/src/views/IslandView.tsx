import { useMemo, useState, type ReactNode } from "react"
import * as Menu from "@radix-ui/react-dropdown-menu"
import { CopyPlus, Ellipsis, Info, LoaderCircle, PictureInPicture2, RefreshCw, Scan } from "lucide-react"
import { api, isTauri, type AppSettings, type DockEdgeDto } from "@/lib/api"
import { cn } from "@/lib/utils"
import { PLATFORMS } from "@/lib/platforms"
import { SelectField, type SelectOption } from "@/components/ui/SelectField"
import { useToast } from "@/components/ui/Toast"
import { useSettings } from "@/hooks/useSettings"
import { useConnections } from "@/hooks/useConnections"

/** 本体 + 分身最多 8 个（后端 island_clones 同一上限） */
const MAX_ISLANDS = 8

const POSITION_OPTIONS: SelectOption[] = [
  { value: "free", label: "自由移动" },
  { value: "top", label: "顶部停靠" },
  { value: "bottom", label: "底部停靠" },
  { value: "left", label: "左侧停靠" },
  { value: "right", label: "右侧停靠" },
]

const SELECT = "h-8 rounded-[8px] border-jolu-border bg-jolu-bg px-2.5 text-jolu-text hover:bg-jolu-stage"

/**
 * 灵动岛页（Jolu.pen 模块 A 画板 01 / 02「完整灵动岛设置」）。
 * 设置即时生效，真值在后端，与托盘、灵动岛右键菜单共用同一份。
 */
export function IslandView() {
  const cfg = useSettings()
  const s = cfg.settings
  const savePreferences = cfg.setDisplayPreferences
  const disabled = !cfg.ready || cfg.saving

  return (
    <div className="flex w-full flex-col gap-3">
      {cfg.error && (
        <div role="alert" className="flex items-center justify-between gap-3 rounded-[10px] border border-jolu-danger bg-jolu-danger-soft px-4 py-2.5 text-xs text-jolu-danger">
          <span>{cfg.error}</span>
          <button type="button" disabled={cfg.loading || cfg.saving} onClick={() => void cfg.reload()} className="shrink-0 rounded-md px-2 py-1 font-medium hover:bg-jolu-danger-pressed">
            重新读取
          </button>
        </div>
      )}
      <div className="flex w-full flex-col gap-4">
        <Row
          card
          title="显示灵动岛"
          desc="控制本体与全部分身；与右上角按钮、系统托盘同步"
          className="h-[60px] rounded-[10px]"
          control={<Switch on={s.island_visible} disabled={disabled || !isTauri} label="显示灵动岛" onChange={(on) => void cfg.setIslandVisible(on)} />}
        />

        <div className="grid w-full grid-cols-2 gap-5">
          <Group title="窗口行为">
            <Row title="灵动岛置顶" desc="本体与全部分身保持在其他窗口之上" control={<Switch on={s.always_on_top} disabled={disabled} label="灵动岛置顶" onChange={(on) => void cfg.setTopmost(on)} />} />
            <Row title="贴边停靠" desc="拖到屏幕边缘自动吸附；关闭后解除停靠" control={<Switch on={s.dock_enabled} disabled={disabled} label="贴边停靠" onChange={(on) => void cfg.setDockEnabled(on)} />} />
            <Row title="免打扰" desc="暂停增量提示、水位提醒与通知；继续采集" control={<Switch on={s.dnd} disabled={disabled} label="免打扰" onChange={(on) => void cfg.setDnd(on)} />} />
          </Group>
          <Group title="显示外观">
            <Row
              title="透明度"
              desc="仅影响灵动岛，菜单保持清晰"
              control={<SelectField className={cn(SELECT, "w-[142px]")} label="透明度" disabled={disabled} value={String(s.island_opacity)} options={[60, 70, 80, 90, 100].map((n) => ({ value: String(n), label: `${n}%` }))} onValueChange={(v) => void savePreferences({ island_opacity: Number(v) })} />}
            />
            <Row
              title="大小"
              desc="用于探出与展开内容"
              control={<SelectField className={cn(SELECT, "w-[142px]")} label="大小" disabled={disabled} value={String(s.island_scale)} options={[{ value: "85", label: "紧凑 · 85%" }, { value: "100", label: "标准 · 100%" }, { value: "115", label: "大号 · 115%" }]} onValueChange={(v) => void savePreferences({ island_scale: Number(v) })} />}
            />
            <Row
              title="缩小后大小"
              desc="仅调整停靠条"
              control={<SelectField className={cn(SELECT, "w-[142px]")} label="缩小后大小" disabled={disabled} value={String(s.island_shrink_scale)} options={[{ value: "75", label: "迷你 · 75%" }, { value: "100", label: "标准 · 100%" }, { value: "125", label: "大号 · 125%" }]} onValueChange={(v) => void savePreferences({ island_shrink_scale: Number(v) })} />}
            />
          </Group>
        </div>

        <ClonesSection settings={s} ready={cfg.ready} />

        <Row
          card
          title="自动刷新间隔"
          desc="额度、余额与组织用量共用；本机会话采集仍实时运行"
          className="h-14 rounded-[9px]"
          control={
            <div className="flex items-center gap-2.5">
              <SelectField className={cn(SELECT, "w-[120px]")} label="自动刷新间隔" disabled={disabled} value={String(s.refresh_minutes)} options={[1, 5, 15, 30].map((n) => ({ value: String(n), label: `${n} 分钟` }))} onValueChange={(v) => void savePreferences({ refresh_minutes: Number(v) })} />
              <ActionButton action="refresh" icon={<RefreshCw aria-hidden className="size-4" />} label="立即刷新" done="已开始刷新" />
            </div>
          }
        />

        <Row
          card
          title="重置窗口位置"
          desc="恢复主面板尺寸与位置；本体及分身回到主显示器、解除停靠并重新显示"
          className="h-14 rounded-[9px]"
          control={<ActionButton action="reset_layout" icon={<Scan aria-hidden className="size-4" />} label="重置位置" done="已重置窗口位置" />}
        />
      </div>

      <p className="flex items-center gap-[7px] text-[11px] text-jolu-muted">
        <Info aria-hidden className="size-3.5 shrink-0" />
        设置即时生效；主面板与灵动岛同步切换主题。
      </p>
    </div>
  )
}

/** 「连接与分身」：本体在第一行，分身依次排列 */
function ClonesSection({ settings, ready }: { settings: AppSettings; ready: boolean }) {
  const conn = useConnections()
  const toast = useToast()
  const cfg = useSettings()
  const [source, setSource] = useState("")
  const [creating, setCreating] = useState(false)
  const count = 1 + settings.island_clones.length
  const hidden = !settings.island_visible

  const rows = useMemo(() => [
    { id: null as string | null, name: "本体", connectionId: settings.island_connection_id, connectionName: settings.island_connection_name, edge: settings.dock.edge },
    ...settings.island_clones.map((clone, index) => ({ id: clone.id as string | null, name: `分身 ${index + 1}`, connectionId: clone.connection_id, connectionName: clone.connection_name, edge: clone.dock.edge })),
  ], [settings])

  const connectionOptions = useMemo<SelectOption[]>(() => conn.connections.map((c) => ({
    value: c.id,
    label: `${PLATFORMS.find((p) => p.id === c.platformId)?.name ?? c.platformId} · ${c.name}`,
  })), [conn.connections])

  const sourceRow = rows.find((row) => (row.id ?? "") === source) ?? rows[0]
  const sourceOptions = rows.map((row) => ({ value: row.id ?? "", label: row.name }))

  const create = () => {
    setCreating(true)
    void api.islandAction("clone_create", sourceRow.id)
      .then(() => toast.success("已开启分身", `沿用${sourceRow.name}的连接`))
      .catch((reason) => toast.danger("开启分身失败", String(reason)))
      .finally(() => setCreating(false))
  }

  return (
    <section className="flex w-full flex-col gap-2">
      <div className="flex h-8 items-center justify-between gap-4">
        <div className="flex items-baseline gap-3">
          <h2 className="text-sm font-semibold text-jolu-text">连接与分身</h2>
          <span className="tnum text-[11px] text-jolu-muted">{count} / {MAX_ISLANDS}</span>
        </div>
        <div className="flex items-center gap-2">
          <SelectField
            className={cn(SELECT, "w-[130px]")}
            label="新分身来源"
            disabled={!ready}
            value={sourceRow.id ?? ""}
            options={sourceOptions}
            triggerLabel={`来源：${sourceRow.name}`}
            onValueChange={setSource}
          />
          <button
            type="button"
            disabled={!ready || !isTauri || creating || count >= MAX_ISLANDS}
            onClick={create}
            className="flex h-8 items-center gap-1.5 rounded-lg bg-jolu-selected px-2.5 text-xs font-medium text-jolu-on outline-none hover:brightness-[0.97] active:bg-jolu-pressed focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:pointer-events-none disabled:opacity-45"
          >
            {creating ? <LoaderCircle aria-hidden className="size-4 animate-spin" /> : <CopyPlus aria-hidden className="size-4" />}
            开启分身
          </button>
        </div>
      </div>

      <div role="table" aria-label="灵动岛窗口" className="overflow-hidden rounded-[9px] border border-jolu-border">
        <div role="row" className="grid h-7 grid-cols-[110px_minmax(0,1fr)_160px_32px] items-center gap-4 bg-jolu-side px-4 text-[11px] text-jolu-muted">
          <span role="columnheader">目标窗口</span>
          <span role="columnheader">显示连接</span>
          <span role="columnheader">显示位置</span>
          <span role="columnheader" className="whitespace-nowrap">操作</span>
        </div>
        {rows.map((row) => (
          <div key={row.id ?? "primary"} role="row" className="grid h-12 grid-cols-[110px_minmax(0,1fr)_160px_32px] items-center gap-4 border-t border-jolu-border px-4">
            <span role="cell" className="flex min-w-0 items-center gap-2 text-xs text-jolu-text">
              <PictureInPicture2 aria-hidden className="size-4 shrink-0 text-jolu-on" />
              <span className="truncate">{row.name}</span>
            </span>
            <span role="cell" className="min-w-0">
              <SelectField
                className={cn(SELECT, "w-full rounded-[6px]")}
                label={`${row.name}的显示连接`}
                disabled={!ready || hidden || conn.loading}
                value={row.connectionId ?? ""}
                options={connectionOptions}
                triggerLabel={row.connectionId ? undefined : row.connectionName ?? (conn.loading ? "读取连接中…" : "未选择连接")}
                onValueChange={(id) => void cfg.setIslandConnection(id, row.id)}
              />
            </span>
            <span role="cell">
              <SelectField
                className={cn(SELECT, "w-full")}
                label={`${row.name}的显示位置`}
                disabled={!ready || hidden || !isTauri}
                value={row.edge ?? "free"}
                options={POSITION_OPTIONS}
                onValueChange={(value) => {
                  void api.islandPosition(value === "free" ? null : value as DockEdgeDto, row.id)
                    .catch((reason) => toast.danger("调整显示位置失败", String(reason)))
                }}
              />
            </span>
            <span role="cell">
              <RowMenu name={row.name} canDestroy={count > 1 && isTauri} onDestroy={() => {
                void api.islandAction("clone_destroy", row.id)
                  .then(() => toast.success(`已销毁${row.name}`))
                  .catch((reason) => toast.danger("销毁失败", String(reason)))
              }} />
            </span>
          </div>
        ))}
      </div>
      {conn.error && <p role="status" className="text-[11px] text-jolu-danger">连接列表读取失败：{conn.error}</p>}
      <p className="text-[11px] text-jolu-muted">新分身沿用来源窗口的连接；最多 8 个，最后一个不可销毁。关闭显示时，连接与位置操作禁用。</p>
    </section>
  )
}

/** 分身「更多」菜单：危险操作收起在这里（Jolu.pen 画板 104「分身操作 · 收起危险操作」） */
function RowMenu({ name, canDestroy, onDestroy }: { name: string; canDestroy: boolean; onDestroy: () => void }) {
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger
        aria-label={`${name}的更多操作`}
        className="grid size-8 place-items-center rounded-lg bg-jolu-side text-jolu-muted outline-none hover:bg-jolu-stage active:bg-jolu-track focus-visible:outline-2 focus-visible:outline-jolu-accent data-[state=open]:bg-jolu-stage"
      >
        <Ellipsis aria-hidden className="size-[18px]" />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content align="end" sideOffset={6} collisionPadding={12} className="motion-popover z-[100] min-w-[140px] rounded-[8px] border border-jolu-border bg-jolu-bg p-1 shadow-popover">
          <Menu.Item
            disabled={!canDestroy}
            onSelect={onDestroy}
            className="flex h-8 cursor-default select-none items-center rounded-[6px] px-2.5 text-xs text-jolu-danger outline-none data-[highlighted]:bg-jolu-danger-soft data-[disabled]:text-jolu-muted data-[disabled]:opacity-60"
          >
            销毁此灵动岛
          </Menu.Item>
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  )
}

/** 立即刷新 / 重置位置：中性底按钮，执行中禁用并转圈 */
function ActionButton({ action, icon, label, done }: { action: "refresh" | "reset_layout"; icon: ReactNode; label: string; done: string }) {
  const toast = useToast()
  const [busy, setBusy] = useState(false)
  return (
    <button
      type="button"
      disabled={busy || !isTauri}
      onClick={() => {
        setBusy(true)
        void api.islandAction(action)
          .then(() => toast.success(done))
          .catch((reason) => toast.danger(`${label}失败`, String(reason)))
          .finally(() => setBusy(false))
      }}
      className="flex h-8 shrink-0 items-center gap-1.5 rounded-lg bg-jolu-side px-2.5 text-xs font-medium text-jolu-text outline-none hover:bg-jolu-stage active:bg-jolu-track focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:pointer-events-none disabled:opacity-45 [&_svg]:text-jolu-muted"
    >
      {busy ? <LoaderCircle aria-hidden className="size-4 animate-spin" /> : icon}
      {label}
    </button>
  )
}

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex min-w-0 flex-col gap-2">
      <h2 className="text-sm font-semibold text-jolu-text">{title}</h2>
      <div className="flex flex-col overflow-hidden rounded-[9px] border border-jolu-border [&>*+*]:border-t [&>*+*]:border-jolu-border">{children}</div>
    </section>
  )
}

/** 设置行：标题 13/500 + 说明 11 灰字，控件靠右；card 为独立带框的一行 */
function Row({ title, desc, control, card = false, className }: { title: string; desc: string; control: ReactNode; card?: boolean; className?: string }) {
  return (
    <div className={cn("flex h-14 items-center gap-[18px] px-4", card && "border border-jolu-border bg-jolu-bg", className)}>
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="text-[13px] font-medium text-jolu-text">{title}</span>
        <span className="truncate text-[11px] text-jolu-muted" title={desc}>{desc}</span>
      </div>
      <div className="shrink-0">{control}</div>
    </div>
  )
}

/** 开关 34×20，开启 jolu-accent、关闭 jolu-track，滑块 14 */
function Switch({ on, onChange, disabled, label }: { on: boolean; onChange: (next: boolean) => void; disabled?: boolean; label: string }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!on)}
      className={cn(
        "motion-toggle flex h-5 w-[34px] shrink-0 items-center rounded-full p-[3px] outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-jolu-accent disabled:opacity-45",
        on ? "bg-jolu-accent" : "bg-jolu-track",
      )}
    >
      <span className="motion-toggle-thumb size-3.5 rounded-full bg-white" style={{ transform: `translateX(${on ? 14 : 0}px)` }} />
    </button>
  )
}
