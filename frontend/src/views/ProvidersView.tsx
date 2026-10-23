import { ContentTransition, LoadingSurface } from "@/components/ui/LoadingSurface"
import { useState } from "react"
import { Blocks, Code, Hexagon, Info, Plus, UsersRound } from "lucide-react"
import { api } from "@/lib/api"
import { cn } from "@/lib/utils"
import { PROVIDERS, canConnect, connectionBadge, type ProviderEntry } from "@/lib/providerCatalog"
import { useConnections } from "@/hooks/useConnections"
import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { JoluButton } from "@/components/ui/JoluButton"
import { Tip } from "@/components/ui/Tip"
import { ConnectDialog } from "@/components/providers/ConnectDialog"
import { AccountsPanel } from "@/components/providers/AccountsPanel"
import type { PlatformId } from "@/types"

export type ProvidersTab = "connect" | "accounts"

/**
 * 供应商页（Jolu.pen 模块 B）：「供应商连接」画板 05 与「账号管理」画板 07–10 两个页签。
 */
export function ProvidersView({ initialTab = "connect" }: { initialTab?: ProvidersTab }) {
  const [tab, setTab] = useState<ProvidersTab>(initialTab)
  const conn = useConnections()
  const [connecting, setConnecting] = useState<PlatformId | null>(null)

  /**
   * 订阅连接保存：本机读取入库的连接默认暂停查询（旧界面需手动启用）。新稿里点「保存连接」即表示同意接入，
   * 所以只恢复这次新增的连接；已存在的连接保持原状态。
   */
  const saveLocal = async (platform: string, kind: "auth" | "api", name?: string) => {
    const before = new Set((await api.listConnections()).map((c) => c.id))
    const summary = await conn.readLocal(platform, kind, name)
    const added = (await api.listConnections()).filter((c) => !before.has(c.id) && c.platform === platform && c.kind === kind)
    await Promise.all(added.map((c) => api.setConnectionPaused(c.id, false)))
    if (added.length) await conn.reload()
    return summary
  }

  return (
    <div className="flex w-full flex-col gap-4">
      <div role="tablist" aria-label="供应商" className="flex h-11 gap-6 border-b border-jolu-border">
        <TabButton active={tab === "connect"} icon={<Blocks aria-hidden className="size-[17px]" />} onClick={() => setTab("connect")}>供应商连接</TabButton>
        <TabButton active={tab === "accounts"} icon={<UsersRound aria-hidden className="size-[17px]" />} onClick={() => setTab("accounts")}>账号管理</TabButton>
      </div>

      <ContentTransition identity={`${tab}-${conn.loading && !conn.connections.length}`} className="flex flex-col gap-4">
      {conn.loading && !conn.connections.length && !conn.error ? <LoadingSurface layout={tab === "connect" ? "providers" : "cards"} label="正在读取供应商连接" /> : tab === "connect" ? (
        <ConnectGrid counts={countByPlatform(conn.connections)} loading={conn.loading} error={conn.error} onConnect={setConnecting} onRetry={() => void conn.reload()} />
      ) : (
        <AccountsPanel conn={conn} onConnect={setConnecting} onBrowseProviders={() => setTab("connect")} />
      )}
      </ContentTransition>

      {connecting && (
        <ConnectDialog
          platform={connecting}
          actions={{ connections: conn.connections, readLocal: saveLocal, add: conn.add }}
          onClose={() => setConnecting(null)}
        />
      )}
    </div>
  )
}

function countByPlatform(connections: readonly { platformId: PlatformId }[]): Map<PlatformId, number> {
  const counts = new Map<PlatformId, number>()
  for (const c of connections) counts.set(c.platformId, (counts.get(c.platformId) ?? 0) + 1)
  return counts
}

function TabButton({ active, icon, onClick, children }: { active: boolean; icon: React.ReactNode; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onClick={onClick}
      className={cn(
        "-mb-px flex items-center gap-2 border-b-2 px-0.5 text-sm outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent",
        active ? "border-jolu-accent font-semibold text-jolu-text [&_svg]:text-jolu-on" : "border-transparent text-jolu-muted hover:text-jolu-text",
      )}
    >
      {icon}
      {children}
    </button>
  )
}

/** 画板 05「连接你的 AI 工具」：自定义供应商 + 九个平台，每行四张 */
function ConnectGrid({ counts, loading, error, onConnect, onRetry }: {
  counts: Map<PlatformId, number>
  loading: boolean
  error: string | null
  onConnect: (platform: PlatformId) => void
  onRetry: () => void
}) {
  return (
    <>
      <div className="flex flex-col gap-2">
        <h2 className="text-2xl font-semibold text-jolu-text">连接你的 AI 工具</h2>
        <p className="text-[13px] text-jolu-muted">选择已有供应商，或自定义服务地址，通过订阅账号或 API Key 建立连接。</p>
      </div>
      {error && (
        <div role="alert" className="flex items-center justify-between gap-3 rounded-[10px] bg-jolu-danger-soft px-4 py-2.5 text-xs text-jolu-danger">
          <span>连接列表读取失败：{error}；账号数量暂不可用。</span>
          <JoluButton size="sm" variant="danger-soft" onClick={onRetry}>重新读取</JoluButton>
        </div>
      )}
      <div className="grid grid-cols-4 gap-4">
        {PROVIDERS.map((entry) => {
          const count = loading || error ? null : counts.get(entry.id) ?? 0
          return entry.id === "custom"
            ? <CustomProviderCard key={entry.id} count={count} onConnect={() => onConnect(entry.id)} />
            : <ProviderCard key={entry.id} entry={entry} count={count} onConnect={() => onConnect(entry.id)} />
        })}
      </div>
      <p className="flex items-center gap-2.5 rounded-[10px] bg-jolu-side p-4 text-xs text-jolu-muted">
        <Info aria-hidden className="size-[17px] shrink-0" />
        支持的认证方式随供应商、账号权限与接入能力显示；仅支持订阅的账号不展示 API Key 入口。
      </p>
    </>
  )
}

function Card({ logo, badge, badgeTone, title, desc, action }: {
  logo: React.ReactNode
  badge: string
  badgeTone: "on" | "muted"
  title: string
  desc: string
  action: React.ReactNode
}) {
  return (
    <div className="flex h-[172px] min-w-0 flex-col gap-2.5 rounded-xl border border-jolu-border bg-jolu-bg p-4">
      <div className="flex h-7 items-center justify-between">
        {logo}
        <span className={cn("rounded-[5px] px-2 py-[5px] text-[11px] leading-none", badgeTone === "on" ? "bg-jolu-selected text-jolu-on" : "bg-jolu-stage text-jolu-muted")}>{badge}</span>
      </div>
      <span className="truncate text-base font-semibold text-jolu-text">{title}</span>
      <span className="truncate text-[11px] text-jolu-muted">{desc}</span>
      <span className="flex-1" />
      {action}
    </div>
  )
}

function ProviderCard({ entry, count, onConnect }: { entry: ProviderEntry; count: number | null; onConnect: () => void }) {
  const ready = canConnect(entry)
  const button = (
    <JoluButton variant="soft" className="w-full" disabled={!ready} onClick={onConnect}>
      <Plus aria-hidden />
      连接 {entry.name}
    </JoluButton>
  )
  return (
    <Card
      logo={<span className="grid size-7 place-items-center"><PlatformLogo platform={entry.id} size={22} /></span>}
      // 待接入的平台不显示账号数量，改为说明状态（待确认：Jolu.pen 未画此状态）
      badge={!ready ? "待接入" : count === null ? "读取中" : connectionBadge(count)}
      badgeTone={ready && count ? "on" : "muted"}
      title={entry.name}
      desc={entry.access}
      action={ready ? button : <Tip label={`${entry.name} 连接尚未接入`}><span className="flex">{button}</span></Tip>}
    />
  )
}

/**
 * 自定义供应商卡片（画板 05 第一张）：徽标「自定义」、按钮「添加自定义」，点击打开 API Key 连接弹窗（D-11）。
 * 已有连接时徽标改显示账号数量（待确认：画板 05 只画了未连接时的「自定义」徽标）。
 */
function CustomProviderCard({ count, onConnect }: { count: number | null; onConnect: () => void }) {
  return (
    <Card
      logo={
        <span className="relative grid size-7 place-items-center rounded-lg bg-jolu-selected text-jolu-on">
          <Hexagon aria-hidden className="absolute size-[22px]" />
          <Code aria-hidden className="size-3" />
        </span>
      }
      badge={count ? connectionBadge(count) : "自定义"}
      badgeTone="on"
      title="自定义供应商"
      desc="名称 / Base URL / API Key"
      action={
        <JoluButton variant="soft" className="w-full" onClick={onConnect}>
          <Plus aria-hidden />
          添加自定义
        </JoluButton>
      }
    />
  )
}
