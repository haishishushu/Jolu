import { queryCache } from "@/lib/queryCache"
import { useContentMotion } from "@/lib/motion"
import { LoadingSurface } from "@/components/ui/LoadingSurface"
import { useCallback, useEffect, useMemo, useState } from "react"
import { Blocks, Check, CircleCheck, Copy, Download, FolderCog, FolderOpen, Pause, Plus, RefreshCw, RotateCcw, Search, TriangleAlert, X } from "lucide-react"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import { skillsApi, type SkillItemDto, type SkillsOverviewDto } from "@/lib/skillsApi"
import {
  EMPTY_FILTERS, STATUS_OPTIONS, filterSkills, hasFilters, scanLocation, scannedLabel, scannedProjects, scopeOptions, skillStats, targetProjects,
  toggleBlocked, type ScopeFilter, type SkillFilters, type StatusFilter,
} from "@/lib/skillsManager"
import { demoSkillsOverview } from "@/mock/skills"
import { JoluButton } from "@/components/ui/JoluButton"
import { useToast } from "@/components/ui/Toast"
import { MenuFilter } from "@/components/usage/usageControls"
import { SkillDetail } from "@/components/skills/SkillDetail"
import { SkillEditor, type EditorTarget } from "@/components/skills/SkillEditor"
import { SkillsTable, skillKey } from "@/components/skills/SkillsTable"
import { CopyDialog, DirectoryDialog, ImportDialog, RemoveDialog } from "@/components/skills/skillDialogs"
import { ProviderMenu } from "@/components/skills/skillParts"

type Mode = { kind: "list" } | { kind: "detail"; item: SkillItemDto } | { kind: "editor"; target: EditorTarget; back: SkillItemDto | null }

function errorText(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason)
}

/**
 * Skills 管理（Jolu.pen 模块 F，画板 74–85）：按供应商浏览本机 Skills，查看详情与文件、新建 / 编辑、导入、复制、启停与移除。
 * 已核对目录规则的只有 Claude Code 与 Codex；其他供应商标「待接入」。导入支持本地目录与 Git 仓库（本机 git 浅克隆）。
 */
export function SkillsView({ active }: { active: boolean }) {
  const toast = useToast()
  const [platform, setPlatform] = useState("claude")
  const [data, setData] = useState<SkillsOverviewDto | null>(() => queryCache.get("skills:claude"))
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [tick, setTick] = useState(0)
  /** 「扫描目录」时连会话记录里的项目目录一起重读，其余刷新用缓存 */
  const [force, setForce] = useState(false)
  const [filters, setFilters] = useState<SkillFilters>(EMPTY_FILTERS)
  const [searchDraft, setSearchDraft] = useState("")
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState<Set<string>>(new Set())
  const [mode, setMode] = useState<Mode>({ kind: "list" })
  const [dialog, setDialog] = useState<null | { kind: "import" } | { kind: "dirs" } | { kind: "copy"; items: SkillItemDto[] } | { kind: "remove"; item: SkillItemDto }>(null)
  const [batch, setBatch] = useState<string | null>(null)
  const [now, setNow] = useState(() => Date.now())

  const refresh = useCallback(() => setTick((t) => t + 1), [])
  const rescan = () => {
    setForce(true)
    refresh()
  }

  useEffect(() => {
    if (!active) return
    let stale = false
    setLoading(true)
    const load = isTauri ? skillsApi.overview(platform, force) : Promise.resolve(demoSkillsOverview(platform))
    setForce(false)
    load.then((value) => {
      if (stale) return
      queryCache.set(`skills:${platform}`, value)
      setData(value)
      setError(null)
    }, (reason: unknown) => !stale && setError(errorText(reason))).finally(() => !stale && setLoading(false))
    return () => { stale = true }
  }, [platform, tick, active])

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 30_000)
    return () => window.clearInterval(timer)
  }, [])

  const items = useMemo(() => data?.items ?? [], [data])
  const visible = useMemo(() => filterSkills(items, filters), [items, filters])
  const stats = skillStats(items)
  const filtered = hasFilters(filters)
  const selectedItems = items.filter((i) => selected.has(skillKey(i)))
  const scanned = useMemo(() => scannedProjects(data?.projects ?? [], platform), [data, platform])
  const targets = useMemo(() => targetProjects(data?.projects ?? []), [data])

  const contentMotion = useContentMotion(`${mode.kind}-${platform}-${!!data}`)

  const changePlatform = (next: string) => {
    setData(queryCache.get<SkillsOverviewDto>(`skills:${next}`))
    setLoading(true)
    setError(null)
    setPlatform(next)
    setSelected(new Set())
    setFilters((f) => ({ ...f, scope: "all" }))
  }

  const clearFilters = () => {
    setFilters(EMPTY_FILTERS)
    setSearchDraft("")
  }

  const toggle = async (item: SkillItemDto, enabled: boolean) => {
    const key = skillKey(item)
    if (!isTauri) {
      setData((d) => d && { ...d, items: d.items.map((i) => (skillKey(i) === key ? { ...i, enabled, status: enabled ? "enabled" : "disabled" } : i)) })
      return
    }
    setBusy((b) => new Set(b).add(key))
    try {
      await skillsApi.setEnabled(item.path, enabled)
      const note = item.platform === "claude"
        ? enabled ? "已从 Jolu 停用区移回原目录" : "已先备份，再移到 Jolu 停用区；同名副本不受影响"
        : "按这份 SKILL.md 的路径写入 config.toml，修改前已备份"
      toast.success(enabled ? `已启用 ${item.name}` : `已停用 ${item.name}`, note)
    } catch (reason) {
      toast.danger(enabled ? "启用失败" : "停用失败", errorText(reason))
    } finally {
      setBusy((b) => {
        const next = new Set(b)
        next.delete(key)
        return next
      })
      refresh()
    }
  }

  /** 批量启停：逐项反馈，只读或格式异常项跳过不改（画板 84 · 02） */
  const batchToggle = async (enabled: boolean) => {
    const targets = selectedItems.filter((i) => !toggleBlocked(i))
    const skipped = selectedItems.length - targets.length
    const failed: string[] = []
    for (const item of targets) {
      if (!isTauri) continue
      try {
        await skillsApi.setEnabled(item.path, enabled)
      } catch (reason) {
        failed.push(`${item.name}：${errorText(reason)}`)
      }
    }
    if (!isTauri) {
      setData((d) => d && { ...d, items: d.items.map((i) => (targets.some((t) => skillKey(t) === skillKey(i)) ? { ...i, enabled, status: enabled ? "enabled" : "disabled" } : i)) })
    }
    const done = targets.length - failed.length
    setBatch(`结果：${done} 项已${enabled ? "启用" : "停用"}${skipped ? `，${skipped} 项不可切换未修改` : ""}${failed.length ? `，${failed.length} 项失败` : ""}。`)
    if (failed.length) toast.warn("部分项目未能修改", failed.join("；"))
    refresh()
  }

  const openDir = async (item: SkillItemDto) => {
    if (!isTauri) return toast.info("浏览器预览无法打开目录")
    try {
      await skillsApi.openDir(item.path)
    } catch (reason) {
      toast.danger("打开目录失败", errorText(reason))
    }
  }

  const edit = async (item: SkillItemDto) => {
    if (!isTauri) {
      const { demoSkillDetail } = await import("@/mock/skills")
      return setMode({ kind: "editor", target: { mode: "edit", item, detail: demoSkillDetail(item) }, back: null })
    }
    try {
      const detail = await skillsApi.detail(item.path)
      setMode({ kind: "editor", target: { mode: "edit", item: detail.item, detail }, back: null })
    } catch (reason) {
      toast.danger("读取失败", errorText(reason))
    }
  }

  const dialogs = (
    <>
      {dialog?.kind === "import" && <ImportDialog platform={platform} platforms={data?.platforms ?? []} projects={targets} onClose={() => setDialog(null)} onDone={() => { setDialog(null); refresh() }} />}
      {dialog?.kind === "dirs" && data && (
        <DirectoryDialog platform={platform} roots={data.roots} projects={data.projects} onClose={() => setDialog(null)} onSaved={() => { setDialog(null); refresh() }} />
      )}
      {dialog?.kind === "copy" && <CopyDialog items={dialog.items} projects={targets} onClose={() => setDialog(null)} onDone={() => { setDialog(null); refresh() }} />}
      {dialog?.kind === "remove" && (
        <RemoveDialog item={dialog.item} onClose={() => setDialog(null)} onDone={() => { setDialog(null); setMode({ kind: "list" }); refresh() }} />
      )}
    </>
  )

  if (mode.kind === "detail") {
    return (
      <>
        <SkillDetail
          item={mode.item}
          onBack={() => setMode({ kind: "list" })}
          onEdit={(item, detail) => setMode({ kind: "editor", target: { mode: "edit", item, detail }, back: item })}
          onCopy={(item) => setDialog({ kind: "copy", items: [item] })}
          onOpenDir={(item) => void openDir(item)}
        />
        {dialogs}
      </>
    )
  }

  if (mode.kind === "editor") {
    return (
      <SkillEditor
        key={mode.target.mode === "edit" ? mode.target.item.path : "new"}
        target={mode.target}
        items={items}
        platforms={data?.platforms ?? []}
        projects={targets}
        onCancel={() => setMode(mode.back ? { kind: "detail", item: mode.back } : { kind: "list" })}
        onSaved={(path, savedPlatform) => {
          refresh()
          const back = mode.back
          if (back && back.path === path) setMode({ kind: "detail", item: { ...back } })
          else {
            if (platform !== "all" && savedPlatform !== platform) setPlatform(savedPlatform)
            setMode({ kind: "list" })
          }
        }}
      />
    )
  }

  const nothingDetected = !!data && !data.roots.some((r) => r.exists) && items.length === 0

  return (
    <div ref={contentMotion} className="flex w-full flex-col gap-4">
      <div className="flex h-12 items-center justify-between gap-4">
        <div className="flex flex-col gap-[5px]">
          <h2 className="text-[22px] font-semibold text-jolu-text">我的 Skills</h2>
          <p className="text-xs text-jolu-muted">把常用工作方法，交给每个 AI 工具。</p>
        </div>
        <div className="flex gap-2">
          <JoluButton variant="outline" className="h-[34px]" disabled={loading} onClick={rescan}>
            <RefreshCw aria-hidden className={cn(loading && "animate-spin")} />扫描目录
          </JoluButton>
          <JoluButton variant="outline" className="h-[34px]" onClick={() => setDialog({ kind: "import" })}><Download aria-hidden />导入 Skill</JoluButton>
          <JoluButton variant="primary" className="h-[34px]" onClick={() => setMode({ kind: "editor", target: { mode: "new", platform }, back: null })}><Plus aria-hidden />新建 Skill</JoluButton>
        </div>
      </div>

      <div className="flex h-[38px] items-center gap-2">
        <ProviderMenu value={platform} platforms={data?.platforms ?? []} onChange={changePlatform} />
        <MenuFilter trailingChevron label="作用范围筛选" value={filters.scope} options={scopeOptions(items, scanned)} onChange={(v) => setFilters((f) => ({ ...f, scope: v as ScopeFilter }))} />
        <MenuFilter trailingChevron label="状态筛选" value={filters.status} options={STATUS_OPTIONS} onChange={(v) => setFilters((f) => ({ ...f, status: v as StatusFilter }))} />
        <label className="flex h-[38px] min-w-0 flex-1 items-center gap-2 rounded-lg border border-jolu-border bg-jolu-bg px-3 focus-within:border-jolu-accent">
          <Search aria-hidden className="size-4 shrink-0 text-jolu-muted" />
          <input
            value={searchDraft}
            onChange={(e) => {
              setSearchDraft(e.target.value)
              setFilters((f) => ({ ...f, search: e.target.value }))
            }}
            placeholder="搜索名称、描述或路径"
            aria-label="搜索 Skills"
            className="min-w-0 flex-1 bg-transparent text-xs text-jolu-text outline-none placeholder:text-jolu-muted"
          />
          {searchDraft && (
            <button type="button" aria-label="清除搜索" onClick={() => { setSearchDraft(""); setFilters((f) => ({ ...f, search: "" })) }} className="grid size-5 place-items-center rounded text-jolu-muted hover:text-jolu-text">
              <X aria-hidden className="size-3.5" />
            </button>
          )}
        </label>
        <JoluButton variant="outline" className="h-[34px]" disabled={!data} onClick={() => setDialog({ kind: "dirs" })}><FolderCog aria-hidden />目录设置</JoluButton>
      </div>

      <div className="grid grid-cols-3 gap-3">
        <Stat icon={<Blocks />} value={data ? stats.installed : null} label="已安装" note={platform === "all" ? "全部供应商 · 已检测目录" : "当前供应商 · 已检测目录"} />
        <Stat icon={<CircleCheck />} value={data ? stats.enabled : null} label="已启用" note="新会话可用 · 依工具加载规则" />
        <Stat icon={<TriangleAlert />} value={data ? stats.conflicts + stats.invalid : null} label="待处理" note={data ? `${stats.conflicts} 项同名冲突 · ${stats.invalid} 项配置异常` : "扫描完成后显示"} />
      </div>

      <div className="flex h-9 items-center gap-2 rounded-lg bg-jolu-side px-3">
        <FolderOpen aria-hidden className="size-4 shrink-0 text-jolu-muted" />
        <span className="min-w-0 truncate text-[11px] text-jolu-text">{data ? scanLocation(scanned) : "正在读取目录…"}</span>
        {!!data?.errors.length && <span className="shrink-0 text-[11px] text-jolu-danger" title={data.errors.map((e) => `${e.path}：${e.reason}`).join("\n")}>· {data.errors.length} 个目录扫描失败</span>}
        <span className="flex-1" />
        <span className="shrink-0 text-[10px] text-jolu-muted">{data ? scannedLabel(data.scanned_at, now) : ""}{!isTauri && " · 示例数据"}</span>
      </div>

      {error && (
        <div role="alert" className="flex items-center justify-between gap-3 rounded-[10px] bg-jolu-danger-soft px-4 py-2.5 text-xs text-jolu-danger">
          <span className="min-w-0 truncate">{data ? `扫描失败，暂时显示上次结果：${error}` : `扫描失败：${error}`}</span>
          <JoluButton size="sm" variant="danger-soft" onClick={refresh}>重试</JoluButton>
        </div>
      )}

      {selectedItems.length > 0 && (
        <div className="flex flex-wrap items-center gap-2 rounded-xl border border-jolu-border bg-jolu-bg px-4 py-2.5">
          <span className="text-xs font-semibold text-jolu-text">已选择 {selectedItems.length} 个 Skill</span>
          <JoluButton variant="soft" size="sm" onClick={() => void batchToggle(true)}><Check aria-hidden />启用</JoluButton>
          <JoluButton variant="neutral" size="sm" onClick={() => void batchToggle(false)}><Pause aria-hidden />停用</JoluButton>
          <JoluButton variant="neutral" size="sm" onClick={() => setDialog({ kind: "copy", items: selectedItems })}><Copy aria-hidden />复制</JoluButton>
          <JoluButton variant="outline" size="sm" onClick={() => { setSelected(new Set()); setBatch(null) }}>取消选择</JoluButton>
          {batch && <span className="text-xs text-jolu-on">{batch}</span>}
        </div>
      )}

      {!data && loading ? (
        <LoadingSurface layout="table" label="正在扫描 Skills 目录" />
      ) : nothingDetected ? (
        <section className="flex flex-col items-start gap-3 rounded-xl border border-jolu-border bg-jolu-bg p-5">
          <span className="text-[13px] font-semibold text-jolu-text">没有检测到 Skills 目录</span>
          <span className="text-[11px] text-jolu-muted">可以添加项目目录，或新建 / 导入一个 Skill；未检测到目录不代表工具不支持 Skills。</span>
          <div className="flex gap-2">
            <JoluButton variant="soft" size="sm" onClick={() => setDialog({ kind: "dirs" })}><FolderOpen aria-hidden />选择目录</JoluButton>
            <JoluButton variant="neutral" size="sm" onClick={refresh}><RefreshCw aria-hidden />重新扫描</JoluButton>
          </div>
        </section>
      ) : visible.length === 0 && data ? (
        <section className="flex flex-col items-start gap-3 rounded-xl border border-jolu-border bg-jolu-bg p-5">
          <span className="text-[13px] font-semibold text-jolu-text">{filtered ? "没有匹配的 Skill" : "这个供应商还没有 Skill"}</span>
          <span className="text-[11px] text-jolu-muted">{filtered ? "调整范围、状态或搜索条件后再试。" : "新建或导入一个 Skill 开始使用。"}</span>
          {filtered && <JoluButton variant="neutral" size="sm" onClick={clearFilters}><RotateCcw aria-hidden />清空筛选</JoluButton>}
        </section>
      ) : (
        <SkillsTable
          items={visible}
          total={items.length}
          selected={selected}
          busy={busy}
          showPlatform={platform === "all"}
          onSelect={(item) => setSelected((s) => {
            const next = new Set(s)
            const key = skillKey(item)
            if (next.has(key)) next.delete(key)
            else next.add(key)
            return next
          })}
          onSelectAll={(all) => setSelected(all ? new Set(visible.map(skillKey)) : new Set())}
          onToggle={(item, enabled) => void toggle(item, enabled)}
          onOpen={(item) => setMode({ kind: "detail", item })}
          onEdit={(item) => void edit(item)}
          onCopy={(item) => setDialog({ kind: "copy", items: [item] })}
          onRemove={(item) => setDialog({ kind: "remove", item })}
          onOpenDir={(item) => void openDir(item)}
        />
      )}

      <p className="text-[11px] text-jolu-muted">启用仅控制工具是否加载 Skill，不会立即执行；操作能力以所选工具适配结果为准。</p>
      {dialogs}
    </div>
  )
}

/** value 为 null 表示尚未扫描完成，显示「—」而不是 0 */
function Stat({ icon, value, label, note }: { icon: React.ReactNode; value: number | null; label: string; note: string }) {
  return (
    <section className="flex h-[82px] items-center gap-3 rounded-[10px] border border-jolu-border bg-jolu-bg p-3.5">
      <span aria-hidden className="grid size-9 shrink-0 place-items-center rounded-[10px] bg-jolu-selected text-jolu-on [&_svg]:size-[18px]">{icon}</span>
      <span className="flex min-w-0 flex-col gap-1">
        <span className="flex items-baseline gap-2.5">
          <span className="tnum text-2xl font-semibold leading-none text-jolu-text">{value ?? "—"}</span>
          <span className="text-xs text-jolu-muted">{label}</span>
        </span>
        <span className="truncate text-[10px] text-jolu-muted">{note}</span>
      </span>
    </section>
  )
}
