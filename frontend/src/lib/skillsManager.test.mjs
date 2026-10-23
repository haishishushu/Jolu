import assert from "node:assert/strict"
import test from "node:test"
import {
  filterSkills, frontField, hasNameConflict, inlineParts, markdownBlocks, modifiedLabel, scanLocation, scopeLabel, scopeOptions,
  scannedProjects, setFrontField, skillStats, skillTemplate, splitFrontmatter, globalChanges, globalRowView, staleParkedNote, targetDirHint, targetProjects, toggleBlocked, validSkillName,
} from "./skillsManager.ts"

const item = (name, extra = {}) => ({
  platform: "claude", name, dir_name: name, description: `${name} 描述`, scope: "global", project: null, source: "local",
  enabled: true, status: "enabled", issue: null, path: `C:/Users/me/.claude/skills/${name}`, original_path: null, disabled_by: null,
  readonly: false, symlink: false, modified_ms: null,
  ...extra,
})

test("筛选：范围、状态与搜索同时生效", () => {
  const items = [
    item("frontend-design"),
    item("docs-writer", { scope: "project", project: "D:/develop/jolu", path: "D:/develop/jolu/.claude/skills/docs-writer" }),
    item("release-notes", { status: "disabled", enabled: false }),
  ]
  assert.deepEqual(filterSkills(items, { scope: "project:D:/develop/jolu", status: "all", search: "" }).map((i) => i.name), ["docs-writer"])
  assert.deepEqual(filterSkills(items, { scope: "all", status: "disabled", search: "" }).map((i) => i.name), ["release-notes"])
  assert.deepEqual(filterSkills(items, { scope: "all", status: "all", search: "JOLU" }).map((i) => i.name), ["docs-writer"])
  assert.equal(scopeLabel(items[1]), "jolu")
  assert.deepEqual(scopeOptions(items, ["D:/develop/jolu"]).map((o) => o.label), ["全部范围", "全局", "项目 · jolu"])
})

test("统计与扫描位置文案", () => {
  const stats = skillStats([item("a"), item("b", { status: "conflict" }), item("c", { status: "invalid" }), item("d", { status: "disabled" })])
  assert.deepEqual(stats, { installed: 4, enabled: 1, conflicts: 1, invalid: 1 })
  assert.equal(scanLocation([]), "全局目录")
  assert.equal(scanLocation(["D:/develop/jolu"]), "全局目录 + jolu 项目目录")
  assert.equal(scanLocation(["a", "b"]), "全局目录 + 2 个项目目录")
})

test("项目：扫描只含有 Skills 目录的，新建目标包含全部未忽略项目", () => {
  const projects = [
    { path: "D:/a", source: "manual", ignored: false, tools: ["claude"] },
    { path: "D:/b", source: "session", ignored: false, tools: ["codex"] },
    { path: "D:/c", source: "session", ignored: false, tools: [] },
    { path: "D:/d", source: "session", ignored: true, tools: ["claude", "codex"] },
  ]
  assert.deepEqual(scannedProjects(projects, "claude"), ["D:/a"])
  assert.deepEqual(scannedProjects(projects, "all"), ["D:/a", "D:/b"])
  assert.deepEqual(targetProjects(projects), ["D:/a", "D:/b", "D:/c"])
})

test("启停开关：工具配置停用、同步与格式异常不可切换", () => {
  assert.equal(toggleBlocked(item("a")), null)
  assert.equal(toggleBlocked(item("a", { disabled_by: "parked", status: "disabled" })), null)
  assert.match(toggleBlocked(item("a", { disabled_by: "tool", status: "disabled" })), /所有同名副本/)
  assert.match(toggleBlocked(item("a", { scope: "synced", readonly: true })), /claude\.ai/)
  assert.match(toggleBlocked(item("a", { status: "invalid" })), /格式异常/)
  assert.equal(toggleBlocked(item("a", { platform: "codex", scope: "system", readonly: true })), null)
})

test("名称规则与同名冲突", () => {
  assert.ok(validSkillName("api-review") && validSkillName("a1"))
  assert.ok(!validSkillName("Api") && !validSkillName("-a") && !validSkillName("a-") && !validSkillName("a b") && !validSkillName(""))
  const items = [item("api-review"), item("docs", { scope: "project", project: "D:/p" })]
  assert.ok(hasNameConflict(items, "claude", "global", null, "api-review"))
  assert.ok(!hasNameConflict(items, "codex", "global", null, "api-review"))
  assert.ok(hasNameConflict(items, "claude", "project", "D:/p", "docs"))
  assert.ok(!hasNameConflict(items, "claude", "project", "D:/q", "docs"))
  assert.ok(!hasNameConflict([item("parked", { disabled_by: "parked" })], "claude", "global", null, "parked"))
})

test("frontmatter 读写保留其余内容", () => {
  const text = "---\nname: api-review\ndescription: >\n  旧描述\nallowed-tools: Read\n---\n# API\n"
  assert.equal(frontField(text, "name"), "api-review")
  const next = setFrontField(text, "description", "核对: 契约")
  assert.equal(next, "---\nname: api-review\ndescription: \"核对: 契约\"\nallowed-tools: Read\n---\n# API\n")
  assert.equal(setFrontField("# 正文", "name", "x"), "---\nname: x\n---\n\n# 正文")
  assert.deepEqual(splitFrontmatter("无 frontmatter"), { front: null, body: "无 frontmatter" })
  const template = skillTemplate("api-review", "在提交前核对 API")
  assert.equal(frontField(template, "name"), "api-review")
  assert.ok(splitFrontmatter(template).body.startsWith("\n# Api Review"))
})

test("Markdown 预览块与行内格式", () => {
  const blocks = markdownBlocks("# 标题\n\n第一段\n续行\n\n1. 一\n2. 二\n- 甲\n\n```\ncode\n```\n---")
  assert.deepEqual(blocks, [
    { kind: "heading", level: 1, text: "标题" },
    { kind: "paragraph", text: "第一段\n续行" },
    { kind: "list", ordered: true, items: ["一", "二"] },
    { kind: "list", ordered: false, items: ["甲"] },
    { kind: "code", text: "code" },
    { kind: "rule" },
  ])
  assert.deepEqual(inlineParts("运行 `pnpm test` 并 **记录**"), [
    { kind: "text", text: "运行 " }, { kind: "code", text: "pnpm test" }, { kind: "text", text: " 并 " }, { kind: "strong", text: "记录" },
  ])
})

test("最近修改时间文案", () => {
  const now = new Date(2026, 9, 22, 18, 0)
  assert.equal(modifiedLabel(new Date(2026, 9, 22, 16, 30).getTime(), now), "今天 16:30")
  assert.equal(modifiedLabel(new Date(2026, 9, 21, 9, 5).getTime(), now), "昨天 09:05")
  assert.equal(modifiedLabel(new Date(2026, 9, 18, 14, 2).getTime(), now), "10/18 14:02")
  assert.equal(modifiedLabel(new Date(2025, 2, 2).getTime(), now), "2025/3/2")
  assert.equal(modifiedLabel(null, now), "未知")
})

test("目标目录提示：全局目录以后端 global_dir 为准（更换全局目录后跟着变）", () => {
  assert.equal(targetDirHint("claude", "global", null), "~/.claude/skills/")
  assert.equal(targetDirHint("codex", "project", "D:/repo"), "D:/repo/.agents/skills/")
  const platforms = [
    { id: "claude", global_dir: "E:\\my-skills" },
    { id: "codex", global_dir: "C:/Users/me/.agents/skills/" },
  ]
  assert.equal(targetDirHint("claude", "global", null, platforms), "E:\\my-skills\\")
  assert.equal(targetDirHint("codex", "global", null, platforms), "C:/Users/me/.agents/skills/")
  assert.equal(targetDirHint("claude", "project", "D:/repo", platforms), "D:/repo/.claude/skills/", "项目目录不受影响")
  assert.equal(targetDirHint("claude", "global", null, [{ id: "claude", global_dir: null }]), "~/.claude/skills/")
})

const root = (extra = {}) => ({
  platform: "claude", scope: "global", project: null, path: "C:/Users/me/.claude/skills", exists: true, readonly: false,
  overridable: true, overridden: false, default_path: null, parked_here: 0, ...extra,
})

test("全局目录草稿：更换 / 恢复默认只改草稿，按草稿显示路径与已更换状态", () => {
  const r = root({ parked_here: 2 })
  assert.deepEqual(globalRowView(r, undefined), { path: r.path, overridden: false, defaultPath: null, changed: false, staleParked: 0 })
  assert.deepEqual(globalRowView(r, "E:/mine"), { path: "E:/mine", overridden: true, defaultPath: r.path, changed: true, staleParked: 2 })
  // 选回默认位置（大小写与分隔符不同）等同未更换
  assert.deepEqual(globalRowView(r, "c:\\users\\me\\.claude\\skills\\"), { path: r.path, overridden: false, defaultPath: null, changed: false, staleParked: 0 })
  const over = root({ path: "E:/mine", overridden: true, default_path: "C:/Users/me/.claude/skills", parked_here: 1 })
  assert.deepEqual(globalRowView(over, undefined), { path: "E:/mine", overridden: true, defaultPath: "C:/Users/me/.claude/skills", changed: false, staleParked: 0 })
  assert.deepEqual(globalRowView(over, null), { path: "C:/Users/me/.claude/skills", overridden: false, defaultPath: null, changed: true, staleParked: 1 })
  assert.deepEqual(globalRowView(over, "F:/other"), { path: "F:/other", overridden: true, defaultPath: "C:/Users/me/.claude/skills", changed: true, staleParked: 1 })
})

test("保存时只提交有改动的全局目录：更换带路径，恢复默认为 null", () => {
  const roots = [
    root(),
    root({ platform: "codex", path: "C:/Users/me/.agents/skills", overridable: false }),
    root({ platform: "codex", path: "E:/cx", overridden: true, default_path: "C:/Users/me/.codex/skills" }),
  ]
  assert.deepEqual(globalChanges(roots, {}), [])
  assert.deepEqual(globalChanges(roots, { claude: "E:/mine", codex: null }), [
    { platform: "claude", path: "E:/mine" },
    { platform: "codex", path: null },
  ])
  assert.deepEqual(globalChanges(roots, { claude: "C:/Users/me/.claude/skills" }), [], "选回原目录不算改动")
  assert.equal(staleParkedNote(3), "有 3 个已停用的 Skill 仍记录在原目录，启用后会回到原目录，不会出现在新目录的列表里。")
})
