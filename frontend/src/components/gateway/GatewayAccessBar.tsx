import { useEffect, useRef, useState } from "react"
import { Copy, RefreshCw } from "lucide-react"
import { gatewayApi, type GatewayStatusDto } from "@/lib/gatewayApi"
import { gatewayRunLine } from "@/lib/gatewayPage"
import { JoluSwitch } from "@/components/ui/JoluSwitch"
import { ConfirmDialog } from "@/components/ui/ConfirmDialog"
import { useToast } from "@/components/ui/Toast"
import { cn } from "@/lib/utils"

export function GatewayAccessBar({ status, loading, busy, desktop, sources, models, agents, onToggle }: {
  status: GatewayStatusDto | null; loading: boolean; busy: boolean; desktop: boolean
  sources: number; models: number; agents: number; onToggle: (on: boolean) => void
}) {
  const toast = useToast()
  const [mask, setMask] = useState<string | null>(null)
  const [keyError, setKeyError] = useState(false)
  const [working, setWorking] = useState(false)
  const [confirmRefresh, setConfirmRefresh] = useState(false)
  const lock = useRef(false)
  const running = status?.mode === "relay" && status.enabled
  const line = gatewayRunLine(status)
  const address = status ? `http://127.0.0.1:${status.port}` : null
  useEffect(() => {
    if (!desktop) return
    let active = true
    gatewayApi.accessKey(false).then(value => { if (active) { setMask(value); setKeyError(false) } })
      .catch(() => { if (active) setKeyError(true) })
    return () => { active = false }
  }, [desktop])
  async function act(work: () => Promise<unknown>, message: string) {
    if (lock.current) return
    lock.current = true
    setWorking(true)
    try { await work(); toast.success(message) }
    catch (error) { toast.danger("操作未完成", String(error)) }
    finally { lock.current = false; setWorking(false) }
  }
  const button = "inline-flex h-7 shrink-0 items-center gap-1 rounded-md px-2 text-[11px] text-jolu-on hover:bg-jolu-bg focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-40"
  return <section aria-label="模型转发与手动接入" className="flex flex-col gap-3 rounded-xl bg-jolu-status-success-bg px-4 py-3">
    <div className="flex flex-wrap items-center gap-x-5 gap-y-2">
      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
        <span className={cn("h-2 w-2 shrink-0 rounded-full", running ? "bg-jolu-accent" : line.tone === "danger" ? "bg-jolu-danger" : "bg-jolu-muted")} />
        <span className="text-[14px] font-semibold text-jolu-text">{loading ? "读取中…" : !desktop ? "请在桌面端开启模型转发" : !status ? "暂时无法读取运行状态" : line.text}</span>
        <span className="text-[11px] text-jolu-muted">{running ? "已连接的 AI 工具可使用模型池中的模型" : "开启后，AI 工具可使用这里的模型"}</span>
      </div>
      <span className="text-[11px] text-jolu-muted">来源 {sources} · 模型 {models} · 已连接工具 {agents}</span>
      <label className="flex shrink-0 items-center gap-2 text-[11px] text-jolu-text" title="关闭后，一键接入的工具恢复原来的连接方式">
        <JoluSwitch on={!!running} label="开启模型转发" disabled={busy || !desktop || loading || !status} onChange={onToggle} />开启模型转发
      </label>
    </div>
    <div className="flex flex-wrap items-center gap-x-5 gap-y-2 border-t border-jolu-border pt-2 text-[11px]">
      <div className="flex min-w-0 items-center gap-2">
        <span className="shrink-0 text-jolu-muted">网关地址</span>
        <code className="select-all text-jolu-text">{address ?? "暂不可用"}</code>
        <button type="button" className={button} aria-label="复制网关地址" disabled={!address || working} onClick={() => void act(() => navigator.clipboard.writeText(address!), "网关地址已复制")}><Copy size={13} />复制</button>
      </div>
      <div className="flex min-w-0 items-center gap-2">
        <span className="shrink-0 text-jolu-muted">API Key</span>
        <code className="text-jolu-text" aria-live="polite">{mask ?? (keyError ? "读取失败" : desktop ? "读取中…" : "桌面端可用")}</code>
        <button type="button" className={button} aria-label="复制 API Key" disabled={!mask || working || !desktop} onClick={() => void act(() => gatewayApi.copyKey(), "API Key 已复制")}><Copy size={13} />复制</button>
        <button type="button" className={button} aria-label={keyError ? "重试读取 API Key" : "刷新 API Key"} disabled={!desktop || working || (!mask && !keyError)} title="刷新只影响手动配置的工具，一键接入保持可用" onClick={() => keyError
          ? void act(async () => { setMask(await gatewayApi.accessKey(false)); setKeyError(false) }, "API Key 已就绪")
          : setConfirmRefresh(true)}><RefreshCw size={13} />{keyError ? "重试" : "刷新"}</button>
      </div>
      <span className="text-[10px] text-jolu-muted">仅供本机手动接入，一键接入无需填写</span>
    </div>
    {confirmRefresh && <ConfirmDialog
      title="刷新网关 API Key？"
      description="旧 Key 将立即失效。手动配置的工具需要重新填写新 Key；一键接入保持可用。"
      confirmLabel="确认刷新"
      danger
      onClose={() => setConfirmRefresh(false)}
      onConfirm={async () => {
        const next = await gatewayApi.accessKey(true)
        setMask(next)
        setKeyError(false)
        toast.success("API Key 已刷新，请重新复制到手动配置的工具")
      }}
    />}
  </section>
}
