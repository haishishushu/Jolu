/**
 * 设置页（Jolu.pen 模块 G · 画板 86–103）的纯逻辑：分类、外部跳转映射、数值格式与校验。
 * 界面组件在 components/settings/，这里只放可以 node --test 直接验证的部分。
 */

export type SettingsTab = "general" | "appearance" | "network" | "data" | "notifications" | "about"

/** 左侧分类导航，顺序与文案取自画板 86 */
export const SETTINGS_TABS: readonly { id: SettingsTab; label: string; desc: string }[] = [
  { id: "general", label: "常规", desc: "从打开 Jolu，到随时唤回工作，按你的习惯设置。" },
  { id: "appearance", label: "外观", desc: "调整主题与阅读密度，下面直接预览效果。" },
  { id: "network", label: "网络与代理", desc: "分别管理 Jolu 联网方式与 CLI 本地转发。" },
  { id: "data", label: "数据与存储", desc: "管理本地统计数据，导出与清理前可预览范围。" },
  { id: "notifications", label: "通知", desc: "控制提示方式，保持采集与任务状态可见。" },
  { id: "about", label: "关于与更新", desc: "了解当前版本，选择合适的时间更新。" },
]

/** 托盘 / 菜单发来的设置分区（旧设置页的分区 id）换算成新分类；认不出的分区回到「常规」 */
export function tabForSection(section: string | null | undefined): SettingsTab {
  switch (section) {
    case "appearance":
      return "appearance"
    case "proxy":
    case "network":
      return "network"
    case "data":
      return "data"
    case "notifications":
      return "notifications"
    case "about":
      return "about"
    default:
      return "general"
  }
}

/** 字节数换算：1 MB 以下用 KB，保留一位小数；未知值返回 null，由界面显示「—」 */
export function formatBytes(bytes: number | null | undefined): string | null {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return null
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`
}

/** 计数：千分位；真实的 0 显示 0，未知值返回 null */
export function formatCount(value: number | null | undefined): string | null {
  if (value == null || !Number.isFinite(value)) return null
  return value.toLocaleString("en-US")
}

/** 导入预览的三个数字（画板 100 · 05）：「1,200 条有效 · 180 条重复 · 2 条无效」 */
export function importCountsText(preview: { valid: number; duplicate: number; invalid: number }): string {
  return `${formatCount(preview.valid)} 条有效 · ${formatCount(preview.duplicate)} 条重复 · ${formatCount(preview.invalid)} 条无效`
}

/** 导入完成提示（画板 100 · 05）：跳过 = 重复项 + 无效项 */
export function importDoneText(summary: { requests_changed: number; skipped: number }): string {
  return `完成：已导入 ${formatCount(summary.requests_changed)} 条，跳过 ${formatCount(summary.skipped)} 条`
}

/**
 * 清理截止日期（本地时区，YYYY-MM-DD）。后端按「当前时刻 − days × 24 小时」计算截止点，
 * 预览返回 cutoff_ms 时直接传入 cutoffMs，保证显示与实际删除范围一致。
 */
export function cleanupCutoffDate(days: number, now: Date = new Date(), cutoffMs?: number): string {
  const d = new Date(cutoffMs ?? now.getTime() - days * 86_400_000)
  const pad = (n: number) => String(n).padStart(2, "0")
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

/** 后端允许的保留天数（`set_retention_days` / `cleanup_preview` 共用） */
export const RETENTION_OPTIONS = [30, 90, 180, 365] as const

/** 端口校验（画板 100 · 02）：返回错误文案，合法时返回 null */
export function portError(draft: string): string | null {
  const text = draft.trim()
  if (!/^\d+$/.test(text)) return "请输入 1024–65535 的整数"
  const port = Number(text)
  return port >= 1024 && port <= 65535 ? null : "请输入 1024–65535 的整数"
}

/** 余额提醒阈值：大于 0、不超过 10 亿；合法返回数值，否则 null */
export function parseThreshold(draft: string): number | null {
  const text = draft.trim()
  if (!/^\d+(\.\d+)?$/.test(text)) return null
  const value = Number(text)
  return value > 0 && value <= 1e9 ? value : null
}

export type SaveState = "saved" | "saving" | "dirty" | "failed"

/**
 * 页头保存状态（画板 86 / 100 / 102）：失败优先，其次保存中，再次有未保存草稿，最后已保存。
 */
export function saveState({ saving, error, dirty }: { saving: boolean; error: string | null; dirty: boolean }): SaveState {
  if (error) return "failed"
  if (saving) return "saving"
  if (dirty) return "dirty"
  return "saved"
}

export const SAVE_STATE_LABEL: Record<SaveState, string> = {
  saved: "已保存",
  saving: "保存中…",
  dirty: "有未保存更改",
  failed: "保存失败",
}
