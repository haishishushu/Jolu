import { useCallback, useEffect, useState } from "react"
import { ChevronRight, Info } from "lucide-react"
import { api, isTauri } from "@/lib/api"
import { settingsApi, type StorageCountsDto } from "@/lib/settingsApi"
import { RETENTION_OPTIONS, formatBytes, formatCount } from "@/lib/settingsPage"
import { useSettings } from "@/hooks/useSettings"
import { useToast } from "@/components/ui/Toast"
import { ActionButton, PendingBadge, SelectButton, SettingsGroup, SettingsRow } from "./settingsUi"
import { CleanupDialog, ExportDialog } from "./DataDialogs"

type Cfg = ReturnType<typeof useSettings>

/** 读取中 / 失败 / 有值分别显示，未知值不以 0 代替 */
type Loaded<T> = { state: "loading" } | { state: "failed"; reason: string } | { state: "ok"; value: T }

/** 数据与存储（画板 92 / 98）：概览、导入导出、历史保留与存储位置 */
export function DataSection({ cfg, onOpenSkills }: { cfg: Cfg; onOpenSkills: () => void }) {
  const toast = useToast()
  const [size, setSize] = useState<Loaded<number>>({ state: "loading" })
  const [counts, setCounts] = useState<Loaded<StorageCountsDto>>({ state: "loading" })
  const [dialog, setDialog] = useState<null | "export" | "cleanup">(null)
  const [importing, setImporting] = useState(false)
  const disabled = !cfg.ready || cfg.saving

  const load = useCallback(() => {
    if (!isTauri) {
      setSize({ state: "failed", reason: "浏览器预览不读取本机数据" })
      setCounts({ state: "failed", reason: "浏览器预览不读取本机数据" })
      return
    }
    api.dataInfo().then((info) => setSize({ state: "ok", value: info.size_bytes }), (reason) => setSize({ state: "failed", reason: String(reason) }))
    settingsApi.storageCounts().then((value) => setCounts({ state: "ok", value }), (reason) => setCounts({ state: "failed", reason: String(reason) }))
  }, [])
  useEffect(load, [load])

  const runImport = () => {
    setImporting(true)
    api.importDataFile()
      .then((result) => {
        // 用户在文件选择器里取消，不弹提示
        if (!result) return
        toast.success("导入完成", `更新 ${result.requests_changed.toLocaleString("zh-CN")} 条请求、${result.sessions_changed.toLocaleString("zh-CN")} 条会话标题；重复项已跳过`)
        load()
      })
      .catch((reason) => toast.danger("导入失败", String(reason)))
      .finally(() => setImporting(false))
  }

  const tile = (label: string, loaded: Loaded<unknown>, text: string | null) => (
    <div className="flex min-w-0 flex-1 flex-col justify-center gap-1.5 rounded-[10px] bg-jolu-side px-4">
      <span className="text-[11px] text-jolu-muted">{label}</span>
      <span className="tnum truncate text-[22px] font-semibold text-jolu-text" title={loaded.state === "failed" ? loaded.reason : undefined}>
        {loaded.state === "loading" ? "读取中…" : loaded.state === "failed" ? "—" : text ?? "—"}
      </span>
    </div>
  )

  const failure = size.state === "failed" ? size.reason : counts.state === "failed" ? counts.reason : null

  return (
    <>
      <div className="flex h-[86px] gap-3">
        {tile("统计数据", size, size.state === "ok" ? formatBytes(size.value) : null)}
        {tile("请求记录", counts, counts.state === "ok" ? `${formatCount(counts.value.requests)} 条` : null)}
        {tile("会话索引", counts, counts.state === "ok" ? `${formatCount(counts.value.sessions)} 个` : null)}
      </div>
      {failure && isTauri && <p role="alert" className="-mt-2 text-[11px] text-jolu-danger">本地数据概览读取失败：{failure}</p>}

      <SettingsGroup title="导入与导出" desc="统计记录和会话标题可迁移，导入时自动去重。">
        <SettingsRow
          title="导出本地数据"
          desc="选择供应商后导出 JSON；按日期范围导出待接入。"
          control={<ActionButton disabled={!isTauri} onClick={() => setDialog("export")}>导出数据</ActionButton>}
        />
        <SettingsRow
          title={<span className="inline-flex items-center gap-2">导入已有数据<PendingBadge>预览待接入</PendingBadge></span>}
          desc="选择文件后直接导入，重复记录自动跳过；导入前预览数量待接入。"
          control={<ActionButton busy={importing} disabled={!isTauri} onClick={runImport}>{importing ? "导入中…" : "导入数据"}</ActionButton>}
        />
        <div className="mb-4 flex items-center gap-2.5 rounded-[9px] bg-jolu-side p-3.5 text-[11px] text-jolu-muted">
          <Info aria-hidden className="size-4 shrink-0" />
          导出文件不包含 API Key、登录凭证和原始会话正文。
        </div>
      </SettingsGroup>

      <SettingsGroup title="历史数据保留" desc="默认不自动删除历史数据。">
        <SettingsRow
          title="自动清理规则"
          desc="仅影响 Jolu 本地统计记录。"
          control={
            <SelectButton
              label="自动清理规则"
              value={String(cfg.settings.retention_days ?? "")}
              disabled={disabled}
              onChange={(v) => void cfg.setRetentionDays(v ? Number(v) : null)}
              options={[{ value: "", label: "不自动清理" }, ...RETENTION_OPTIONS.map((n) => ({ value: String(n), label: `保留 ${n} 天` }))]}
            />
          }
        />
        <SettingsRow
          title="手动清理历史"
          desc="预览日期和记录数量后再确认。"
          control={<ActionButton disabled={!isTauri} onClick={() => setDialog("cleanup")}>预览清理</ActionButton>}
        />
      </SettingsGroup>

      <SettingsGroup title="存储位置" desc="查看本机数据与 Skills 所在位置。">
        <SettingsRow
          title="Jolu 数据目录"
          desc="打开当前应用数据文件夹。"
          control={<ActionButton disabled={!isTauri} onClick={() => void api.openDataDirectory().catch((reason) => toast.danger("打开数据目录失败", String(reason)))}>打开文件夹</ActionButton>}
        />
        <SettingsRow
          title="Skills 扫描目录"
          desc="项目与全局目录在 Skills 模块管理。"
          control={<ActionButton tone="soft" onClick={onOpenSkills}>前往 Skills<ChevronRight aria-hidden className="size-3.5" /></ActionButton>}
        />
      </SettingsGroup>

      {dialog === "export" && <ExportDialog onClose={() => setDialog(null)} />}
      {dialog === "cleanup" && (
        <CleanupDialog
          initialDays={cfg.settings.retention_days ?? 90}
          onClose={() => setDialog(null)}
          onExport={() => setDialog("export")}
          onDone={() => { setDialog(null); load() }}
        />
      )}
    </>
  )
}
