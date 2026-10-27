/**
 * 仅供浏览器预览的一键签到示例数据（对应 Jolu.pen 画板 149–150 的示例）。
 * 桌面端读取失败时不得回落到这里。
 */
import type { CheckinOutcome, CheckinOverview, CheckinRecordDto, CheckinStatistics, CheckinUpdate } from "@/lib/checkinApi"

const DAY = 86_400_000

const ACCOUNTS = [
  { id: "demo-workbuddy-cn", platform: "workbuddy", edition: "cn", name: "WorkBuddy", account: "user@example.com", state: "claimed", reward: 100, unit: "积分", week: 3, month: 12, all: 26 },
  { id: "demo-qoder-cn", platform: "qoder", edition: "cn", name: "Qoder", account: "momo@example.com", state: "ready", reward: 100, unit: "Credits", week: 2, month: 9, all: 18 },
  { id: "demo-trae-cn", platform: "trae", edition: "cn", name: "Trae", account: "个人账号", state: "ready", reward: 50, unit: "积分", week: 4, month: 15, all: 41 },
  { id: "demo-qoder-global", platform: "qoder", edition: "global", name: "Qoder", account: "alex@example.com", state: "claimed", reward: 100, unit: "Credits", week: 5, month: 20, all: 33 },
] as const

type DemoAccount = (typeof ACCOUNTS)[number]
const claimedIds = new Set<string>(ACCOUNTS.filter(a => a.state === "claimed").map(a => a.id))

function outcome(a: DemoAccount, claimed: boolean): CheckinOutcome {
  return { state: claimed ? "claimed" : "ready", message: claimed ? "今日奖励已领取" : "今日奖励可领取", reward: a.reward, unit: a.unit, period_key: null, reset_at: null }
}

function statistics(a: DemoAccount, claimed: boolean): CheckinStatistics {
  const extra = claimed && a.state === "ready" ? 1 : 0
  const days = (n: number) => ({ days: n + extra, reward: (n + extra) * a.reward })
  return { has_records: true, unit: a.unit, week: days(a.week), month: days(a.month), all: days(a.all) }
}

function update(a: DemoAccount): CheckinUpdate {
  const claimed = claimedIds.has(a.id)
  return { id: a.id, account_key: `demo:${a.id}`, account: a.account, outcome: outcome(a, claimed), checked_at: Date.now(), statistics: statistics(a, claimed) }
}

function records(): CheckinRecordDto[] {
  const now = Date.now()
  const rows = ACCOUNTS.filter(a => claimedIds.has(a.id)).map((a, i) => ({ a, at: now - (i + 1) * 47 * 60_000 }))
  const trae = ACCOUNTS[2]
  rows.push({ a: trae, at: now - DAY })
  return rows.map(({ a, at }, i) => ({ id: i + 1, connection_id: a.id, name: a.account, platform: a.platform, edition: a.edition, outcome: outcome(a, true), created_ms: at }))
}

function find(id: string): DemoAccount {
  const account = ACCOUNTS.find(a => a.id === id)
  if (!account) throw new Error("示例账号不存在")
  return account
}

export const demoCheckinApi = {
  overview: async (): Promise<CheckinOverview> => ({
    accounts: ACCOUNTS.map(a => ({ id: a.id, platform: a.platform, edition: a.edition, name: a.name, account: a.account, connection_status: "active" })),
    records: records(),
  }),
  status: async (id: string): Promise<CheckinUpdate> => update(find(id)),
  claim: async (id: string): Promise<CheckinUpdate> => {
    claimedIds.add(find(id).id)
    return update(find(id))
  },
}
