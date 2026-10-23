// 仅由 async-components.e2e.mjs 在独立浏览器里提供，不写入产品或真实后端。
import React from "/node_modules/.vite/deps/react.js"
import ReactDOM from "/node_modules/.vite/deps/react-dom_client.js"
import { SkillDetail } from "/src/components/skills/SkillDetail.tsx"
import { SkillEditor } from "/src/components/skills/SkillEditor.tsx"
import { TaskDetail } from "/src/components/automation/TaskDetail.tsx"
import { skillsApi } from "/src/lib/skillsApi.ts"
import { automationApi } from "/src/lib/automationApi.ts"
import { demoSkillsOverview, demoSkillDetail } from "/src/mock/skills.ts"
import { demoTaskDetail } from "/src/mock/automation.ts"
import { TipProvider } from "/src/components/ui/Tip.tsx"
import { ToastProvider } from "/src/components/ui/Toast.tsx"
import "/src/index.css"

const calls = []
const now = Date.now()
const overview = demoSkillsOverview("claude")
const items = [0, 1, 2].map((index) => ({
  ...overview.items[0], name: `fixture-skill-${index}`, dir_name: `fixture-skill-${index}`,
  description: "合成测试描述", path: `C:/synthetic-fixtures/skill-${index}`,
}))
const details = items.map((item) => ({
  ...demoSkillDetail(item),
  files: ["SKILL.md", "a.txt", "b.txt", "c.txt"].map((rel) => ({ rel, dir: false, depth: 0, size: 20 })),
}))
function deferred(kind, args) {
  return new Promise((resolve, reject) => calls.push({ kind, args, resolve, reject, settled: false }))
}
skillsApi.detail = (path) => deferred("detail", { path })
skillsApi.file = (path, rel) => deferred("file", { path, rel })
skillsApi.save = (input) => deferred("save", input)
automationApi.quota = (connectionId, force) => deferred("quota", { connectionId, force })

const root = ReactDOM.createRoot(document.getElementById("root"))
const noop = () => {}
let saved = 0
function render(Component, props) {
  root.render(React.createElement(TipProvider, null,
    React.createElement(ToastProvider, null,
      React.createElement("div", { className: "p-6 bg-jolu-bg text-jolu-text" }, React.createElement(Component, props)))))
}
function request(kind, ordinal) {
  const entry = calls.filter((call) => call.kind === kind)[ordinal]
  if (!entry || entry.settled) throw new Error("合成请求不存在或已经完成")
  entry.settled = true
  return entry
}
window.fixture = {
  count: (kind) => calls.filter((call) => call.kind === kind).length,
  savedCount: () => saved,
  showDetail(index) {
    render(SkillDetail, { item: items[index], onBack: noop, onEdit: noop, onCopy: noop, onOpenDir: noop })
  },
  showEditor() {
    render(SkillEditor, {
      target: { mode: "new", platform: "claude" }, items: [], platforms: overview.platforms,
      projects: ["D:/synthetic-project"], onCancel: noop, onSaved: () => { saved++ },
    })
  },
  showTask(connectionId, check) {
    const detail = demoTaskDetail(24, now)
    detail.item = { ...detail.item, connection_id: connectionId,
      active: { ...detail.item.active, connection_id: connectionId, last_check_at: check } }
    render(TaskDetail, {
      detail, now, account: { name: `合成账号 ${connectionId}`, kind: "auth" },
      onBack: noop, onAction: async () => {}, onEdit: noop, onCancel: noop, onDelete: noop,
      onOpenRun: noop, onOpenSession: noop, onOpenProviders: noop,
    })
  },
  completeDetail(ordinal, index) { request("detail", ordinal).resolve(details[index]) },
  completeFile(ordinal, text) { request("file", ordinal).resolve({ text, binary: false, size: text.length, truncated: false }) },
  completeSave(ordinal) { request("save", ordinal).resolve({ path: "C:/synthetic-fixtures/saved", modified_ms: now, backup: null }) },
  completeQuota(ordinal, percent) {
    request("quota", ordinal).resolve({ state: "available", reason: null, resets_at: null,
      windows: [{ key: "5h", name: "5 小时窗口", used_percent: percent, resets_at: null, blocking: false }] })
  },
  fail(kind, ordinal, reason) { request(kind, ordinal).reject(new Error(reason)) },
}
