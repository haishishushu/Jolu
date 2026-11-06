# 供应商额度描述符（ProviderQuotaDescriptor）调研与草案

- 日期：2026-11-05
- 性质：纯调研 + 设计草案，不含实现。除本文件外未改动任何文件，未执行 git，未启停进程。
- 背景：胶囊灵动岛要接入的供应商持续增加（已知将新增 MiniMax、Qwen/DashScope、Grok/xAI、DeepSeek），而各家的「额度形态」互不相同：5 小时 + 7 天滚动窗口、按日 token 桶、包月 credit、按量计费余额、免费额度。当前实现把这套差异写进了共享组件（`authQuotaRows` 的 5h/7d 占位、`quotaMap` 的 TONE、`CapsuleIsland` 的 `LABEL` 与 `platform === "grok"` 特判、`remainingDisplay` 的按平台文案），每接一家都要再散点改一遍。

> 取证说明：本文所有接口结论都来自实际抓取到的页面，抓取失败或只有二手来源的条目一律标注「未找到官方文档」或「待复核」，不做补写。抓取状态见 §7。

---

## 1. 结论摘要

1. **模型层元数据已经有事实标准，可以直接借用，不用自建。** `models.dev`、LiteLLM 的 `model_prices_and_context_window.json`、OpenRouter `/api/v1/models` 三家在「模型 id / 上下文与输出上限 / 价格单位 / 能力位」上高度趋同；OpenCode 干脆直接消费前两者（AI SDK + models.dev）。Jolu 需要的模型名、上下文、价格、`cost_estimate` 估算口径都能从这三家取，不必自己维护表。

2. **额度 / 订阅 / 剩余量：没有任何一家统一，也没有形成草案。** 唯一方向正确的标准是 IETF `draft-ietf-httpapi-ratelimit-headers-11`（`RateLimit-Policy` + `RateLimit`），但它有四个硬伤，**只能当补充来源，不能当统一剩余量来源**：
   - 单位注册表只有 `requests` / `content-bytes` / `concurrent-requests`，**没有 token**，而模型订阅额度几乎都按 token；
   - 它描述的是「本请求所属策略的可用配额」，不是「账户订阅窗口剩余」，服务方可以不发 `RateLimit-Policy`（草案附录 B.1 明确允许）；
   - 只有发生请求才有 header。灵动岛是常显的，空闲、未走转发时读不到；
   - `t` 是「有效窗口剩余秒数」，不是绝对重置时刻，`r` 是配额数不是百分比，换算必然带推测。

3. **各家的额度查询方式差异是「形态差异」而不是「数值差异」，所以描述符的第一等字段必须是窗口形态（`windowKind`），不是数值接口。** 同样返回一个百分比，Claude 是 5h + 7d 滚动，OpenCode Go 是 rolling(5h) + 周 + 月，DeepSeek 只有货币余额（无窗口、无重置），Qwen 连官方接口都没有。

4. **插件化先例里，最贴合我们需求的是 Agent Plugins 1.0 的形态：封闭 schema 清单 + `$schema` 版本标记 + 固定组件位置 + 逐组件「无效就跳过、不影响其它组件」的失败边界。** OpenCode plugin 轻量但没有清单可做构建期校验；LiteLLM 的 provider 注册是反面教材（新增一家要改 5 处核心文件）。

5. **可落地的目标形态**：每家供应商 = 一个 `providers/<id>.json` 描述符（声明窗口形态、单位、重置语义、置信度、轮询与失败语义、文案）；需要联网的再加一个后端 adapter 文件并在注册表登记一行。灵动岛的模板选择、几何（0/1/2 圈环、贴边两条轨道）、标签全部改由描述符驱动，共享组件零改动。现状要达到这个目标，需要把 15 处以上散落的平台分支收敛掉（清单见 §6.4）。

---

## 2. 可借用的元数据标准对比

| 项目 | 统一了什么 | 缺什么（对我们的关键缺口） | 许可 | 更新频率 |
|---|---|---|---|---|
| [models.dev](https://models.dev/)（[api.json](https://models.dev/api.json)，仓库 [sst/models.dev](https://github.com/sst/models.dev)） | 供应商条目（`id / name / env / npm / api / doc`）+ 模型条目（`id`、`canonical_model_id`、`name`、`family`、`release_date`、`last_updated`、`modalities{in,out}`、`limit{context,input,output}`、`cost{input,output,cache_read,cache_write,tiers}`、能力位 `reasoning / reasoning_options / tool_call / structured_output / temperature / attachment / open_weights`、`status`、`deprecated`）。价格单位：**USD / 百万 token**（站点表格即按此展示） | 没有任何额度、订阅、窗口、剩余字段；第三方聚合条目的 `cost` 出现 `0.00`（免费/未定价混在一起） | 站点自称 open-source；仓库 LICENSE 本次未核对 | 社区 PR；每个模型带 `release_date` / `last_updated` |
| [LiteLLM `model_prices_and_context_window.json`](https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json)（[字段规范](https://docs.litellm.ai/docs/provider_registration/add_model_pricing)） | 逐 token 成本键（`input_cost_per_token`、`output_cost_per_token`、`cache_read_input_token_cost`、`cache_creation_input_token_cost`、`*_batches`、`*_above_272k_tokens`）、`max_input_tokens` / `max_output_tokens` / `max_tokens`、`mode`、能力位（`supports_function_calling`、`supports_vision`、`supports_prompt_caching`、`supports_reasoning`、`supports_pdf_input`…）、`aliases`、`deprecation_date`、`supported_regions` | 同样没有额度/订阅字段；价格是**逐 token 美元**，与 models.dev 的「每百万」不同单位，混用会差 6 个数量级 | 开源仓库；LICENSE 本次未核对 | 跟随 main 分支 PR，几乎每个版本都动 |
| [OpenRouter `/api/v1/models`](https://www.openrouter.ai/docs/client-sdks/typescript/models/model) | `id`（`vendor/model`）、`canonicalSlug`、`name`、`created`、`contextLength`、`pricing{prompt,completion}`（**字符串，USD / token**）、`architecture{inputModalities,outputModalities,modality}`、`supportedParameters`、`topProvider`、`perRequestLimits`、`endpoints` | `perRequestLimits` 是**单次请求**的上下文/输出上限，容易被误读成账户额度；无账户级额度 | 服务公开可读；数据使用条款见其站点 | 随模型上下架实时变化 |
| [Vercel AI SDK](https://ai-sdk.dev/docs/ai-sdk-core/provider-management)（[customProvider](https://ai-sdk.dev/docs/reference/ai-sdk-core/custom-provider)、provider registry） | 统一的是**调用接口**：`providerId/modelId` 字符串寻址、`provider.languageModel(id)`、`customProvider` 可做别名与默认设置注入；元数据本身交给 models.dev / gateway | 不含目录、不含价格（要自己接 models.dev）；不含额度 | 开源（仓库 LICENSE 本次未核对） | 随 SDK major 版本 |
| [OpenCode provider 配置](https://opencode.ai/docs/providers/) | 「一条声明 + 一个 SDK 包」的分工：`provider.<id> = { npm:"@ai-sdk/...", name, options.baseURL, models{<key>:{id}}, blacklist/whitelist }`，`$schema` 指向 `https://opencode.ai/config.json`；目录来自 models.dev | 同样没有额度/订阅字段；`blacklist/whitelist` 只是选择器，不是能力探测 | 开源（仓库 LICENSE 本次未核对） | 随版本 |
| [Agent Plugins 1.0](https://agent-plugins.org/specification)（[magpie 的市场用法](https://github.com/apache/magpie/blob/main/docs/setup/marketplace.md)） | 统一的是**贡献格式**：根 `plugin.json` 顶层字段封闭为 `$schema / name / version / description / author / homepage / repository / license / keywords / extensions`；`$schema` 是版本标记，客户端**不得联网取 schema**，不认识的版本直接拒绝；组件位置固定（`skills/`、`mcp.json`），清单不能内联组件配置；未知顶层字段「报告并忽略，继续加载」，其它 schema 违规才致命；单个组件坏了只 skip 该组件 | 与额度无关（v1 只定义 skills 与 MCP servers 两类组件） | 规范公开；magpie 为 Apache-2.0 | 规范按 `1.0.0` 版本化，2026-08-06 发布 |
| [OpenCode plugin](https://opencode.ai/docs/plugins/) | 扩展点：导出一个函数，返回 hooks 对象（`tool.execute.before/after`、`event`、`session.*` 等），随 `opencode.json` 的 `plugin: []` 从 npm 安装 | 没有静态 manifest，无法在「没装、没跑」时校验能力声明；没有版本协商 | 开源（LICENSE 本次未核对） | 随版本 |
| [MCP](https://modelcontextprotocol.io/docs/2026-07-28/learn/architecture) | 运行时协议：客户端 `list` 发现能力、`tools/list_changed` 通知变更 | 是协议不是清单：能力只能在连上之后问出来，构不成构建期契约 | 公开规范 | 按 `2025-06-18` / `2026-07-28` 等版本 |

**统一了什么**：模型 id（`vendor/model` 或 `canonical_model_id`）、上下文与输出上限、价格（但单位三套：$/百万 token、$/token、字符串 $/token）、能力位（reasoning / tool_call / vision / caching / structured output）。

**没有统一什么**：额度窗口（5h/7d/日/月/credit）、订阅状态、剩余量、重置时刻、账户与分区的归属。这三家里没有任何一家提供「这个账号这个窗口还剩多少」。

---

## 3. 额度语义的行业现状

### 3.1 RateLimit header 能不能当「实时剩余」的统一来源

标准草案：[draft-ietf-httpapi-ratelimit-headers-11](https://datatracker.ietf.org/doc/draft-ietf-httpapi-ratelimit-headers/)（2026-05-23，WG Document，尚未成为 RFC；[纯文本](https://www.ietf.org/archive/id/draft-ietf-httpapi-ratelimit-headers-11.txt)）。要点：

- `RateLimit-Policy`：`"<name>";q=<配额>;qu=<单位>;w=<窗口秒>;pk=<分区>`，`q` 必填，`qu` 缺省为 `requests`，`w` 是**信息性**的（FAQ 第 2 条明确「w 只是提示，限流行为只依赖 limit / remaining / reset」）。
- `RateLimit`：`"<name>";r=<可用配额>;t=<有效窗口秒>;pk=…`，`r` 必填。
- 单位注册表（IANA「HTTP RateLimit Quota Units」）初始只有三项：`requests`、`content-bytes`、`concurrent-requests`。
- `t` 是**剩余秒数**（对齐 `Retry-After` 的 delay-seconds），刻意不用绝对时间戳，理由是对时钟同步不敏感。
- 头可以出现在 200，也可以出现在 429；草案明确「不规定非成功响应是否计入配额」。
- 附录「RateLimit header fields currently used on the web」记录了现实中的混乱：`X-RateLimit-Limit / -Remaining / -Reset` 及其按窗口拆分的变体（`x-ratelimit-limit-minute|hour|day`），并且 **`X-RateLimit-Remaining` 在不同实现里含义不同**——「窗口剩余秒数」「剩余毫秒」「UTC 时间戳」「HTTP-date / RFC3339 日期」都出现过。

**结论：不能作为统一剩余量来源，只能作为 `confidence = derived` 的补充来源。** 具体理由：

| 硬伤 | 后果 | 在 Jolu 里的处理 |
|---|---|---|
| 单位表里没有 token | 订阅额度按 token 计，`qu=requests` 的 `r` 无法换算成「还剩百分之几」 | 只有能同时读到上限与已用时才可换算，否则 `unit=percent` 不可得 → 记 unknown |
| 语义是「本请求所属策略」，且策略可缺省 | 同一个账户的 5h / 7d 是两条不同 policy，服务方可只发其一或不发 Policy 头 | 不能据此判断「有没有 5h/7d 窗口」，窗口形态仍须由描述符声明 |
| 需要有请求才有 header | 灵动岛常显、空闲时没有上游响应可读 | 只能缓存「最近一次观测」并标 `observedAt`，无观测即 unknown |
| `t` ≠ `resetsAt` | 直接把 `t` 当重置倒计时会算错（还受中间代理、缓存影响，草案 §7.3 要求忽略来自缓存的 RateLimit） | 描述符里 `resetsAtKind` 区分 `absolute` / `relative-seconds` / `unknown`，禁止隐式换算 |

可用场景只有一个，但很值：Jolu 的模型入口（模块 J）替用户向上游转发时，从**自己发出的那一次响应**里读 `x-ratelimit-*` / `anthropic-ratelimit-*` 等头，得到「刚才那次请求时刻的剩余」，作为官方接口缺失时的降级观测，标 `confidence = derived` + `observedAt`，界面必须与官方值区分。

> 关于 Cloudflare 的「rate limit 规范」：本次未找到 Cloudflare 官方发布的、面向客户端的统一「剩余量响应头」规范文档（其速率限制产品文档面向前置限流配置）。此项标注**未找到官方文档**，不作为依据。

### 3.2 各家官方额度接口清单

「有/无/需登录态」列的是**能否用 Jolu 已保存的凭证类型拿到额度**。

| 供应商 | 形态（窗口） | 端点 | 凭证 | 官方文档 | 能给 `resetsAt` | 备注 |
|---|---|---|---|---|---|---|
| Claude 订阅 | 5h + 7d 滚动 | `GET https://api.anthropic.com/api/oauth/usage`（beta 头 `oauth-2025-04-20`） | OAuth accessToken | **未找到官方文档**（实现依据见 `backend/src/quota.rs` 模块文档与既有实现） | 是（绝对时刻） | 端点非公开文档 API，字段可能随客户端变化 |
| Claude API Key | 组织用量/费用，不是订阅额度 | `GET /v1/organizations/usage_report/messages`、`cost_report` | Admin Key | 官方有（[Rate limits](https://platform.claude.com/en/api/rate-limits) 本次跨域跳转未取到正文） | — | 普通 Key 403；界面回退本机会话统计 |
| Codex 订阅 | 接口返回什么算什么（`primary_window` / `secondary_window`，按 `limit_window_seconds` 归类 5h=18000、7d=604800） | `GET https://chatgpt.com/backend-api/wham/usage` | Codex Auth accessToken + `ChatGPT-Account-Id` | **未找到官方文档** | 是（`reset_at` 秒级时间戳） | 实测 Pro 账号的 `primary` 就是 7d，不能假定 primary=5h |
| OpenAI API Key | 组织用量/费用 | `/v1/organization/usage/completions`、`/v1/organization/costs` | Admin Key | 官方有（`platform.openai.com/docs/api-reference/usage`，本次抓取被 Cloudflare 拦截） | — | 同上，非订阅额度 |
| 智谱 GLM（BigModel / Z.ai） | 5h（`unit=3`）+ 周（`unit=6`） | `GET {open.bigmodel.cn\|api.z.ai}/api/monitor/usage/quota/limit` | **裸 key**（`Authorization: <key>`，无 `Bearer`） | **未找到官方文档**（控制台同源端点） | 是（毫秒时间戳） | `number` 字段实测有 5/7/1 多种，只锚定 `unit` |
| Kimi For Coding | 5h + 周 | `GET api.kimi.com/coding/v1/usages` | Bearer | **未找到官方文档** | 是 | 逐包 + 汇总两种结构 |
| MiniMax（国内/国际） | 5h + 周（激活时） | `GET {api.minimaxi.com\|api.minimax.io}/v1/api/openplatform/coding_plan/remains` | Bearer | **未找到官方文档**；[账户 FAQ](https://platform.minimax.cn/docs/faq/about-account) 只说明余额在控制台（账户管理 → 余额），未提供查询 API | 是（`end_time` / `weekly_end_time`，毫秒） | 控制台同源，非公开文档 |
| OpenCode Go | rolling(5h) + 周 + 月 | `GET opencode.ai/zen/go/v1/usage` | Bearer | **未找到官方文档** | 部分（`resetsAt`，占位值需过滤） | 百分比可能是 0–1 小数 |
| ZCode | `data.balances[]` 每个 bucket 一个窗口 | `GET zcode.z.ai/api/v1/zcode-plan/billing/balance` | ZCode 客户端 JWT + `X-Device-Mid` | **未找到官方文档** | 是 | 依赖本机 ZCode 客户端身份 |
| **DeepSeek** | **只有货币余额，无窗口** | `GET /user/balance`（[官方文档](https://api-docs.deepseek.com/api/get-user-balance)） | Bearer API Key | **有** | **否**（响应里没有重置时刻） | 返回 `is_available` + `balance_infos[]{currency: CNY\|USD, total_balance, granted_balance, topped_up_balance}`，**金额是字符串**；`granted_balance` 有「未过期」语义但**不给到期时刻** |
| **Qwen / DashScope（百炼）** | 未找到官方额度接口 | — | — | **未找到官方文档**提供以 DashScope API Key 查询余额/免费额度的接口；账户余额在阿里云 BSS：[QueryAccountBalance](https://www.alibabacloud.com/help/en/user-center/developer-reference/api-bssopenapi-2017-12-14-queryaccountbalance)（RAM 签名、**账户级**、`AvailableAmount`/`AvailableCashAmount`/`Currency`） | 需 RAM AK/SK（控制面） | 否 | 与 Jolu「不保存控制面 AK/SK」的既有约束冲突（同火山方舟），只能 `windowKind=none` |
| **xAI / Grok** | 订阅侧接口未取得正文 | 官方存在 [Management API](https://docs.x.ai/docs/key-information/using-management-api) / [Management API 参考](https://docs.x.ai/developers/rest-api-reference/management)，面向团队账单与用量 | **Management Key**（不是推理 API Key）+ Team | 官方文档存在，**本次抓取失败（网络），端点字段待复核** | 待复核 | 与现状实现一致：Grok 的「API 预付余额」需独立 Management Key，普通 API Key 读不到（`frontend/src/lib/remainingDisplay.ts` 已如此处理） |
| 火山方舟 | 需控制面 AK/SK | 控制面 OpenAPI | AK/SK | 官方有 | — | 已明确不支持（`backend/src/coding_plan.rs`） |
| Gemini | — | 本机登录/CLI 授权 | 桌面登录 | — | — | API Key 不提供额度（`platformCatalog.json` 限制说明） |
| Qoder / WorkBuddy / Trae | 积分/credits | 各家客户端同源 | 桌面登录/网页授权 | 部分有 | 部分 | 已有独立 adapter（`*_quota.rs`） |

**形态归类**（这就是 `windowKind` 的现实来源）：

- 5h + 周滚动：Claude、Codex、智谱、Kimi、MiniMax、OpenCode Go（+ 月）
- 月/包月 credit：ZCode bucket、OpenCode Go 月窗
- 货币余额（无窗口）：DeepSeek（官方）、xAI（Management Key）、sub2api / new-api 网关
- 免费额度：百炼等控制台侧，**本次未拿到任何官方查询接口**
- 无：Qwen/DashScope（API Key 路径）、Gemini API Key、火山

---

## 4. `ProviderQuotaDescriptor` 草案

### 4.1 设计约束

1. **一个描述符只描述「形态与语义」，不描述「怎么发请求」。** 网络调用留在后端 adapter，描述符负责声明「这个窗口叫什么、什么单位、重置怎么读、置信度上限多少、失败了怎么画」。
2. **未知与真实零必须分开**（沿用仓库既有纪律）：`unit` 未知不等于 `0`，`windowKind: none` 不等于「没有剩余」。
3. **不允许隐式换算**：`relative-seconds` 到 `resetsAt` 的换算必须在描述符里显式声明（`resetsAtKind`），不允许组件层猜。
4. **共享组件只读描述符，不认识供应商 id。** 描述符之外不允许再出现 `platform === "grok"` 这类分支。

### 4.2 字段清单

```ts
/** 描述符版本：与 Agent Plugins 的做法一致，用 $schema 指向固定版本，客户端不联网取 schema */
export interface ProviderQuotaDescriptor {
  $schema: "https://jolu.app/schemas/quota-descriptor/1.0.0.json"
  /** 与 platformCatalog.json 的 id 一致；新增供应商时这是唯一新 id 的来源 */
  providerId: string
  /** 展示用家族文案（取代 remainingDisplay 里按平台硬编码的标题） */
  labels: {
    familyName: string          // "Grok"、"DeepSeek"、"智谱 GLM"
    planNoun: string            // "SuperGrok"、"余额"、"Coding Plan"
    /** 取不到时的统一说法，不得写成 0 */
    unknownText: string         // "—"
    /** 能力不支持/无接口时给用户的原因，取代 platforms.ts 的 limitation */
    unavailableReason: string
  }
  /** 额度数据从哪来。决定 confidence 上限，也决定界面上「这是官方值还是推测值」 */
  quotaSource: QuotaSource
  /** 需要什么凭证。决定能力探测走哪条路、未登录怎么提示 */
  authKind: AuthKind
  /** 窗口清单。空数组 = 该来源没有窗口概念（只有余额或什么都没有） */
  windows: WindowSpec[]
  /** 轮询与失败语义（对齐现有 QUOTA_CACHE 行为，见 4.4） */
  poll: PollPolicy
  /** 降级顺序：按数组顺序尝试，前一个不可用才用后一个 */
  degradation: QuotaSource[]
  /** 与现有 PlatformCapability 对齐的能力声明，供供应商页/灵动岛判断入口 */
  capabilities: PlatformCapability[]
  /** 能力探测：只读、零成本、声明期望字段 */
  probe: ProbeSpec
}

export type QuotaSource =
  | "official-api"        // 有官方文档的接口（DeepSeek /user/balance）
  | "console-same-origin" // 控制台同源、非公开文档（智谱 monitor/usage/quota/limit、MiniMax remains）
  | "local-client"        // 读原工具本机登录后查询（Claude oauth/usage、Codex wham/usage、Qoder/Trae/WorkBuddy）
  | "gateway"             // 用户自建网关（sub2api / new-api / one-api）
  | "response-header"     // 从转发响应头读 RateLimit，derived
  | "local-estimate"      // 本机会话统计推算，derived
  | "none"                // 确认没有可验证来源

export type AuthKind =
  | "oauth" | "api-key" | "device-code" | "desktop-session"
  | "client-jwt+device"   // ZCode：JWT + X-Device-Mid
  | "management-key"      // xAI：独立 Management Key + Team
  | "none"

/** 窗口形态：这是整个描述符的第一等字段，决定模板与几何 */
export type WindowKind =
  | "rolling-5h"   // 5 小时滚动窗
  | "weekly-7d"    // 7 天/周窗
  | "daily"        // 按日桶
  | "monthly"      // 月窗 / 包月额度
  | "credit"       // credit / 积分 / 余额（无窗口，只有余量）
  | "none"         // 该来源没有额度概念（如官方 API Key 只给组织用量）

/** 计量单位。与 RateLimit 草案的单位表对齐，但补 token/credit/currency（草案里没有） */
export type QuotaUnit = "token" | "request" | "credit" | "percent" | "currency"

export interface WindowSpec {
  /** 稳定 key，前端配色与选择器只认它：5h / 7d / 1d / 30d / credits */
  id: string
  windowKind: WindowKind
  /** 家族文案，取代 CapsuleIsland 的 LABEL 常量与 remainingDisplay 的分支 */
  label: string
  unit: QuotaUnit
  /** 单位是 currency 时给 ISO 4217（CNY / USD），否则 null */
  currency?: string | null
  /** 作用域：账户级 / 按模型 / 按 bucket（ZCode balances[]） */
  scope: "account" | "model" | "bucket"
  /** 接口能直接给出的上限（拿不到就 null，不要用「已用 × 假设总量」倒推） */
  limit?: number | null
  /** 重置时刻语义，禁止组件层猜 */
  resetsAtKind: "absolute" | "relative-seconds" | "unknown"
  /** 该窗口是否参与「套餐制」判定（取代 planCoverage 里写死的 ["5h","7d"]） */
  countsAsPlan: boolean
  /** 渲染意图：允许画环 / 允许进贴边轨道 / 只出现在卡片里 */
  display: { ring: boolean; rail: boolean; chip: boolean }
}

export interface PollPolicy {
  /** 成功后的 TTL（秒），与 lib.rs 现状 5 分钟一致 */
  okSeconds: number
  /** 429 时的退避区间（秒，取服务端提示并 clamp） */
  rateLimited: { minSeconds: number; maxSeconds: number }
  /** 连续失败的指数退避 */
  failure: { baseSeconds: number; maxSeconds: number }
  /** 稳定的「不支持」状态：不做指数退避，固定间隔轻探 */
  stableFailureSeconds: number
}

export interface ProbeSpec {
  /** 探测方法：GET / POST / local-read */
  method: "GET" | "POST" | "local-read"
  /** 零成本：探测不得产生计费调用（不得发 chat/completions 试探额度） */
  free: true
  /** 响应里必须出现的字段路径；缺任意一个 → degraded，不是失败 */
  expectsFields: string[]
  /** 判定为「需要重新登录」的状态码/条件 */
  reauthOn: string[]
}
```

### 4.3 字段取值与来源

| 字段 | 取值 | 来源 / 依据 |
|---|---|---|
| `$schema` | 固定 URL + 版本号 | 借 Agent Plugins 1.0 §5.2：客户端不得联网取 schema，不认识的版本直接拒绝（[规范](https://agent-plugins.org/specification)） |
| `providerId` | 与 `frontend/src/lib/platformCatalog.json` 的 `id` 同值 | 现有唯一配置源（后端 `platforms.rs` 用 `include_str!` 读同一文件） |
| `labels.*` | 家族名词 + 计划名 + 未知占位 + 不支持原因 | 取代 `remainingDisplay.ts` 的 `UNAVAILABLE`、`CapsuleIsland.tsx` 的 `LABEL`、`platformCatalog.json` 的 `limitation` |
| `quotaSource` | 见 `QuotaSource` 七值 | §3.2 每个供应商的「官方文档」列：有文档 → `official-api`；控制台同源 → `console-same-origin`；读本机登录 → `local-client`；转发响应头 → `response-header` |
| `authKind` | 见 `AuthKind` | §3.2 凭证列；`management-key` 专为 xAI，`client-jwt+device` 专为 ZCode |
| `windows[].windowKind` | 六值枚举 | §3.2 形态归类表 |
| `windows[].unit` | `token`/`request`/`credit`/`percent`/`currency` | 接口实际返回：Codex 给 `used_percent` → `percent`；DeepSeek 给金额 → `currency`；ZCode 给 bucket → `credit`；组织用量给 token |
| `windows[].resetsAtKind` | `absolute`（Zhipu/Kimi/MiniMax/Codex 的毫秒或秒时间戳）、`relative-seconds`（仅 RateLimit `t`）、`unknown`（DeepSeek 余额、组织用量） | §3.1 结论：`t` 不得当 `absolute` |
| `windows[].countsAsPlan` | `rolling-5h` / `weekly-7d` 为 true，其余 false | 现有 `planCoverage.ts` 的 `PLAN_WINDOW_KEYS = ["5h","7d"]` 语义保持不变，只是搬进描述符 |
| `windows[].display` | 环/轨道/文字三开关 | A3 设计稿只允许「Auth 两窗 → 两圈、单来源 → 一圈、API 连接 → 不画环」（`docs/specs/2026-11-04-capsule-island-design.md` §122–125） |
| `poll` | `ok=300`、`rateLimited=1..1800`、`failure=15×2ⁿ≤300`、`stableFailure=60` | `backend/src/lib.rs:563-566` 的 `cache_ttl` 现状，直接抄成默认值 |
| `degradation` | 例：`["official-api","response-header","none"]`（DeepSeek 无头部来源 → `["official-api","none"]`） | §3.1：头部只作降级观测；`none` 必须显式出现，禁止「静默什么也不显示」 |
| `capabilities` | 现有 `PlatformCapability` 子集 | `frontend/src/types.ts` 已有枚举（`subscription_quota` / `gateway_balance` / `credits` / `organization_usage`…） |
| `probe.expectsFields` | 如 DeepSeek `["is_available","balance_infos[0].total_balance"]` | 来自 §3.2 实际响应结构；字段缺失 → `degraded` 而非「0 元」 |

### 4.4 `confidence` 与失败语义

`confidence` 不进描述符的静态字段，而是**每次查询结果的运行时标注**：

| confidence | 何时 | 界面要求 |
|---|---|---|
| `official` | `quotaSource` ∈ `official-api` / `local-client` / `console-same-origin`，且 `probe.expectsFields` 全部命中 | 正常渲染数值 |
| `derived` | 来自 `response-header` / `local-estimate`，或由 `used_percent` 反算剩余 | 必须与官方值可区分（例如附「估算/上次请求观测」与观测时刻 `observedAt`） |
| `unknown` | 查询失败 / 字段缺失 / `quotaSource = none` | 只画空轨道 + 原因，**禁止画成 0% 或满格** |

`failureSemantics` 固定为四类，不允许自由发挥（对齐 `QuotaState` 五态与 `dockQuotaView` 的三形态）：

| 失败 | 语义 | 渲染 |
|---|---|---|
| `unauthorized` | 凭证失效 | 保留上次值但标 stale，同时给「重新登录」入口；不得清成 0 |
| `forbidden` | 权限不足（Admin/Management Key 缺失） | 显示「此余额需要更高级别凭证」类原因（DeepSeek 之外的 xAI/阿里云即此例） |
| `rate_limited` | 429 | 按 `poll.rateLimited` 退避，保留上次值 |
| `shape-changed` | 字段缺失/格式变化 | `degraded`：能读的字段照常显示，缺的显示未知；描述符版本不匹配时整家降级到 `degradation` 的下一个来源 |

### 4.5 三个样例

```jsonc
// providers/deepseek.json —— 只有货币余额，没有窗口，没有重置时刻
{
  "$schema": "https://jolu.app/schemas/quota-descriptor/1.0.0.json",
  "providerId": "deepseek",
  "labels": { "familyName": "DeepSeek", "planNoun": "账户余额", "unknownText": "—",
              "unavailableReason": "未保存可查询余额的 API Key" },
  "quotaSource": "official-api",
  "authKind": "api-key",
  "windows": [
    { "id": "credits", "windowKind": "credit", "label": "余额", "unit": "currency", "currency": "CNY",
      "scope": "account", "limit": null, "resetsAtKind": "unknown", "countsAsPlan": false,
      "display": { "ring": false, "rail": true, "chip": true } }
  ],
  "poll": { "okSeconds": 300, "rateLimited": { "minSeconds": 1, "maxSeconds": 1800 },
            "failure": { "baseSeconds": 15, "maxSeconds": 300 }, "stableFailureSeconds": 60 },
  "degradation": ["official-api", "none"],
  "capabilities": ["api_connection", "gateway_balance"],
  "probe": { "method": "GET", "free": true,
             "expectsFields": ["is_available", "balance_infos[0].total_balance"],
             "reauthOn": ["401"] }
}
```

```jsonc
// providers/minimax.json —— 5h + 周双窗，控制台同源、无公开文档
{
  "$schema": "https://jolu.app/schemas/quota-descriptor/1.0.0.json",
  "providerId": "minimax",
  "labels": { "familyName": "MiniMax", "planNoun": "Coding Plan", "unknownText": "—",
              "unavailableReason": "未找到官方额度查询文档；仅登录后读取本机套餐凭证" },
  "quotaSource": "console-same-origin",
  "authKind": "api-key",
  "windows": [
    { "id": "5h", "windowKind": "rolling-5h", "label": "5 小时额度", "unit": "percent",
      "scope": "account", "limit": null, "resetsAtKind": "absolute", "countsAsPlan": true,
      "display": { "ring": true, "rail": true, "chip": true } },
    { "id": "7d", "windowKind": "weekly-7d", "label": "周额度", "unit": "percent",
      "scope": "account", "limit": null, "resetsAtKind": "absolute", "countsAsPlan": true,
      "display": { "ring": true, "rail": true, "chip": true } }
  ],
  "poll": { "okSeconds": 300, "rateLimited": { "minSeconds": 1, "maxSeconds": 1800 },
            "failure": { "baseSeconds": 15, "maxSeconds": 300 }, "stableFailureSeconds": 60 },
  "degradation": ["console-same-origin", "none"],
  "capabilities": ["api_connection", "subscription_quota"],
  "probe": { "method": "GET", "free": true,
             "expectsFields": ["base_resp.status_code"], "reauthOn": ["401"] }
}
```

```jsonc
// providers/qwen.json —— 未找到官方额度接口：显式声明 none，界面画朴素态
{
  "$schema": "https://jolu.app/schemas/quota-descriptor/1.0.0.json",
  "providerId": "qwen",
  "labels": { "familyName": "Qwen", "planNoun": "免费额度", "unknownText": "—",
              "unavailableReason": "未找到官方文档提供以 DashScope API Key 查询余额或免费额度的接口；账户余额需阿里云 RAM AccessKey，Jolu 不保存此类凭证" },
  "quotaSource": "none",
  "authKind": "api-key",
  "windows": [],
  "poll": { "okSeconds": 1800, "rateLimited": { "minSeconds": 60, "maxSeconds": 1800 },
            "failure": { "baseSeconds": 60, "maxSeconds": 600 }, "stableFailureSeconds": 600 },
  "degradation": ["none"],
  "capabilities": ["api_connection"],
  "probe": { "method": "GET", "free": true, "expectsFields": [], "reauthOn": [] }
}
```

---

## 5. 第三方贡献适配器的 manifest 形态

### 5.1 先例评测

| 先例 | 怎么声明 | 怎么版本化 | 怎么探测/降级 | 对我们合适吗 |
|---|---|---|---|---|
| [Agent Plugins 1.0](https://agent-plugins.org/specification) | 根 `plugin.json`，顶层字段**封闭**为 10 个；`extensions.<reverse-domain>` 承载客户端私有数据；组件位置固定 | `$schema` 指向 `.../1.0.0/plugin.schema.json`；客户端不得联网取 schema；不认识的版本**拒绝**；未知顶层字段「报告并忽略」，其它违规才致命 | §6.2 / §7.2.2：缺失的固定位置不算错误；某个组件坏了只 skip 该组件，其它继续加载 | **最贴合**：我们要的正是「清单封闭、版本显式、坏一家不影响别家」 |
| [OpenCode plugin](https://opencode.ai/docs/plugins/) | 导出函数返回 hooks；`opencode.json` 里 `plugin: []` 从 npm 装 | 由 npm 包版本承担 | 运行时 hook，无静态探测 | 轻，但**没有构建期契约**：不装不跑就不知道能力 |
| [MCP](https://modelcontextprotocol.io/docs/2026-07-28/learn/architecture) | 无清单，运行时 `list` 发现 | 协议版本协商 | 只能连上后问 | 语义好但太晚：构不成离线校验 |
| [LiteLLM provider 注册](https://docs.litellm.ai/docs/provider_registration/) | 写 `BaseConfig` 子类，并**同时**改 `litellm/__init__.py`、`main.py`、`constants.py`、`get_llm_provider_logic.py`、必要时的 `streaming_handler.py` | 跟主仓库版本 | 无探测声明 | **反面教材**：正是我们要消灭的「接一家改多处核心文件」 |
| [magpie 市场](https://github.com/apache/magpie/blob/main/docs/setup/marketplace.md) | 一个仓库里并存 AP1 清单与各客户端私有清单，都指向同一份 `skills/`；`check-family-plugins.py` 强制版本一致、校验 AP1 封闭 schema | 语义化版本 + `plugin.json` 的 `version` | 各客户端按自己的清单读，读不认就拒绝 | 值得学的两点：**同一份数据 + 多份薄清单**、**CI 强制 schema 封闭** |

### 5.2 建议形态

**一家 = 一个 JSON 描述符文件 + 可选一个 adapter 源文件。** 描述符即清单；不引入第二套 manifest。

```
frontend/src/lib/providers/
  deepseek.json        # §4.5 的 ProviderQuotaDescriptor
  minimax.json
  qwen.json
  <id>.json
  index.ts             # 目录扫描 + 校验 + 导出只读注册表（唯一消费入口）
backend/src/providers/
  deepseek.rs          # 只在需要联网时存在；导出 query(secret) -> QuotaState
  registry.rs          # id -> query 函数的登记表（一行一家）
```

**带版本与能力探测的完整样例（TS 侧清单 + 后端登记）：**

```ts
// frontend/src/lib/providers/index.ts（示意，标 [PLACEHOLDER] 的是允许第三方填的位置）
import type { ProviderQuotaDescriptor } from "@/lib/quotaDescriptor"

export const DESCRIPTOR_SCHEMA = "https://jolu.app/schemas/quota-descriptor/1.0.0.json"

/** 未知顶层字段：报告并保留其余（借 AP1 §5.2 规则），不整份拒绝 */
/** 未知 windowKind / unit：只 skip 该窗口并记一条 report，不整家失败 */
export interface LoadReport {
  providerId: string
  descriptorVersion: string
  unknownFields: string[]
  skippedWindows: { id: string; reason: "unknown-windowKind" | "unknown-unit" | "bad-resetsAtKind" }[]
  degraded: boolean
}

export function loadDescriptor(raw: unknown): { descriptor: ProviderQuotaDescriptor | null; report: LoadReport }
export function registry(): readonly ProviderQuotaDescriptor[]
```

```jsonc
// 第三方贡献者提交的最小清单 + 探测声明（含版本号）
{
  "$schema": "https://jolu.app/schemas/quota-descriptor/1.0.0.json",
  "providerId": "acme-gateway",
  "descriptorVersion": "1.0.0",          // 描述符自身版本，与 $schema 分开：schema 约束形状，这个声明内容修订
  "labels": {
    "familyName": "Acme Gateway",
    "planNoun": "[PLACEHOLDER: 套餐名]",
    "unknownText": "—",
    "unavailableReason": "[PLACEHOLDER: 为什么这里读不到]"
  },
  "quotaSource": "gateway",
  "authKind": "api-key",
  "windows": [
    {
      "id": "5h", "windowKind": "rolling-5h", "label": "[PLACEHOLDER: 5 小时]",
      "unit": "percent", "scope": "account", "limit": null,
      "resetsAtKind": "absolute", "countsAsPlan": true,
      "display": { "ring": true, "rail": true, "chip": true }
    }
  ],
  "poll": { "okSeconds": 300, "rateLimited": { "minSeconds": 1, "maxSeconds": 1800 },
            "failure": { "baseSeconds": 15, "maxSeconds": 300 }, "stableFailureSeconds": 60 },
  "degradation": ["gateway", "response-header", "none"],
  "capabilities": ["api_connection", "subscription_quota"],
  "probe": { "method": "GET", "free": true,
             "expectsFields": ["[PLACEHOLDER: 顶层字段]", "quota_5_hour.usage_percentage"],
             "reauthOn": ["401", "403"] }
}
```

```rust
// backend/src/providers/registry.rs（示意）
/// 只有需要联网的供应商才登记；纯声明式（如 qwen 的 none）不需要这一行。
pub(crate) fn adapter(id: &str) -> Option<fn(&str) -> crate::quota::QuotaState> {
    match id {
        "deepseek" => Some(crate::providers::deepseek::query),
        // [PLACEHOLDER: 新增一家在此加一行]
        _ => None,
    }
}
```

### 5.3 三级能力探测与降级

| 级别 | 时机 | 机制 | 失败后果 |
|---|---|---|---|
| 静态 | 构建期 | JSON Schema 校验（`$schema` 必须命中已知版本）；未知顶层字段 → 记录并继续；未知 `windowKind`/`unit` → skip 该窗口 | 单窗口失效，不影响其它窗口与其它供应商 |
| 夹具 | 单测 | 每家一份真实响应样例（脱敏），跑解析器断言：窗口数、单位、`resetsAtKind`、未知值不被当 0 | 解析回归在 CI 拦住 |
| 运行 | 首次/每次查询 | `probe.expectsFields` 全命中 → `official`；部分命中 → `degraded`；`reauthOn` 命中 → `unauthorized`；都不行 → 按 `degradation` 落到下一个来源，最终 `none` | 界面永远有确定文案，不出现假 0、假满格 |

版本化策略（照 AP1 的严格度，但不搞致命拒绝）：

- `$schema` 不匹配且无法兼容 → 该供应商整体 `none`，界面显示 `labels.unavailableReason` + 「适配器版本过旧」，**不影响其它供应商启动**。
- `descriptorVersion` 只用于展示与诊断（写进诊断报告），不参与渲染分支。
- 迁移期允许「同一 providerId 两份描述符」（旧常量版与新描述符版）并存，用一条一致性测试断言两者对同一 fixture 产生相同窗口，再删旧的。

---

## 6. 对灵动岛的映射

### 6.1 字段 → 模板

现有实现里「模板」由三个隐式条件决定：`data.kind === "auth"`、`data.remainingMetrics` / `data.localMetrics`、以及是否画环（`CapsuleIsland.tsx:120`）。改为描述符驱动后：

| 模板 | 判定（只看描述符 + 查询结果） | 现状对应 |
|---|---|---|
| **订阅环** | `windows` 中存在 `countsAsPlan: true` 的窗口，且 `quotaSource ≠ none`，且查询态 ∈ {ok, loading} | `showQuotas = data.kind === "auth" && !unavailable && !remainingMetrics && !localMetrics`（`CapsuleIsland.tsx:120`） |
| **计量** | 无 `countsAsPlan` 窗口，但有 `unit ∈ {currency, credit, token, request}` 的窗口或组织用量/本机费用 | `remainingMetrics` / `localMetrics` 分支（余额、今日 token、今日费用） |
| **朴素** | `quotaSource = none`、`degradation` 全落空、未连接、或查询失败 | 「只显示破折号 + 原因」的既有兜底 |

关键点：**「API 连接不画环」不应由 `kind === "api"` 决定，而应由描述符 `display.ring` 决定**。DeepSeek 是 API Key 连接但只有余额（`ring: false, chip: true`）；而某个 API Key 连接若真有 5h/7d 套餐（如智谱个人 Key），现状也会因为 `kind === "auth"` 才画环而被漏掉——这正是 `CapsuleIsland.tsx:118-121` 注释里写的「API 连接也可能返回套餐窗口，但按原型不画成环」的妥协点。

### 6.2 字段 → 几何

| 几何 | 判定字段 | 现状对应 | 约束 |
|---|---|---|---|
| 环数 0/1/2 | `windows.filter(w => w.display.ring).length`，上限 2；`swallow` 规则：`rolling-5h` 在内、`weekly-7d` 在外 | `ringWindows = [...quotaRows].reverse()`（`CapsuleIsland.tsx:123`）——`reverse()` 只在恰好是 5h+7d 时正确 | A3 稿只画过 0/1/2 圈（`docs/specs/2026-11-04-capsule-island-design.md` §122–125），**不引入第三圈** |
| 弧段 | `Bot.tsx` 的 `strokeDasharray` 弧长由 `usedPercent` 决定；环色阈值 80/100 不变 | `ringColor()`（`CapsuleIsland.tsx:44`），其中 `q.key === "5h" ? cyan : green` 是硬编码 | 颜色改由 `windowKind` 决定：`rolling-5h→cyan`、`weekly-7d→green`、`monthly/daily→blue`、`credit→neutral`；阈值判定（`quotaBarClass` / 80 / 100）**保持不变** |
| 贴边条（两条轨道） | `windows.filter(w => w.display.rail).slice(0, 2)` | `DockedIsland` 固定两条轨道 + `unlimited` 彩虹双轨 | `unlimited` 语义应改为「描述符声明无任何轨道窗口」，与「查询失败空轨道」「真实耗尽红满格」保持三分（`dockQuotaView` 现状已如此，只需换输入） |
| 刻度环 | — | 不存在 | **A3 稿与 A2 稿都没有刻度环**。按 AGENTS.md「设计稿里没有的界面标注待确认」，本文不提出实现，只标记为待确认项 |

### 6.3 字段 → 标签

| 文案 | 字段 | 现状 |
|---|---|---|
| 家族名 / 计划名 | `labels.familyName` + `labels.planNoun` | `remainingDisplay.ts` 里逐平台写死（「SuperGrok 周剩余」「ZCode · BigModel 套餐剩余」「xAI API 预付余额」） |
| 窗口名 | `windows[].label` | `CapsuleIsland.tsx:36` 的 `LABEL = { "5h": "5 小时", "7d": "7 天" }`，未命中的回落到后端 `window_name` |
| 未知占位 | `labels.unknownText` | 现在是两套：`UNKNOWN_REMAINING = "-"` 与 Grok 路径的 `"—"`（`remainingDisplay.ts:44-48` 注释解释了这个差异） |
| 不可用原因 | `labels.unavailableReason` | `platformCatalog.json` 的 `limitation` + `remainingDisplay.ts` 的 `UNAVAILABLE` 两份重复文案 |

### 6.4 新增一家供应商需要改的文件清单

**现状（加一家平均要动 15 处以上，且共享组件被逐次修改）**：

| # | 文件 | 现在要改什么 |
|---|---|---|
| 1 | `frontend/src/types.ts` | `PlatformId` 联合类型加 id |
| 2 | `frontend/src/lib/platformCatalog.json` | 加平台条目 + capabilities + limitation |
| 3 | `frontend/src/lib/platforms.ts` | `FETCH_ONLY_PLATFORMS` 是否包含 |
| 4 | `frontend/src/lib/providerCatalog.ts` | `PROVIDERS` 加卡片、Base URL、接入方式 |
| 5 | `frontend/src/components/brand/PlatformLogo.tsx` | 图标映射 |
| 6 | `frontend/src/lib/authQuotaRows.ts` | 若是非 5h/7d 形态，需绕过默认占位 |
| 7 | `frontend/src/lib/quotaMap.ts` | `TONE` 加窗口 key 配色 |
| 8 | `frontend/src/lib/planCoverage.ts` | 「是否套餐」口径（目前写死 5h/7d） |
| 9 | `frontend/src/lib/dockQuotaView.ts` | 贴边条三形态（间接依赖 5h/7d） |
| 10 | `frontend/src/lib/remainingDisplay.ts` | 加整块 `if (platform === ...)` 分支与文案 |
| 11 | `frontend/src/components/island/CapsuleIsland.tsx` | `LABEL`、`ringColor`、`platform === "grok"` 特判 |
| 12 | `frontend/src/components/island/UsageIsland.tsx` | 同上的 `platform === "grok"` ×2 |
| 13 | `frontend/src/hooks/useIslandWindowModel.ts` | `localQuotaPlatform`、`grokLocalQuota`、`nativeMonitor` 等平台分支 |
| 14 | `frontend/src/hooks/useQuota.ts` | `local` 模式枚举 `boolean \| "grok" \| "zcode"` |
| 15 | `backend/src/lib.rs` | `connection_quota` 的 `match (platform, kind)` 两处 + `native` 分支 |
| 16 | `backend/src/coding_plan.rs` | `detect_provider` 域名表 + `PlanProvider` 枚举 + 解析函数 |
| 17 | `backend/src/platforms.rs` | 无（读同一份 JSON），但当 capabilities 语义变化时要同步 |
| 18 | `backend/src/provider_key.rs` / `creds.rs` | 是否允许保存该平台 API Key |

**目标（只加一个文件，共享组件零改动）**：

| 动作 | 文件 | 说明 |
|---|---|---|
| **新增（唯一必改）** | `frontend/src/lib/providers/<id>.json` | §4.5 形态的描述符，含窗口、单位、重置语义、轮询/失败、文案、能力 |
| 新增（仅当需要联网） | `backend/src/providers/<id>.rs` | 一个 `query(secret) -> QuotaState` 函数 |
| 登记（一行） | `backend/src/providers/registry.rs` | `"<id>" => Some(...)`；纯声明式供应商不登记 |
| 静态资源（可选） | `frontend/src/assets/platforms/<id>.png` | Logo 由描述符 `labels`/约定路径兜底，不再改 `PlatformLogo.tsx` |
| **不改** | `CapsuleIsland.tsx`、`UsageIsland.tsx`、`DockedIsland.tsx`、`UsageRow.tsx`、`Bot.tsx`、`quotaMap.ts`、`authQuotaRows.ts`、`dockQuotaView.ts`、`planCoverage.ts`、`remainingDisplay.ts`、`useIslandWindowModel.ts`、`useQuota.ts` | 全部改成读 `registry()` 与运行时 `confidence`；平台 id 降级为字符串，不再是联合类型 |

**迁移顺序（每步行为不变，可单独验收）**：

1. 加 `quotaDescriptor.ts` + `providers/*.json`（先把现有 11 家写成描述符），与旧常量并存；加一致性测试：对每家的既有 fixture，描述符推导出的窗口集合/配色/文案与旧函数逐项相等。
2. 把 `LABEL`、`TONE`、`ringColor` 的供应商分支、`remainingDisplay` 的平台分支改为读注册表；`PlatformId` 放宽为 `string`（保留一个 `KNOWN_PLATFORM_IDS` 常量给枚举处用）。
3. 把 `dockQuotaView` / `planCoverage` 的「5h/7d」口径换成 `countsAsPlan`；把 `authQuotaRows` 的默认占位改为「描述符声明了该窗口但本次没取到 → 占位；未声明 → 不占位」（否则新供应商会被画上假的 5h/7d 空行）。
4. 后端把 `lib.rs` 的两处 `match (platform, kind)` 收敛到 `providers::adapter(id)`，`coding_plan.rs` 的域名表迁到描述符的 `quotaSource` + host 声明（或其 adapter 内部）。
5. 删除被取代的旧分支与旧常量，留一条测试断言「新增描述符无需改共享组件」（用一个虚构 `acme-gateway` 描述符跑渲染快照）。

---

## 7. 参考链接

**模型/供应商元数据**

- models.dev 站点：https://models.dev/
- models.dev 数据接口：https://models.dev/api.json
- models.dev 仓库：https://github.com/sst/models.dev
- LiteLLM 价格与上下文规范：https://docs.litellm.ai/docs/provider_registration/add_model_pricing
- LiteLLM 价格表：https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json
- LiteLLM provider 接入（多处改动的反例）：https://docs.litellm.ai/docs/provider_registration/
- OpenRouter 模型对象字段：https://www.openrouter.ai/docs/client-sdks/typescript/models/model
- OpenRouter 供应商接入：https://openrouter.ai/docs/guides/providers/integration
- Vercel AI SDK provider 与模型管理：https://ai-sdk.dev/docs/ai-sdk-core/provider-management
- Vercel AI SDK `customProvider`：https://ai-sdk.dev/docs/reference/ai-sdk-core/custom-provider
- OpenCode provider 配置：https://opencode.ai/docs/providers/
- OpenCode 配置 schema：https://opencode.ai/config.json
- OpenCode plugin：https://opencode.ai/docs/plugins/

**额度/限流语义**

- IETF `RateLimit` 头草案（11 版）：https://datatracker.ietf.org/doc/draft-ietf-httpapi-ratelimit-headers/
- 草案纯文本（含单位注册表与现存 `x-ratelimit-*` 实践附录）：https://www.ietf.org/archive/id/draft-ietf-httpapi-ratelimit-headers-11.txt
- OpenAI 速率限制文档（本次抓取被 Cloudflare 拦截）：https://platform.openai.com/docs/guides/rate-limits
- OpenAI 用量/费用 API（同上，未取到正文）：https://platform.openai.com/docs/api-reference/usage
- Anthropic 速率限制文档（跨域跳转，未取到正文）：https://platform.claude.com/en/api/rate-limits

**各家额度查询**

- DeepSeek 查询余额（官方，**唯一确认有文档的**）：https://api-docs.deepseek.com/api/get-user-balance
- 阿里云 BSS 账户余额（RAM 签名，账户级）：https://www.alibabacloud.com/help/en/user-center/developer-reference/api-bssopenapi-2017-12-14-queryaccountbalance
- MiniMax 账户 FAQ（余额仅在控制台）：https://platform.minimax.cn/docs/faq/about-account
- xAI Management API（官方文档存在，本次抓取失败）：https://docs.x.ai/docs/key-information/using-management-api
- xAI Management API 参考（同上）：https://docs.x.ai/developers/rest-api-reference/management
- 智谱 `monitor/usage/quota/limit`、Kimi `coding/v1/usages`、MiniMax `coding_plan/remains`、OpenCode `zen/go/v1/usage`、ZCode `zcode-plan/billing/balance`：**均未找到官方文档**（控制台同源端点，实现依据见 `backend/src/coding_plan.rs` 与 `backend/src/quota.rs` 模块文档）

**插件化 / 清单先例**

- Agent Plugins 1.0 规范：https://agent-plugins.org/specification
- Agent Plugins 1.0 JSON Schema：https://agent-plugins.org/schemas/1.0.0/plugin.schema.json
- magpie 市场（同数据多清单、CI 强制封闭 schema）：https://github.com/apache/magpie/blob/main/docs/setup/marketplace.md
- MCP 架构与能力发现：https://modelcontextprotocol.io/docs/2026-07-28/learn/architecture
- MCP 规范：https://modelcontextprotocol.io/specification

**本次抓取状态**

| 链接 | 状态 |
|---|---|
| models.dev / api.json / OpenRouter 模型字段 / LiteLLM 两页 / AI SDK 两页 / OpenCode 两页 | 200，正文已读 |
| IETF datatracker 与 txt | 200，正文已读 |
| DeepSeek `/user/balance` | 200，正文已读 |
| 阿里云 QueryAccountBalance / MiniMax 账户 FAQ | 200，正文已读 |
| Agent Plugins 规范 / magpie 市场文档 / MCP 架构 | 200，正文已读 |
| `platform.openai.com` 全部页面 | 403（Cloudflare 拦截），未取正文 |
| `platform.claude.com`、`docs.claude.com` | 跨域跳转失败，未取正文 |
| `docs.x.ai` 全部页面 | 网络失败，未取正文 |

---

## 8. Jolu 现状证据索引

| 结论 | 证据 |
|---|---|
| 「套餐」口径写死 5h/7d | `frontend/src/lib/planCoverage.ts:9`（`PLAN_WINDOW_KEYS`） |
| 窗口配色按 key 写死 | `frontend/src/lib/quotaMap.ts:10-17`（`TONE`） |
| 缺窗口时补 5h/7d 空行 | `frontend/src/lib/authQuotaRows.ts:5-10` |
| 环色按 key 分支 + Grok 特判 | `frontend/src/components/island/CapsuleIsland.tsx:36,44-49,121` |
| 贴边条固定两条轨道 | `frontend/src/components/island/DockedIsland.tsx:18,83-116` |
| 贴边三形态判定 | `frontend/src/lib/dockQuotaView.ts:31-54` |
| 逐平台剩余文案 | `frontend/src/lib/remainingDisplay.ts:19-109` |
| 灵动岛平台分支与 local 额度 | `frontend/src/hooks/useIslandWindowModel.ts:75-97,150-163` |
| `useQuota` 的 local 模式枚举 | `frontend/src/hooks/useQuota.ts:61-86`（`boolean \| "grok" \| "zcode"`） |
| 后端按 (platform, kind) 分派 | `backend/src/lib.rs:1360-1411` |
| 套餐供应商按域名识别 | `backend/src/coding_plan.rs:39-58` |
| 供应商窗口命名常量 | `backend/src/coding_plan.rs:20-23`（`W5H/W7D/W30D/WCREDITS`） |
| 额度缓存 TTL 与退避 | `backend/src/lib.rs:563-566` |
| 平台能力目录（前后端共用同一份） | `frontend/src/lib/platformCatalog.json`、`backend/src/platforms.rs:14`（`include_str!` 读同一文件） |
| 胶囊额度呈现规则（≤2 圈 / 单圈 / API 不画环） | `docs/specs/2026-11-04-capsule-island-design.md:122-125` |
