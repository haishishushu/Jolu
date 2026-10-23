import { useEffect, useState } from "react"
import { BellRing, ChevronRight, Info } from "lucide-react"
import { cn } from "@/lib/utils"
import { api } from "@/lib/api"
import { parseThreshold } from "@/lib/settingsPage"
import { useSettings } from "@/hooks/useSettings"
import { useToast } from "@/components/ui/Toast"
import { ActionButton, SettingsGroup, SettingsRow, Switch } from "./settingsUi"

type Cfg = ReturnType<typeof useSettings>
type SetDirty = (key: string, dirty: boolean) => void

/** 通知（画板 94）：免打扰、余额提醒与自动化任务提醒；系统通知由后端发出；当前插件版本不支持 Windows 点击回调，正文写明任务名 */
export function NotificationSection({ cfg, setDirty }: { cfg: Cfg; setDirty: SetDirty }) {
  const toast = useToast()
  const s = cfg.settings
  const disabled = !cfg.ready || cfg.saving
  const saved = s.balance_alert_threshold
  const [amount, setAmount] = useState(saved == null ? "5.00" : saved.toFixed(2))
  const [currency, setCurrency] = useState(s.balance_alert_currency)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)

  useEffect(() => {
    if (saved != null) setAmount(saved.toFixed(2))
    setCurrency(s.balance_alert_currency)
  }, [saved, s.balance_alert_currency])

  const value = parseThreshold(amount)
  const dirty = saved != null && (value !== saved || currency !== s.balance_alert_currency)
  useEffect(() => { setDirty("balance-alert", dirty) }, [dirty, setDirty])
  useEffect(() => () => setDirty("balance-alert", false), [setDirty])

  const save = (threshold: number | null) => {
    if (threshold !== null && !/^[A-Z]{3}$/.test(currency)) {
      setMessage("币种须为三位大写字母，例如 USD、CNY")
      return
    }
    setBusy(true)
    setMessage(null)
    // 余额提醒保存会推送 settings-changed，共享设置随之更新
    api.setBalanceAlert(threshold, currency)
      .then(() => toast.success(threshold === null ? "已关闭低余额提醒" : `余额低于 ${threshold} ${currency} 时提醒`))
      .catch((reason) => toast.danger("余额提醒保存失败", String(reason)))
      .finally(() => setBusy(false))
  }

  const toggle = (on: boolean) => {
    if (!on) return save(null)
    if (value === null) {
      setMessage("请输入大于 0、不超过 10 亿的阈值")
      return
    }
    save(value)
  }

  return (
    <>
      <SettingsGroup title="免打扰" desc="只暂停提示与动效，不停止采集和自动化任务。">
        <SettingsRow
          className="min-h-[72px]"
          title="启用免打扰"
          desc="与系统托盘菜单共用同一设置。"
          control={<Switch label="启用免打扰" on={s.dnd} disabled={disabled} onChange={(on) => void cfg.setDnd(on)} />}
        />
      </SettingsGroup>

      <SettingsGroup title="余额提醒" desc="API 余额低于阈值时提醒；未提供余额的供应商不触发。">
        <SettingsRow
          className="min-h-[72px]"
          title="低余额提醒"
          desc="仅适用于可以获取余额的 API 连接。"
          control={<Switch label="低余额提醒" on={saved != null} disabled={disabled || busy} onChange={toggle} />}
        />
        <div className="flex min-h-[72px] flex-col justify-center gap-1.5 border-t border-jolu-border py-3">
          <div className="flex items-center gap-5">
            <div className="flex min-w-0 flex-1 flex-col gap-1.5">
              <span className="text-[13px] font-medium text-jolu-text">提醒阈值</span>
              <span className="text-[11px] text-jolu-muted">按账户原币种比较，不自动换算汇率。</span>
            </div>
            <span className={cn("inline-flex h-[34px] items-center rounded-lg border bg-jolu-bg pl-3 pr-2 focus-within:border-jolu-accent", message ? "border-jolu-danger" : "border-jolu-border")}>
              <input aria-label="提醒阈值" inputMode="decimal" value={amount} disabled={busy}
                onChange={(e) => { setAmount(e.target.value); setMessage(null) }}
                className="tnum w-16 bg-transparent text-right text-xs text-jolu-text outline-none" />
              <input aria-label="币种" value={currency} maxLength={3} disabled={busy}
                onChange={(e) => { setCurrency(e.target.value.toUpperCase()); setMessage(null) }}
                className="ml-1.5 w-9 bg-transparent text-xs text-jolu-text outline-none" />
            </span>
            <ActionButton tone="soft" busy={busy} disabled={disabled || value === null || (saved != null && !dirty)} onClick={() => value !== null && save(value)}>保存阈值</ActionButton>
          </div>
          {message && <span role="alert" className="text-right text-[11px] text-jolu-danger">{message}</span>}
          {saved == null && !message && <span className="text-right text-[11px] text-jolu-muted">当前未开启；保存阈值即开启提醒</span>}
        </div>
      </SettingsGroup>

      <SettingsGroup title="任务提醒" desc="自动化相关通知偏好。">
        <SettingsRow
          className="min-h-[72px]"
          title="额度恢复与任务继续"
          desc="额度重新确认可用并恢复执行后提醒。"
          control={<Switch label="额度恢复与任务继续" on={s.notify_task_resumed} disabled={disabled} onChange={(on) => void cfg.setTaskNotify({ resumed: on })} />}
        />
        <SettingsRow
          className="min-h-[72px]"
          title="任务完成与失败"
          desc="提供查看任务入口，失败显示简短原因。"
          control={<Switch label="任务完成与失败" on={s.notify_task_finished} disabled={disabled} onChange={(on) => void cfg.setTaskNotify({ finished: on })} />}
        />
      </SettingsGroup>

      <div className="flex flex-col gap-2.5 rounded-xl border border-jolu-border bg-jolu-bg p-4 shadow-sm">
        <span className="flex items-center gap-2 text-[13px] font-medium text-jolu-text">
          <BellRing aria-hidden className="size-4 text-jolu-on" />
          任务已继续
        </span>
        <span className="text-xs text-jolu-muted">Codex 额度已恢复，「完善登录流程」正在原会话中继续。</span>
        <span aria-hidden className="inline-flex h-[34px] w-28 items-center justify-center gap-1.5 rounded-lg bg-jolu-selected text-xs font-medium text-jolu-on">
          查看任务<ChevronRight className="size-3.5" />
        </span>
        <span className="text-[10px] text-jolu-muted">通知样式预览 · 示例</span>
      </div>

      <div className="flex items-start gap-2.5 rounded-[9px] bg-jolu-side p-3.5 text-[11px] text-jolu-muted">
        <Info aria-hidden className="size-4 shrink-0" />
        系统通知权限关闭时显示引导入口；免打扰期间不重复弹出提醒。
      </div>
    </>
  )
}
