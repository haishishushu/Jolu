import * as Menu from "@radix-ui/react-dropdown-menu"
import { useEffect, useRef, useState } from "react"
import { ChevronDown, ChevronRight, Copy, Download, Folder, FolderOpen, GitBranch, Info, LoaderCircle, Plus, RefreshCw, Square, SquareCheck, Trash2, X } from "lucide-react"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import {
  skillsApi, type SkillCopyResultDto, type SkillImportAction, type SkillImportScanDto, type SkillItemDto, type SkillProjectDto, type SkillRootDto, type SkillScope,
} from "@/lib/skillsApi"
import { SKILL_PLATFORMS, projectLabel, scopeLabel, targetDirHint } from "@/lib/skillsManager"
import { demoImportScan } from "@/mock/skills"
import { JoluButton } from "@/components/ui/JoluButton"
import { Tip } from "@/components/ui/Tip"
import { useToast } from "@/components/ui/Toast"
import { providerName } from "@/components/usage/usageControls"
import { DialogShell, Field, FieldSelect, INPUT_CLASS, Tag } from "./skillParts"

function errorText(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason)
}

function scopeOptions(projects: string[]) {
  return [{ value: "global", label: "全局" }, ...projects.map((p) => ({ value: `project:${p}`, label: `项目 · ${projectLabel(p)}` }))]
}

function parseScope(key: string): { scope: SkillScope; project: string | null } {
  return key.startsWith("project:") ? { scope: "project", project: key.slice("project:".length) } : { scope: "global", project: null }
}

/* ───────────── 导入（画板 82） ───────────── */

export function ImportDialog({ platform, projects, onClose, onDone }: {
  platform: string
  projects: string[]
  onClose: () => void
  onDone: () => void
}) {
  const toast = useToast()
  const [target, setTarget] = useState(platform === "all" ? "claude" : platform)
  const [scopeKey, setScopeKey] = useState("global")
  const [source, setSource] = useState<string | null>(isTauri ? null : "D:/Downloads/jolu-skills/")
  const [scan, setScan] = useState<SkillImportScanDto | null>(null)
  const [scanning, setScanning] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [actions, setActions] = useState<Record<string, SkillImportAction>>({})
  const [enable, setEnable] = useState(false)
  const [importing, setImporting] = useState(false)
  // Git 仓库来源（D-16；画板 82 只画了入口，输入区与状态文案待确认）
  const [mode, setMode] = useState<"local" | "git">("local")
  const [gitUrl, setGitUrl] = useState("")
  const [gitBranch, setGitBranch] = useState("")
  const [fetching, setFetching] = useState(false)
  const checkout = useRef<string | null>(null)
  const closed = useRef(false)
  const { scope, project } = parseScope(scopeKey)

  /** 删除上一次拉取的临时克隆目录（导入完成、取消、重新获取或切回本地目录时） */
  const dropCheckout = () => {
    const dir = checkout.current
    checkout.current = null
    if (dir && isTauri) void skillsApi.importGitCleanup(dir).catch(() => undefined)
  }
  useEffect(() => {
    closed.current = false
    return () => {
      closed.current = true
      dropCheckout()
    }
  }, [])

  useEffect(() => {
    if (!source) return
    let stale = false
    setScanning(true)
    setError(null)
    const load = isTauri ? skillsApi.importScan(source, target, scope, project) : Promise.resolve(demoImportScan(targetDirHint(target, scope, project)))
    load.then((result) => {
      if (stale) return
      setScan(result)
      setPicked(new Set(result.items.filter((i) => i.status === "ok").map((i) => i.name)))
      setActions(Object.fromEntries(result.items.filter((i) => i.status === "conflict").map((i) => [i.name, "skip" as SkillImportAction])))
    }, (reason: unknown) => !stale && (setScan(null), setError(errorText(reason)))).finally(() => !stale && setScanning(false))
    return () => { stale = true }
  }, [source, target, scope, project])

  const pick = async () => {
    if (!isTauri) return
    try {
      const folder = await skillsApi.pickFolder("选择要导入的 Skills 目录")
      if (folder) setSource(folder)
    } catch (reason) {
      setError(errorText(reason))
    }
  }

  const switchMode = (next: "local" | "git") => {
    if (next === mode || fetching) return
    dropCheckout()
    setMode(next)
    setScan(null)
    setError(null)
    setSource(next === "local" && !isTauri ? "D:/Downloads/jolu-skills/" : null)
  }

  const gitUrlOk = gitUrl.trim().startsWith("https://")
  const fetchGit = async () => {
    if (!gitUrlOk || fetching) return
    dropCheckout()
    setSource(null)
    setScan(null)
    setError(null)
    if (!isTauri) return setSource(gitUrl.trim())
    setFetching(true)
    try {
      const result = await skillsApi.importGit(gitUrl.trim(), gitBranch.trim() || null, target, scope, project)
      // 弹窗已关闭：结果用不上了，直接删掉临时目录
      if (closed.current) return void skillsApi.importGitCleanup(result.checkout_dir).catch(() => undefined)
      checkout.current = result.checkout_dir
      setSource(result.scan.source)
    } catch (reason) {
      if (!closed.current) setError(errorText(reason))
    } finally {
      if (!closed.current) setFetching(false)
    }
  }

  const choices = (scan?.items ?? []).flatMap((item): { name: string; action: SkillImportAction }[] => {
    if (item.status === "ok") return picked.has(item.name) ? [{ name: item.name, action: "import" }] : []
    if (item.status === "conflict" && actions[item.name] === "overwrite") return [{ name: item.name, action: "overwrite" }]
    return []
  })
  const counts = {
    ok: scan?.items.filter((i) => i.status === "ok").length ?? 0,
    conflict: scan?.items.filter((i) => i.status === "conflict").length ?? 0,
    invalid: scan?.items.filter((i) => i.status === "invalid").length ?? 0,
  }
  const step = !source ? 1 : importing ? 3 : 2

  const submit = async () => {
    if (!source || !choices.length) return
    if (!isTauri) {
      toast.success(`已导入 ${choices.length} 项（示例）`, "浏览器预览不会复制文件")
      return onDone()
    }
    setImporting(true)
    try {
      const result = await skillsApi.importSkills(source, target, scope, project, choices, enable)
      if (result.failed.length) toast.warn(`已导入 ${result.imported.length} 项，${result.failed.length} 项失败`, result.failed.map((f) => `${f.path}：${f.reason}`).join("；"))
      else toast.success(`已导入 ${result.imported.length} 项`, enable ? undefined : "导入项默认停用，可在列表中启用")
      onDone()
    } catch (reason) {
      setError(errorText(reason))
    } finally {
      setImporting(false)
    }
  }

  return (
    <DialogShell width={680} title="导入 Skills" onClose={onClose} busy={importing}>
      <div className="flex h-7 items-center gap-3 text-xs">
        {["选择来源", "检查内容", "确认导入"].map((label, i) => (
          <span key={label} className="flex items-center gap-3">
            {i > 0 && <ChevronRight aria-hidden className="size-4 text-jolu-muted" />}
            <span className={cn(i + 1 <= step ? "text-jolu-on" : "text-jolu-muted", i + 1 === step && "font-semibold")}>{i + 1}  {label}</span>
          </span>
        ))}
      </div>
      <div role="radiogroup" aria-label="导入来源" className="flex gap-2">
        <JoluButton role="radio" aria-checked={mode === "local"} variant={mode === "local" ? "soft" : "outline"} className="h-[34px]" disabled={fetching} onClick={() => switchMode("local")}><Folder aria-hidden />本地目录</JoluButton>
        <JoluButton role="radio" aria-checked={mode === "git"} variant={mode === "git" ? "soft" : "outline"} className="h-[34px]" disabled={fetching} onClick={() => switchMode("git")}><GitBranch aria-hidden />Git 仓库</JoluButton>
      </div>
      {mode === "local" ? (
        <div className="flex min-h-[58px] items-center gap-2.5 rounded-lg bg-jolu-side p-3">
          <FolderOpen aria-hidden className="size-4 shrink-0 text-jolu-on" />
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <span className="truncate font-mono text-xs text-jolu-text">{source ?? "尚未选择目录"}</span>
            <span className="text-[10px] text-jolu-muted">
              {scanning ? "正在检查…" : scan ? `已扫描 ${scan.items.length} 个 Skill · 包含引用文件与资源` : "选择包含 SKILL.md 的目录，或其上一级目录"}
            </span>
          </div>
          <JoluButton variant="outline" size="sm" className="h-[30px]" onClick={() => void pick()}>{source ? "更换目录" : "选择目录"}</JoluButton>
        </div>
      ) : (
        <div className="flex flex-col gap-2 rounded-lg bg-jolu-side p-3">
          <form className="flex items-end gap-2.5" onSubmit={(e) => { e.preventDefault(); void fetchGit() }}>
            <label className="flex min-w-0 flex-1 flex-col gap-1.5">
              <span className="text-[11px] text-jolu-muted">仓库地址</span>
              <input value={gitUrl} onChange={(e) => setGitUrl(e.target.value)} disabled={fetching} placeholder="https://github.com/owner/repo" spellCheck={false} aria-label="仓库地址" className={cn(INPUT_CLASS, "font-mono")} />
            </label>
            <label className="flex w-[150px] shrink-0 flex-col gap-1.5">
              <span className="text-[11px] text-jolu-muted">分支（可选）</span>
              <input value={gitBranch} onChange={(e) => setGitBranch(e.target.value)} disabled={fetching} placeholder="默认分支" spellCheck={false} aria-label="分支（可选）" className={cn(INPUT_CLASS, "font-mono")} />
            </label>
            <JoluButton type="submit" variant="outline" className="h-[38px] shrink-0" disabled={!gitUrlOk || fetching}>
              {fetching ? <LoaderCircle aria-hidden className="animate-spin" /> : <Download aria-hidden />}{fetching ? "获取中…" : "获取"}
            </JoluButton>
          </form>
          <span className="truncate text-[10px] text-jolu-muted" title={scan?.source}>
            {fetching ? "正在用本机 git 拉取仓库（最长 60 秒）…"
              : scanning ? "正在检查…"
                : scan ? `已扫描 ${scan.items.length} 个 Skill · 包含引用文件与资源`
                  : gitUrl.trim() && !gitUrlOk ? "仓库地址只支持 https:// 开头" : "只接受 https:// 地址；用本机 git 拉取到临时目录，导入后自动删除"}
          </span>
        </div>
      )}
      <div className="flex gap-3.5">
        <Field label="目标供应商">
          <FieldSelect label="目标供应商" value={target} onChange={setTarget} options={SKILL_PLATFORMS.map((id) => ({ value: id, label: providerName(id) }))} />
        </Field>
        <Field label="安装范围">
          <FieldSelect label="安装范围" value={scopeKey} onChange={setScopeKey} options={scopeOptions(projects)} />
        </Field>
      </div>
      <span className="truncate text-[11px] text-jolu-muted">目标：{scan?.target ?? targetDirHint(target, scope, project)}</span>

      {error && <p role="alert" className="rounded-lg bg-jolu-danger-soft px-3 py-2 text-[11px] text-jolu-danger">{error}</p>}

      {scan && (
        <div className="flex flex-col overflow-hidden rounded-[10px] border border-jolu-border">
          <div className="flex h-9 items-center justify-between bg-jolu-side px-3">
            <span className="text-xs font-semibold text-jolu-text">待导入项目</span>
            <span className="text-[10px] text-jolu-muted">{counts.ok} 可导入 · {counts.conflict} 冲突 · {counts.invalid} 异常</span>
          </div>
          {!scan.items.length && <p className="px-3 py-4 text-xs text-jolu-muted">这个目录里没有找到带 SKILL.md 的 Skill</p>}
          <div className="max-h-[240px] overflow-y-auto">
            {scan.items.map((item) => {
              const checked = picked.has(item.name)
              return (
                <div key={item.name} className="flex h-[60px] items-center gap-2.5 border-t border-jolu-border px-3">
                  {item.status === "ok" ? (
                    <button type="button" role="checkbox" aria-checked={checked} aria-label={`导入 ${item.name}`} onClick={() => setPicked((set) => {
                      const next = new Set(set)
                      if (next.has(item.name)) next.delete(item.name)
                      else next.add(item.name)
                      return next
                    })} className="grid size-5 place-items-center rounded outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent">
                      {checked ? <SquareCheck aria-hidden className="size-4 text-jolu-on" /> : <Square aria-hidden className="size-4 text-jolu-muted" />}
                    </button>
                  ) : (
                    <Square aria-hidden className="size-4 shrink-0 text-jolu-muted opacity-50" />
                  )}
                  <div className="flex min-w-0 flex-1 flex-col gap-1">
                    <span className="truncate text-xs font-medium text-jolu-text">{item.name}</span>
                    <span className="truncate text-[10px] text-jolu-muted">{item.issue ?? "目录与元信息有效"}</span>
                  </div>
                  {item.status === "ok" && <Tag tone="on">可导入</Tag>}
                  {item.status === "invalid" && <Tag tone="danger">不可导入</Tag>}
                  {item.status === "conflict" && (
                    <Menu.Root modal={false}>
                      <Menu.Trigger className="inline-flex h-7 items-center gap-1 rounded-lg border border-jolu-border bg-jolu-bg px-3 text-xs font-medium text-jolu-text outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent">
                        {actions[item.name] === "overwrite" ? "覆盖" : "跳过"}<ChevronDown aria-hidden className="size-3.5" />
                      </Menu.Trigger>
                      <Menu.Portal>
                        <Menu.Content align="end" sideOffset={6} className="motion-popover z-[130] w-[220px] rounded-xl border border-jolu-border bg-jolu-bg p-1.5 shadow-popover">
                          <Menu.Item className="flex h-8 cursor-default items-center rounded-md px-2 text-xs text-jolu-text outline-none data-[highlighted]:bg-jolu-side" onSelect={() => setActions((a) => ({ ...a, [item.name]: "skip" }))}>跳过</Menu.Item>
                          <Menu.Item className="flex min-h-8 cursor-default flex-col items-start justify-center rounded-md px-2 py-1 text-xs text-jolu-text outline-none data-[highlighted]:bg-jolu-side" onSelect={() => setActions((a) => ({ ...a, [item.name]: "overwrite" }))}>
                            覆盖
                            <span className="text-[10px] text-jolu-muted">先备份目标目录，再替换</span>
                          </Menu.Item>
                        </Menu.Content>
                      </Menu.Portal>
                    </Menu.Root>
                  )}
                </div>
              )
            })}
          </div>
        </div>
      )}

      <button type="button" role="checkbox" aria-checked={enable} onClick={() => setEnable(!enable)} className="flex h-7 items-center gap-2 self-start rounded-md text-xs text-jolu-text outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent">
        {enable ? <SquareCheck aria-hidden className="size-4 text-jolu-on" /> : <Square aria-hidden className="size-4 text-jolu-muted" />}
        导入后启用
        <span className="text-[10px] text-jolu-muted">默认关闭；关闭时 Claude Code 放入 Jolu 停用区，Codex 按路径停用</span>
      </button>
      <div className="flex gap-2 rounded-lg bg-jolu-side p-3 text-[11px] text-jolu-muted">
        <Info aria-hidden className="size-4 shrink-0" />
        <span>导入仅复制文件，不跟随符号链接；冲突项默认跳过，选择覆盖时先备份目标目录；格式异常项需先修复。</span>
      </div>
      <span className="flex-1" />
      <div className="flex justify-end gap-2">
        <JoluButton variant="outline" className="h-[34px]" disabled={importing} onClick={onClose}>取消</JoluButton>
        <JoluButton variant="primary" className="h-[34px]" disabled={!choices.length || importing || scanning} onClick={() => void submit()}>
          <Download aria-hidden />{importing ? "导入中…" : `导入 ${choices.length} 项`}
        </JoluButton>
      </div>
    </DialogShell>
  )
}

/* ───────────── 复制到其他供应商（画板 84 · 03） ───────────── */

export function CopyDialog({ items, projects, onClose, onDone }: {
  items: SkillItemDto[]
  projects: string[]
  onClose: () => void
  onDone: () => void
}) {
  const toast = useToast()
  const first = items[0]
  const [target, setTarget] = useState(first.platform === "claude" ? "codex" : "claude")
  const [scopeKey, setScopeKey] = useState("global")
  const [checks, setChecks] = useState<Record<string, SkillCopyResultDto | string> | null>(null)
  const [copying, setCopying] = useState(false)
  const { scope, project } = parseScope(scopeKey)

  useEffect(() => {
    let stale = false
    setChecks(null)
    if (!isTauri) {
      setChecks(Object.fromEntries(items.map((i) => [i.path, { target: `${targetDirHint(target, scope, project)}${i.dir_name}`, copied: false, conflict: false, issue: null }])))
      return
    }
    Promise.all(items.map((i) => skillsApi.copy(i.path, target, scope, project, true).then((r) => [i.path, r] as const, (reason: unknown) => [i.path, errorText(reason)] as const)))
      .then((list) => !stale && setChecks(Object.fromEntries(list)))
    return () => { stale = true }
  }, [items, target, scope, project])

  const ready = checks ? items.filter((i) => {
    const c = checks[i.path]
    return typeof c === "object" && !c.conflict && !c.issue
  }) : []

  const submit = async () => {
    if (!isTauri) {
      toast.success(`已复制 ${ready.length} 项（示例）`)
      return onDone()
    }
    setCopying(true)
    const failed: string[] = []
    for (const item of ready) {
      try {
        await skillsApi.copy(item.path, target, scope, project, false)
      } catch (reason) {
        failed.push(`${item.name}：${errorText(reason)}`)
      }
    }
    setCopying(false)
    if (failed.length) toast.warn(`已复制 ${ready.length - failed.length} 项，${failed.length} 项失败`, failed.join("；"))
    else toast.success(`已复制 ${ready.length} 项到 ${providerName(target)}`, "源文件保留，创建了独立副本")
    onDone()
  }

  const single = items.length === 1
  return (
    <DialogShell width={520} title="复制到其他供应商" onClose={onClose} busy={copying}>
      <p className="truncate text-xs text-jolu-text">
        {single ? first.name : `已选择 ${items.length} 个 Skill`}  ·  {providerName(first.platform)} → {providerName(target)}
      </p>
      <div className="flex gap-3">
        <Field label="目标供应商">
          <FieldSelect label="目标供应商" value={target} onChange={setTarget} options={SKILL_PLATFORMS.map((id) => ({ value: id, label: providerName(id) }))} />
        </Field>
        <Field label="安装范围">
          <FieldSelect label="安装范围" value={scopeKey} onChange={setScopeKey} options={scopeOptions(projects)} />
        </Field>
      </div>
      <p className="text-[11px] text-jolu-muted">确认目标支持当前 Skill 格式后才允许复制。默认保留源文件，创建独立副本。</p>
      <div className="flex max-h-[220px] flex-col gap-1.5 overflow-y-auto">
        {!checks && <LoaderCircle aria-hidden className="size-4 animate-spin text-jolu-muted" />}
        {checks && items.map((item) => {
          const c = checks[item.path]
          const text = typeof c === "string" ? c : c.conflict ? "目标已有同名 Skill，不会覆盖" : c.issue ?? "兼容性校验通过"
          const good = typeof c === "object" && !c.conflict && !c.issue
          return (
            <div key={item.path} className="flex items-center gap-2 text-[11px]">
              {!single && <span className="w-32 shrink-0 truncate text-jolu-text">{item.name}</span>}
              <Tag tone={good ? "on" : "danger"}>{text}</Tag>
              {typeof c === "object" && <span className="min-w-0 truncate font-mono text-[10px] text-jolu-muted" title={c.target}>{c.target}</span>}
            </div>
          )
        })}
      </div>
      <div className="flex justify-end gap-2">
        <JoluButton variant="outline" className="h-[34px]" disabled={copying} onClick={onClose}>取消</JoluButton>
        <JoluButton variant="primary" className="h-[34px]" disabled={!ready.length || copying} onClick={() => void submit()}>
          <Copy aria-hidden />{copying ? "复制中…" : single ? "复制" : `复制 ${ready.length} 项`}
        </JoluButton>
      </div>
    </DialogShell>
  )
}

/* ───────────── 移除确认（画板 84 · 04） ───────────── */

export function RemoveDialog({ item, onClose, onDone }: { item: SkillItemDto; onClose: () => void; onDone: () => void }) {
  const toast = useToast()
  const [removing, setRemoving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const submit = async () => {
    if (!isTauri) {
      toast.success("已移除（示例）")
      return onDone()
    }
    setRemoving(true)
    try {
      const backup = await skillsApi.remove(item.path)
      toast.success(`已移除 ${item.name}`, `备份保存在 ${backup}`)
      onDone()
    } catch (reason) {
      setError(errorText(reason))
    } finally {
      setRemoving(false)
    }
  }
  return (
    <DialogShell width={480} title="移除此安装项？" onClose={onClose} busy={removing}>
      <p className="text-xs text-jolu-text">{item.name}  ·  {providerName(item.platform)}{item.scope === "project" ? ` / ${scopeLabel(item)}` : ""}</p>
      <p className="break-all font-mono text-[11px] text-jolu-muted">{item.path}</p>
      <p className="text-[11px] text-jolu-muted">只影响此目录中的安装项，其他供应商副本保留。运行中的任务不立即中断；后续加载将不再使用。</p>
      <Tag tone="on">移除前自动备份到应用数据目录</Tag>
      {error && <p role="alert" className="text-[11px] text-jolu-danger">{error}</p>}
      <div className="flex justify-end gap-2">
        <JoluButton variant="outline" className="h-[34px]" disabled={removing} onClick={onClose}>取消</JoluButton>
        <JoluButton variant="danger-soft" className="h-[34px]" disabled={removing} onClick={() => void submit()}>
          <Trash2 aria-hidden />{removing ? "移除中…" : "移除 Skill"}
        </JoluButton>
      </div>
    </DialogShell>
  )
}

/* ───────────── 扫描目录设置（画板 84 · 01） ───────────── */
/* ───────────── 扫描目录设置（画板 84 · 01） ───────────── */

/**
 * 全局目录由工具决定（只显示）；项目目录 = 会话记录自动带出 + 手动添加。
 * 自动带出的可以忽略 / 恢复（只记在 skills.json，不删会话数据），手动添加的可以移除。
 */
export function DirectoryDialog({ platform, roots, projects, onClose, onSaved }: {
  platform: string
  roots: SkillRootDto[]
  projects: SkillProjectDto[]
  onClose: () => void
  onSaved: () => void
}) {
  const [manual, setManual] = useState(() => projects.filter((p) => p.source === "manual").map((p) => p.path))
  const [ignored, setIgnored] = useState(() => new Set(projects.filter((p) => p.ignored).map((p) => p.path)))
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const globals = roots.filter((r) => r.scope === "global")
  const sessions = projects.filter((p) => p.source === "session")
  const known = (path: string) => projects.find((p) => p.path === path)

  const add = async () => {
    if (!isTauri) return
    try {
      const folder = await skillsApi.pickFolder("添加项目目录")
      if (folder && !manual.includes(folder)) setManual([...manual, folder])
    } catch (reason) {
      setError(errorText(reason))
    }
  }
  const save = async () => {
    if (!isTauri) return onSaved()
    setSaving(true)
    try {
      await skillsApi.setProjects(manual)
      for (const p of sessions) {
        if (p.ignored !== ignored.has(p.path)) await skillsApi.ignoreProject(p.path, ignored.has(p.path))
      }
      onSaved()
    } catch (reason) {
      setError(errorText(reason))
    } finally {
      setSaving(false)
    }
  }

  const skillDirs = (path: string) => {
    const p = known(path)
    if (!p) return "保存后扫描"
    const found = [p.claude && "Claude Code", p.codex && "Codex"].filter(Boolean)
    return found.length ? `含 ${found.join(" / ")} 的项目 Skills 目录` : "没有 Skills 目录 · 可作为新建或复制的目标"
  }

  const row = (key: string, label: string, path: string, note: string | null, action: React.ReactNode, dim = false) => (
    <div key={key} className={cn("flex items-center gap-2.5 rounded-lg bg-jolu-side p-3", dim && "opacity-55")}>
      <FolderOpen aria-hidden className="size-4 shrink-0 text-jolu-muted" />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="text-[10px] text-jolu-muted">{label}</span>
        <span className="truncate font-mono text-[11px] text-jolu-text" title={path}>{path}</span>
        {note && <span className="text-[10px] text-jolu-muted">{note}</span>}
      </div>
      {action}
    </div>
  )

  return (
    <DialogShell width={600} title="扫描目录设置" onClose={onClose} busy={saving}>
      <p className="text-[11px] text-jolu-muted">供应商：{platform === "all" ? "全部" : providerName(platform)} · 全局目录由工具决定；项目目录来自会话记录和手动添加</p>
      <div className="flex max-h-[400px] flex-col gap-2 overflow-y-auto">
        {globals.map((r) => row(
          `g-${r.path}`,
          `${providerName(r.platform)} · 全局目录`,
          r.path,
          r.exists ? null : "未检测到这个目录",
          <Tip label="更换全局目录待接入"><span tabIndex={0}><JoluButton variant="outline" size="sm" disabled>更换</JoluButton></span></Tip>,
        ))}
        {manual.map((p) => row(`m-${p}`, "项目目录 · 手动添加", p, skillDirs(p),
          <JoluButton variant="outline" size="sm" onClick={() => setManual(manual.filter((x) => x !== p))}><X aria-hidden />移除</JoluButton>,
        ))}
        {sessions.map((p) => {
          const off = ignored.has(p.path)
          return row(`s-${p.path}`, off ? "项目目录 · 来自会话记录 · 已忽略" : "项目目录 · 来自会话记录", p.path, skillDirs(p.path),
            <JoluButton variant="outline" size="sm" onClick={() => setIgnored((set) => {
              const next = new Set(set)
              if (off) next.delete(p.path)
              else next.add(p.path)
              return next
            })}>{off ? "恢复" : "忽略"}</JoluButton>,
            off,
          )
        })}
        {!manual.length && !sessions.length && <p className="px-1 text-[11px] text-jolu-muted">会话记录里还没有项目目录；可以手动添加。</p>}
      </div>
      {error && <p role="alert" className="text-[11px] text-jolu-danger">{error}</p>}
      <div className="flex gap-2">
        <JoluButton variant="neutral" size="sm" disabled={!isTauri || saving} onClick={() => void add()}><Plus aria-hidden />添加目录</JoluButton>
        <JoluButton variant="soft" size="sm" disabled={saving} onClick={() => void save()}><RefreshCw aria-hidden />保存并扫描</JoluButton>
      </div>
      <p className="text-[10px] text-jolu-muted">只扫描含有 Skills 目录的项目；已不存在的目录自动跳过。忽略或移除项目不会删除任何文件或会话数据。</p>
    </DialogShell>
  )
}
