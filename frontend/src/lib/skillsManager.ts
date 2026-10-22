import type { SkillItemDto, SkillProjectDto, SkillScope, SkillSource, SkillStatus } from "./skillsApi"

/** 模块 F · Skills 管理的纯逻辑：筛选、统计、文案、frontmatter 与 Markdown 预览 */

/** 已核对目录与启停方式的工具（Context7：Claude Code / Codex） */
export const SKILL_PLATFORMS = ["claude", "codex"] as const

/** 画板 76 的供应商菜单：没有可靠目录依据的工具标「待接入」 */
export const PENDING_SKILL_PROVIDERS: { id: string; name: string }[] = [
  { id: "gemini", name: "Gemini" },
  { id: "grok", name: "Grok" },
  { id: "opencode", name: "OpenCode" },
  { id: "zcode", name: "ZCode" },
  { id: "qoder", name: "Qoder" },
  { id: "workbuddy", name: "WorkBuddy" },
]

export function projectLabel(path: string | null): string {
  if (!path) return "项目"
  const parts = path.split(/[\\/]+/).filter(Boolean)
  return parts[parts.length - 1] ?? path
}

export function scopeLabel(item: Pick<SkillItemDto, "scope" | "project">): string {
  switch (item.scope) {
    case "global": return "全局"
    case "project": return projectLabel(item.project)
    case "system": return "系统内置"
    case "synced": return "claude.ai 同步"
  }
}

const SOURCE_LABELS: Record<SkillSource, string> = {
  local: "本地",
  git: "Git 仓库",
  project: "项目",
  imported: "导入",
  synced: "claude.ai",
  system: "系统内置",
}

export function sourceLabel(source: SkillSource): string {
  return SOURCE_LABELS[source]
}

const STATUS_LABELS: Record<SkillStatus, string> = {
  enabled: "已启用",
  disabled: "已停用",
  conflict: "同名待确认",
  invalid: "格式异常",
}

export function statusLabel(status: SkillStatus): string {
  return STATUS_LABELS[status]
}

export type ScopeFilter = "all" | "global" | "system" | "synced" | `project:${string}`
export type StatusFilter = "all" | SkillStatus

export interface SkillFilters {
  scope: ScopeFilter
  status: StatusFilter
  search: string
}

export const EMPTY_FILTERS: SkillFilters = { scope: "all", status: "all", search: "" }

export function hasFilters(f: SkillFilters): boolean {
  return f.scope !== "all" || f.status !== "all" || !!f.search.trim()
}

export function matchesScope(item: SkillItemDto, scope: ScopeFilter): boolean {
  if (scope === "all") return true
  if (scope.startsWith("project:")) return item.scope === "project" && item.project === scope.slice("project:".length)
  return item.scope === scope
}

export function filterSkills(items: readonly SkillItemDto[], f: SkillFilters): SkillItemDto[] {
  const q = f.search.trim().toLowerCase()
  return items.filter((item) =>
    matchesScope(item, f.scope)
    && (f.status === "all" || item.status === f.status)
    && (!q || [item.name, item.description ?? "", item.path].some((text) => text.toLowerCase().includes(q))),
  )
}

/** 「全部范围」下拉：只列出当前结果里出现过的范围 */
export function scopeOptions(items: readonly SkillItemDto[], projects: readonly string[]): { value: ScopeFilter; label: string }[] {
  const options: { value: ScopeFilter; label: string }[] = [{ value: "all", label: "全部范围" }, { value: "global", label: "全局" }]
  for (const p of projects) options.push({ value: `project:${p}`, label: `项目 · ${projectLabel(p)}` })
  if (items.some((i) => i.scope === "system")) options.push({ value: "system", label: "系统内置" })
  if (items.some((i) => i.scope === "synced")) options.push({ value: "synced", label: "claude.ai 同步" })
  return options
}

export const STATUS_OPTIONS: { value: StatusFilter; label: string }[] = [
  { value: "all", label: "全部状态" },
  { value: "enabled", label: "已启用" },
  { value: "disabled", label: "已停用" },
  { value: "conflict", label: "同名待确认" },
  { value: "invalid", label: "格式异常" },
]

export interface SkillStats {
  installed: number
  enabled: number
  conflicts: number
  invalid: number
}

/** 顶部三项统计：按当前供应商，不受范围 / 状态 / 搜索影响 */
export function skillStats(items: readonly SkillItemDto[]): SkillStats {
  return {
    installed: items.length,
    enabled: items.filter((i) => i.status === "enabled").length,
    conflicts: items.filter((i) => i.status === "conflict").length,
    invalid: items.filter((i) => i.status === "invalid").length,
  }
}

/** 实际扫描到 Skills 目录的项目（不含已忽略的） */
export function scannedProjects(projects: readonly SkillProjectDto[], platform: string): string[] {
  return projects
    .filter((p) => !p.ignored && (platform === "claude" ? p.claude : platform === "codex" ? p.codex : p.claude || p.codex))
    .map((p) => p.path)
}

/** 新建 / 导入 / 复制可选的项目：会话记录与手动添加的全部项目，没有 Skills 目录的也可以 */
export function targetProjects(projects: readonly SkillProjectDto[]): string[] {
  return projects.filter((p) => !p.ignored).map((p) => p.path)
}

/** 当前扫描位置：「全局目录 + jolu 项目目录」 */
export function scanLocation(scanned: readonly string[]): string {
  const n = scanned.length
  if (n === 0) return "全局目录"
  if (n === 1) return `全局目录 + ${projectLabel(scanned[0])} 项目目录`
  return `全局目录 + ${n} 个项目目录`
}

/** 启停开关不可用的原因；null 表示可以切换 */
export function toggleBlocked(item: Pick<SkillItemDto, "platform" | "status" | "disabled_by" | "readonly" | "scope" | "symlink">): string | null {
  if (item.status === "invalid") return "格式异常，修复后才能启用"
  if (item.disabled_by === "tool") return "由工具配置停用：这条配置按名称生效，会影响所有同名副本；请在配置文件中修改"
  if (item.platform === "claude" && item.scope === "synced") return "claude.ai 同步的 Skill 请在 claude.ai 中开关"
  if (item.platform === "claude" && item.symlink) return "符号链接安装项不能移动，请到原位置管理"
  return null
}

export function scannedLabel(scannedAt: number, now: number = Date.now()): string {
  const minutes = Math.floor((now - scannedAt) / 60_000)
  if (minutes < 1) return "刚刚扫描"
  if (minutes < 60) return `${minutes} 分钟前扫描`
  return `${Math.floor(minutes / 60)} 小时前扫描`
}

function pad(n: number): string {
  return String(n).padStart(2, "0")
}

/** 最近修改：今天 16:30 / 昨天 09:12 / 10/18 14:02 / 2025/3/2 */
export function modifiedLabel(ms: number | null, now: Date = new Date()): string {
  if (ms === null) return "未知"
  const d = new Date(ms)
  const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime()
  const diff = Math.round((day(now) - day(d)) / 86_400_000)
  if (diff === 0) return `今天 ${hm}`
  if (diff === 1) return `昨天 ${hm}`
  if (d.getFullYear() === now.getFullYear()) return `${d.getMonth() + 1}/${d.getDate()} ${hm}`
  return `${d.getFullYear()}/${d.getMonth() + 1}/${d.getDate()}`
}

/** 名称规则与后端一致：小写字母、数字与连字符，1–64 位，不以连字符开头或结尾 */
export function validSkillName(name: string): boolean {
  return /^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/.test(name)
}

/** 与后端一致：同一目标目录已有同名目录即冲突 */
export function hasNameConflict(items: readonly SkillItemDto[], platform: string, scope: SkillScope, project: string | null, name: string): boolean {
  // 停用区里的副本不在原目录，不占用名称（启用时后端再检查原位置）
  return items.some((i) => i.disabled_by !== "parked" && i.platform === platform && i.scope === scope && (scope !== "project" || i.project === project) && i.dir_name === name)
}

/** 新建与导入的目标目录提示（与后端 write_root 一致） */
export function targetDirHint(platform: string, scope: SkillScope, project: string | null): string {
  const base = scope === "project" ? `${project ?? "<项目>"}/` : "~/"
  if (platform === "claude") return `${base}.claude/skills/`
  return `${base}.agents/skills/`
}

/* ───────────── frontmatter ───────────── */

export function splitFrontmatter(text: string): { front: string | null; body: string } {
  const normalized = text.replace(/^﻿/, "").replace(/\r\n/g, "\n")
  if (!normalized.startsWith("---\n")) return { front: null, body: normalized }
  const end = normalized.indexOf("\n---", 4)
  if (end < 0) return { front: null, body: normalized }
  const after = normalized.indexOf("\n", end + 4)
  return { front: normalized.slice(4, end), body: after < 0 ? "" : normalized.slice(after + 1) }
}

export function frontField(text: string, key: string): string | null {
  const { front } = splitFrontmatter(text)
  if (front === null) return null
  for (const line of front.split("\n")) {
    const match = /^([A-Za-z0-9_-]+):(.*)$/.exec(line)
    if (match && match[1] === key) {
      const value = match[2].trim().replace(/^(["'])(.*)\1$/, "$2")
      return value || null
    }
  }
  return null
}

function yamlValue(value: string): string {
  return /^[\s"'>|[{&*!%@`]|[:#]|\s$/.test(value) || value === "" ? JSON.stringify(value) : value
}

/** 写入或新增 frontmatter 字段；没有 frontmatter 时新建一段 */
export function setFrontField(text: string, key: string, value: string): string {
  const { front, body } = splitFrontmatter(text)
  const line = `${key}: ${yamlValue(value)}`
  if (front === null) return `---\n${line}\n---\n${body.startsWith("\n") ? body : `\n${body}`}`
  const lines = front.split("\n")
  const index = lines.findIndex((l) => l.startsWith(`${key}:`))
  if (index >= 0) {
    // 原值是多行写法时一并替换掉缩进的续行
    let end = index + 1
    while (end < lines.length && /^[ \t]/.test(lines[end])) end++
    lines.splice(index, end - index, line)
  } else {
    lines.push(line)
  }
  return `---\n${lines.join("\n")}\n---\n${body}`
}

/** 画板 80「插入模板」：取设计稿示例的结构（其余模板待确认） */
export function skillTemplate(name: string, description: string): string {
  const title = name.split("-").filter(Boolean).map((w) => w[0].toUpperCase() + w.slice(1)).join(" ") || "New Skill"
  return [
    "---",
    `name: ${yamlValue(name || "new-skill")}`,
    `description: ${yamlValue(description || "说明何时使用这个 Skill")}`,
    "---",
    "",
    `# ${title}`,
    "",
    "## 何时使用",
    "说明适用的任务与触发条件。",
    "",
    "## 执行步骤",
    "1. 第一步。",
    "2. 第二步。",
    "",
    "## 输出",
    "说明需要交付的结果。",
    "",
  ].join("\n")
}

/* ───────────── Markdown 预览（只覆盖 SKILL.md 常见写法） ───────────── */

export type MarkdownBlock =
  | { kind: "heading"; level: number; text: string }
  | { kind: "paragraph"; text: string }
  | { kind: "list"; ordered: boolean; items: string[] }
  | { kind: "code"; text: string }
  | { kind: "rule" }

export function markdownBlocks(markdown: string): MarkdownBlock[] {
  const lines = markdown.replace(/\r\n/g, "\n").split("\n")
  const blocks: MarkdownBlock[] = []
  let paragraph: string[] = []
  const flush = () => {
    // 保留换行：SKILL.md 常把文件清单逐行列出（画板 78「附带资源」）
    if (paragraph.length) blocks.push({ kind: "paragraph", text: paragraph.join("\n") })
    paragraph = []
  }
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    if (/^\s*```/.test(line)) {
      flush()
      const code: string[] = []
      i++
      while (i < lines.length && !/^\s*```/.test(lines[i])) code.push(lines[i++])
      blocks.push({ kind: "code", text: code.join("\n") })
      continue
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line)
    if (heading) {
      flush()
      blocks.push({ kind: "heading", level: heading[1].length, text: heading[2].trim() })
      continue
    }
    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) {
      flush()
      blocks.push({ kind: "rule" })
      continue
    }
    const item = /^\s*(?:([-*+])|(\d+)[.)])\s+(.*)$/.exec(line)
    if (item) {
      flush()
      const ordered = !item[1]
      const last = blocks[blocks.length - 1]
      if (last?.kind === "list" && last.ordered === ordered) last.items.push(item[3])
      else blocks.push({ kind: "list", ordered, items: [item[3]] })
      continue
    }
    if (!line.trim()) {
      flush()
      continue
    }
    paragraph.push(line.trim())
  }
  flush()
  return blocks
}

export type InlinePart = { kind: "text" | "code" | "strong"; text: string }

/** 行内：`代码` 与 **加粗** */
export function inlineParts(text: string): InlinePart[] {
  const parts: InlinePart[] = []
  const pattern = /`([^`]+)`|\*\*([^*]+)\*\*/g
  let last = 0
  for (let m = pattern.exec(text); m; m = pattern.exec(text)) {
    if (m.index > last) parts.push({ kind: "text", text: text.slice(last, m.index) })
    parts.push(m[1] !== undefined ? { kind: "code", text: m[1] } : { kind: "strong", text: m[2] })
    last = m.index + m[0].length
  }
  if (last < text.length) parts.push({ kind: "text", text: text.slice(last) })
  return parts
}

export function formatBytes(size: number | null): string {
  if (size === null) return ""
  if (size < 1024) return `${size} B`
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`
  return `${(size / 1024 / 1024).toFixed(1)} MB`
}
