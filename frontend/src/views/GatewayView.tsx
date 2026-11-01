import { useCallback, useEffect, useState } from "react"
import { Cable, LayoutDashboard, List, ShieldAlert } from "lucide-react"
import { cn } from "@/lib/utils"
import { isTauri } from "@/lib/api"
import { gatewayApi, type GatewayMode, type GatewayStatusDto, type WiringStatusDto } from "@/lib/gatewayApi"
import {
  agentLabel, canEscape, canServe, circuitText, gatewayRunLine, nextMode, wiredCount, wiringLine,
} from "@/lib/gatewayPage"
import { useConnections } from "@/hooks/useConnections"
import { useToast } from "@/components/ui/Toast"
import { JoluSwitch } from "@/components/ui/JoluSwitch"
import { StatusDot } from "@/components/ui/primitives"

export type GatewayTab = "overview" | "detail" | "wiring"

const TABS: { value: GatewayTab; label: string; icon: typeof LayoutDashboard }[] = [
  { value: "overview", label: "总览", icon: LayoutDashboard },
  { value: "detail", label: "连接详情", icon: List },
  { value: "wiring", label: "工具接线", icon: Cable },
]

/**
 * 账号汇聚网关（Jolu.pen 模块 J，画板 161–166）。
 *
 * 总览 / 连接详情 / 工具接线三页签。凭证永不出现在本页：连接列表来自 connections 的 DTO，
 * 只有掩码；网关状态来自 `gateway_status`，同样不含凭证。
 */
export function GatewayView({ tab, onTab }: { tab: GatewayTab; onTab: (tab: GatewayTab) => void }) {
  const toast = useToast()
  const { connections } = useConnections()
  const [status, setStatus] = useState<GatewayStatusDto | null>(null)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)

  const refresh = useCallback(async () => {
    if (!isTauri) {
      setStatus(null)
      setLoading(false)
      return
    }
    try {
      setStatus(await gatewayApi.status())
    } catch (error) {
      toast.danger("读取网关状态失败", String(error))
    } finally {
      setLoading(false)
    }
  }, [toast])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const run = async (work: () => Promise<unknown>, ok: string) => {
    if (busy) return
    setBusy(true)
    try {
      await work()
      toast.success(ok)
      await refresh()
    } catch (error) {
      toast.danger("操作失败", String(error))
    } finally {
      setBusy(false)
    }
  }

  const mode = status?.mode === "relay" ? "relay" : "observe"
  const runLine = gatewayRunLine(status)

  return (
    <div className="flex h-full flex-col">
      <GatewayTabs active={tab} wired={wiredCount(status)} onTab={onTab} />
      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pt-4">
        <div className="flex flex-col gap-1">
          <h1 className="text-[22px] font-semibold text-jolu-text">账号汇聚网关</h1>
          <p className="text-xs text-jolu-muted">
            把本机已登录的订阅与 API Key 汇聚成统一模型源，供各 AI 工具调用。
          </p>
        </div>

        {!isTauri && (
          <Notice tone="muted">浏览器预览不读取网关状态；请在桌面端查看真实数据。</Notice>
        )}

        {tab === "overview" && (
          <>
            <Card title="网关状态" desc={circuitText(status)} icon={<StatusDot tone={runLine.tone === "on" ? "success" : runLine.tone === "danger" ? "danger" : "neutral"} />}>
              <Row label={runLine.text} hint={status ? `端口 ${status.port} · 模型池见「连接详情」` : undefined}>
                <ModeSwitch
                  value={mode}
                  disabled={busy || loading || !isTauri}
                  onChange={(next) =>
                    run(() => gatewayApi.setMode(next, status?.default_connection ?? null), next === "relay" ? "已切换到网关模式" : "已切回观测模式")
                  }
                />
              </Row>
            </Card>

            <Card title="模型池" desc={`${connections.length} 条连接可加入；开启后出现在各工具的模型选择器里`} icon={<StatusDot tone="neutral" />}>
              {connections.length === 0 ? (
                <Empty>还没有连接。先在「供应商」页建立连接，再回到这里开启对外服务。</Empty>
              ) : (
                connections.map((c) => (
                  <Row key={c.id} label={c.name} hint={serveHint(c.platformId, c.kind, c.masked, c.serve_enabled)}>
                    <JoluSwitch
                      on={c.serve_enabled}
                      label={`${c.name} 对外服务`}
                      disabled={busy || !isTauri || !canServe(c.platformId, c.kind)}
                      onChange={(next) =>
                        run(() => gatewayApi.setServe(c.id, next), next ? `已把 ${c.name} 加入模型池` : `已移出 ${c.name}`)
                      }
                    />
                  </Row>
                ))
              )}
            </Card>

            <Card
              title="紧急切回直连"
              desc="还原所有工具配置并停止网关；连续失败达阈值也会自动触发。"
              icon={<ShieldAlert className="h-4 w-4 text-jolu-danger" />}
              danger
            >
              <Row
                label={status ? `当前状态：${status.enabled ? "运行中" : "已停止"}` : "读取中…"}
                hint="切回后各工具恢复原有直连方式，本页设置会保留。"
              >
                <button
                  type="button"
                  disabled={busy || !canEscape(status) || !isTauri}
                  onClick={() => run(() => gatewayApi.escape(), "已切回直连")}
                  className="h-8 rounded-lg bg-jolu-danger px-3.5 text-xs text-white disabled:opacity-40"
                >
                  切回直连
                </button>
              </Row>
            </Card>
          </>
        )}

        {tab === "detail" && (
          <Card title="连接详情" desc="每条连接的协议与掩码凭证；凭证原值只在后端使用">
            {connections.length === 0 ? (
              <Empty>还没有连接。</Empty>
            ) : (
              connections.map((c) => (
                <Row
                  key={c.id}
                  label={c.name}
                  hint={[
                    c.kind === "api" ? "API Key" : "官方订阅",
                    c.protocol ?? (c.platformId === "claude" ? "anthropic" : "openai"),
                    c.baseUrl ?? "官方端点",
                    c.masked,
                  ].join(" · ")}
                >
                  <span className={cn("text-xs", c.serve_enabled ? "text-jolu-on" : "text-jolu-muted")}>
                    {c.serve_enabled ? "对外服务中" : "未开启"}
                  </span>
                </Row>
              ))
            )}
          </Card>
        )}

        {tab === "wiring" && (
          <Card title="工具接线" desc="接入后该工具的新会话请求本机网关，配置在断开时按原值还原。">
            {!status ? (
              <Empty>{loading ? "读取中…" : "无法读取接线状态"}</Empty>
            ) : (
              status.wiring.map((entry) => (
                <WiringRow
                  key={entry.agent}
                  entry={entry}
                  busy={busy || !isTauri}
                  onToggle={(wired) =>
                    run(
                      () => gatewayApi.setWiring(entry.agent, wired),
                      wired ? `已接入 ${agentLabel(entry.agent)}` : `已断开 ${agentLabel(entry.agent)}`,
                    )
                  }
                />
              ))
            )}
          </Card>
        )}
      </div>
    </div>
  )
}

function serveHint(platform: string, kind: string, masked: string, serving: boolean): string {
  const kindText = kind === "api" ? "API Key" : "官方订阅"
  if (!canServe(platform, kind)) return `${kindText} · ${platform} 的订阅暂不支持对外服务`
  return `${kindText} · ${masked} · ${serving ? "对外服务中" : "未开启"}`
}

function GatewayTabs({ active, wired, onTab }: { active: GatewayTab; wired: number; onTab: (tab: GatewayTab) => void }) {
  return (
    <div className="flex h-11 items-center gap-6 border-b border-jolu-border">
      {TABS.map((tab) => {
        const on = tab.value === active
        const Icon = tab.icon
        return (
          <button
            key={tab.value}
            type="button"
            onClick={() => onTab(tab.value)}
            className={cn(
              "flex h-11 items-center gap-2 border-b-2 text-[13px] transition-colors",
              on ? "border-jolu-accent text-jolu-text" : "border-transparent text-jolu-muted hover:text-jolu-text",
            )}
          >
            <Icon className={cn("h-4 w-4", on ? "text-jolu-on" : "text-jolu-muted")} />
            {tab.label}
            {tab.value === "wiring" && wired > 0 && (
              <span className="rounded-full bg-jolu-selected px-1.5 py-0.5 text-[10px] text-jolu-on">{wired}</span>
            )}
          </button>
        )
      })}
    </div>
  )
}

function Card({
  title, desc, icon, danger, children,
}: {
  title: string
  desc?: string
  icon?: React.ReactNode
  danger?: boolean
  children: React.ReactNode
}) {
  return (
    <section
      className={cn(
        "overflow-hidden rounded-xl border",
        danger ? "border-jolu-danger bg-jolu-danger-soft" : "border-jolu-border bg-jolu-bg",
      )}
    >
      <header className="flex h-16 items-center gap-2.5 px-4">
        {icon}
        <div className="flex min-w-0 flex-col gap-0.5">
          <h2 className="text-sm font-semibold text-jolu-text">{title}</h2>
          {desc && <p className="truncate text-[11px] text-jolu-muted">{desc}</p>}
        </div>
      </header>
      <div className="flex flex-col">{children}</div>
    </section>
  )
}

function Row({ label, hint, children }: { label: React.ReactNode; hint?: string; children?: React.ReactNode }) {
  return (
    <div className="flex min-h-14 items-center gap-3 border-t border-jolu-border px-4 py-2">
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="truncate text-[13px] text-jolu-text">{label}</span>
        {hint && <span className="truncate text-[11px] text-jolu-muted">{hint}</span>}
      </div>
      {children}
    </div>
  )
}

function WiringRow({ entry, busy, onToggle }: { entry: WiringStatusDto; busy: boolean; onToggle: (wired: boolean) => void }) {
  const line = wiringLine(entry)
  return (
    <div className="flex min-h-14 items-center gap-3 border-t border-jolu-border px-4 py-2">
      <StatusDot tone={entry.wired ? "success" : "neutral"} />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-[13px] text-jolu-text">{agentLabel(entry.agent)}</span>
        <span className={cn("truncate text-[11px]", line.tone === "on" ? "text-jolu-on" : "text-jolu-muted")}>
          {line.text}
          {entry.notice && entry.wired ? ` · ${entry.notice}` : ""}
        </span>
        {entry.config_path && <span className="truncate text-[10px] text-jolu-muted">{entry.config_path}</span>}
      </div>
      <button
        type="button"
        disabled={busy || (!entry.wired && !!entry.reason && !entry.config_path)}
        onClick={() => onToggle(!entry.wired)}
        className={cn(
          "h-8 rounded-lg border px-3 text-xs",
          entry.wired
            ? "border-jolu-border text-jolu-muted"
            : "border-transparent bg-jolu-selected text-jolu-on",
          "disabled:opacity-40",
        )}
      >
        {entry.wired ? "断开" : "接线"}
      </button>
    </div>
  )
}

function Notice({ tone, children }: { tone: "muted" | "danger"; children: React.ReactNode }) {
  return (
    <div
      className={cn(
        "rounded-lg px-3 py-2 text-[11px]",
        tone === "danger" ? "bg-jolu-status-error-bg text-jolu-danger" : "bg-jolu-side text-jolu-muted",
      )}
    >
      {children}
    </div>
  )
}

function Empty({ children }: { children: React.ReactNode }) {
  return <div className="border-t border-jolu-border px-4 py-6 text-center text-xs text-jolu-muted">{children}</div>
}

function ModeSwitch({ value, disabled, onChange }: { value: GatewayMode; disabled: boolean; onChange: (mode: GatewayMode) => void }) {
  return (
    <div className="flex items-center gap-1 rounded-lg bg-jolu-track p-0.5">
      {(["observe", "relay"] as const).map((mode) => (
        <button
          key={mode}
          type="button"
          disabled={disabled}
          onClick={() => value !== mode && onChange(mode)}
          className={cn(
            "h-7 rounded-md px-3 text-xs transition-colors",
            value === mode ? "bg-jolu-bg text-jolu-text shadow-sm" : "text-jolu-muted",
            "disabled:opacity-40",
          )}
        >
          {mode === "observe" ? "观测" : "网关"}
        </button>
      ))}
    </div>
  )
}

export type { GatewayMode }
export { nextMode }
