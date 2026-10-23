import { useEffect, useRef, useState } from "react"
import { Activity, CircleAlert, CircleCheck, Globe, Info, Link, Monitor, Network } from "lucide-react"
import { cn } from "@/lib/utils"
import { api, isTauri, type ProxyStatusDto } from "@/lib/api"
import { portError } from "@/lib/settingsPage"
import { settingsApi } from "@/lib/settingsApi"
import { PLATFORMS } from "@/lib/platforms"
import {
  NETWORK_MODES, diagnosisSummary, isNetworkDraft, latencyText, networkStatusLine, normalizeNetworkMode, probeStatus, proxyUrlError,
  type NetworkDiagnosisDto, type NetworkMode, type Tone,
} from "@/lib/networkSettings"
import { useSettings } from "@/hooks/useSettings"
import { useToast } from "@/components/ui/Toast"
import { Tip } from "@/components/ui/Tip"
import { ActionButton, Segmented, SettingsGroup, SettingsRow, Switch } from "./settingsUi"

type Cfg = ReturnType<typeof useSettings>
type SetDirty = (key: string, dirty: boolean) => void

const INPUT = "h-[34px] rounded-lg border border-jolu-border bg-jolu-bg px-3 text-xs text-jolu-text outline-none placeholder:text-jolu-muted focus-visible:border-jolu-accent disabled:opacity-45"

const TONE_TEXT: Record<Tone, string> = { ok: "text-jolu-on", danger: "text-jolu-danger", muted: "text-jolu-muted" }

/** 网络与代理（画板 90 / 102）：应用联网方式、CLI 本地代理与连接诊断 */
export function NetworkSection({ cfg, setDirty }: { cfg: Cfg; setDirty: SetDirty }) {
  const toast = useToast()
  const s = cfg.settings
  const savedMode = normalizeNetworkMode(s.network_mode)
  // 草稿：手动代理在点「保存配置」前不生效；跟随系统 / 直连切换即保存
  const [mode, setMode] = useState<NetworkMode>(savedMode)
  const [urlDraft, setUrlDraft] = useState(s.network_proxy_url ?? "")
  useEffect(() => { setMode(savedMode) }, [savedMode])
  useEffect(() => { setUrlDraft(s.network_proxy_url ?? "") }, [s.network_proxy_url])

  const draft = isNetworkDraft(mode, urlDraft, s)
  useEffect(() => { setDirty("network-proxy", draft) }, [draft, setDirty])
  useEffect(() => () => setDirty("network-proxy", false), [setDirty])
  const urlError = mode === "manual" && urlDraft.trim() !== "" ? proxyUrlError(urlDraft) : null
  const disabled = !cfg.ready || cfg.saving

  const choose = (next: NetworkMode) => {
    setMode(next)
    // 手动代理等「保存配置」；切回已保存的方式只是放弃草稿，不必重写设置
    if (next === "manual" || next === savedMode) return
    void cfg.setNetwork(next, null).then((saved) => {
      // 失败由页面顶部的错误条播报，界面回到已保存的方式
      if (!saved) { setMode(savedMode); return }
      toast.success(next === "direct" ? "已切换为直连" : "已切换为跟随系统", "额度查询与检查更新按新的联网方式发出")
    })
  }
  const canSaveManual = !disabled && draft && !proxyUrlError(urlDraft)
  const saveManual = () => {
    if (!canSaveManual) return
    void cfg.setNetwork("manual", urlDraft.trim()).then((saved) => {
      if (!saved) return
      // 后端会规整地址（协议小写、去掉结尾的 /），用保存结果回填，避免一直显示草稿态
      const url = saved.network_proxy_url ?? ""
      setUrlDraft(url)
      toast.success("已切换为手动代理", url)
    })
  }

  return (
    <>
      <SettingsGroup icon={Globe} title="应用联网" desc="用于 Jolu 的额度查询与更新，不修改 CLI 配置。">
        <div className="flex h-12 items-center gap-3 border-t border-jolu-border">
          <span className="flex-1 text-[13px] font-medium text-jolu-text">连接方式</span>
          <Segmented label="连接方式" value={mode} disabled={disabled} items={NETWORK_MODES} onChange={choose} />
        </div>
        {mode === "manual" ? (
          <div className="flex flex-col gap-2 pt-2 pb-3.5">
            <div className="flex items-center gap-2">
              <label className={cn("flex h-9 min-w-0 flex-1 items-center gap-2 rounded-lg border border-jolu-border px-2.5 focus-within:border-jolu-accent", urlError && "border-jolu-danger")}>
                <Link aria-hidden className="size-[15px] shrink-0 text-jolu-muted" />
                <input
                  aria-label="代理地址"
                  value={urlDraft}
                  disabled={disabled}
                  aria-invalid={!!urlError}
                  placeholder="http://127.0.0.1:7890"
                  spellCheck={false}
                  onChange={(e) => setUrlDraft(e.target.value)}
                  onKeyDown={(e) => { if (e.key === "Enter") saveManual() }}
                  className="min-w-0 flex-1 bg-transparent text-xs text-jolu-text outline-none placeholder:text-jolu-muted disabled:opacity-45"
                />
              </label>
              <ActionButton tone="soft" disabled={!canSaveManual} onClick={saveManual}>保存配置</ActionButton>
            </div>
            {urlError
              ? <span role="alert" className="text-[10px] text-jolu-danger">{urlError}</span>
              : <span className="text-[10px] text-jolu-muted">{urlDraft.trim() === "" ? "填写代理服务地址，仅支持 http:// 或 https://" : networkStatusLine(mode, s, urlDraft)}</span>}
          </div>
        ) : (
          <div className="flex h-[42px] items-center gap-2 border-t border-jolu-border text-[11px] text-jolu-muted">
            <Monitor aria-hidden className="size-3.5" />
            <span className="flex-1">{networkStatusLine(mode, s, urlDraft)}</span>
            <span className="text-[10px]">切换后自动保存</span>
          </div>
        )}
      </SettingsGroup>

      <ProxyGroup cfg={cfg} setDirty={setDirty} />

      <DiagnosisPanel mode={mode} proxyUrl={mode === "manual" ? urlDraft.trim() : null} draft={draft} blocked={!!(mode === "manual" && proxyUrlError(urlDraft))} />

      <p className="text-[10px] text-jolu-muted">官方订阅直连保持原方式；缺少延迟数据时显示「—」。</p>
    </>
  )
}

type DiagnosisState =
  | { phase: "idle" }
  | { phase: "running" }
  | { phase: "done"; result: NetworkDiagnosisDto; draft: boolean }
  | { phase: "error"; message: string }

/** 连接诊断（画板 90 / 102）：用当前草稿参数检测，不保存设置 */
function DiagnosisPanel({ mode, proxyUrl, draft, blocked }: { mode: NetworkMode; proxyUrl: string | null; draft: boolean; blocked: boolean }) {
  const [state, setState] = useState<DiagnosisState>({ phase: "idle" })
  // 每次检测一个序号；参数一变序号就过期，进行中的检测结果到达后直接丢弃
  const request = useRef(0)
  useEffect(() => { request.current++; setState({ phase: "idle" }) }, [mode, proxyUrl])

  const run = () => {
    const id = ++request.current
    setState({ phase: "running" })
    settingsApi.diagnoseNetwork(mode, proxyUrl)
      .then((result) => { if (request.current === id) setState({ phase: "done", result, draft }) })
      .catch((reason) => { if (request.current === id) setState({ phase: "error", message: String(reason) }) })
  }

  return (
    <div className="flex flex-col gap-2.5 rounded-xl bg-jolu-side p-4">
      <div className="flex items-center gap-2.5">
        <Activity aria-hidden className="size-[18px] shrink-0 text-jolu-on" />
        <span className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="text-[13px] font-medium text-jolu-text">连接诊断</span>
          <span className="text-[11px] text-jolu-muted">检查当前网络配置和已连接供应商的连通性。</span>
        </span>
        <ActionButton
          icon={<Activity aria-hidden className="size-3.5" />}
          busy={state.phase === "running"}
          disabled={!isTauri || blocked}
          onClick={run}
        >
          测试连接
        </ActionButton>
      </div>
      {state.phase === "idle" && (
        <div className="flex items-center gap-3 text-[11px] text-jolu-muted">
          <span>尚未检测</span>
          <span className="text-[10px]">检测不会发送模型请求</span>
        </div>
      )}
      {state.phase === "running" && <div role="status" className="text-[11px] text-jolu-muted">正在检测…</div>}
      {state.phase === "error" && <div role="alert" className="text-[11px] text-jolu-danger">检测失败：{state.message}</div>}
      {state.phase === "done" && <DiagnosisResult result={state.result} draft={state.draft} />}
    </div>
  )
}

function DiagnosisResult({ result, draft }: { result: NetworkDiagnosisDto; draft: boolean }) {
  const summary = diagnosisSummary(result)
  const allOk = summary.every((item) => item.tone === "ok")
  const StatusIcon = allOk ? CircleCheck : CircleAlert
  return (
    <div role="status" className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px]">
        <StatusIcon aria-hidden className={cn("size-3.5", allOk ? "text-jolu-on" : "text-jolu-danger")} />
        {summary.map((item) => <span key={item.text} className={TONE_TEXT[item.tone]}>{item.text}</span>)}
        {result.proxy && !result.proxy.ok && result.proxy.error && <span className="text-jolu-muted">{result.proxy.error}</span>}
        <span className="text-[10px] text-jolu-muted">{draft ? "草稿检测" : "当前配置"}</span>
      </div>
      {/* 待确认：画板 102 只画了摘要行；逐个连接的结果按「名称 · 状态 · 延迟」一行一个列出 */}
      {result.endpoints.length > 0 && (
        <ul className="flex flex-col border-t border-jolu-border">
          {result.endpoints.map((row, index) => {
            const status = probeStatus(row.probe)
            return (
              <li key={`${row.connection_name}-${index}`} className="flex h-8 items-center gap-3 border-b border-jolu-border text-[11px] last:border-b-0">
                <span className="min-w-0 flex-1 truncate text-xs text-jolu-text">
                  {row.connection_name}
                  <span className="ml-2 text-[11px] text-jolu-muted">{platformName(row.platform)}</span>
                </span>
                <span className={cn("max-w-[45%] truncate", TONE_TEXT[status.tone])}>{status.text}</span>
                <span className="w-14 shrink-0 text-right tabular-nums text-jolu-muted">{latencyText(row.probe)}</span>
              </li>
            )
          })}
        </ul>
      )}
    </div>
  )
}

function platformName(id: string): string {
  return PLATFORMS.find((platform) => platform.id === id)?.name ?? id
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
