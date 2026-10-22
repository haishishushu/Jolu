import { useEffect, useState } from "react"
import { ArrowLeft, Copy, File, FileText, Folder, FolderOpen, Image, Info, LoaderCircle, Pencil } from "lucide-react"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import { skillsApi, type SkillDetailDto, type SkillFileContentDto, type SkillItemDto } from "@/lib/skillsApi"
import { formatBytes, modifiedLabel, scopeLabel, sourceLabel, statusLabel } from "@/lib/skillsManager"
import { demoSkillDetail } from "@/mock/skills"
import { JoluButton } from "@/components/ui/JoluButton"
import { Tip } from "@/components/ui/Tip"
import { useToast } from "@/components/ui/Toast"
import { providerName } from "@/components/usage/usageControls"
import { MarkdownPreview, SkillIcon, Tag } from "./skillParts"

const SCOPE_DETAIL: Record<SkillItemDto["scope"], string> = {
  global: "全局 · 当前系统用户",
  project: "项目",
  system: "系统内置 · 工具托管",
  synced: "claude.ai 同步 · 只读",
}

function fileIcon(rel: string, dir: boolean) {
  if (dir) return Folder
  if (/\.(png|jpe?g|gif|webp|svg)$/i.test(rel)) return Image
  if (/\.(md|txt)$/i.test(rel)) return FileText
  return File
}

/** Skill 详情与文件（画板 78）：文件列表、来源信息与 SKILL.md 预览 / 源码 */
export function SkillDetail({ item, onBack, onEdit, onCopy, onOpenDir }: {
  item: SkillItemDto
  onBack: () => void
  onEdit: (item: SkillItemDto, detail: SkillDetailDto) => void
  onCopy: (item: SkillItemDto) => void
  onOpenDir: (item: SkillItemDto) => void
}) {
  const toast = useToast()
  const [detail, setDetail] = useState<SkillDetailDto | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [file, setFile] = useState("SKILL.md")
  const [fileContent, setFileContent] = useState<SkillFileContentDto | null>(null)
  const [tab, setTab] = useState<"preview" | "source">("preview")

  useEffect(() => {
    setDetail(null)
    setError(null)
    setFile("SKILL.md")
    if (!isTauri) return setDetail(demoSkillDetail(item))
    skillsApi.detail(item.path).then(setDetail, (reason: unknown) => setError(String(reason)))
  }, [item])

  useEffect(() => {
    setFileContent(null)
    if (file === "SKILL.md" || !detail) return
    if (!isTauri) return setFileContent({ text: file.endsWith(".png") ? null : `# ${file}\n\n（示例文件内容）`, binary: file.endsWith(".png"), size: 0, truncated: false })
    skillsApi.file(item.path, file).then(setFileContent, (reason: unknown) => setFileContent({ text: `读取失败：${String(reason)}`, binary: false, size: 0, truncated: false }))
  }, [file, detail, item.path])

  const current = detail?.item ?? item
  const isSkillFile = file === "SKILL.md"
  const text = isSkillFile ? detail?.content ?? "" : fileContent?.text ?? null
  const markdown = isSkillFile || /\.md$/i.test(file)

  const copyPath = async () => {
    try {
      await navigator.clipboard.writeText(current.path)
      toast.success("已复制路径")
    } catch {
      toast.danger("复制失败", "请检查剪贴板权限")
    }
  }

  return (
    <div className="flex w-full flex-col gap-4">
      <div className="flex h-[34px] items-center gap-2">
        <JoluButton variant="outline" className="h-[34px]" onClick={onBack}><ArrowLeft aria-hidden />返回 Skills</JoluButton>
        <span className="flex-1" />
        <JoluButton variant="outline" className="h-[34px]" onClick={() => onOpenDir(current)}><FolderOpen aria-hidden />打开目录</JoluButton>
        <JoluButton variant="outline" className="h-[34px]" onClick={() => onCopy(current)}><Copy aria-hidden />复制到供应商</JoluButton>
        {current.readonly ? (
          <Tip label={current.symlink ? "符号链接安装项只读，请到原位置修改" : "工具托管的安装项只读，可复制为自定义"}>
            <span tabIndex={0}><JoluButton variant="primary" className="h-[34px]" disabled><Pencil aria-hidden />编辑 Skill</JoluButton></span>
          </Tip>
        ) : (
          <JoluButton variant="primary" className="h-[34px]" disabled={!detail} onClick={() => detail && onEdit(current, detail)}><Pencil aria-hidden />编辑 Skill</JoluButton>
        )}
      </div>

      <section className="flex min-h-[92px] items-center gap-4 rounded-xl border border-jolu-border bg-jolu-bg p-5">
        <SkillIcon item={current} size={48} />
        <div className="flex min-w-0 flex-1 flex-col gap-[7px]">
          <h2 className="truncate text-xl font-semibold text-jolu-text">{current.name}</h2>
          <p className="truncate text-xs text-jolu-muted">{current.issue ?? current.description ?? "没有描述"}</p>
        </div>
        <Tag>{providerName(current.platform)}</Tag>
        <Tag>{scopeLabel(current)}</Tag>
        <Tag tone={current.status === "enabled" ? "on" : current.status === "disabled" ? "muted" : "danger"}>{statusLabel(current.status)}</Tag>
      </section>

      {error ? (
        <p role="alert" className="rounded-[10px] bg-jolu-danger-soft px-4 py-3 text-xs text-jolu-danger">读取失败：{error}</p>
      ) : (
        <div className="flex min-h-[650px] gap-4">
          <aside className="flex w-[280px] shrink-0 flex-col gap-4 rounded-xl border border-jolu-border bg-jolu-bg p-4">
            <h3 className="text-sm font-semibold text-jolu-text">文件与来源</h3>
            <div className="flex max-h-[260px] flex-col gap-0.5 overflow-y-auto">
              {!detail && <LoaderCircle aria-hidden className="size-4 animate-spin text-jolu-muted" />}
              {detail?.files.map((entry) => {
                const Icon = fileIcon(entry.rel, entry.dir)
                const active = !entry.dir && entry.rel === file
                const name = entry.rel.split("/").pop() ?? entry.rel
                return (
                  <button
                    key={entry.rel}
                    type="button"
                    disabled={entry.dir}
                    onClick={() => setFile(entry.rel)}
                    title={entry.rel}
                    style={{ paddingLeft: 8 + entry.depth * 14 }}
                    className={cn(
                      "flex h-8 shrink-0 items-center gap-2 rounded-[7px] pr-2 text-left text-xs outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:cursor-default",
                      active ? "bg-jolu-selected text-jolu-on" : "text-jolu-text enabled:hover:bg-jolu-side",
                    )}
                  >
                    <Icon aria-hidden className={cn("size-4 shrink-0", active ? "text-jolu-on" : "text-jolu-muted")} />
                    <span className="min-w-0 flex-1 truncate">{entry.dir ? `${name} /` : name}</span>
                    {!entry.dir && <span className="shrink-0 text-[10px] text-jolu-muted">{formatBytes(entry.size)}</span>}
                  </button>
                )
              })}
              {detail?.files_truncated && <span className="px-2 text-[10px] text-jolu-muted">文件较多，只显示前 200 项</span>}
            </div>
            <span className="h-px bg-jolu-border" />
            <Meta label="来源" value={sourceLabel(current.source)} />
            <Meta label="作用范围" value={current.scope === "project" ? `项目 · ${scopeLabel(current)}` : SCOPE_DETAIL[current.scope]} />
            <Meta label="最近修改" value={modifiedLabel(current.modified_ms)} />
            <Meta label="校验结果" value={current.issue ?? "名称、描述与目录结构有效"} danger={!!current.issue} />
            <span className="text-[10px] text-jolu-muted">{current.original_path ? "原所在目录" : "所在目录"}</span>
            <span className="break-all font-mono text-[11px] text-jolu-muted">{current.original_path ?? current.path}</span>
            {current.original_path && (
              <span className="text-[10px] text-jolu-muted">已移到 Jolu 停用区（工具不会加载），启用时移回原目录：<span className="break-all font-mono">{current.path}</span></span>
            )}
            <JoluButton variant="outline" size="sm" className="h-[30px] self-start" onClick={() => void copyPath()}><Copy aria-hidden />复制路径</JoluButton>
          </aside>

          <section className="flex min-w-0 flex-1 flex-col gap-[18px] rounded-xl border border-jolu-border bg-jolu-bg p-6">
            <div role="tablist" aria-label="内容视图" className="flex h-8 shrink-0 gap-5 border-b border-jolu-border">
              {(["preview", "source"] as const).map((key) => (
                <button
                  key={key}
                  type="button"
                  role="tab"
                  aria-selected={tab === key}
                  onClick={() => setTab(key)}
                  className={cn("-mb-px border-b-2 text-xs outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent", tab === key ? "border-jolu-accent font-semibold text-jolu-on" : "border-transparent text-jolu-muted hover:text-jolu-text")}
                >
                  {key === "preview" ? "预览" : isSkillFile ? "Markdown 源码" : "源码"}
                </button>
              ))}
              <span className="flex-1" />
              {!isSkillFile && <span className="self-center truncate text-[10px] text-jolu-muted">{file}</span>}
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto">
              {!detail || (!isSkillFile && !fileContent) ? (
                <LoaderCircle aria-hidden className="size-4 animate-spin text-jolu-muted" />
              ) : text === null ? (
                <p className="text-xs text-jolu-muted">二进制文件不预览{fileContent?.size ? `（${formatBytes(fileContent.size)}）` : ""}</p>
              ) : tab === "preview" && markdown ? (
                <MarkdownPreview source={text} stripFront={isSkillFile} />
              ) : (
                <pre className="whitespace-pre-wrap break-words font-mono text-[11px] leading-5 text-jolu-text">{text}</pre>
              )}
              {(isSkillFile ? detail?.truncated : fileContent?.truncated) && <p className="mt-3 text-[10px] text-jolu-muted">文件较大，只显示前 512 KB</p>}
            </div>
          </section>
        </div>
      )}

      <div className="flex min-h-[42px] items-center gap-2 rounded-lg bg-jolu-side px-3.5 text-[11px] text-jolu-muted">
        <Info aria-hidden className="size-4 shrink-0" />
        启用不代表立即执行。保存或启停后的生效时机以工具自身的加载规则为准（各工具的具体时机待确认）。
      </div>
    </div>
  )
}

function Meta({ label, value, danger }: { label: string; value: string; danger?: boolean }) {
  return (
    <div className="flex flex-col gap-[5px]">
      <span className="text-[10px] text-jolu-muted">{label}</span>
      <span className={cn("text-xs", danger ? "text-jolu-danger" : "text-jolu-text")}>{value}</span>
    </div>
  )
}
