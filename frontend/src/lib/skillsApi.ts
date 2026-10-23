import { invoke } from "./api"

/** 模块 F · Skills 管理的后端接口（backend/src/skills.rs） */

export type SkillScope = "global" | "project" | "system" | "synced"
export type SkillStatus = "enabled" | "disabled" | "conflict" | "invalid"
export type SkillSource = "local" | "git" | "project" | "imported" | "synced" | "system"

export interface SkillItemDto {
  platform: string
  /** frontmatter 的 name，缺失时为目录名 */
  name: string
  dir_name: string
  description: string | null
  scope: SkillScope
  /** 项目范围的项目目录 */
  project: string | null
  source: SkillSource
  enabled: boolean
  status: SkillStatus
  issue: string | null
  /** 当前所在目录；Jolu 停用区里的副本为停用区路径 */
  path: string
  /** 停用区里的副本停用前所在目录 */
  original_path: string | null
  /** parked：Jolu 停用区；path：Codex 按路径停用；tool：工具配置按名称停用（只读，影响所有同名副本） */
  disabled_by: "parked" | "path" | "tool" | null
  /** 工具托管或符号链接：只读查看 */
  readonly: boolean
  symlink: boolean
  modified_ms: number | null
}

export interface SkillScanErrorDto {
  path: string
  reason: string
}

export interface SkillRootDto {
  platform: string
  scope: SkillScope
  project: string | null
  path: string
  exists: boolean
  readonly: boolean
  /** 工具主目录，可在「扫描目录设置」里更换（D-18） */
  overridable: boolean
  /** 已更换为其他目录 */
  overridden: boolean
  /** 已更换时的默认位置，「恢复默认」后回到这里 */
  default_path: string | null
  /** 停用区里原位置在这个目录下的 Skill 数；更换目录后它们启用时仍回到这里 */
  parked_here: number
}

export interface SkillPlatformDto {
  id: string
  detected: boolean
  count: number
  /** 目录规则已按官方文档核实 */
  verified: boolean
  /** 新建、导入、复制到全局时的目标目录；工具不支持写入时为 null */
  global_dir: string | null
}

export interface SkillProjectDto {
  path: string
  /** manual：手动添加；session：来自会话记录 */
  source: "manual" | "session"
  ignored: boolean
  /** 含有项目级 Skills 目录的工具 id */
  tools: string[]
}

export interface SkillsOverviewDto {
  platforms: SkillPlatformDto[]
  items: SkillItemDto[]
  roots: SkillRootDto[]
  projects: SkillProjectDto[]
  errors: SkillScanErrorDto[]
  scanned_at: number
}

export interface SkillFileEntryDto {
  rel: string
  dir: boolean
  depth: number
  size: number | null
}

export interface SkillDetailDto {
  item: SkillItemDto
  files: SkillFileEntryDto[]
  files_truncated: boolean
  content: string
  truncated: boolean
}

export interface SkillFileContentDto {
  text: string | null
  binary: boolean
  size: number
  truncated: boolean
}

export interface SkillSaveInput {
  platform: string
  scope: SkillScope
  project: string | null
  name: string
  content: string
  original_path: string | null
  expected_modified_ms: number | null
  enable: boolean
}

export interface SkillSaveResultDto {
  path: string
  modified_ms: number | null
  backup: string | null
}

export interface SkillCopyResultDto {
  target: string
  copied: boolean
  conflict: boolean
  issue: string | null
}

export interface SkillImportItemDto {
  name: string
  path: string
  description: string | null
  status: "ok" | "conflict" | "invalid"
  issue: string | null
}

export interface SkillImportScanDto {
  source: string
  target: string
  items: SkillImportItemDto[]
}

export type SkillImportAction = "import" | "overwrite" | "skip"

export interface SkillImportResultDto {
  imported: string[]
  skipped: string[]
  failed: SkillScanErrorDto[]
}

export interface SkillGitImportScanDto {
  /** 应用临时目录下的克隆目录，导入完成或取消后调用 `importGitCleanup` */
  checkout_dir: string
  /** `scan.source` 是实际导入来源：克隆根目录，或仓库里的 `skills/` 子目录 */
  scan: SkillImportScanDto
}

export interface SkillDirSettingsInput {
  /** 手动添加的项目目录（完整列表） */
  projects: string[]
  /** 会话记录带出的项目里改了忽略状态的 */
  ignore: { path: string; ignored: boolean }[]
  /** 改了全局目录的工具；path 为 null 表示恢复默认 */
  globals: { platform: string; path: string | null }[]
}

/** 编辑保存时文件已被其他程序改动（画板 84「检测到外部修改」） */
export const SKILL_EXTERNAL_CHANGE = "external_change"

export const skillsApi = {
  /** `force` 时重新读取会话记录里的项目目录（否则用 10 分钟内的缓存） */
  overview: (platform: string, force = false) => invoke<SkillsOverviewDto>("skills_overview", { platform, force }),
  detail: (path: string) => invoke<SkillDetailDto>("skill_detail", { path }),
  file: (path: string, rel: string) => invoke<SkillFileContentDto>("skill_file", { path, rel }),
  /** 返回启停后的目录：Claude Code 停用后位于 Jolu 停用区 */
  setEnabled: (path: string, enabled: boolean) => invoke<string>("skill_set_enabled", { path, enabled }),
  save: (input: SkillSaveInput) => invoke<SkillSaveResultDto>("skill_save", { input }),
  copy: (path: string, platform: string, scope: SkillScope, project: string | null, dryRun: boolean) =>
    invoke<SkillCopyResultDto>("skill_copy", { path, platform, scope, project, dryRun }),
  remove: (path: string) => invoke<string>("skill_remove", { path }),
  importScan: (source: string, platform: string, scope: SkillScope, project: string | null) =>
    invoke<SkillImportScanDto>("skills_import_scan", { source, platform, scope, project }),
  importSkills: (source: string, platform: string, scope: SkillScope, project: string | null, choices: { name: string; action: SkillImportAction }[], enable: boolean) =>
    invoke<SkillImportResultDto>("skills_import", { source, platform, scope, project, choices, enable }),
  setProjects: (projects: string[]) => invoke<string[]>("skills_set_projects", { projects }),
  ignoreProject: (path: string, ignored: boolean) => invoke<void>("skills_ignore_project", { path, ignored }),
  pickFolder: (title: string) => invoke<string | null>("skills_pick_folder", { title }),
  openDir: (path: string) => invoke<void>("skill_open_dir", { path }),
  /** 从 Git 仓库导入（D-16）：本机 git 浅克隆到临时目录并扫描，只接受 https:// */
  importGit: (url: string, branch: string | null, platform: string, scope: SkillScope, project: string | null) =>
    invoke<SkillGitImportScanDto>("skills_import_git", { url, branch, platform, scope, project }),
  importGitCleanup: (checkoutDir: string) => invoke<void>("skills_import_git_cleanup", { checkoutDir }),
  /** 「扫描目录设置」的「保存并扫描」：项目目录、忽略状态、全局目录（D-18）一次提交，任一项无效则都不写入 */
  saveDirSettings: (input: SkillDirSettingsInput) => invoke<void>("skills_save_dir_settings", { input }),
}
