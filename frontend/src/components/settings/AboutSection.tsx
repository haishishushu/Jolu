import { useEffect, useState } from "react"
import { ChevronRight, ExternalLink, Info, RefreshCw } from "lucide-react"
import { openUrl } from "@/lib/api"
import { settingsApi } from "@/lib/settingsApi"
import { UPDATES_ENABLED, progressPercent } from "@/lib/updateState"
import { AppIcon } from "@/components/brand/AppIcon"
import { Wordmark } from "@/components/brand/Wordmark"
import { REPO_URL } from "@/components/panel/PageHeader"
import { useSettings } from "@/hooks/useSettings"
import { useUpdate } from "@/components/panel/UpdateContext"
import { useToast } from "@/components/ui/Toast"
import { ActionButton, SettingsGroup, SettingsRow, Switch } from "./settingsUi"

/** 关于与更新（画板 96）：检查更新默认启用；构建时停用（UPDATES_ENABLED = false）则如实显示停用状态 */
export function AboutSection() {
  const cfg = useSettings()
  const [version, setVersion] = useState<string | null>(null)
  const interval = cfg.settings.update_interval_minutes ?? 30
  const intervalHint = cfg.settings.auto_check_updates
    ? `每 ${formatInterval(interval)} 检查一次；可在下拉中选择常用值或自定义`
    : "关闭自动检查时不生效"
  useEffect(() => {
    let stopped = false
    settingsApi.appVersion().then((v) => { if (!stopped) setVersion(v) }).catch(() => {})
    return () => { stopped = true }
  }, [])
  const versionText = version ? `v${version}` : "—"

  return (
    <>
      <div className="flex h-[152px] items-center gap-5 rounded-xl bg-jolu-side px-6">
        <AppIcon size={72} />
        <div className="flex min-w-0 flex-col gap-2">
          <Wordmark width={81} height={33} />
          <span className="text-[13px] text-jolu-muted">起名寓意：Join + Loop，把 AI 工具连接成环</span>
          <span className="text-[11px] text-jolu-muted">{versionText} · Windows</span>
        </div>
      </div>

      <SettingsGroup title="应用更新" desc="检查完成后显示真实结果，网络失败可重试。">
        {UPDATES_ENABLED ? <UpdateRow versionText={versionText} /> : (
          <SettingsRow
            className="min-h-[72px]"
            title={`当前版本 ${versionText}`}
            desc="检查更新暂时停用，新版本请到 GitHub Releases 下载。"
            control={<ActionButton icon={<RefreshCw aria-hidden className="size-3.5" />} disabled>暂未开放</ActionButton>}
          />
        )}
        <SettingsRow
          className="min-h-[72px]"
          title="自动检查更新"
          desc={UPDATES_ENABLED ? "启动时检查一次，之后按下面的间隔复查；新版本自动在后台下载，安装由你确认。" : "当前发布通道尚未开放，偏好会保留；启用更新后生效。"}
          control={<Switch label="自动检查更新" on={cfg.settings.auto_check_updates} disabled={!cfg.ready || cfg.isSaving("auto_check_updates")} onChange={(on) => void cfg.setAutoCheckUpdates(on)} />}
        />
        <SettingsRow
          className="min-h-[72px]"
          title="检查间隔"
          desc={intervalHint}
          control={
            <UpdateIntervalPicker
              value={cfg.settings.update_interval_minutes ?? 30}
              disabled={!cfg.ready || !cfg.settings.auto_check_updates || cfg.isSaving("update_interval")}
              onCommit={(minutes) => void cfg.setUpdateInterval(minutes)}
            />
          }
        />
      </SettingsGroup>

      {cfg.error && <p role="alert" className="text-xs text-jolu-danger">{cfg.error}</p>}

      <SettingsGroup title="帮助与项目" desc="反馈问题、查看更新记录与开源项目。">
        <SettingsRow
          className="min-h-[72px]"
          title="问题反馈"
          desc="遇到问题或有功能建议？欢迎提交 Issue。"
          control={<ActionButton tone="soft" onClick={() => void openUrl(`${REPO_URL}/issues`)}>提交 Issues<ExternalLink aria-hidden className="size-3.5" /></ActionButton>}
        />
        <SettingsRow
          className="min-h-[72px]"
          title="GitHub"
          desc="查看源代码或反馈问题。"
          control={<ActionButton tone="soft" onClick={() => void openUrl(REPO_URL)}>访问项目<ChevronRight aria-hidden className="size-3.5" /></ActionButton>}
        />
        <SettingsRow
          className="min-h-[72px]"
          title="更新日志"
          desc="了解版本变更与修复内容。"
          control={<ActionButton tone="soft" onClick={() => void openUrl(`${REPO_URL}/releases`)}>查看日志<ChevronRight aria-hidden className="size-3.5" /></ActionButton>}
        />
      </SettingsGroup>

      <div className="flex items-start gap-2.5 rounded-[9px] bg-jolu-side p-3.5 text-[11px] text-jolu-muted">
        <Info aria-hidden className="size-4 shrink-0" />
        下载完成后再选择安装。存在运行中的自动化任务时，会先提示处理任务再重启。
      </div>
    </>
  )
}

/** 检查更新启用后的行：沿用 UpdateContext 的检查、下载与安装流程 */
function UpdateRow({ versionText }: { versionText: string }) {
  const toast = useToast()
  const { phase, info, progress, checkNow, openUpdate } = useUpdate()
  const checking = phase === "checking"
  const hasUpdate = !!info && phase !== "ready"
  const percent = progressPercent(progress.downloaded, progress.total)
  const check = () => {
    void checkNow()
      .then((available) => toast.success(available ? "发现新版本，正在后台下载" : "已是最新版本"))
      .catch((reason) => toast.danger("检查更新失败", String(reason)))
  }
  const version = info ? `v${info.availableVersion}` : ""
  const desc = !hasUpdate ? "启动时和每 30 分钟自动检查，新版本会在后台下载。"
    : phase === "downloaded" ? `${version} 安装包已下载并校验，点击安装即进入安装界面，装完自动重启`
      : phase === "downloading" ? `正在后台下载 ${version}${percent === null ? "" : `（${percent}%）`}`
        : phase === "installing" ? `正在打开 ${version} 安装界面`
          : phase === "downloadError" ? `${version} 下载失败，可重试`
            : `发现新版本 ${version}`
  return (
    <SettingsRow
      className="min-h-[72px]"
      title={`当前版本 ${versionText}`}
      desc={desc}
      control={hasUpdate
        ? <ActionButton tone="accent" busy={phase === "installing"} onClick={openUpdate}>
          {phase === "downloaded" || phase === "installing" ? "安装" : phase === "downloading" ? "查看下载" : phase === "downloadError" ? "重试下载" : "下载"} {version}
        </ActionButton>
        : <ActionButton icon={<RefreshCw aria-hidden className="size-3.5" />} busy={checking} onClick={check}>{checking ? "检查中…" : "检查更新"}</ActionButton>}
    />
  )
}

/** 常用间隔；不在列表中的值走「自定义」输入。 */
const PRESET_INTERVALS = [5, 10, 15, 30, 60, 120, 240] as const

export function formatInterval(minutes: number): string {
  if (minutes < 60) return `${minutes} 分钟`
  if (minutes % 60 === 0) return `${minutes / 60} 小时`
  return `${Math.floor(minutes / 60)} 小时 ${minutes % 60} 分钟`
}

/**
 * 检查间隔选择：预设下拉 + 自定义输入。
 * 自定义值只在离开输入框或按回车时提交，避免每敲一个数字就写一次设置。
 */
function UpdateIntervalPicker({
  value, disabled, onCommit,
}: {
  value: number
  disabled: boolean
  onCommit: (minutes: number) => void
}) {
  const [custom, setCustom] = useState(false)
  const [draft, setDraft] = useState(String(value))
  useEffect(() => { setDraft(String(value)) }, [value])
  const preset = PRESET_INTERVALS.includes(value as (typeof PRESET_INTERVALS)[number])

  const commit = () => {
    const minutes = Number(draft)
    if (!Number.isFinite(minutes) || minutes < 5 || minutes > 1440) {
      setDraft(String(value))
      setCustom(false)
      return
    }
    onCommit(minutes)
    setCustom(false)
  }

  if (custom) {
    return (
      <div className="flex items-center gap-2">
        <input
          type="number"
          min={5}
          max={1440}
          value={draft}
          disabled={disabled}
          aria-label="自定义检查间隔（分钟）"
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => { if (event.key === "Enter") commit() }}
          className="jolu-control h-8 w-24 rounded-lg border border-jolu-border bg-jolu-bg px-2 text-xs text-jolu-text outline-none disabled:opacity-45"
        />
        <span className="text-meta text-jolu-muted">分钟</span>
      </div>
    )
  }

  return (
    <select
      value={preset ? String(value) : "custom"}
      disabled={disabled}
      aria-label="检查间隔"
      onChange={(event) => {
        if (event.target.value === "custom") {
          setCustom(true)
          return
        }
        onCommit(Number(event.target.value))
      }}
      className="jolu-control h-8 rounded-lg border border-jolu-border bg-jolu-bg px-2 text-xs text-jolu-text outline-none disabled:opacity-45"
    >
      {PRESET_INTERVALS.map((minutes) => (
        <option key={minutes} value={minutes}>{formatInterval(minutes)}</option>
      ))}
      <option value="custom">自定义…</option>
    </select>
  )
}
