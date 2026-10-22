/**
 * 会话线程页的浏览器预览示例（取自 Jolu.pen 画板 40 / 42 的示例内容）。
 * 只在没有 Tauri 的 `pnpm dev` 预览里使用；桌面端读取失败时不回落到这里。
 */
import type { MessagePageDto, SessionItemDto, SessionListDto } from "@/lib/api"

const HOUR = 3_600_000

function make(now: number): SessionItemDto[] {
  const today = new Date(now)
  const at = (daysAgo: number, h: number, m: number) => new Date(today.getFullYear(), today.getMonth(), today.getDate() - daysAgo, h, m).getTime()
  const row = (platform: string, id: string, title: string | null, cwd: string | null, last: number, status: SessionItemDto["status"], tokens: number[]): SessionItemDto => ({
    platform, session_id: id, title, cwd, first_ts: last - 0.4 * HOUR, last_ts: last, requests: 12,
    fresh_input: tokens[0], cache_read: tokens[1], cache_write: tokens[2], output: tokens[3],
    connection_id: null, status, has_source: status !== "usage_only",
  })
  return [
    row("codex", "session_demo_71fa0c1d2c08", "优化 Jolu 的会话界面", "D:/develop/jolu", at(0, 12, 38), "done", [12_400, 8_600, 0, 5_200]),
    row("claude", "session_demo_82ac5e1b91f0", "排查缓存 Token 统计", "D:/develop/jolu", at(0, 12, 26), "running", [9_000, 21_000, 1_400, 4_100]),
    row("codex", "session_demo_3c11d0a7e5b2", "实现日期范围筛选", "D:/develop/jolu", at(1, 16, 0), "done", [6_000, 4_000, 0, 2_000]),
    row("claude", "session_demo_a92e33b4c710", "整理 Skills 管理方案", "D:/develop/jolu", at(1, 10, 20), "done", [3_200, 7_400, 600, 1_900]),
    row("claude", "session_demo_5d7b81c2f0aa", "缓存命中率统计口径", "D:/develop/jolu", at(1, 17, 44), "done", [12_400, 8_600, 1_400, 5_200]),
    row("codex", "session_demo_0be4f6a8d913", "调整供应商卡片布局", "D:/develop/jolu", at(2, 9, 10), "done", [4_100, 2_200, 0, 1_300]),
    row("gemini", "session_demo_6f2d0e9a4b18", "梳理登录引导文案", "D:/develop/jolu-docs", at(0, 11, 40), "recent", [2_400, 0, 0, 900]),
    row("claude", "session_demo_c3a9b7e15d42", "会话缓存数据补全", "D:/develop/jolu-docs", at(2, 15, 0), "usage_only", [8_800, 3_200, 400, 2_100]),
    row("codex", "session_demo_e81f2c6d0a77", "补充安装步骤", "D:/develop/jolu-docs", at(3, 14, 0), "done", [1_800, 900, 0, 700]),
    row("claude", "session_demo_19ab7c3e6f05", null, null, at(5, 20, 0), "done", [700, 300, 0, 200]),
  ]
}

export function demoSessions(now: number): SessionListDto {
  return { sessions: make(now), files: 9 }
}

export function demoMessages(platform: string, sessionId: string): MessagePageDto {
  const usageOnly = sessionId === "session_demo_c3a9b7e15d42" || platform === "gemini"
  const base = Date.now() - HOUR
  return {
    available: !usageOnly,
    total: usageOnly ? 0 : 24,
    offset: usageOnly ? 0 : 22,
    messages: usageOnly ? [] : [
      { role: "user", ts: base, text: "把会话列表和详情放在同一页，切换会话时不要丢失筛选条件。", tools: [] },
      {
        role: "assistant",
        ts: base + 360_000,
        text: "已整理为左右分栏：左侧保留筛选和会话列表，右侧展示当前会话。切换会话时保留列表位置。\n\n• 按供应商、项目和时间快速定位\n• 账号无法确认归属时单独标注\n• 从详情返回原工具继续会话",
        tools: [{ name: "查看文件", summary: "SessionList.tsx", output: "已读取 128 行\n输出按原记录展示\n长内容可展开或复制" }],
      },
    ],
    models: [platform === "codex" ? "gpt-5" : "claude-sonnet-4-5"],
    cost: 0.42,
    priced_requests: 12,
    requests: 12,
  }
}
