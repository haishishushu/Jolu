import { useEffect, useState } from "react"
import { Activity, CircleCheck, Globe, Info, Monitor, Network } from "lucide-react"
import { cn } from "@/lib/utils"
import { api, isTauri, type ProxyStatusDto } from "@/lib/api"
import { portError } from "@/lib/settingsPage"
import { useSettings } from "@/hooks/useSettings"
import { useToast } from "@/components/ui/Toast"
import { Tip } from "@/components/ui/Tip"
import { ActionButton, PendingBadge, Segmented, SettingsGroup, SettingsRow, Switch } from "./settingsUi"

type Cfg = ReturnType<typeof useSettings>
type SetDirty = (key: string, dirty: boolean) => void

const INPUT = "h-[34px] rounded-lg border border-jolu-border bg-jolu-bg px-3 text-xs text-jolu-text outline-none placeholder:text-jolu-muted focus-visible:border-jolu-accent disabled:opacity-45"

/** 网络与代理（画板 90 / 102）：CLI 本地代理已接入；应用联网方式与连接诊断待接入 */
export function NetworkSection({ cfg, setDirty }: { cfg: Cfg; setDirty: SetDirty }) {
  return (
    <>
      <SettingsGroup icon={Globe} title="应用联网" desc="用于 Jolu 的额度查询与更新，不修改 CLI 配置。">
        <div className="flex h-12 items-center gap-3 border-t border-jolu-border">
          <span className="inline-flex flex-1 items-center gap-2 text-[13px] font-medium text-jolu-text">连接方式<PendingBadge /></span>
          <Segmented label="连接方式" value="system" disabled items={[{ value: "system", label: "跟随系统" }, { value: "direct", label: "直连" }, { value: "manual", label: "手动代理" }]} />
        </div>
        <div className="flex h-[42px] items-center gap-2 border-t border-jolu-border text-[11px] text-jolu-muted">
          <Monitor aria-hidden className="size-3.5" />
          <span className="flex-1">应用联网方式尚未接入，当前沿用程序默认的网络设置</span>
          <span className="text-[10px]">切换后自动保存</span>
        </div>
      </SettingsGroup>

      <ProxyGroup cfg={cfg} setDirty={setDirty} />

      <div className="flex flex-col gap-2.5 rounded-xl bg-jolu-side p-4">
        <div className="flex items-center gap-2.5">
          <Activity aria-hidden className="size-4 text-jolu-on" />
          <span className="flex min-w-0 flex-1 flex-col gap-1">
            <span className="inline-flex items-center gap-2 text-[13px] font-medium text-jolu-text">连接诊断<PendingBadge /></span>
            <span className="text-[11px] text-jolu-muted">检查当前网络配置和已连接供应商的连通性。</span>
          </span>
          <ActionButton icon={<Activity aria-hidden className="size-3.5" />} disabled>测试连接</ActionButton>
        </div>
        <div className="flex items-center gap-3 text-[11px] text-jolu-muted">
          <span>尚未检测</span>
          <span className="text-[10px]">检测不会发送模型请求</span>
        </div>
      </div>

      <p className="text-[10px] text-jolu-muted">官方订阅直连保持原方式；缺少延迟数据时显示「—」。</p>
    </>
  )
}

/** CLI 本地代理：开关即接管或还原；端口草稿不覆盖当前运行配置，点击应用后生效 */
function ProxyGroup({ cfg, setDirty }: { cfg: Cfg; setDirty: SetDirty }) {
  const toast = useToast()
  const [status, setStatus] = useState<ProxyStatusDto | null>(null)
  const [portDraft, setPortDraft] = useState(String(cfg.settings.proxy_port))
  const [busy, setBusy] = useState(false)
  const s = cfg.settings

  useEffect(() => {
    if (!isTauri) return
    let stopped = false
    api.proxyStatus().then((next) => { if (!stopped) setStatus(next) }).catch(() => {})
    return () => { stopped = true }
  }, [s.proxy_enabled, s.proxy_port])

  useEffect(() => { setPortDraft(String(s.proxy_port)) }, [s.proxy_port])

  const error = portError(portDraft)
  const changed = portDraft.trim() !== String(s.proxy_port)
  useEffect(() => { setDirty("proxy-port", changed) }, [changed, setDirty])
  useEffect(() => () => setDirty("proxy-port", false), [setDirty])

  const apply = (patch: { enabled?: boolean; fallback?: boolean; port?: number }) => {
    if (busy) return
    setBusy(true)
    const enabled = patch.enabled ?? s.proxy_enabled
    const fallback = patch.fallback ?? s.proxy_fallback_direct
    const port = patch.port ?? s.proxy_port
    void cfg.setProxy(port, enabled, fallback)
      .then((ok) => {
        // 失败由页面顶部的错误条播报，这里不重复出声
        if (!ok) return
        if (patch.enabled !== undefined) {
          if (enabled) toast.success(`本地代理已启用 · 127.0.0.1:${port}`, "CLI 网关流量经本机转发并记录用时 / 首字")
          else toast.success("已停止本地代理", "CLI 配置已还原直连")
        } else if (patch.port !== undefined) {
          toast.success(`监听端口已改为 ${port}`, enabled ? "已按新端口重新接管" : "启用本地代理后按此端口监听")
        } else if (patch.fallback !== undefined) {
          toast.success(fallback ? "已开启失败自动回退直连" : "已关闭失败自动回退直连")
        }
      })
      .finally(() => {
        setBusy(false)
        if (isTauri) api.proxyStatus().then(setStatus).catch(() => {})
      })
  }

  const disabled = !cfg.ready || busy
  const runningNote = (() => {
    if (!s.proxy_enabled) return "当前关闭 · 本地 Token 统计正常采集"
    if (status?.fallback_reason) return `已自动回退直连：${status.fallback_reason}`
    if (!status) return "正在读取代理状态…"
    if (!status.running) return "正在启动监听…"
    return `运行中 · 127.0.0.1:${status.port}`
  })()

  return (
    <SettingsGroup icon={Network} title="CLI 本地代理" desc="转发适配的网关请求，用于采集首字与总耗时。">
      <SettingsRow
        className="min-h-[60px]"
        title="启用本地代理"
        desc={runningNote}
        descClassName={status?.fallback_reason && s.proxy_enabled ? "text-jolu-danger" : undefined}
        control={<Switch label="启用本地代理" on={s.proxy_enabled} disabled={disabled} onChange={(on) => apply({ enabled: on })} />}
      />
      <div className="flex flex-col gap-1.5 border-t border-jolu-border py-3">
        <div className="flex items-center gap-5">
          <div className="flex min-w-0 flex-1 flex-col gap-1.5">
            <span className="text-[13px] font-medium text-jolu-text">监听端口</span>
            <span className="text-[11px] text-jolu-muted">仅监听本机 127.0.0.1</span>
          </div>
          <input
            aria-label="监听端口"
            inputMode="numeric"
            value={portDraft}
            disabled={disabled}
            aria-invalid={!!error}
            onChange={(e) => setPortDraft(e.target.value.replace(/[^\d]/g, ""))}
            className={cn(INPUT, "w-[100px]", error && "border-jolu-danger")}
          />
          <ActionButton
            disabled={disabled || !changed || !!error}
            onClick={() => apply({ port: Number(portDraft) })}
          >
            {s.proxy_enabled ? "应用并重启" : "保存端口"}
          </ActionButton>
        </div>
        {error && changed && <span role="alert" className="text-right text-[11px] text-jolu-danger">{error}</span>}
      </div>
      <SettingsRow
        className="min-h-[60px]"
        title="失败自动回退直连"
        desc="连续 3 次上游不可达后，还原配置并停止代理。"
        control={<Switch label="失败自动回退直连" on={s.proxy_fallback_direct} disabled={disabled} onChange={(on) => apply({ fallback: on })} />}
      />
      {s.proxy_enabled && status && (
        <div className="flex flex-col gap-1.5 border-t border-jolu-border py-3">
          <span className="text-[11px] text-jolu-muted">当前接管</span>
          <TakeoverRow name="Claude Code" taken={status.claude_taken_over} reason={status.claude_taken_over ? `转发到 ${status.claude_upstream ?? "原上游"}` : "官方直连或环境变量上游暂不支持接管"} />
          <TakeoverRow name="Codex" taken={status.codex_taken_over} reason={status.codex_taken_over ? `转发到 ${status.codex_upstream ?? "原上游"}` : "官方订阅暂不支持接管"} />
          {status.last_error && <span role="status" className="text-[11px] text-jolu-danger">最近转发错误：{status.last_error}</span>}
        </div>
      )}
    </SettingsGroup>
  )
}

function TakeoverRow({ name, taken, reason }: { name: string; taken: boolean; reason: string }) {
  return (
    <div className="flex h-6 items-center gap-2 text-xs">
      <span className="flex-1 text-jolu-text">{name}</span>
      <span className={cn("text-[11px]", taken ? "text-jolu-on" : "text-jolu-muted")}>{taken ? "已接管" : "未接管"}</span>
      <Tip label={reason}>
        <span tabIndex={0} aria-label={reason} className="grid size-6 place-items-center rounded-md outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent">
          {taken ? <CircleCheck aria-hidden className="size-3.5 text-jolu-on" /> : <Info aria-hidden className="size-3.5 text-jolu-muted" />}
        </span>
      </Tip>
    </div>
  )
}
