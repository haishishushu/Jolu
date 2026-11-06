import { useEffect, useRef, useState } from "react"
import { Check, Copy, RefreshCw } from "lucide-react"
import { gatewayApi, type GatewayStatusDto } from "@/lib/gatewayApi"
import { gatewayRunLine } from "@/lib/gatewayPage"
import { JoluSwitch } from "@/components/ui/JoluSwitch"
import { ConfirmDialog } from "@/components/ui/ConfirmDialog"
import { useToast } from "@/components/ui/Toast"
import { cn } from "@/lib/utils"
import { StatusCapsule } from "./StatusCapsule"

/**
 * 网关连接信息 · 单行 Base URL 与 API Key（Jolu.pen `PA0Zm`，统一网关开发说明 §3.1）。
 *
 * - Base URL 显示实际监听端口，复制完整值；不追加任何协议路径；
 * - API Key 只显示后端返回的掩码，复制由后端直接写剪贴板，前端拿不到完整密钥；
 * - 刷新替换手动接入 Key（旧 Key 立即失效），不改 Agent 配置、不刷新供应商凭据；
 * - 复制成功在按钮上短暂显示对勾，失败走通知；结果出来之前不显示成功。
 */
export function GatewayAccessBar({ status, loading, busy, desktop, sources, models, agents, onToggle }: {
  status: GatewayStatusDto | null; loading: boolean; busy: boolean; desktop: boolean
  sources: number; models: number; agents: number; onToggle: (on: boolean) => void
}) {
  const toast = useToast()
  const [mask, setMask] = useState<string | null>(null)
  const [keyError, setKeyError] = useState(false)
  const [working, setWorking] = useState<"url" | "key" | "refresh" | null>(null)
  const [copied, setCopied] = useState<"url" | "key" | null>(null)
  const [confirmRefresh, setConfirmRefresh] = useState(false)
  const copiedTimer = useRef<number | null>(null)
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
  useEffect(() => () => { if (copiedTimer.current) window.clearTimeout(copiedTimer.current) }, [])

  async function act(kind: "url" | "key" | "refresh", work: () => Promise<unknown>, failure: string, copiedKind?: "url" | "key") {
    if (lock.current) return
    lock.current = true
    setWorking(kind)
    try {
      await work()
      if (copiedKind) {
        setCopied(copiedKind)
        if (copiedTimer.current) window.clearTimeout(copiedTimer.current)
        copiedTimer.current = window.setTimeout(() => setCopied(null), 1200)
      }
    } catch (error) {
      toast.danger(failure, String(error))
    } finally {
      lock.current = false
      setWorking(null)
    }
  }

  const statusText = loading ? "读取中…" : !desktop ? "仅桌面端可用" : !status ? "状态未知"
    : running ? "已接入" : status.mode === "relay" ? "未启动" : status.mode === "observe" ? "未接入" : "状态未知"
  const iconButton = "grid size-10 shrink-0 place-items-center rounded-md text-jolu-muted outline-none hover:bg-jolu-stage hover:text-jolu-text focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-40"

  return <section aria-label="网关连接信息" className="flex h-16 shrink-0 items-center gap-4 overflow-x-auto rounded-xl bg-jolu-selected px-5">
    <StatusCapsule
      tone={running ? "on" : desktop && status?.mode === "relay" ? "danger" : "off"}
      label={statusText}
      title={!desktop ? "仅桌面端可用" : status ? `${line.text}；点击${running ? "关闭" : "允许"}接入` : undefined}
      disabled={busy || !desktop || loading || !status}
      onClick={() => onToggle(!running)}
    />

    <div className="flex h-10 shrink-0 items-center gap-2.5 rounded-lg bg-jolu-bg pl-2.5">
      <span className="whitespace-nowrap text-[11px] text-jolu-muted">Base URL</span>
      <code className="select-all whitespace-nowrap font-mono text-[11px] text-jolu-text">{address ?? "暂不可用"}</code>
      <button type="button" className={iconButton} aria-label="复制 Base URL" title="复制 Base URL" disabled={!address || working !== null}
        onClick={() => void act("url", () => navigator.clipboard.writeText(address!), "复制 Base URL 失败", "url")}>
        {copied === "url" ? <Check aria-hidden className="gw6-copied size-[15px] text-jolu-on" /> : <Copy aria-hidden className="size-[15px]" />}
      </button>
    </div>

    <div className="flex h-10 shrink-0 items-center gap-2 rounded-lg bg-jolu-bg pl-2.5">
      <span className="whitespace-nowrap text-[11px] text-jolu-muted">API Key</span>
      <code className="whitespace-nowrap font-mono text-[11px] text-jolu-text" aria-live="polite">
        {mask ?? (keyError ? "读取失败" : desktop ? "读取中…" : "桌面端可用")}
      </code>
      <button type="button" className={iconButton} aria-label="复制 API Key" title="复制 API Key" disabled={!mask || working !== null || !desktop}
        onClick={() => void act("key", () => gatewayApi.copyKey(), "复制 API Key 失败", "key")}>
        {copied === "key" ? <Check aria-hidden className="gw6-copied size-[15px] text-jolu-on" /> : <Copy aria-hidden className="size-[15px]" />}
      </button>
      <button type="button" className={iconButton} aria-busy={working === "refresh"}
        aria-label={keyError ? "重新读取 API Key" : "刷新 API Key"}
        title={keyError ? "重新读取 API Key" : "刷新手动接入 Key；一键接入的工具不受影响"}
        disabled={!desktop || working !== null || (!mask && !keyError)}
        onClick={() => keyError
          ? void act("refresh", async () => { setMask(await gatewayApi.accessKey(false)); setKeyError(false) }, "读取 API Key 失败")
          : setConfirmRefresh(true)}>
        <RefreshCw aria-hidden className={cn("size-[15px]", working === "refresh" && "animate-spin")} />
      </button>
    </div>

    <div className="flex shrink-0 items-center gap-3" aria-label={`来源 ${sources} · 模型 ${models} · Agent ${agents}`}>
      <Stat value={sources} label="来源" />
      <span className="h-4 w-px bg-jolu-border" />
      <Stat value={models} label="模型" />
      <span className="h-4 w-px bg-jolu-border" />
      <Stat value={agents} label="Agent" />
    </div>

    <span className="min-w-4 flex-1" />

    <label className="flex shrink-0 items-center gap-2 whitespace-nowrap text-[11px] font-semibold text-jolu-text" title="关闭后，一键接入的工具恢复原来的连接方式">
      允许接入
      <JoluSwitch on={!!running} label="允许接入" disabled={busy || !desktop || loading || !status} onChange={onToggle} />
    </label>

    {confirmRefresh && <ConfirmDialog
      title="刷新网关 API Key？"
      description="旧 Key 将立即失效。手动配置的工具需要重新填写新 Key；一键接入的工具与供应商凭据不受影响。"
      confirmLabel="确认刷新"
      danger
      onClose={() => setConfirmRefresh(false)}
      onConfirm={async () => {
        if (lock.current) return false
        lock.current = true
        setWorking("refresh")
        try {
          const next = await gatewayApi.accessKey(true)
          setMask(next)
          setKeyError(false)
          toast.success("API Key 已刷新，请重新复制到手动配置的工具")
        } finally {
          lock.current = false
          setWorking(null)
        }
      }}
    />}
  </section>
}

function Stat({ value, label }: { value: number; label: string }) {
  return <span className="flex items-baseline gap-[5px] whitespace-nowrap">
    <span className="font-mono text-[12px] font-semibold tabular-nums text-jolu-text">{value}</span>
    <span className="text-[11px] text-jolu-muted">{label}</span>
  </span>
}
