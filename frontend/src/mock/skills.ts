import type { SkillDetailDto, SkillImportScanDto, SkillItemDto, SkillsOverviewDto } from "@/lib/skillsApi"

/** 仅供浏览器预览的 Skills 示例数据（对齐画板 74 的列表）；桌面端出错时不回落到这里 */

const HOME = "C:/Users/demo/.claude/skills"
const PROJECT = "D:/develop/jolu"

function demo(name: string, description: string | null, extra: Partial<SkillItemDto> = {}): SkillItemDto {
  return {
    platform: "claude",
    name,
    dir_name: name,
    description,
    scope: "global",
    project: null,
    source: "local",
    enabled: true,
    status: "enabled",
    issue: null,
    path: `${HOME}/${name}`,
    original_path: null,
    disabled_by: null,
    readonly: false,
    symlink: false,
    modified_ms: Date.now() - 90 * 60_000,
    ...extra,
  }
}

const inProject = (name: string): Partial<SkillItemDto> => ({ scope: "project", project: PROJECT, source: "project", path: `${PROJECT}/.claude/skills/${name}` })

export function demoSkillsOverview(platform: string): SkillsOverviewDto {
  const claude: SkillItemDto[] = [
    demo("frontend-design", "设计界面布局与视觉规范"),
    demo("code-review", "审查变更与潜在回归", { source: "git" }),
    demo("project-conventions", "项目编码与目录约定", inProject("project-conventions")),
    demo("test-runner", "选择测试并整理验证结果"),
    demo("git-commit-zh", "生成中文提交信息", { source: "git" }),
    demo("docs-writer", "编写与维护项目文档", inProject("docs-writer")),
    demo("release-notes", "汇总版本更新内容", { enabled: false, status: "disabled", disabled_by: "tool", issue: "由工具配置停用：这条配置按名称生效，会影响所有同名副本" }),
    demo("ui-audit", "检查界面一致性与可访问性", {
      ...inProject("ui-audit"), source: "local", enabled: false, status: "disabled", disabled_by: "parked",
      original_path: `${PROJECT}/.claude/skills/ui-audit`, path: "C:/Users/demo/AppData/Roaming/jolu/skills-disabled/20261022-claude/ui-audit",
    }),
    demo("frontend-design", "设计界面布局与视觉规范（项目版）", { ...inProject("frontend-design"), status: "conflict", issue: "同名条目：项目与全局同时存在" }),
    demo("api-contract", null, { source: "imported", enabled: false, status: "invalid", issue: "SKILL.md 缺少 description" }),
  ]
  const codex: SkillItemDto[] = [
    demo("code-review", "审查变更与潜在回归", { platform: "codex", path: "C:/Users/demo/.agents/skills/code-review" }),
    demo("test-runner", "选择测试并整理验证结果", { platform: "codex", path: "C:/Users/demo/.agents/skills/test-runner" }),
    demo("skill-creator", "创建与更新 Skill", { platform: "codex", scope: "system", source: "system", readonly: true, path: "C:/Users/demo/.codex/skills/.system/skill-creator" }),
  ]
  const items = platform === "claude" ? claude : platform === "codex" ? codex : [...claude, ...codex]
  return {
    platforms: [
      { id: "claude", detected: true, count: claude.length },
      { id: "codex", detected: true, count: codex.length },
    ],
    items,
    roots: [
      { platform: "claude", scope: "global", project: null, path: HOME, exists: true, readonly: false },
      { platform: "claude", scope: "project", project: PROJECT, path: `${PROJECT}/.claude/skills`, exists: true, readonly: false },
      { platform: "codex", scope: "global", project: null, path: "C:/Users/demo/.agents/skills", exists: true, readonly: false },
    ].filter((r) => platform === "all" || r.platform === platform) as SkillsOverviewDto["roots"],
    projects: [
      { path: PROJECT, source: "session", ignored: false, claude: true, codex: false },
      { path: "D:/develop/jolu-docs", source: "session", ignored: false, claude: false, codex: false },
      { path: "D:/work/api-proxy", source: "manual", ignored: false, claude: false, codex: false },
    ],
    errors: [],
    scanned_at: Date.now(),
  }
}

const DEMO_CONTENT = `---
name: frontend-design
description: 设计界面布局、视觉层级与交互细节。
---

# Frontend Design

为现有项目建立清晰、统一且可落地的界面设计。

## 适用场景
新建页面、调整布局、优化表格或审查交互一致性时使用。

## 执行步骤
1. 阅读当前界面、主题与用户约束。
2. 确定页面信息层级和主要操作。
3. 复用已有组件、颜色和图标。
4. 检查空状态、加载、错误与键盘操作。

## 输出要求
说明具体改动，给出可检查的设计结果；未确定的能力明确标注。

## 附带资源
references/design-checklist.md
assets/preview.png
`

export function demoSkillDetail(item: SkillItemDto): SkillDetailDto {
  return {
    item,
    files: [
      { rel: "SKILL.md", dir: false, depth: 0, size: 812 },
      { rel: "references", dir: true, depth: 0, size: null },
      { rel: "references/design-checklist.md", dir: false, depth: 1, size: 1260 },
      { rel: "assets", dir: true, depth: 0, size: null },
      { rel: "assets/preview.png", dir: false, depth: 1, size: 48_213 },
    ],
    files_truncated: false,
    content: item.name === "frontend-design" ? DEMO_CONTENT : `---\nname: ${item.name}\ndescription: ${item.description ?? ""}\n---\n\n# ${item.name}\n\n${item.description ?? "（示例内容）"}\n`,
    truncated: false,
  }
}

export function demoImportScan(target: string): SkillImportScanDto {
  return {
    source: "D:/Downloads/jolu-skills/",
    target,
    items: [
      { name: "api-review", path: "D:/Downloads/jolu-skills/api-review", description: "在提交前核对 API 契约和错误处理", status: "ok", issue: null },
      { name: "frontend-design", path: "D:/Downloads/jolu-skills/frontend-design", description: "设计界面", status: "conflict", issue: "目标已有同名 Skill" },
      { name: "quick-test", path: "D:/Downloads/jolu-skills/quick-test", description: null, status: "invalid", issue: "SKILL.md 缺少 description，需先修复" },
    ],
  }
}
