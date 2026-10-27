import { invoke } from "./api"

export type CheckinState = "unknown" | "ready" | "claimed" | "inactive" | "unsupported" | "unauthorized" | "forbidden" | "rate_limited" | "failed"
export interface CheckinOutcome {
  state: CheckinState
  message: string
  reward: number | null
  unit: string
  period_key: string | null
  /** Unix 秒；未知则为空，以服务端状态为准。 */
  reset_at: number | null
  server_time?: number | null
  official_statistics?: OfficialCheckinStatistics | null
}
export interface OfficialCheckinStatistics {
  unit: string
  scope: string
  week: { days: number | null; reward: number | null } | null
  month: { days: number | null; reward: number | null } | null
  all: { days: number | null; reward: number | null } | null
  note: string
}
export interface CheckinAccountDto {
  id: string
  platform: string
  edition: string | null
  name: string
  /** 官方账号显示值，与用户旧连接名称分开。 */
  account?: string | null
  connection_status: string
}
export interface CheckinPeriodStats {
  days: number
  reward: number | null
}
export interface CheckinStatistics {
  has_records: boolean
  unit: string
  week: CheckinPeriodStats
  month: CheckinPeriodStats
  all: CheckinPeriodStats
}
export interface CheckinUpdate {
  id: string
  account_key: string | null
  account?: string | null
  outcome: CheckinOutcome
  checked_at: number
  statistics: CheckinStatistics | null
}
export interface CheckinRecordDto {
  id: number
  connection_id: string
  name: string
  platform: string
  edition: string
  outcome: CheckinOutcome
  created_ms: number
}
export interface CheckinOverview {
  accounts: CheckinAccountDto[]
  records: CheckinRecordDto[]
}
export const checkinApi = {
  overview: () => invoke<CheckinOverview>("daily_checkin_overview"),
  status: (id: string) => invoke<CheckinUpdate>("daily_checkin_status", { id }),
  claim: (id: string, expectedAccountKey: string) => invoke<CheckinUpdate>("daily_checkin_claim", { id, expectedAccountKey }),
}
