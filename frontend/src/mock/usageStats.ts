/**
 * 用量统计页的浏览器预览示例（取自 Jolu.pen 画板 30 / 32 / 34 的示例数值）。
 * 只在没有 Tauri 的 `pnpm dev` 预览里使用；桌面端查询失败时不回落到这里。
 */
import type { StatsActivityDto, StatsCallPageDto, StatsCallRowDto, StatsOverviewDto } from "@/lib/api"
import { addDays, dateKey } from "@/lib/usageStats"

const series = [
  [120, 80, 140, 30], [90, 60, 120, 20], [60, 40, 80, 10], [150, 90, 200, 40],
  [420, 260, 520, 90], [610, 380, 700, 120], [890, 500, 850, 140],
]

export function demoOverview(start: number): StatsOverviewDto {
  return {
    tokens: { fresh_input: 4_620_000, output: 2_240_000, cache_read: 4_940_000, cache_write: 680_000, total: 12_480_000 },
    requests: 846,
    failed: 12,
    cache_hit_rate: 0.482,
    cost: { amount: 18.42, priced_requests: 810, requests: 846 },
    previous_total: 10_523_000,
    trend: {
      bucket: "2h",
      points: series.map(([i, o, r, w], index) => ({
        start: start + index * 7_200_000,
        end: start + (index + 1) * 7_200_000,
        label: `${String(index * 2).padStart(2, "0")}:00`,
        fresh_input: i * 1000, output: o * 1000, cache_read: r * 1000, cache_write: w * 1000,
        total: (i + o + r + w) * 1000,
        requests: Math.round((i + o) / 20),
        cost: (i + o) / 400,
      })),
    },
    models: [
      { platform: "claude", model: "claude-sonnet-4-5", requests: 400, tokens: 6_200_000, cache_hit_rate: 0.521, avg_duration_ms: 1600, cost: 8.8 },
      { platform: "codex", model: "gpt-5", requests: 126, tokens: 2_160_000, cache_hit_rate: 0.43, avg_duration_ms: 3200, cost: 4 },
      { platform: "gemini", model: "gemini-2.5-flash", requests: 200, tokens: 2_120_000, cache_hit_rate: 0.452, avg_duration_ms: 1400, cost: 2.72 },
      { platform: "claude", model: "claude-haiku-4-5", requests: 80, tokens: 1_200_000, cache_hit_rate: 0.4, avg_duration_ms: 900, cost: 1.8 },
      { platform: "codex", model: "gpt-5-mini", requests: 40, tokens: 800_000, cache_hit_rate: 0.385, avg_duration_ms: null, cost: null },
    ],
    providers: [
      { platform: "claude", requests: 420, tokens: 6_800_000, cost: { amount: 10, priced_requests: 410, requests: 420 } },
      { platform: "codex", requests: 290, tokens: 3_760_000, cost: { amount: 4, priced_requests: 270, requests: 290 } },
      { platform: "gemini", requests: 86, tokens: 1_120_000, cost: { amount: 2.72, priced_requests: 86, requests: 86 } },
      { platform: "grok", requests: 30, tokens: 480_000, cost: { amount: 0, priced_requests: 0, requests: 30 } },
      { platform: "opencode", requests: 20, tokens: 320_000, cost: { amount: 1.1, priced_requests: 14, requests: 20 } },
    ],
    attributed: 778,
    sessions: [
      { platform: "claude", session_id: "demo-1", title: "排查缓存 Token 统计", tokens: 2_860_000, cwd: "D:/develop/jolu" },
      { platform: "codex", session_id: "demo-2", title: "优化 Jolu 的会话界面", tokens: 1_920_000, cwd: "D:/develop/jolu" },
      { platform: "gemini", session_id: "demo-3", title: "梳理登录引导文案", tokens: 680_000, cwd: null },
      { platform: "codex", session_id: "demo-4", title: "接入模型列表接口", tokens: 520_000, cwd: "D:/work/api-gateway" },
      { platform: "claude", session_id: "demo-5", title: null, tokens: 410_000, cwd: null },
    ],
  }
}

export function demoActivity(now: Date): StatsActivityDto {
  const last = dateKey(now)
  const yearAgo = addDays(last, -364)
  const offset = (new Date(`${yearAgo}T00:00:00`).getDay() + 6) % 7
  const first = addDays(yearAgo, -offset)
  const days: StatsActivityDto["days"] = []
  // 最近三个月有规律的示例用量，更早为空
  for (let i = 0; i < 92; i++) {
    const tokens = [0, 120_000, 480_000, 1_200_000, 2_600_000][(i * 7 + 3) % 5]
    if (tokens) days.push({ date: addDays(last, -i), tokens, requests: Math.round(tokens / 15_000) })
  }
  days.reverse()
  return { first_date: first, last_date: last, days, today_active_minutes: 169, week_active_days: 3, month_active_days: 14 }
}

const DEMO_CALLS: Omit<StatsCallRowDto, "id" | "ts">[] = [
  { call_id: "call_demo_0138", platform: "claude", model: "claude-sonnet-4-5", session_id: "demo-1", fresh_input: 8_400, cache_read: 12_000, cache_write: 0, output: 1_200, total: 21_600, cost: 0.026, effort: "high", duration_ms: 1600, first_token_ms: 400, status_code: null, failed: false },
  { call_id: "call_demo_0137", platform: "codex", model: "gpt-5", session_id: "demo-2", fresh_input: 12_000, cache_read: 18_000, cache_write: null, output: 4_200, total: 34_200, cost: 0.082, effort: "high", duration_ms: 3200, first_token_ms: 800, status_code: 200, failed: false },
  { call_id: "call_demo_0136", platform: "codex", model: "gpt-5-mini", session_id: "demo-4", fresh_input: 3_000, cache_read: 4_800, cache_write: null, output: 600, total: 8_400, cost: 0.009, effort: null, duration_ms: 800, first_token_ms: 200, status_code: 200, failed: false },
  { call_id: "call_demo_0135", platform: "gemini", model: "gemini-2.5-pro", session_id: "demo-3", fresh_input: 5_200, cache_read: 6_400, cache_write: null, output: 1_200, total: 12_800, cost: null, effort: "medium", duration_ms: 2100, first_token_ms: 500, status_code: null, failed: false },
  { call_id: "call_demo_0134", platform: "grok", model: "grok-4", session_id: null, fresh_input: null, cache_read: null, cache_write: null, output: null, total: null, cost: null, effort: null, duration_ms: 400, first_token_ms: null, status_code: 429, failed: true },
  { call_id: "call_demo_0133", platform: "claude", model: "claude-haiku-4-5", session_id: "demo-5", fresh_input: 6_000, cache_read: 9_200, cache_write: 0, output: 1_200, total: 16_400, cost: 0.02, effort: "medium", duration_ms: 1500, first_token_ms: 300, status_code: null, failed: false },
  { call_id: "call_demo_0132", platform: "claude", model: "claude-sonnet-4-5", session_id: "demo-1", fresh_input: 7_500, cache_read: 9_600, cache_write: 0, output: 1_800, total: 18_900, cost: 0.041, effort: "high", duration_ms: 2600, first_token_ms: 600, status_code: null, failed: false },
  { call_id: "call_demo_0131", platform: "codex", model: "gpt-5-mini", session_id: "demo-4", fresh_input: 2_400, cache_read: 4_000, cache_write: null, output: 400, total: 6_800, cost: 0.008, effort: "low", duration_ms: 900, first_token_ms: 200, status_code: 200, failed: false },
  { call_id: "call_demo_0130", platform: "gemini", model: "gemini-2.5-flash", session_id: "demo-3", fresh_input: 4_000, cache_read: 5_400, cache_write: null, output: 800, total: 10_200, cost: 0.012, effort: null, duration_ms: 1100, first_token_ms: 300, status_code: null, failed: false },
  { call_id: "call_demo_0129", platform: "codex", model: "gpt-5", session_id: null, fresh_input: null, cache_read: null, cache_write: null, output: null, total: null, cost: null, effort: null, duration_ms: 30_000, first_token_ms: null, status_code: 504, failed: true },
]

export function demoCalls(end: number, page: number): StatsCallPageDto {
  return {
    rows: DEMO_CALLS.map((row, index) => ({ ...row, id: 1000 - index, ts: end - index * 66_000 - (page - 1) * 660_000 })),
    total_count: 846,
    unpriced_count: 36,
    page,
    page_size: 10,
    page_count: 85,
    models: ["claude-haiku-4-5", "claude-sonnet-4-5", "gemini-2.5-flash", "gemini-2.5-pro", "gpt-5", "gpt-5-mini", "grok-4"],
  }
}
