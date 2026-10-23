import { useCallback, useEffect, useState } from "react"
import { ChevronRight, Info, Upload } from "lucide-react"
import { api, isTauri, type ImportPreviewDto, type ImportSummaryDto } from "@/lib/api"
import { settingsApi, type StorageCountsDto } from "@/lib/settingsApi"
import { RETENTION_OPTIONS, formatBytes, formatCount, importCountsText, importDoneText } from "@/lib/settingsPage"
import { useSettings } from "@/hooks/useSettings"
import { useToast } from "@/components/ui/Toast"
import { ActionButton, SelectButton, SettingsGroup, SettingsRow } from "./settingsUi"
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
  // 选择文件并预览中 / 已有预览（确认前可重新选择文件）
  const [previewing, setPreviewing] = useState(false)
  const [preview, setPreview] = useState<ImportPreviewDto | null>(null)
  const disabled = !cfg.ready

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

  // 导入分两步（D-26）：先选文件看预览，确认后才写入
  const pickImport = () => {
    setPreviewing(true)
    api.previewImportFile()
      .then((result) => {
        // 用户在文件选择器里取消，保留原来的预览
        if (result) setPreview(result)
      })
      // 格式不支持等整体错误：提示原因，导入按钮仍可重新选择文件
      .catch((reason) => toast.danger("无法读取备份", String(reason)))
      .finally(() => setPreviewing(false))
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
          desc="选择供应商与日期范围后导出 JSON。"
          control={<ActionButton disabled={!isTauri} onClick={() => setDialog("export")}>导出数据</ActionButton>}
        />
        <SettingsRow
          title="导入已有数据"
          desc="预览记录数量、重复项和无效项后确认。"
          // 有预览卡片（含确认导入进行中）时禁用，先取消或关闭卡片再重新选择文件
          control={<ActionButton busy={previewing} disabled={!isTauri || preview !== null} onClick={pickImport}>{previewing ? "读取中…" : "导入数据"}</ActionButton>}
        />
        {preview && (
          <ImportPreviewCard
            key={preview.token}
            preview={preview}
            onImported={load}
            onExpired={() => setPreview(null)}
            onClose={() => {
              // 释放后端暂存的备份；已导入或已过期时后端什么都不做，失败也不影响关闭
              void api.discardImport(preview.token).catch(() => {})
              setPreview(null)
            }}
          />
        )}
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
              disabled={disabled || cfg.isSaving("retention_days")}
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

/**
 * 导入预览卡片（画板 100 · 05）：文件名、有效 / 重复 / 无效三个数字，确认后只导入有效且不重复的记录。
 * 「查看原因」展开列表与「取消 / 关闭」按钮画板未画，待确认。
 */
function ImportPreviewCard({ preview, onImported, onExpired, onClose }: {
  preview: ImportPreviewDto
  onImported: () => void
  onExpired: () => void
  onClose: () => void
}) {
  const toast = useToast()
  const [busy, setBusy] = useState(false)
  const [done, setDone] = useState<ImportSummaryDto | null>(null)
  const [showReasons, setShowReasons] = useState(false)

  const confirm = () => {
    setBusy(true)
    api.confirmImport(preview.token)
      .then((summary) => { setDone(summary); onImported() })
      .catch((reason) => {
        toast.danger("导入失败", String(reason))
        // 预览只能用一次：失败或过期后都要重新选择文件
        onExpired()
      })
      .finally(() => setBusy(false))
  }

  return (
    <section aria-label="导入预览" className="mb-4 flex flex-col gap-3 rounded-xl border border-jolu-border bg-jolu-bg p-4">
      <h4 className="text-sm font-semibold text-jolu-text">导入预览</h4>
      <p className="break-all text-[11px] leading-normal text-jolu-muted">{preview.file_name || "备份文件"} · 校验完成后才能确认。</p>
      <p className="tnum text-xs text-jolu-text">{importCountsText(preview)}</p>
      <p className="flex flex-wrap items-center gap-2 text-[11px] text-jolu-muted">
        重复项跳过；无效项可查看原因。
        {preview.invalid > 0 && (
          <button type="button" aria-expanded={showReasons} onClick={() => setShowReasons((v) => !v)}
            className="rounded text-jolu-on outline-none hover:underline focus-visible:outline-2 focus-visible:outline-jolu-accent">
            {showReasons ? "收起原因" : "查看原因"}
          </button>
        )}
      </p>
      {showReasons && (
        <ul className="flex flex-col gap-1 rounded-[9px] bg-jolu-side p-3 text-[11px] text-jolu-muted">
          {preview.invalid_reasons.map((reason, index) => <li key={index} className="break-all">{reason}</li>)}
          {preview.invalid > preview.invalid_reasons.length && <li>仅显示前 {preview.invalid_reasons.length} 条原因。</li>}
        </ul>
      )}
      {done ? (
        <div className="flex items-center gap-3">
          <p role="status" className="flex-1 text-[11px] text-jolu-on">{importDoneText(done)}</p>
          <ActionButton onClick={onClose}>关闭</ActionButton>
        </div>
      ) : (
        <div className="flex gap-2">
          <ActionButton tone="accent" icon={<Upload aria-hidden className="size-4" />} busy={busy} disabled={preview.valid === 0} onClick={confirm}>
            导入 {formatCount(preview.valid)} 条
          </ActionButton>
          <ActionButton disabled={busy} onClick={onClose}>取消</ActionButton>
        </div>
      )}
    </section>
  )
}
