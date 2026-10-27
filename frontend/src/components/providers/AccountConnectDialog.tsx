import { api } from "@/lib/api"
import type { ProviderId } from "@/lib/providerCatalog"
import { useConnections } from "@/hooks/useConnections"
import { ConnectDialog } from "./ConnectDialog"

/** 所有入口复用同一连接表单和保存规则；保存后刷新发起操作的页面。 */
export function AccountConnectDialog({ platform, onClose, onSaved }: {
  platform: ProviderId
  onClose: () => void
  onSaved?: () => void
}) {
  const conn = useConnections()
  const readLocal = async (provider: string, kind: "auth" | "api", name?: string, edition?: string) => {
    const before = new Set((await api.listConnections()).map(c => c.id))
    const summary = await conn.readLocal(provider, kind, name, edition)
    const added = (await api.listConnections()).filter(c => !before.has(c.id) && c.platform === provider && c.kind === kind && (!edition || c.edition === edition))
    await Promise.all(added.map(c => api.setConnectionPaused(c.id, false)))
    if (added.length) await conn.reload()
    return summary
  }
  return <ConnectDialog platform={platform} actions={{ connections: conn.connections, readLocal, add: conn.add }} onClose={onClose} onSaved={onSaved} />
}
