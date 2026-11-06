# 灵动岛 × 供应商联动调研（2026-11-05）

**性质**：纯调研，不改代码、不动 `docs/ui/Jolu.pen`、不动 `README.md` / `docs/assets` / `output/island-prototype`，不执行 git，不启停进程。本次唯一写入就是本文档。

**要回答的问题**：在 ZCode 里用不同 API Key / 不同供应商跑会话时，灵动岛能不能说清"这是哪一家"，并且除了被动显示数字之外还能联动什么。

**方法**：

1. 读代码，确认现有可用于归因的字段、事件与推送链路（路径与行号写在正文与 §9 证据索引里）。
2. 只读查询本机 `~/.zcode/cli/db/db.sqlite`（`mode=ro`）与 `~/.zcode/v2/provider_config.json` / `credentials.json` 的结构，核实"ZCode 到底记不记得是哪家"。
3. 联网核对社区标准（OpenTelemetry、Claude Code、OpenCode、OpenAI、magpie），逐条标注"能给什么 / 缺什么"；查不到就写「未找到」。

> 安全边界：本机 `provider_config.json` 里的 `apiKey` 是明文（实测长度 49 / 67 / 35，`sk-` 前缀），`credentials.json` 里是 `enc:` 前缀密文。本文只记录"有/无、长度、前缀"这类结构事实，不记录任何凭证内容，也不建议把凭证写进库、日志或前端。

---

## 1. 现状与缺口

### 1.1 现存可归因字段

| 字段 | 位置 | 归因粒度 | 目前是否已用 | 缺口 |
|---|---|---|---|---|
| `model_usage.provider_id` | ZCode 库（`~/.zcode/cli/db/db.sqlite`） | **每条请求** | **否**，`native_sources.rs:617-624` 的 SELECT 没有取这一列 | 采集侧漏列，是本文最大的单点缺口 |
| `model_usage.model_id` | 同上 | 每条请求 | 是（`requests.model`） | 只有模型，没有"哪家" |
| `model_usage.logical_request_id` / `attempt_index` / `retry_count` | 同上 | 每条请求 / 每次重试 | 否 | 可用于区分"重试换源"，未接 |
| `model_usage.trace_id` / `span_id` / `query_source` / `agent` / `mode` / `variant` | 同上 | 每条请求 | 否 | 与 OTel / 会话归并有关，未接 |
| `message.data.providerId` / `modelId` | ZCode 库 | 每条 assistant 消息 | 是，但**只在会话线程正文里展示**（`session_threads.rs:839-840`） | 岛不读会话正文，等于岛拿不到 |
| `part.data.type`（`step-start` / `step-finish` / `tool` / `text` / `reasoning`） | ZCode 库 | 每个步骤 | 是（正文渲染、令牌结算） | 未作为"正在写"信号上报 |
| `turn_usage`（`started_at` / `first_token_at` / `completed_at` / `status`） | ZCode 库 | 每个用户轮次 | **否** | 轮次级 run/idle 的最强证据，完全未读 |
| `session.last_outcome` | ZCode 库 | 每会话 | 否 | 会话结果字段，未读 |
| `requests.platform` | Jolu `usage.db` | 每条请求 | 是 | 只是"哪个 CLI"，不是"哪家供应商" |
| `requests.source` | Jolu `usage.db` | 每条请求 | 是（如 `native:zcode:main`、`local-session`、`gateway-subscription`） | 是采集来源标记，不是凭证身份 |
| `requests.connection_id` | Jolu `usage.db`（`db.rs:326` 后加列） | **每条请求** | 是，但**只有经过 Jolu 网关 / 代理的流量才会被写上** | 直连跑会话 → 恒为 NULL → 用量页显示「未归属账号」 |
| `connections.id` / `slug` / `account_fp` | Jolu `usage.db` | 每条连接 | 是（`gateway/identity.rs:65`） | `account_fp` 只能从 Jolu 自己存的 `secret` 算，读不到别人的钥匙就算不出来 |
| `gateway_route_log`（`agent` / `requested_model` / `source` / `source` slug / 状态码 / 耗时） | Jolu `usage.db` | 每次转发 | 是（岛展开卡片的"模型入口"段） | 没有 `connection_id`、没有 `session_id`、没有 Token / 费用；只留最近 1000 行 |
| `island_platform` / `island_connection_id` / `island_clones` | `settings.rs:26-74` | 岛实例级 | 是 | 本体只能绑**一个**平台 + 一个连接；多供应商并列只能靠分身 |

### 1.2 现状链路（一条会话 → 供应商 → Key）

```
ZCode 会话 ──(Jolu 只读扫库)──> requests(platform='zcode', session_id='native:zcode:main:<裸id>',
                                         model=<model_id>, connection_id=NULL)
                              │
                              ├─ live-usage 推送 ──> 灵动岛（只有 Token / 会话 / 状态）
                              └─ 会话线程页 ──> 每条消息显示 providerId（岛拿不到）
```

- **能到"哪家"的唯一现成路径，是流量经过 Jolu 网关**：`gateway/activity.rs` 发 `gateway-route-start` / `gateway-route`，事件里带 `source`（连接 slug）；`route_log` 落库；`requests.connection_id` 由 `db::insert_gateway_record` / `apply_proxy_timing` 补写。这条链路的**局限**是：它只知道"Jolu 把请求转给了哪个连接"，而用户如果在 ZCode 里直接配了第三方 Key（不走 Jolu 网关），这条路完全没有。
- **直连跑会话时，Jolu 现在只能说"这是 ZCode 本机会话，无法按账号区分"** —— 这句就是产品自己在岛上的说明文案（`useIslandWindowModel.ts:306`），等于现状已经承认了这个缺口。
- 岛目前的"正在写"是从上一条链路的衍生物：`islandWriting.ts` 三个并列信号 = `sessions.state === 'running'` ∪ 网关 `inFlight` ∪ "最近活跃窗口内还有写入"（窗口 90s，`db.rs:1196`）。

### 1.3 本机实测（2026-11-05 22:00 前后，只读）

这一步是为了回答"ZCode 到底记不记得是哪家"，结论是**记得，而且比预期完整**：

| 观测 | 结果 |
|---|---|
| `model_usage` 行数 | 8043 |
| `provider_id` 为空的行 | **0（100% 有值）** |
| 出现过的 provider | `account:zai-start-plan` 2577、`new-provider-2` 2338、`zai-api-2` 1264、`new-provider` 1090、`deepseek` 765、`new-provider-3` 9 |
| `model_usage.status` 分布 | `completed` 7904、`error` 110、`cancelled` 29 —— **没有任何 running / pending 行**，与上一轮结论一致；代码里 `zcode_session_state`（`native_sources.rs:592`）其实留了 `running` 分支和 10 分钟宽限，但本机数据永远不会命中 |
| `turn_usage` | 415 行，`completed_at` 为 NULL 的 **0 行**，状态只有 completed / cancelled / error → **轮次级也不能当实时信号** |
| `part` 类型计数 | `tool` 22527、`step-start` 19682、`step-finish` 19439、`text` 12675、`reasoning` 11993、`file` 286、`timeline` 126、`compaction` 103 |
| `step-start` − `step-finish` | 243 条差额，全部是"只有 1 个 part 的旧消息"，最早可到几天前 → 不能单独当"正在跑" |
| 一步的耗时量级 | 最新一轮 278 秒内 13 次模型请求（单请求 9.4–11.8 秒），即**步骤粒度约 10–20 秒** |
| 库文件 | `db.sqlite` 476 MB，`-wal` 4.3 MB 常驻；`db.sqlite` mtime 18:34、`-wal` 18:36（与最后一条 turn 完成时刻对齐）、`-shm` 22:06（是 Jolu 自己读库触碰的，`watcher.rs:87-90` 已排除） |
| `~/.zcode/v2/provider_config.json` | `config.providerOrder` 7 项，`providerRules` 是**列表**（每项 `providerId` / `templateId` / `providerName` / `config.access.type` / `config.access.apiKey`（明文）/ `config.api.type`（`anthropic-messages` / `openai-responses`）/ `config.api.baseUrl` / `personalModelIds` / `modelOrder`） |
| `~/.zcode/v2/credentials.json` | 键名形如 `account-provider:coding-plan:account:<uuid>:api-key`、`project-key-location:v1:%5B%22https%3A%2F%2Fapi.z.ai%22...%5D`，值一律 `enc:` 前缀（密文） |

**由此得到的第一个硬结论**：`provider_id` 已经是**逐请求**、100% 覆盖的字段，Jolu 只差一列就能回答"这次请求打的是哪家"。同一平台多账号（例如两把 Anthropic 兼容 Key）在 ZCode 里本来就是两个不同的 `providerId`，天然可分。

---

## 2. 可接的社区标准

判断标准只有一条：**它能不能回答"现在这一下是哪个供应商 / 哪把 Key 在被调用"**，而不只是 Token 数。

| 标准 | 链接 | 能给什么 | 缺什么（针对本需求） |
|---|---|---|---|
| OpenTelemetry GenAI semconv | [gen-ai.md 属性注册表](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/registry/attributes/gen-ai.md) | 明确的逐请求属性：`gen_ai.provider.name`、`gen_ai.request.model`、`gen_ai.response.model`、`gen_ai.usage.input_tokens` / `output_tokens` / `cache_read.input_tokens` / `cache_write.input_tokens` / `reasoning.output_tokens`、`gen_ai.conversation.id` | **只给"供应商类型"，不给"哪把 Key"**。规范自己写明 `gen_ai.provider.name` 取"instrumentation 的最佳认知，可能与真实上游不一致（例如客户端配在代理/托管平台后面）"，且 `gen_ai.conversation.id` 明确"不得当作实时会话标识"使用（属性注册表注释 [22]、[4]）。没有 key id / 账号 id 类属性 |
| Claude Code hooks | [Hooks reference](https://code.claude.com/docs/en/hooks) | 生命周期事件齐全：`SessionStart` / `SessionEnd`、`UserPromptSubmit`、`Stop` / `StopFailure`、`PreToolUse` / `PostToolUse`、以及 **`PreModelSwitch` / `PostModelSwitch`**；公共输入字段含 `session_id`、`transcript_path`、`cwd` | 公共输入字段里**没有 provider / key**；`PreModelSwitch` 只覆盖模型切换，不覆盖换 Key。只能当"新会话开始 / 轮次结束 / 模型切换"的时机信号 |
| Claude Code OpenTelemetry | [Monitoring](https://code.claude.com/docs/en/monitoring-usage) | `claude_code.api_request` 等事件带模型与 Token；标准属性含 `session.id`、`organization.id`、**`user.account_uuid` / `user.account_id`**（与 Anthropic admin API 同格式）；可经 `OTEL_RESOURCE_ATTRIBUTES` 追加自定义键 | `gen_ai.system` 文档写明**恒为 `anthropic`** → 走第三方 base URL 时它依然说 anthropic，无法区分"是哪家代理 / 哪把 Key"。`user.account_id` 只能区分**账号**，区分不了同一账号下的多把 Key |
| OpenCode server / plugins | [Server API](https://opencode.ai/docs/server)、[Plugins](https://opencode.ai/docs/plugins) | `GET /event`（SSE 总线，首个事件 `server.connected`）、`GET /global/event`、**`GET /session/status`（逐会话状态）**；消息对象本身带 `providerID` / `modelID`（`/session/:id/init`、`/summarize` 的请求体就是 `{ providerID, modelID }`）；插件可订阅 `message.updated`、`session.idle` 等 | 服务端事件的具体 payload 字段清单本轮**未找到**（文档只给 OpenAPI 与 `types.gen.ts` 链接，需按类型文件核对才敢断言 providerID 是否随事件下发）；且这只对 OpenCode 生效，ZCode 没有同类文档 |
| OpenAI Admin Usage API | [usage_completions_params.py](https://github.com/openai/openai-python/blob/main/src/openai/types/admin/organization/usage_completions_params.py) | `group_by` 支持 `project_id` / `user_id` / **`api_key_id`** / `model` / `batch` / `service_tier` / `api_source`，并可用 `api_key_ids` 过滤；`bucket_width` 1m/1h/1d | 需要**组织级 admin key**；是**分桶报表**（非实时，默认按天），不能驱动"现在正在写"；只覆盖 OpenAI 自己的 Key，覆盖不到第三方中转 |
| Anthropic Usage & Cost Admin API | — | **未找到**本轮可核对的字段说明（`docs.claude.com` / `platform.claude.com` 都跨域跳转到 `anthropic.com`；SDK `api.md` 里只见 `organization.api_keys` / `external_keys` / RBAC 等资源，没有 messages 用量报表资源） | 无法确认是否支持 `api_key_id` 分组。**落地前必须按官方页面重新核对，不得先写进实现** |
| magpie | [reference.md](https://github.com/yetone/magpie/blob/main/docs/reference.md)、[README](https://github.com/yetone/magpie) | 直接可抄的产品判断：**"Usage → Overview → Gateway keys 按客户端 key 归类，而不是按供应商凭证"**，`Usage → Requests` 同一筛选；用量从本地 usage log 读所以能跨重启；`GET /v1/magpie/route` 可选路预览（含 `tier` / `tier_served`）；额度用尽自动切下一个账号；会话带成本与标题、可一键在终端重开 | 它解决问题的方式是**让流量必须经它**（改各 Agent 的 base URL）。对"用户在 ZCode 里直接配 Key、根本没经过 Jolu"的场景无效；名字里带 activity 的事件流本轮**未找到**，能核到的只有上述 HTTP 端点与 Usage 页面 |

**第二个硬结论**：没有任何一个公开标准能直接告诉你"这是哪把 Key"。能回答"哪把 Key"的只有三类证据：

1. **网关侧自己签发的 key / 自己转发的连接 id**（magpie 的 Gateway keys 做法，Jolu 已有 `connection_id`）；
2. **官方 admin 用量报表里的 `api_key_id`**（OpenAI 有，Anthropic 未核对到；都滞后）；
3. **本地应用自己的 provider 配置**（ZCode 的 `provider_id` + `provider_config.json`，最实时、最贴本需求）。

所以归因链路必须做成"三源合并"，而不是指望接一个标准就够了。

---

## 3. 「一条会话 → 供应商 / 账号」归因链路设计

### 3.1 目标模型

```
Request（一次模型调用）
  └─ Session（会话）        ← 已有：requests.session_id / sessions 表
       └─ Provider（哪家供应商，如 zai-api-2 / new-provider / deepseek）
            └─ Credential（哪把 Key / 哪个订阅账号）
                 └─ Connection（Jolu 的连接 id，用于额度、余额、筛选、跳转）
```

四段里第 1、2 段已经有了；缺的是 3、4 段的落库与合并。

### 3.2 三条链路（按实时性排序）

**链路 A（已有，最强但覆盖面窄）：网关 / 代理归因**

- 数据来源：`gateway/activity.rs` 事件（`source` slug、`agent`、`requested_model`）、`gateway_route_log`、`requests.connection_id`。
- 覆盖面：**只有经过 Jolu 网关的流量**（含 CLI 被接线到网关、以及 Jolu 代理补写 `dedup_key` 命中的本机记录，见 `gateway/adapter/mod.rs` 的 `apply_proxy_timing`）。
- 延迟：0（请求开始即发 `gateway-route-start`）。
- 缺口：没有 `connection_id` 进事件（只有 slug）、没有 `session_id`、没有 Token。

**链路 B（新增，最贴本需求）：ZCode 本机库 provider 归因**

- 数据来源：`model_usage.provider_id`（逐请求、100% 覆盖）+ `~/.zcode/v2/provider_config.json`（`providerName` / `api.type` / `api.baseUrl` / `access.type` / `apiKey`）+ `credentials.json`（`enc:` 密文，仅用于识别"这是账号型 provider"）。
- 延迟：与现有扫描一致（写入后几十毫秒 → 90 秒内可见；见 §5 关于"为什么这仍是滞后信号"）。
- 覆盖面：**直连跑会话也有**，这是它相对链路 A 的核心价值。

**链路 C（可选，补充跨机器/跨工具）：官方 admin 用量**

- 数据来源：OpenAI `/organization/usage/completions`（`group_by=api_key_id,model`）。
- 只能在"用户愿意存 admin key"时启用；延迟按小时/天；用途是**校验 + 兜底**，不能当实时信号。

### 3.3 链路 B 的具体设计（不改代码，只给落点）

1. **采集**：`native_sources::zcode()` 的 SELECT 增加 `u.provider_id`（`native_sources.rs:617-624` 现在选的 16 列里没有它），并把它写进 `requests`。有两种落点，建议后者：
   - 粗暴法：把 `provider_id` 拼进 `model`（`<provider>/<model>`），复用现有字符串列 —— 便宜但污染模型名，筛选/计价都会踩坑，**不建议**。
   - 正规法：新增列 `requests.provider`（TEXT，可空）+ 索引 `(platform, provider, ts)`；历史行迁移后为 NULL，界面按"未归属"表达，符合仓库既有约定（未知 ≠ 零 ≠ 失败）。
2. **解析**：后端只读 `provider_config.json`，产出 `providerId → { name, api_type, base_host, access_type }` 映射；`baseUrl` 只保留 host（如 `api.z.ai`），**不保留完整 URL、不保留 key**。
3. **指纹匹配**（把 provider 接到 Jolu 的连接上）：
   - 若 `access.type` 是 api-key 且 key 明文可读 → 复用 `gateway/identity.rs:65` 的 `account_fingerprint(platform, kind, secret)` 直接算 `account_fp`，与 `connections.account_fp` 对上即"就是这条连接"；
   - 对不上 → 标"非 Jolu 管理的来源"，显示 `providerName` + `base_host`，**不猜**它等于哪条连接。
4. **对外只暴露安全字段**：`{ providerId, providerName, apiType, baseHost, fp8 }`，其中 `fp8` 是 `account_fp` 前 8 位，仅用于前端"同账号"分组显示；key 原值与完整指纹不出后端。
5. **降级**：`provider_id` 在 `provider_config.json` 里找不到（本机实测存在这种可能：`model_usage` 里的 `account:zai-start-plan` 不在 `providerRules` 的 7 项里）→ 显示原始 id + "配置里未找到"，仍可参与分组，不参与连接匹配。

### 3.4 多账号 / 同平台如何区分

| 判据 | 能区分什么 | 证据来源 | 强度 | 备注 |
|---|---|---|---|---|
| ZCode `provider_id` | 同一平台的两把 Key、两个自定义供应商 | `model_usage.provider_id`（本机 6 个不同值） | 强（逐请求） | 用户自定义命名（`new-provider` / `new-provider-2`）语义弱，需要 `providerName` 补充 |
| `account_fp`（key 指纹） | 跨客户端"同一把 Key" | `identity.rs:65`，需能读到 key 明文 | 强 | 只能匹配 Jolu 自己存过的同一把 key |
| `baseUrl` host | 同 provider 类型下的不同中转站 | `provider_config.json` 的 `config.api.baseUrl` | 中 | 只到"哪个域名"，同域多账号区分不了 |
| 官方 `api_key_id` | 同组织内多把 Key | OpenAI admin usage（Anthropic 未核对到） | 强但滞后 | 需 admin key |
| 网关 `connection_id` / slug | Jolu 视角的"哪个连接" | `requests.connection_id`、`route_log.source` | 强（逐请求） | 只对经网关的流量成立 |
| `user.account_id`（Claude Code OTel） | 同机多账号订阅 | monitoring-usage 标准属性 | 中 | 需开遥测；不含 key |
| 会话内启发式（"这个会话一直用 provider X"） | 减少重复请求开销 | `session_id × provider_id` 直方图 | 弱（仅缓存） | 一个会话中途换供应商是真实存在的，只能当缓存不能当事实 |

**冲突处理顺序**（写死，避免以后各说各话）：`网关 connection_id` > `key 指纹匹配` > `provider_id` > `baseUrl host` > 会话启发式缓存。前两者冲突时不合并，按"两个来源并存"展示（例如"Jolu 连接 A / ZCode provider B"），不改写任何一方。

### 3.5 另一条产品级的简化路线（值得写出来，供决策）

magpie 的答案是"把 Key 归类到客户端 key 上，而不是供应商凭证"。对应到 Jolu：如果用户在 ZCode 里把 base URL 指到 Jolu 网关，那 `connection_id` 就自动有了，链路 B 完全不需要。**代价**是要求用户改 ZCode 配置（Jolu 已有接线层可以做这件事，见 `gateway/wiring.rs`）。所以 §6 里要问的不是"要不要做链路 B"，而是"要不要同时支持两条路"。

---

## 4. 联动交互清单（按实现成本排序）

成本口径：**S** ≈ 只改前端 + 一个推送字段；**M** ≈ 后端加列/聚合 + 新事件；**L** ≈ 新页面意图 + 跨窗口状态 + 写操作。每项都写数据来源、需要的后端事件、UI 变更、失败与降级。

### P0（先做，都是"现有数据就能出结论"的）

| # | 交互 | 成本 | 数据来源 | 后端事件 | UI 变更 | 失败 / 降级 |
|---|---|---|---|---|---|---|
| 1 | **岛上标出"这一下是哪家"**：胶囊尾部与展开卡片的会话行显示 `providerName`（`zai-api-2` → "Z.ai Coding Plan 2"）+ base host | M | 链路 B（`model_usage.provider_id` → `provider_config.json`） | `live-usage` 的 `active_sessions[]` / `delta_sessions[]` 增加 `provider` 对象 | 会话行右侧一行小字；胶囊收缩态不加（空间不足），只在展开态与 tooltip | provider 解析失败 → 显示原始 id + "来源未知"；`provider_config.json` 读不到 → 整段隐藏，不显示空格子 |
| 2 | **点岛跳到对的地方**：点会话 → 主窗口「会话线程」定位该会话；点"用量" → 「用量统计」按 `platform + connection + 今天` 预筛 | M | 现有页面 + `usageFilters.ts`（已有账号筛选） | 新 `MainIntent::Sessions { session_id }` / `MainIntent::Usage { platform, connection_id, range }`；`api.ts:441` 的 `open_main_section` 目前只接受 `gateway\|island\|connections`，需要扩 | 胶囊展开卡片每条会话可点；底部加"用量"链接 | 会话已删 → 落到会话列表首页并提示；连接已删 → 用量页只带平台筛选 |
| 3 | **今日按来源分解**（"今天这 5 家各花了多少"）：展开卡片里 3–5 行小表 | M | `requests`（`platform` + 新 `provider` 列）+ 现有今日区间 | 无新事件（随展开卡片查询拉一次，可用 `live-usage` 的 `data_changed` 触发重查） | 展开卡片新增一段，限高 5 行 + "查看全部"跳用量页 | 归因率低时标"n/m 条已归属"，不把未归属并入任何一行 |
| 4 | **"正在写"改成三档可信度**：确证 / 推测 / 不知道，而不是一个布尔值 + 眼睛变绿 | M | §5 的判据组合 | `live-usage` 增加 `writing: { level, evidence, since_ms }` | 眼睛颜色三态（绿 = 确证、青 = 推测、灰 = 不知道）；tooltip 写判据 | 严格模式下会"该亮不亮"，必须让用户能一键回到宽松档 |

### P1（联动，需要新事件或跨窗口状态）

| # | 交互 | 成本 | 数据来源 | 后端事件 | UI 变更 | 失败 / 降级 |
|---|---|---|---|---|---|---|
| 5 | **岛内切换"当前关注的供应商"**：现有下拉只能切平台+连接（`IslandConnectionSwitcher.tsx`），扩展成"关注哪条连接 / 哪个 provider" | M | `settings.island_connection_id` + 链路 B 的 provider 列表 | `settings-changed`（已有） | 下拉分组："Jolu 连接"与"ZCode provider"两组 | provider 无对应连接 → 只作为"只看这家"的显示筛选，不触发额度查询 |
| 6 | **多供应商并列**：本体只显示"主关注"，其余用分身（`island_clones` 已支持 per-connection），并提供"按最近活跃自动选主" | L | `live-usage` 各平台 + 分身设置 | `settings-changed` + 新增 `island-focus` | 卡片顶部加"主关注"标记；右键菜单加"以此分身为主" | 分身数量上限；自动选主可关 |
| 7 | **阈值 / 事件短时动画**：额度 80%、用尽、网关拒绝（429/402）、新会话开始 → 岛上 1.5 秒动效 | M | 额度查询结果（已有）、`gateway-route` 事件（已有）、会话新出现（`live-usage`） | 新增 `island-signal { kind, connection_id, provider, at }`；或前端从现有事件本地合成 | 复用 `IslandMotion` / `dockPulse`；动效不打断 Token 轮播 | `dnd` / 减少动效开启 → 只改颜色不播动画；信号洪泛 → 相同 kind 5 秒内合并 |
| 8 | **自动化"等额度"接进岛**：显示"N 个任务在等额度"，点击进自动化页筛选；额度恢复后回到正常 | M | `automation_runs.status = 'waiting'`、`apply_account_check`（已有） | 新增 `automation-waiting { count, connection_ids }`（AGENTS.md 里"自动化等额度表情未接数据源"就是这块） | 胶囊右侧气泡（与现有"≥2 会话气泡"并列，需要设计稿确认优先级） | 自动化页未打开时也照常统计；计数为 0 时整块不渲染 |
| 9 | **双向联动**：主窗口当前页 → 岛显示什么（例如在看"用量·某连接"时，岛临时切到该连接）；岛点开 → 主窗口跳转 | L | 主窗口页面状态（前端即可）+ 设置项"跟随主窗口" | 新增 `main-context { page, connection_id?, session_id? }` | 卡片顶部显示"跟随中：用量 · 连接 A"并提供"解除" | 不跟随（默认关）时行为与今天一致 |

### P2（价值明确但优先级靠后）

| # | 交互 | 成本 | 数据来源 | 备注 |
|---|---|---|---|---|
| 10 | 岛内直接"暂停 / 续跑"任务 | L | `automation.rs` 写操作 | 写操作 + 岛是小窗口，误触风险高；建议只做"跳转"（#8），不做就地操作 |
| 11 | 多 provider 并列进度环 | L | 额度 + 链路 B | 需要一个 provider 的额度来源；ZCode 的第三方 Key 多数没有额度接口，只有余额 |
| 12 | 接 Claude Code OTel（拿 `user.account_id` 做账号级区分） | L | monitoring-usage 的 OTLP | 需要用户开遥测 + 本地收 OTLP；对"哪家"帮助有限（`gen_ai.system` 恒为 anthropic） |
| 13 | 接 OpenAI admin usage 做校验 | L | 链路 C | 需存 admin key（敏感）、按天滞后 |

**排序理由**：#1–#3 是"用户说的'不只是一堆数字'"的最小可感知集合，且全部基于已经存在的本地数据；#4 是让"眼睛变绿"这件事从"可能是假的"变成"可信"；#5–#9 才是真正意义上的联动，但每一项都要么需要新事件、要么需要新的主窗口意图。按仓库既有规范，涉及主窗口跳转的都要先有 `Jolu.pen` 画板，属于"待确认"。

---

## 5. 误报抑制：判据组合与代价

先纠正一个容易搞错的方向：**现在这个 90 秒派生信号的主要问题是漏报，不是误报。** 因为 ZCode 只在请求结束时落 `model_usage` 行（本机 0 条 running），所以"点灯"发生在**每次请求完成之后**；一轮 278 秒里 13 个请求、单请求约 10 秒，正常节奏下灯几乎一直是亮的，但当某个步骤超过 90 秒（长思考、长工具调用、大文件读写）时，灯会在这 90 秒里**灭掉**，然后下一个请求完成又亮 —— 用户看到的"闪断"就是这个。误报方向则来自另外两件事：后台子代理会话（`workflow_child` / `subagent_child`）也在写同一个库；以及将来若改用"文件 mtime"当判据，用户**只是在 ZCode 里翻历史**也可能写库（同样的隐患存在于 `tasks-index.sqlite`，它是另一个文件，必须显式排除）。

| 判据 | 延迟 | 误报 | 漏报 | 成本 | 权限 / 风险 | 结论 |
|---|---|---|---|---|---|---|
| 现状：最近活跃 90s（`db.rs:1196`） | 一次请求完成后 ≤90s 才灭 | 中（后台/子代理写入、翻历史时若有写库） | **高**（单步 >90s 就灭灯） | 0（已有） | 无 | 保留为**兜底**，不作主信号 |
| `max(model_usage.completed_at)` 距今 | 0（写库即到） | 低 | 高（同现状，因为行只在结束时出现） | S（已有数据） | 无 | 与现状等价，只是更精确的"上次完成时刻" |
| `max(part.time_created)` / `part` 行数增量（轮询 2s） | ≤2s | 低—中（`timeline` / `compaction` / `file` 部件也会写） | 低（若行在轮内落库；**本机无法静态验证**，见 §7） | S—M（读一行或一次 count） | 无 | **推荐主信号**，需排除非 assistant 部件 |
| `turn_usage` 有 `started_at` 且 `completed_at` 为 NULL | 0 | 极低 | 本机 0 命中 → 等于没有信号 | S | 无 | **加分证据**，不能单独用 |
| `step-start` 无对应 `step-finish` | 0 | 极低（方向正确） | 本机 243 条差额全是旧消息 → 目前等于没有信号 | S | 无 | 同上 |
| `-wal` 的 size/mtime 变化（已用于扫描触发，`native_sources.rs:331-345`） | ≤2s（watcher 已监听，`watcher.rs:23` 去抖 120ms） | 中（任何写库都会改 WAL：todo、permission、setting、子代理） | 低 | S（已有 `scan_state` 戳记，只需上报） | 无 | 作**触发**，不作判据本身；必须排除 `-shm`（已排除）与 `tasks-index.sqlite` |
| 进程 CPU / 句柄（ZCode 主进程） | 1–5s | 低（idle 时 CPU≈0） | 低 | M—L | **需用户同意读进程信息**；Windows 需 `OpenProcess`，`session_presence.rs:64-86` 已有"只查询句柄、不注入"的先例 | 只给**严格模式**用，默认关 |
| 文件锁（`.lock`）探测 | 0 | — | — | — | — | **不适用**：ZCode 没有锁文件（`session_presence.rs` 只对 claude / codex 生效） |
| 用户显式"严格模式" | — | 低 | 中（宁可不亮） | S | 无 | 作为开关：只认"turn 未完成 / 行增量 / 网关在途"，并写明"可能不亮" |

**推荐实现（三档，不合并成布尔）**：

```
确证（绿）  = 网关 inFlight>0
            ∨ (turn_usage 最新行 completed_at IS NULL)
            ∨ (part/message 行数在最近 2 次轮询内增加，且增量含 assistant text/reasoning/tool/step-start)
            ∨ (step-start 无 step-finish，且该消息属于最新会话)
推测（青）  = max(part.time_created) 距今 ≤ 一个步骤量级（本机实测 10–20s，取 45s 保守）
            ∨ max(model_usage.completed_at) 距今 ≤ 90s（现状口径）
不知道（灰）= 采集源出错 / 库读不到 / provider 未知
```

严格模式下只保留"确证"两支，其余显示灰色并在 tooltip 说明"严格模式：没有确证不点灯"。宽松模式（默认）三档都亮。这样"眼睛变绿"这件事有了可以解释的证据链，而不是一个猜出来的布尔值。

---

## 6. 需要拍板的问题

1. **是否允许后端只读 `~/.zcode/v2/provider_config.json`？** 该文件里 `apiKey` 是明文。允许的话，读取范围限定为"算指纹 + 取 providerName / base host"，指纹只在内存与库内以 `account_fp` 形式存在，key 原值不落库、不写日志、不出后端 —— 这条要不要写进 AGENTS.md 的凭证条款？
2. **`requests` 加一列 `provider`（可空）是否可接受？** 涉及 `usage.db` 结构变更（需可恢复迁移，历史行为 NULL）。另一个选项是"只在内存/缓存里做归因，不落库"，代价是重启后无法回溯"昨天那 5 家各多少"。
3. **"最近活跃 90s"是否降级为兜底？** 即默认改成"行增量 + turn 状态"为主、90s 为辅。这会改变眼睛点灯的时机（更快灭、更快亮），需要你认可。
4. **是否允许读进程信息**（CPU / 句柄，只查询不注入）来做严格模式？默认关、需要用户在设置里显式打开，可以吗？
5. **岛要不要显示"多家并列"？** 现成本体只能绑一个平台+一个连接（`settings.rs:26-74`），并列只能靠分身。是"本体只显示主关注 + 分身显示其他"，还是要在本体卡片里并列多行？
6. **是否需要新的主窗口跳转意图**（`MainIntent::Usage { platform, connection_id, range }` / `Sessions { session_id }`）？这会让岛能"点开就到对应筛选"，但涉及主窗口接收参数，按规范要先有 `Jolu.pen` 画板 —— 需要你确认是否进入设计稿。
7. **官方 admin usage（OpenAI `api_key_id`）是否要接？** 需要组织级 admin key（比普通 key 敏感得多），且按天滞后；不接也不影响链路 B。
8. **阈值动效默认开还是关？** 现有 `dnd` 与"减少动效"两个开关，默认值需要定。
9. **ZCode provider 名字的展示口径**：`new-provider` / `new-provider-2` 这类用户自定义名要不要原样显示（会不会比"未命名来源 2"更糟）？
10. **是否需要把"这条会话走的是 Jolu 网关还是直连"作为岛上的一等信息**？（这决定了链路 A 与 B 的优先级：走网关时 `connection_id` 可靠，直连时只有 provider。）

---

## 7. 未找到 / 未验证（不得当成已确认）

| 事项 | 状态 |
|---|---|
| Anthropic Usage & Cost Admin API 是否支持 `api_key_id` 分组 | **未找到**（官方文档站跨域跳转，SDK `api.md` 无该资源） |
| OpenCode `message.updated` 等 SSE 事件是否携带 `providerID` / `modelID` | **未找到**（文档只给 OpenAPI / `types.gen.ts` 链接，字段清单未核对） |
| ZCode 是否存在"activity 事件"流或本地 HTTP 端点 | **未找到**（`~/.zcode` 下只有库、日志、配置目录；未发现对外端点文档） |
| ZCode 是否为 OpenCode 分支 | **推断，未证实**：消息/部件模型高度同构（`providerId`/`modelId`、`step-start`/`step-finish`/`tool`/`reasoning`/`file`/`compaction` 部件），但没找到官方声明 |
| `part` / `message` 行是否在**轮次进行中**就落库（决定"行增量"判据的延迟与漏报率） | **未验证**：本轮不允许起进程，且采样时本机没有进行中的轮次（最后一条 turn 3.5 小时前）。需要下一轮真机开一轮 ZCode 会话、2 秒轮询 `select count(*) from part` 才能定论 |
| 其他 ZCode 版本/安装的 `provider_config.json` 是否也明文 | **未验证**（只验证了本机这一份） |
| magpie 名为 "activity" 的事件流 | **未找到**；能核到的是 `GET /v1/magpie/route`、`/v1/magpie/quotas/cards`、`/v1/magpie/limit` 与 Usage 页面 |

---

## 8. 结论（5 条）

1. **归因不是从零做**：ZCode 的 `model_usage.provider_id` 逐请求、100% 覆盖，本机 6 个不同 provider 并存，Jolu 只差"采集少选一列 + 一张 provider 名映射"就能说出"哪家"。
2. **"哪把 Key"没有现成标准可接**：OTel 的 `gen_ai.provider.name` 规范自己承认可能与真实上游不一致且不含凭证；Claude Code 的 `gen_ai.system` 恒为 `anthropic`；Claude hooks 的公共字段里没有 provider / key。能回答 Key 级问题的只有三类证据：网关侧连接 id（已有）、官方 admin 报表的 `api_key_id`（OpenAI 有、Anthropic 未核对到、都滞后）、本地应用自己的 provider 配置（ZCode 最实时）。
3. **岛和主窗口之间已经有一条"半个"联动**：`api.openMainSection` 只认 `gateway|island|connections`，`MainIntent` 已有"带参数跳转"的先例（`Providers{connection_id}` / `Automation{task_id}`），扩成 `Usage` / `Sessions` 是低风险增量；岛目前只能"打开模型入口"，不能"跳到这家供应商的用量"。
4. **"最近活跃 90s"应该降级为兜底**：它的实际问题是长步骤（>90s）时灯灭的漏报，以及后台子代理/翻历史带来的误报；更好的组合是"网关在途 / turn 未完成 / part 行增量（2s 轮询）"为主，90s 为辅，并且把结果做成"确证 / 推测 / 不知道"三档而不是布尔值。
5. **两个必须由你拍板的前置条件**：读 `provider_config.json`（明文 key，只用于算指纹）是否允许，以及 `requests` 加 `provider` 列（结构变更 + 迁移）是否接受。这两个不点头，链路 B 只能退化成"岛内显示 providerName 但不落到统计与筛选"，联动能力会少一大半。

---

## 9. 证据索引

| 结论 / 事实 | 证据位置 |
|---|---|
| 岛上的"正在写"三信号与 90s 兜底 | `frontend/src/lib/islandWriting.ts:26-30`、`frontend/src/hooks/useIslandWindowModel.ts:182-189` |
| 产品自认"无法按账号区分" | `frontend/src/hooks/useIslandWindowModel.ts:306` |
| `live_usage` 只对 codex / claude 走生命周期，其余用 90s 窗口 | `backend/src/db.rs:1196`、`backend/src/db.rs:1357-1361`、`backend/src/db.rs:1262` |
| ZCode 采集未取 `provider_id` | `backend/src/native_sources.rs:617-624` |
| ZCode 状态推导与 10 分钟宽限 | `backend/src/native_sources.rs:588-599` |
| 外部 SQLite 以"主库 + WAL"戳记判变更 | `backend/src/native_sources.rs:330-345` |
| watcher 去抖 120ms、排除 `-shm` | `backend/src/watcher.rs:23`、`backend/src/watcher.rs:87-90` |
| 消息级 `providerId` 已在解析，但只在会话正文 | `backend/src/session_threads.rs:839-840`、`backend/src/session_threads.rs:764-772` |
| 网关事件字段（无 connection_id / session_id / token） | `backend/src/gateway/activity.rs:19-47` |
| 路由流水字段与 1000 行上限 | `backend/src/gateway/route_log.rs:8-19`、`backend/src/gateway/route_log.rs:46-54` |
| 账号指纹算法 | `backend/src/gateway/identity.rs:63-71` |
| `requests.connection_id` 存在且可为空 | `backend/src/db.rs:326`、`backend/src/usage_stats.rs:75-86` |
| 主窗口跳转意图与既有参数化先例 | `backend/src/lib.rs:439-448`、`frontend/src/lib/api.ts:441`、`frontend/src/lib/mainNavigation.ts:57-73` |
| 岛只能绑一个平台 + 一个连接（并列靠分身） | `backend/src/settings.rs:26-74`、`backend/src/island_clones.rs:25-38` |
| 岛内已有连接切换下拉 / 打开模型入口按钮 | `frontend/src/components/island/IslandConnectionSwitcher.tsx:29-56`、`frontend/src/components/island/CapsuleGateway.tsx:82`、`:130-133` |
| 余额阈值通知已有（可复用为岛上信号源） | `backend/src/notify.rs:116-130`、`backend/src/notify.rs:206-215` |
| 自动化"等额度 / 续跑"机制已有，但没有给岛的计数接口 | `backend/src/automation.rs:1455-1465`、`backend/src/automation.rs:1721-1745`（`lib.rs` 中无 automation 计数命令） |
| 社区标准（链接见 §2） | OTel GenAI 属性注册表、Claude Code Hooks / Monitoring、OpenCode Server / Plugins、OpenAI admin usage 参数、magpie reference |
