import type { PlatformId } from "@/types"

/**
 * 连接弹窗「打开登录指引」「切换账号」的外部链接（D-15）。
 *
 * 只收录官方域名、执行时实际打开核实过的 HTTPS 页面；verified 是核实日期。
 * 没有可靠官方页面的平台或种类不收录，按钮保持禁用并提示「暂无官方指引链接」。
 * 「切换账号」各家都没有单独页面：只在官方文档写明「退出当前账号后重新登录」时才收录，
 * 地址指向写这段说明的页面或小节。
 */
export interface ProviderLinks {
  loginGuide?: string
  switchAccount?: string
  verified: string
}

export type ProviderLinkKind = "loginGuide" | "switchAccount"

export const PROVIDER_LINKS: Partial<Record<PlatformId, ProviderLinks>> = {
  // 2026-10-23 核实：Claude Code 官方认证文档；「Log in to Claude Code」小节写明用 /logout 退出后重新登录
  claude: {
    loginGuide: "https://code.claude.com/docs/en/authentication",
    switchAccount: "https://code.claude.com/docs/en/authentication#log-in-to-claude-code",
    verified: "2026-10-23",
  },
  // 2026-10-23 核实：收录官方入口地址，跳转交给浏览器处理（跳转目标将来可能变）。
  // 该地址会跳转到 OpenAI 帮助中心（learn.chatgpt.com）的认证说明，默认显示 ChatGPT 桌面端版本，
  // 页内可切到 Codex CLI 版本（codex login / codex logout）；
  // 锚点在跳转后保留，「Check authentication or sign out」小节写明退出当前凭证后重新登录
  codex: {
    loginGuide: "https://developers.openai.com/codex/auth",
    switchAccount: "https://developers.openai.com/codex/auth#check-authentication-or-sign-out",
    verified: "2026-10-23",
  },
  // 2026-10-23 核实：geminicli.com 是 google-gemini/gemini-cli 仓库登记的官方主页；文档没有切换账号的说明，不收录
  gemini: {
    loginGuide: "https://geminicli.com/docs/get-started/authentication/",
    verified: "2026-10-23",
  },
  // 2026-10-23 核实：xAI 官方 Grok Build 文档；入门页写明首次启动在浏览器完成认证，
  // CLI 参考的 Subcommands 写明 grok login 登录、grok logout 退出并清除缓存凭证
  grok: {
    loginGuide: "https://docs.x.ai/build/overview",
    switchAccount: "https://docs.x.ai/build/cli/reference#subcommands",
    verified: "2026-10-23",
  },
  // 2026-10-23 核实：ZCode 官方文档「连接模型与套餐」，写明连接 BigModel / Z.ai 账号授权登录；未找到退出或切换账号的说明，不收录
  zcode: {
    loginGuide: "https://zcode.z.ai/cn/docs/configuration",
    verified: "2026-10-23",
  },
  // 2026-10-23 核实：Qoder 官方快速开始的「Sign in」步骤；IDE 退出登录未见官方说明，不收录切换账号
  qoder: {
    loginGuide: "https://docs.qoder.com/quick-start",
    verified: "2026-10-23",
  },
  // 2026-10-23 核实：WorkBuddy 官方文档（腾讯云 CodeBuddy）；Windows 安装指南有「登录」小节，
  // 系统设置页写明「退出登录：退出当前账号并切换其他账号登录」
  workbuddy: {
    loginGuide: "https://www.codebuddy.cn/docs/workbuddy/From-Beginner-to-Expert-Guide/Installation-Win-Guide",
    switchAccount: "https://www.codebuddy.cn/docs/workbuddy/From-Beginner-to-Expert-Guide/Function-Description/Setting",
    verified: "2026-10-23",
  },
  // 2026-10-23 核实：TRAE 官方文档；快速开始写明初始设置时登录，IDE 设置总览的「账号」写明「退出登录：退出当前登录的账号」
  trae: {
    loginGuide: "https://docs.trae.ai/ide/get-started-with-trae-desktop",
    switchAccount: "https://docs.trae.ai/ide/ide-settings-overview",
    verified: "2026-10-23",
  },
}

/** 平台的某类外部链接；未收录时返回 null，界面据此禁用按钮 */
export function linkFor(platform: PlatformId, kind: ProviderLinkKind): string | null {
  return PROVIDER_LINKS[platform]?.[kind] ?? null
}
