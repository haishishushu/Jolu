/**
 * 灵动岛与托盘的浏览器预览示例数据（取自旧稿画布，未自行编造；改动时按 Jolu.pen 核对）。
 * 需求文档反复强调：所有数值均为设计示例，不代表已接入或接口验证结果。
 */
import type {
  QuotaWindow,
  SessionActivity,
  StatSource,
} from "@/types"

/** Claude 官方账号双额度（画布 01 / 03 / 08） */
export const CLAUDE_QUOTAS: QuotaWindow[] = [
  {
    key: "5h",
    windowName: "5 小时窗口 · 已用",
    usedPercent: 10,
    usedText: "已用 1.2k / 12k 消息",
    resetCountdown: "4h 51m",
    badgeTone: "purple",
  },
  {
    key: "7d",
    windowName: "7 天窗口 · 已用",
    usedPercent: 59,
    usedText: "已用 44.3k / 75k 消息",
    resetCountdown: "3d 7h",
    badgeTone: "green",
  },
]

export const SESSIONS: SessionActivity[] = [
  { id: "s1", title: "修复登录问题", deltaText: "+8.0k Token", state: "running" },
  { id: "s2", title: "编写单元测试", deltaText: "+3.2k Token", state: "running" },
  {
    id: "s3",
    // 画布 03：演示「过长时单行省略并以…结尾」
    title: "整理接口文档与第三方鉴权流程说明与回归测试清单",
    deltaText: "+1.2k Token",
    state: "running",
  },
]

export const TOTAL_DELTA_TEXT = "+12.4k Token"
export const RUNNING_SESSION_TEXT = "3 个会话运行中"

export const LOCAL_SOURCE: StatSource = {
  id: "local",
  name: "本地会话记录",
  scope: "本机 Claude CLI 会话记录",
  perConnection: false,
  realtimeDelta: true,
  lastUpdatedText: "12 秒前",
}

export const DEMO_MODELS = ["claude-opus-4-1", "claude-sonnet-4"]

