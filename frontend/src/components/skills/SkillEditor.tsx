import * as Menu from "@radix-ui/react-dropdown-menu"
import { useMemo, useState } from "react"
import { Check, ChevronDown, CircleAlert, CircleCheck, FileText, GitCompare, LoaderCircle, Square, SquareCheck } from "lucide-react"
import { cn } from "@/lib/utils"
import { lineDiff } from "@/lib/textDiff"
import { isTauri } from "@/lib/api"
import { SKILL_EXTERNAL_CHANGE, skillsApi, type SkillDetailDto, type SkillItemDto, type SkillPlatformDto, type SkillScope } from "@/lib/skillsApi"
import {
  SKILL_PLATFORMS, frontField, hasNameConflict, projectLabel, setFrontField, skillTemplate, targetDirHint, validSkillName,
} from "@/lib/skillsManager"
import { JoluButton } from "@/components/ui/JoluButton"
import { useToast } from "@/components/ui/Toast"
import { providerName } from "@/components/usage/usageControls"
import { DialogShell, Field, FieldSelect, INPUT_CLASS, MarkdownPreview, Tag } from "./skillParts"

/** 差异弹窗读取到的磁盘版本 */
type DiskVersion = { text: string; modified: number | null; truncated: boolean }

/**
 * 查看差异（D-17；画板 84 · 05 只画了入口，弹窗样式待确认）：按行对比「磁盘版本」与「当前草稿」。
 * 只在磁盘版本里的行标删除（jolu-danger 浅底），只在草稿里的行标新增（jolu-on 浅底）。
 */
function DiffDialog({ disk, draft, onUseDisk, onKeepDraft, onClose }: {
  disk: DiskVersion
  draft: string
  onUseDisk: () => void
  onKeepDraft: () => void
  onClose: () => void
}) {
  const lines = useMemo(() => lineDiff(disk.text, draft), [disk.text, draft])
  const removed = lines.filter((l) => l.kind === "del").length
  const added = lines.filter((l) => l.kind === "add").length
  return (
    <DialogShell width={760} title="查看差异" onClose={onClose}>
      <div className="flex flex-wrap items-center gap-2 text-[11px] text-jolu-muted">
        <span className="rounded bg-jolu-danger-soft px-1.5 py-0.5 font-mono text-jolu-danger">- 磁盘版本</span>
        <span className="rounded bg-jolu-selected px-1.5 py-0.5 font-mono text-jolu-on">+ 当前草稿</span>
        <span className="flex-1" />
        <span>{removed || added ? `删除 ${removed} 行 · 新增 ${added} 行` : "两个版本内容相同"}</span>
      </div>
      {disk.truncated && <p role="alert" className="rounded-lg bg-jolu-danger-soft px-3 py-2 text-[11px] text-jolu-danger">磁盘上的文件超过 512 KB，只对比了前 512 KB；为避免丢失内容，不能直接使用磁盘版本。</p>}
      <div className="max-h-[420px] min-h-[160px] overflow-auto rounded-lg border border-jolu-border bg-jolu-bg py-1 font-mono text-[11px] leading-5">
        {lines.map((line, i) => (
          <div key={i} className={cn("flex min-w-max whitespace-pre px-3", line.kind === "del" && "bg-jolu-danger-soft text-jolu-danger", line.kind === "add" && "bg-jolu-selected text-jolu-on", line.kind === "same" && "text-jolu-text")}>
            <span aria-hidden className="w-4 shrink-0 select-none">{line.kind === "del" ? "-" : line.kind === "add" ? "+" : " "}</span>
            <span className="sr-only">{line.kind === "del" ? "删除：" : line.kind === "add" ? "新增：" : ""}</span>
            {line.text || " "}
          </div>
        ))}
      </div>
      <div className="flex justify-end gap-2">
        <JoluButton variant="outline" className="h-[34px]" disabled={disk.truncated} onClick={onUseDisk}>使用磁盘版本</JoluButton>
        <JoluButton variant="primary" className="h-[34px]" onClick={onKeepDraft}>保留草稿</JoluButton>
      </div>
    </DialogShell>
  )
}

export type EditorTarget =
  | { mode: "new"; platform: string }
  | { mode: "edit"; item: SkillItemDto; detail: SkillDetailDto }

/** 新建与编辑 Skill（画板 80）：基本信息、SKILL.md 编辑器与实时预览、保存前校验 */
export function SkillEditor({ target, items, platforms, projects, onCancel, onSaved }: {
  target: EditorTarget
  items: SkillItemDto[]
  /** 各工具的全局目标目录（目录设置里可能已更换） */
  platforms: SkillPlatformDto[]
  projects: string[]
  onCancel: () => void
  onSaved: (path: string, platform: string) => void
}) {
  const toast = useToast()
  const editing = target.mode === "edit"
  const initialContent = editing ? target.detail.content : skillTemplate("", "")
  const [platform, setPlatform] = useState(editing ? target.item.platform : target.platform === "all" ? "claude" : target.platform)
  const [scopeKey, setScopeKey] = useState(editing ? (target.item.scope === "project" ? `project:${target.item.project}` : target.item.scope) : "global")
  const [name, setName] = useState(editing ? target.item.dir_name : "")
  const [content, setContent] = useState(initialContent)
  const [enable, setEnable] = useState(true)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [external, setExternal] = useState(false)
  const [expected, setExpected] = useState<number | null>(editing ? target.item.modified_ms : null)
  const [leaving, setLeaving] = useState(false)
  const [diff, setDiff] = useState<DiskVersion | null>(null)
  const [loadingDiff, setLoadingDiff] = useState(false)

  const scope: SkillScope = scopeKey.startsWith("project:") ? "project" : (scopeKey as SkillScope)
  const project = scope === "project" ? scopeKey.slice("project:".length) : null
  const description = frontField(content, "description") ?? ""
  const frontName = frontField(content, "name")
  const dirty = content !== initialContent || (!editing && name !== "")

  const problems = useMemo(() => {
    const list: string[] = []
    if (!validSkillName(name)) list.push("名称只能使用小写字母、数字和连字符，最长 64 位")
    if (!description.trim()) list.push("请填写描述")
    if (frontName && frontName !== name) list.push("SKILL.md 里的 name 与名称不一致")
    if (scope === "project" && !project) list.push("请选择项目")
    return list
  }, [name, description, frontName, scope, project])
  const conflict = !editing && validSkillName(name) && hasNameConflict(items, platform, scope, project, name)

  const scopeOptions = [{ value: "global", label: "全局" }, ...projects.map((p) => ({ value: `project:${p}`, label: `项目 · ${projectLabel(p)}` }))]

  const updateName = (value: string) => {
    setName(value)
    if (validSkillName(value)) setContent((text) => setFrontField(text, "name", value))
  }

  const save = async () => {
    if (saving || problems.length || conflict) return
    setSaving(true)
    setError(null)
    try {
      if (!isTauri) {
        toast.success(editing ? "已保存（示例）" : "已创建（示例）", "浏览器预览不会写入文件")
        return onSaved(editing ? target.item.path : `${targetDirHint(platform, scope, project, platforms)}${name}`, platform)
      }
      const result = await skillsApi.save({
        platform,
        scope,
        project,
        name,
        content,
        original_path: editing ? target.item.path : null,
        expected_modified_ms: editing ? expected : null,
        enable,
      })
      toast.success(editing ? "Skill 已保存" : "Skill 已创建", result.backup ? "原文件已备份到应用数据目录" : undefined)
      onSaved(result.path, platform)
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : String(reason)
      if (message === SKILL_EXTERNAL_CHANGE) setExternal(true)
      else setError(message)
    } finally {
      setSaving(false)
    }
  }

  /** 保留草稿：以磁盘上的最新版本为基准，下一次保存覆盖外部修改（覆盖前后端会先备份原文件）。
   *  已在差异弹窗里读过磁盘版本时直接用它的修改时间 */
  const keepDraft = async (latestModified?: number | null) => {
    if (latestModified !== undefined) setExpected(latestModified)
    else if (editing && isTauri) {
      try {
        const latest = await skillsApi.detail(target.item.path)
        setExpected(latest.item.modified_ms)
      } catch (reason) {
        return setError(String(reason))
      }
    }
    setExternal(false)
    setDiff(null)
    toast.info("已保留草稿", "再次保存将覆盖外部修改，原文件会先备份")
  }

  /** 查看差异（D-17）：读取磁盘上的最新 SKILL.md，与当前草稿按行对比 */
  const openDiff = async () => {
    if (!editing) return
    if (!isTauri) return setDiff({ text: initialContent, modified: expected, truncated: false })
    setLoadingDiff(true)
    try {
      const latest = await skillsApi.detail(target.item.path)
      setDiff({ text: latest.content, modified: latest.item.modified_ms, truncated: latest.truncated })
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setLoadingDiff(false)
    }
  }

  /** 使用磁盘版本：草稿替换为磁盘内容，并以它的修改时间作为下次保存的基准 */
  const applyDisk = (disk: DiskVersion) => {
    setContent(disk.text)
    setExpected(disk.modified)
    setExternal(false)
    setDiff(null)
    toast.info("已换成磁盘版本", "草稿中的修改已丢弃")
  }

  const cancel = () => (dirty ? setLeaving(true) : onCancel())
  const ok = !problems.length && !conflict

  return (
    <div className="flex w-full flex-col gap-4">
      <div className="flex h-12 items-center justify-between">
        <div className="flex flex-col gap-1">
          <h2 className="text-[22px] font-semibold text-jolu-text">{editing ? "编辑 Skill" : "新建 Skill"}</h2>
          <p className="text-xs text-jolu-muted">{editing ? "修改 SKILL.md 并预览效果；保存前会先备份原文件。" : "从模板开始，编辑内容并预览效果。"}</p>
        </div>
        <div className="flex gap-2">
          <JoluButton variant="outline" className="h-[34px]" disabled={saving} onClick={cancel}>取消</JoluButton>
          <JoluButton variant="primary" className="h-[34px]" disabled={!ok || saving || external} onClick={() => void save()}>
            <Check aria-hidden />{saving ? "保存中…" : "保存 Skill"}
          </JoluButton>
        </div>
      </div>

      <section className="flex flex-col gap-4 rounded-xl border border-jolu-border bg-jolu-bg p-5">
        <div className="flex gap-4">
          <Field label="名称 *">
            <input value={name} disabled={editing || saving} onChange={(e) => updateName(e.target.value.trim())} placeholder="例如 api-review" aria-label="名称" className={INPUT_CLASS} />
          </Field>
          <Field label="描述 *">
            <input value={description} disabled={saving} onChange={(e) => setContent((text) => setFrontField(text, "description", e.target.value))} placeholder="说明何时使用这个 Skill" aria-label="描述" className={INPUT_CLASS} />
          </Field>
        </div>
        <div className="flex gap-4">
          <Field label="供应商 *">
            <FieldSelect label="供应商" value={platform} disabled={editing || saving} onChange={setPlatform} options={SKILL_PLATFORMS.map((id) => ({ value: id, label: providerName(id) }))} />
          </Field>
          <Field label="作用范围 *">
            <FieldSelect label="作用范围" value={scopeKey} disabled={editing || saving} onChange={setScopeKey} options={editing && scope !== "global" && scope !== "project" ? [{ value: scopeKey, label: scopeKey }] : scopeOptions} />
          </Field>
        </div>
        <p className="truncate text-[11px] text-jolu-muted">
          {editing ? `文件位置：${target.item.path}` : `保存到 ${targetDirHint(platform, scope, project, platforms)}${name || "<名称>"}/`}
          {!editing && projects.length === 0 && " · 需要项目范围时，先在「目录设置」添加项目目录"}
        </p>
      </section>

      <div className="flex h-[490px] gap-4">
        <section className="flex min-w-0 flex-1 flex-col gap-3.5 rounded-xl border border-jolu-border bg-jolu-side p-5">
          <div className="flex h-7 items-center justify-between">
            <span className="font-mono text-[13px] font-semibold text-jolu-text">SKILL.md</span>
            <Menu.Root modal={false}>
              <Menu.Trigger disabled={saving} className="inline-flex h-7 items-center gap-1.5 rounded-lg border border-jolu-border bg-jolu-bg px-3 text-xs font-medium text-jolu-text outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent">
                插入模板<ChevronDown aria-hidden className="size-4" />
              </Menu.Trigger>
              <Menu.Portal>
                <Menu.Content align="end" sideOffset={6} className="motion-popover z-[100] w-[220px] rounded-xl border border-jolu-border bg-jolu-bg p-1.5 shadow-popover">
                  <Menu.Item
                    disabled={saving}
                    className="flex h-8 cursor-default select-none items-center gap-2 rounded-md px-2 text-xs text-jolu-text outline-none data-[highlighted]:bg-jolu-side"
                    onSelect={() => setContent(skillTemplate(name, description))}
                  >
                    <FileText aria-hidden className="size-4 text-jolu-muted" />基础模板（替换当前内容）
                  </Menu.Item>
                  <span className="block px-2 py-1 text-[10px] text-jolu-muted">更多模板待确认</span>
                </Menu.Content>
              </Menu.Portal>
            </Menu.Root>
          </div>
          <textarea
            disabled={saving}
            value={content}
            onChange={(e) => setContent(e.target.value)}
            spellCheck={false}
            aria-label="SKILL.md 内容"
            className="min-h-0 flex-1 resize-none bg-transparent font-mono text-[11px] leading-5 text-jolu-text outline-none"
          />
        </section>
        <section className="flex min-w-0 flex-1 flex-col gap-5 overflow-y-auto rounded-xl border border-jolu-border bg-jolu-bg p-6">
          <span className="text-xs font-semibold text-jolu-on">实时预览</span>
          <MarkdownPreview source={content} />
        </section>
      </div>

      {external ? (
        <section role="alert" className="flex flex-col gap-2.5 rounded-xl border border-jolu-border bg-jolu-bg p-4">
          <Tag tone="danger">检测到外部修改</Tag>
          <p className="text-xs text-jolu-muted">文件已被其他程序修改。当前草稿已保留，查看差异后再决定如何合并。</p>
          <div className="flex gap-2">
            <JoluButton variant="soft" size="sm" disabled={loadingDiff} onClick={() => void openDiff()}>
              {loadingDiff ? <LoaderCircle aria-hidden className="animate-spin" /> : <GitCompare aria-hidden />}查看差异
            </JoluButton>
            <JoluButton variant="neutral" size="sm" onClick={() => void keepDraft()}><FileText aria-hidden />保留草稿</JoluButton>
          </div>
        </section>
      ) : (
        <div className={cn("flex min-h-11 items-center gap-2.5 rounded-lg px-3.5 text-xs", ok ? "bg-jolu-selected text-jolu-on" : "bg-jolu-danger-soft text-jolu-danger")}>
          {ok ? <CircleCheck aria-hidden className="size-4 shrink-0" /> : <CircleAlert aria-hidden className="size-4 shrink-0" />}
          <span className="min-w-0 flex-1 truncate">
            {error ?? (ok ? `名称、描述与目标目录校验通过 · ${editing ? "尚未保存" : "尚未写入"}` : conflict ? "目标已有同名 Skill，请换一个名称或选择其他范围" : problems[0])}
          </span>
          {!editing && validSkillName(name) && <Tag tone={conflict ? "danger" : "on"}>{conflict ? "同名冲突" : "无同名冲突"}</Tag>}
        </div>
      )}

      {!editing && (
        <button type="button" role="checkbox" disabled={saving} aria-checked={enable} onClick={() => setEnable(!enable)} className="flex h-[34px] items-center gap-2 self-start rounded-md text-xs text-jolu-text outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent">
          {enable ? <SquareCheck aria-hidden className="size-4 text-jolu-on" /> : <Square aria-hidden className="size-4 text-jolu-muted" />}
          保存后启用
          <span className="text-[11px] text-jolu-muted">（{platform === "claude" ? "不勾选时放入 Jolu 停用区，工具不会加载，同名副本不受影响" : "不勾选时按这份 SKILL.md 的路径写入 config.toml"}）</span>
        </button>
      )}

      {diff && <DiffDialog disk={diff} draft={content} onUseDisk={() => applyDisk(diff)} onKeepDraft={() => void keepDraft(diff.modified)} onClose={() => setDiff(null)} />}

      {leaving && (
        <DialogShell width={420} title="离开编辑器？" onClose={() => setLeaving(false)}>
          <p className="text-xs text-jolu-muted">有尚未保存的修改。</p>
          <div className="flex justify-end gap-2">
            <JoluButton variant="outline" onClick={() => setLeaving(false)}>继续编辑</JoluButton>
            <JoluButton variant="danger-soft" onClick={onCancel}>放弃修改</JoluButton>
            <JoluButton variant="primary" disabled={!ok || saving} onClick={() => { setLeaving(false); void save() }}>保存</JoluButton>
          </div>
        </DialogShell>
      )}
    </div>
  )
}
