import { PlatformLogo } from "@/components/brand/PlatformLogo"
import { api, isTauri } from "@/lib/api"
import { agentLabel } from "@/lib/gatewayPage"
import { formatDuration, rejectReason, type ActiveRelay, type ActivityState } from "@/lib/gatewayActivity"
import type { GatewayStatusDto, WiringStatusDto } from "@/lib/gatewayApi"
import { cn } from "@/lib/utils"
import type { PlatformId } from "@/types"

/**
 * 胶囊灵动岛的模型入口部分（Jolu.pen A3-7 / A3-8，设计说明 §8.2）。
 * 只在转发模式且监听已启动时出现；观测模式或关闭时一个元素都不画。
 */

/** Agent → 平台标识；没有官方标识的用首字母 */
const AGENT_LOGO: Record<string, PlatformId> = {
  "claude-code": "claude", codex: "codex", gemini: "gemini", opencode: "opencode", zcode: "zcode",
}

export function AgentAvatar({ agent, size, className, dot }: { agent: string | null; size: number; className?: string; dot?: boolean }) {
  const platform = agent ? AGENT_LOGO[agent] : undefined
  return (
    <span className={cn("cap-av", className)} title={agent ? agentLabel(agent) : "未知 Agent"}>
      {platform ? <PlatformLogo platform={platform} size={size} /> : <span className="cap-av-letter" style={{ fontSize: size * 0.75 }}>{agent ? agent.charAt(0).toUpperCase() : "?"}</span>}
      {dot && <i className="cap-av-dot" />}
    </span>
  )
}

/** 已接线的 Agent，按最近一次经网关的时间排序 */
export function wiredAgents(status: GatewayStatusDto | null, activity: ActivityState): WiringStatusDto[] {
  if (!status) return []
  return status.wiring
    .filter((w) => w.wired || w.drift)
    .sort((a, b) => (activity.lastByAgent[b.agent]?.ts ?? 0) - (activity.lastByAgent[a.agent]?.ts ?? 0))
}

/** 胶囊收缩态右侧的头像叠放：最多 3 个，超出 +N；正在转发的描青圈，刚成功的留绿点 */
export function AgentStack({ agents, activity, now }: { agents: WiringStatusDto[]; activity: ActivityState; now: number }) {
  if (agents.length === 0) return null
  const highlight = activity.shown?.phase === "in" ? activity.shown.agent : null
  const okAgent = activity.shown?.phase === "ok" ? activity.shown.agent : null
  const shown = agents.slice(0, 3)
  return (
    <span className="cap-stack" aria-label={`已接线 ${agents.length} 个 Agent`}>
      {shown.map((w) => {
        const last = activity.lastByAgent[w.agent]
        const recentOk = w.agent === okAgent || Boolean(last?.ok && now - last.ts < 10_000)
        return <AgentAvatar key={w.agent} agent={w.agent} size={11} className={cn(w.agent === highlight && "hl")} dot={recentOk} />
      })}
      {agents.length > 3 && <span className="cap-av more">+{agents.length - 3}</span>}
    </span>
  )
}

/** 胶囊收缩态的转发路线：Agent → 来源/模型 · 状态 */
export function RelayRoute({ relay }: { relay: ActiveRelay }) {
  const state = relay.phase === "in" ? <span className="cap-c text-xs font-semibold">转发中</span>
    : relay.phase === "ok" ? <span className="cap-g cap-num text-xs font-semibold">✓ {formatDuration(relay.durationMs)}</span>
    : <span className="cap-o text-xs font-semibold">{rejectReason(relay.statusCode, relay.phase)}</span>
  return (
    <>
      <span className="cap-route">
        <AgentAvatar agent={relay.agent} size={13} className="cap-av-bare" />
        {relay.agent ? agentLabel(relay.agent) : "未知 Agent"}
        <span className="cap-arr">→</span>
        <span className="cap-model">{relay.source ? <s>{relay.source}/</s> : null}{relay.model}</span>
      </span>
      {state}
    </>
  )
}

/** 展开态「模型入口」段 */
export function GatewaySection({ status, activity, agents, now }: {
  status: GatewayStatusDto
  activity: ActivityState
  agents: WiringStatusDto[]
  now: number
}) {
  const inFlight = new Map(activity.inFlight.map((r) => [r.agent ?? "", r]))
  const total = activity.today.ok + activity.today.rejected + activity.today.failed
  const openGateway = () => { if (isTauri) void api.openMainSection("gateway").catch(() => {}) }
  return (
    <div className="cap-sec cap-col">
      <div className="cap-sh">
        <span className="cap-gwt"><i />模型入口 · 转发中</span>
        <span className="cap-port cap-num">127.0.0.1:{status.port}</span>
      </div>
      {agents.length === 0 ? (
        <p className="cap-note">还没有接线的 Agent。在主窗口「模型入口」里把工具接到网关后，这里会显示每次转发的去向。</p>
      ) : (
        <div className="cap-ag-list">
          {agents.map((w) => {
            const active = inFlight.get(w.agent)
            const last = activity.lastByAgent[w.agent]
            const drift = Boolean(w.drift)
            return (
              <div key={w.agent} className={cn("cap-ag", active && "on")}>
                <AgentAvatar agent={w.agent} size={14} className="cap-avl" />
                <span className="cap-ag-t">
                  <span className="cap-ag-n">{agentLabel(w.agent)}</span>
                  <span className="cap-ag-s">
                    {active ? <>正在用 <span className="cap-m"><s>{active.source}/</s>{active.model}</span></>
                      : drift ? <span className="cap-o">接线配置已被改动</span>
                      : last ? <>最近一次 <span className="cap-m">{last.source ? <s>{last.source}/</s> : null}{last.model}</span></>
                      : "今天还没有经过网关"}
                  </span>
                </span>
                <span className="cap-ag-r">
                  {active ? <span className="cap-state cap-c">转发中</span>
                    : drift ? <span className="cap-state cap-o">需处理</span>
                    : <span className="cap-state cap-g">已接线</span>}
                  {!active && !drift && last && (
                    <span className="cap-meta">{ago(last.ts, now)} · {last.ok ? formatDuration(last.durationMs) : <span className="cap-o">{rejectReason(last.statusCode, "rejected")}</span>}</span>
                  )}
                  {drift && <button type="button" className="cap-fix" onClick={openGateway} onDoubleClick={(e) => e.stopPropagation()}>修复</button>}
                </span>
                {active && <span className="cap-flow" />}
              </div>
            )
          })}
        </div>
      )}
      <div className="cap-sum">
        <div><span>今日经网关</span><b className="cap-num">{total} 次</b></div>
        <div><span>成功</span><b className="cap-num cap-g">{activity.today.ok}</b></div>
        <div><span>被拒</span><b className="cap-num cap-o">{activity.today.rejected}</b></div>
        {activity.today.failed > 0 && <div><span>失败</span><b className="cap-num cap-r">{activity.today.failed}</b></div>}
      </div>
      <div className="cap-lnk">
        <span>被拒不会自动换来源</span>
        <button type="button" onClick={openGateway} onDoubleClick={(e) => e.stopPropagation()}>打开模型入口 ›</button>
      </div>
    </div>
  )
}

function ago(ts: number, now: number) {
  const minutes = Math.round((now - ts) / 60_000)
  if (minutes < 1) return "刚刚"
  if (minutes < 60) return `${minutes} 分钟前`
  const hours = Math.round(minutes / 60)
  return hours < 24 ? `${hours} 小时前` : `${Math.round(hours / 24)} 天前`
}
