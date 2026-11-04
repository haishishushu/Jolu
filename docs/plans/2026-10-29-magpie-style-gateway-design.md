# 账号汇聚网关 · 执行开发文档（2026-10-29 初稿 · 2026-11-01 按重新研究修订）

**交给：鼠鼠**
**状态：主体已实现并随 v0.1.6 发布；本文保留历史方案，不再整体视为待实施。当前实现范围以 `docs/specs/2026-11-03-gateway-dedup-and-wiring.md` 为准。**

2026-11-04 核对：来源/Agent 分组、接线去重与路由流水已有实现；路由组、故障转移仍未开始。文末旧复选框不是当前完成状态。真机验证仍缺记录，本轮仅检查 CLI 命令和配置路径是否存在，不证明账号有效或接线请求成功。
**参考实现：https://github.com/yetone/magpie（MIT，Go；仅作设计参考，不复制代码）**

## 0. 一句话说明

把 Jolu 已登记的本机凭证——`connections` 表里的官方订阅（发现/核实的 auth 连接）与 API Key（`api` 连接）——统一挂到一个本机网关后面。各 agent 把 base_url 指向网关后，模型选择器里出现所有凭证背后的模型（形如 `kimi/kimi-k2`、`codex/gpt-5.5`）。**统一接入的层是网关；MCP 桥只是 Claude 订阅适配器的内部零件，不承担通用接入。**

## 1. 修订说明：初稿错在哪里

重新研究 magpie（稀疏克隆读码 + reference.md）后确认了四件事，本版据此重写：

1. **`relay_providers` / `relay_accounts` 两张新表废弃。** Jolu 的 `connections` 表已经是 provider 池：`kind='auth'` 就是订阅账号、`kind='api'` 就是 Key 连接，`base_url` / `protocol` / `secret` / `model` 字段一样不缺。网关只需要一个「是否对外服务」开关。
2. **OAuth 令牌不得注入 `api.anthropic.com`。** Anthropic 把其他 agent 的系统提示词归类为第三方流量，即使请求头看起来和 Claude Code 完全一样。magpie 因此对每次 Claude 订阅生成**驱动真实的本机 `claude` 二进制**（用它自己的登录），调用方的工具经 stdio MCP 桥进到那场会话。初稿 §5 的注入方案整体作废。
3. **四协议互转矩阵降级。** `connections.protocol` 字段已经声明每个连接说 anthropic 还是 openai；Kimi / GLM / ZCode / DeepSeek 都提供两种协议的原生端点（ZCode 目录里的 `baseUrlChoices` 就是证据）。**主路径是同协议直通**，跨协议配对返回明确错误引导用户换端点。唯一必须做转换的是 Codex 订阅适配器（ChatGPT 后端只收流式、拒部分参数）。
4. **magpie 的工程重心在接线层，不在网关。** `internal/agent/` 132 个文件 47,830 行（每个 agent 一个适配器：读凭证、改配置、断开恢复），`internal/gateway/` 82,556 行。Jolu 要补的主要缺口也是接线层，不是转发本身。

## 2. 实施前必读

### 2.1 铁律

| 纪律 | 本功能的态度 |
| --- | --- |
| `proxy.rs`「不注入凭证：请求头透传」 | **仅对反代模式废除**。观测模式保持透传；网关模式改写凭证，两条数据路径不共用 `forward()` |
| `AGENTS.md` 密钥不入日志 / DTO 只带 masked | 继续遵守。网关持有的凭证只在后端内存与 `connections` 表 |
| `cli_apply.rs` 写前备份 + `write_atomic` + 全局切换锁 | **作为接线层的模板原样延伸**到所有 agent |
| `proxy_config.rs`「还原只清理自己的写入，外部意图优先」 | 原样延伸到接线层（magpie 的 `Unwire`/`Restore` 同义） |
| `AGENTS.md` 数据结构变更需可恢复迁移 | 见 §4 |
| 官方订阅直连不动（`probe_original` 的前提） | **仅网关模式放开**；观测模式行为不变 |

### 2.2 前置条件

本机当前无 Rust 工具链（VS 2022 Community 在但缺 MSVC 编译器与 Windows SDK）。先装 `rustup`（选 x86_64-pc-windows-msvc）+ Build Tools 的「使用 C++ 的桌面开发」工作负载，自检：

```bash
rustc -V   # >= 1.90（backend/Cargo.toml 的 rust-version）
cargo test --manifest-path backend/Cargo.toml   # 基线全绿再开工
```

### 2.3 开工前基线

```bash
pnpm --dir frontend typecheck
node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs   # 全绿
cargo test --manifest-path backend/Cargo.toml                           # 全绿
```

## 3. 目标与不做的事

### 3.1 本期要做

1. **网关本体**：与观测模式共用监听端口（`settings.proxy_port`，默认 12731），新增 `/v1/messages`、`/v1/chat/completions`、`/v1/responses`、`/v1/models` 四个入口
2. **连接适配器**：`kind='api'` 连接按 `protocol` 字段同协议直通（换 base_url、凭证头、model 名）
3. **Claude 订阅适配器**：驱动真实 `claude` 二进制 + stdio MCP 工具桥（§7）
4. **Codex 订阅适配器**：凭本机 `auth.json` 的令牌调 ChatGPT 后端，非流式转流式、丢弃后端拒绝的参数（§8）
5. **接线层**：把各 agent 的配置指向网关（Claude Code / Codex 改造现有 `cli_apply.rs` 路径；ZCode / Gemini CLI / OpenCode 新写适配器）（§9）
6. **额度感知路由 + 会话亲和**：复用 `quota.rs` / `coding_plan.rs` 的窗口数据（§10）
7. **归账**：网关模式直接带 `connection_id` 落 `requests` 表（比观测模式的指纹匹配更准）
8. **逃生门**：手动切回直连、连续失败自动熔断、启动自愈（§11）

### 3.2 本期明确不做

- **跨协议翻译**（如 Claude Code → 仅提供 OpenAI 协议的 Key）：返回 400 并在错误里说明「该连接仅提供 OpenAI 协议端点，请为其选择 Anthropic 兼容地址或改用其他连接」。设计上保留扩展位（§5.4），本期不实现
- **Gemini 订阅、Grok 订阅适配器**：Google 已停止向个人账号提供 Gemini CLI 登录的 Code Assist 服务（需 Standard/Enterprise + GCP 项目，且账号在 Antigravity 之外使用有封号风险）；SuperGrok 的 OAuth 转发风控未评估。两项列入待确认（§13），API Key 方式的 grok / gemini 连接照常直通
- 插件系统、隐私脱敏、局域网共享、Docker、WebDAV 同步、Ollama 兼容路由、MCP 形式的辅助工具（配额查询等，独立小功能后续另立项）
- **多 Claude 账号的文件级切换**：桥接前若要换账号需要改写 `~/.claude` 凭证文件，可能与用户正在运行的交互式 Claude Code 冲突。本期桥接只使用「当前本机生效的 Claude 登录」（`current_connection.rs` 的语义），多账号切换列入待确认（§13）

## 4. 数据模型

### 4.1 `connections` 表扩展（唯一 schema 变更）

```sql
ALTER TABLE connections ADD COLUMN serve_enabled INTEGER NOT NULL DEFAULT 0;
```

- `serve_enabled = 1` 的连接进入网关模型池；默认 0，观测行为完全不变
- `kind='auth'` 的连接 `serve_enabled` 只对 claude / codex 平台允许置 1（其余平台的订阅暂无适配器，见 §3.2）
- 沿用现有迁移风格（`db.rs` 里对 `ALTER TABLE ... ADD COLUMN` 的既有写法），旧库升级自动获得该列

**不新建任何 provider/账号表。**`connections` 的既有字段语义映射：

| 网关概念 | connections 字段 |
| --- | --- |
| 模型源 | 一行连接 |
| 上游地址 | `base_url`（auth 连接为空 = 官方端点，由适配器决定） |
| 协议 | `protocol`（anthropic / openai） |
| 凭证 | `secret`（auth 连接配合 `auth_snapshot` 恢复语义） |
| 默认模型 | `model` |
| 状态 | `status`（网关在 401/429/402 时更新，见 §10.3） |

### 4.2 模型命名空间

- 对外模型 id：`<连接 slug>/<模型名>`。slug 取连接 `name` 的 URL 安全化小写形式（建连接时若与既有 slug 冲突则加序号），网关启动/连接变更时重建「slug → 连接」索引
- 每个 `serve_enabled` 连接对外暴露的模型列表来源：`fetch_models`（现有实现，按 `protocol` 发头请求 `/v1/models`）；订阅连接暴露适配器内置清单（Claude 桥 = 该账号可用的 Claude 模型；Codex = ChatGPT 后端返回的清单，失败时用内置兜底表）
- `GET /v1/models` 返回全部池内模型；agent 侧的模型选择器直接消费
- agent 传来的模型 id 未命中任何 slug 时：若 `settings.gateway_default_connection` 指向某连接，用它的 `model` 字段兜底；否则 400，错误信息列出可用 slug

### 4.3 设置项（`backend/src/settings.rs`，全部 `#[serde(default)]`）

```rust
/// 本地代理工作模式：observe（现有观测透传）/ relay（账号汇聚网关）
#[serde(default = "default_proxy_mode")]
pub proxy_mode: String,                       // "observe" | "relay"

/// 网关模式：模型 id 未命中时的兜底连接 id；None = 不兜底直接报错
#[serde(default)]
pub gateway_default_connection: Option<String>,

/// 网关模式：连续失败达该次数自动切回直连（逃生门，见 §11.2）
#[serde(default = "default_gateway_circuit")]
pub gateway_circuit_threshold: u32,           // 默认 5

/// 网关模式：是否允许自动熔断（关闭后只能手动切回）
#[serde(default = "yes")]
pub gateway_auto_escape: bool,

/// 接线状态：每个 agent 是否已指向网关（还原时用）
#[serde(default)]
pub gateway_wiring: std::collections::BTreeMap<String, String>, // agent_id -> 还原所需的原值快照 json
```

**迁移要求**：`proxy_claude_upstream` / `proxy_codex_upstream` 保留不动，观测模式继续使用；`gateway_wiring` 的原值快照让每个 agent 的接线可独立还原。旧设置文件反序列化不受影响（写测试验证）。

### 4.4 `requests` 表：**不改**

2026-10-02 的归账设计已加 `connection_id` 列（`apply_proxy_timing` 回填）。网关模式下转发时已知连接 id，直接随计时写入，比指纹匹配更准；观测模式路径不变。

## 5. 网关本体

### 5.1 模块划分

```
backend/src/gateway/
  mod.rs          GatewayMode、模块导出、与 proxy.rs 的模式分发
  server.rs       axum 路由：四个入口 + 回调端点（MCP 桥用，仅 127.0.0.1）
  route.rs        slug 索引、模型解析、会话亲和表、额度感知排序
  adapter/
    mod.rs        Adapter trait 与按连接选择适配器的逻辑
    direct.rs     anthropic / openai 同协议直通（覆盖全部 kind='api' 连接）
    claude.rs     Claude 订阅：真实二进制编排（§7）
    codex.rs      Codex 订阅：ChatGPT 后端直传（§8）
  wire.rs         调用方协议的事件封装（把适配器产出包回 /v1/messages 等响应）
  wiring/
    mod.rs        接线框架：备份 → 写入 → 校验 → 记录原值；还原
    claude.rs     Claude Code 接线
    codex.rs      Codex 接线
    zcode.rs / gemini.rs / opencode.rs   新增 agent 接线
backend/src/bin/gateway_mcp_bridge.rs    stdio MCP 桥助手（§7.3；独立小二进制）
```

**与 `proxy.rs` 的关系**：`ProxyMode { Observe, Relay }`。`proxy.rs::start()` 按 mode 挂不同 router——Observe 挂现有 `forward`（一行不改），Relay 挂 `gateway::server::router()`。共用：端口绑定、启停/自愈框架、`requests` 落库。

### 5.2 鉴权与归账

- 网关只监听 `127.0.0.1`（与观测模式相同）
- 写入各 agent 的令牌形如 `jolu-<agent>`（如 `jolu-claude-code`、`jolu-zcode`）。网关**接受任意令牌**（magpie 的经验：CLI 端 insist 要有一个 key，但本机回环无需真实验证），令牌仅用于识别调用方归账；识别不出时按请求凭证指纹匹配（现有 `KeyIndex` 机制）兜底
- 每个请求的 `connection_id` 在选路时即确定，随 `TimingCore` 落库；`x-jolu-proxy` 响应头保留

### 5.3 请求路径（以 Claude Code → `kimi/kimi-k2` 为例）

```
Claude Code ── POST /v1/messages (model=kimi/kimi-k2, x-api-key=jolu-claude-code)
   ▼ gateway/server.rs：按 anthropic 入口解析；令牌识别 agent
   ▼ gateway/route.rs：slug "kimi" → 连接 id；检查 protocol 匹配（anthropic == anthropic ✓）；
   │                    会话亲和（§10.2）；额度窗口排序（§10.1）
   ▼ gateway/adapter/direct.rs：
   │    - body 读入（上限沿用 REQUEST_BODY_LIMIT 512MB），
   │      serde_json 解析后仅改 "model" 字段为连接的 model（"kimi-k2"），其余原样序列化
   │    - 头部：剥 hop-by-hop，Authorization/x-api-key 换为该连接 secret，
   │      按 protocol 加 anthropic-version（anthropic）或 Bearer（openai）
   │    - URL = 连接 base_url + 原路径
   ▼ 上游响应流零缓冲透传（沿用 timed_body 的首字/总时长计时）
Claude Code ◀── anthropic SSE 原样回传
```

- **失败语义**：上游连接失败 / DNS / 5xx / 429 / 401 且**首字节未发出** → 可按 §10.4 换连接重试（最多 3 个）；流已开始 → 不重试，原样结束
- 请求体处理选 `serde_json::Value` 往返（简单可靠）；长上下文内存峰值记入 `perf_bench.rs` 用例，优化留待后续

### 5.4 协议匹配与扩展位

| 调用方入口协议 | 连接 protocol | 行为 |
| --- | --- | --- |
| anthropic | anthropic | 直通（§5.3） |
| chat | openai | 直通 |
| responses | responses | 直通（预留；当前无此类 `kind='api'` 连接，Codex 订阅适配器见 §8） |
| anthropic | openai（或反之） | **400 明确报错**（§3.2）；`adapter/mod.rs` 的 trait 设计让未来加翻译层不需要改路由 |

### 5.5 Tauri 命令（`lib.rs` 注册，风格对齐现有 `set_proxy` / `proxy_status`）

```rust
set_proxy_mode(mode, default_connection)      // observe/relay 切换（复用 set_proxy 的启停/回滚框架）
gateway_status() -> GatewayStatus             // 模式、池内连接数、接线状态、熔断状态
gateway_set_serve(connection_id, enabled)     // 连接进出模型池
gateway_escape()                              // 手动逃生门（§11.1）
gateway_test_connection(connection_id)        // 连通性测试（复用 fetch_models）
gateway_wiring_set(agent, wired) / wiring_list()
```

前端 `gatewayApi.ts` 桥接；设置页 UI 归属待确认（§13）。

## 6. Claude 订阅适配器：真实二进制 + MCP 工具桥

> 依据：Anthropic 将第三方系统提示词判为第三方流量，注入 OAuth 的转发请求有风控风险。
> magpie 的对应实现：`claude_subscription.go`（4,778 行）+ `claudebridge/`（210 行 stdio MCP 助手）。
> 本期做最小闭环，结构对齐但代码全部重写。

### 6.1 生成链路

```
agent 请求 model=claude/claude-sonnet-5（入口协议任意）
   ▼ route.rs 命中 kind='auth' 且 platform=claude 的连接
   ▼ adapter/claude.rs：
   1. 取会话对应的 claude 进程（无则创建，见 6.2）
   2. 把请求体规范化为该场会话的下一轮输入（文本 / 图片 / 工具结果）
   3. 工具：把调用方请求里的 tools 声明注入桥助手的工具表（6.3）
   4. claude 进程用自己的 OAuth 凭证完成生成（Jolu 不碰令牌）
   5. 产出事件流 → wire.rs 封装回调用方协议 → 回传
```

### 6.2 进程编排

- **进程粒度 = 会话**。会话键 =（agent 令牌，可推导的会话标识）；Claude Code 的 `metadata.user_id`、Codex 的 session id 可作键，取不到时退化为 agent 令牌（同 agent 串行复用一个进程）
- 进程常驻复用（magpie：「tool results resume the same Claude Code process」）；空闲 10 分钟回收；崩溃则该会话下次请求重建并返回明确错误
- 启动方式：`claude` 非交互模式 + stream-json 输出；工作目录用调用方请求里的 cwd 提示不可靠，默认用户主目录，列入待确认（§13）
- **凭证来源**：只使用当前本机生效的 Claude 登录（`creds::read_local_pair("claude", "auth")`）。Jolu 不刷新、不回写、不持有该令牌——claude 二进制自己管理轮换。发现未登录时返回明确错误引导用户先在 Claude Code 登录
- 同一时刻只允许一个会话使用一个 claude 进程发推理（进程内排队），避免流式交错

### 6.3 MCP 工具桥（`bin/gateway_mcp_bridge.rs`）

- 形态：独立小二进制，由网关在启动 claude 进程时经 `--mcp-config` 挂载为 stdio MCP server（参考 magpie claudebridge 的回调模型，210 行的规模量级）
- 行为：向 claude 进程声明调用方请求里的 tools；claude 调用工具时，助手把 `(tool_call_id, name, arguments)` 经**仅回环的回调端点**（`gateway/server.rs` 提供）交给网关 → 网关把 tool_use 事件封装进对该 agent 的响应流 → agent 本地执行 → 下一个请求携带 tool_result → 网关交给助手 → 同一场 claude 会话继续
- 助手在等待工具结果期间挂起该次 MCP 调用（对齐 magpie 的 listWait 语义）；超时（默认 120s）返回工具错误
- 该桥**只在 Claude 订阅适配器内出现**，其他适配器一律不经 MCP

### 6.4 明确不支持

- 不做多 Claude 账号在桥内的切换（§3.2）
- 不把订阅 token 暴露给任何配置项或日志；官方端点固定由 claude 二进制自己决定
- claude 二进制未安装 / 未登录 → 适配器返回明确错误，不静默降级到其他连接（避免用户在不知情下消耗别的额度）

## 7. Codex 订阅适配器

- 凭证：每次请求**现场重读** `~/.codex/auth.json`（`creds::codex_dir()` 已有路径解析）。`tokens.access_token` 过期前 5 分钟用 `refresh_token` 调 `https://auth.openai.com/oauth/token` 刷新，成功后**回写 auth.json**：回写前比对文件 mtime，外部已改则放弃回写并改用外部值（`proxy_config.rs`「外部意图优先」原则；这是初稿 §5.3 方案 A 的保留部分，仍然正确）
- 上游：ChatGPT 后端（Responses 语义），带 `chatgpt-account-id` 头
- 转换（对齐 magpie 的处理）：后端只支持流式 → 非流式请求转为流式采集后聚合返回；丢弃后端会拒绝的参数（temperature 等），被丢弃的参数记入响应外的诊断字段，不写日志正文
- 失败：401（刷新后仍失败）→ 连接 `status='invalid'`，界面标红；429 → 冷却（§10.3）

## 8. 接线层（`gateway/wiring/`）

### 8.1 框架纪律（从 `cli_apply.rs` 提取，全部沿用）

1. 写前把原文件备份到应用数据目录
2. 临时文件 + rename 原子写
3. 还原只清理自己的写入：当前值不再是网关地址时不覆盖（外部意图优先）
4. 全局配置切换锁 `serialized_switch`（网关接线与观测接管互斥）
5. 接线/还原动作记录进 `settings.gateway_wiring`（原值快照），支持逐 agent 还原

### 8.2 各 agent 适配器

| agent | 接线动作 | 备注 |
| --- | --- | --- |
| Claude Code | `settings.json` 写 `env.ANTHROPIC_BASE_URL=http://127.0.0.1:<port>` + `env.ANTHROPIC_AUTH_TOKEN=jolu-claude-code` | 与 `cli_apply.rs` 同一文件同一写法；网关令牌接受任意值但 CLI insist 要有 |
| Codex | `config.toml` 自定义 provider `base_url=网关` + `auth.json` 的 `OPENAI_API_KEY=jolu-codex`；`wire_api` 按 §5.4 匹配写 `chat` 或 `responses` | 复用 `cli_apply.rs` 的 TOML 保留格式写入 |
| ZCode | provider 配置写 `options.baseURL=网关` + `apiKey=jolu-zcode` | 参考 magpie `zcode.go`：注意其 rules 文件与输出上限行为（不写 rules 时 ZCode 把未知模型截到 32000 token），接线时须同步写 rules；保留用户已有 base 的场景参照 `zcodeKept` 语义 |
| Gemini CLI | `GOOGLE_GEMINI_BASE_URL` 指向网关（经 `~/.gemini/.env` 或 settings） | magpie 的 gemini 接线较薄；Jolu 需实测 env 写入位置 |
| OpenCode | provider JSON 写 baseURL + apiKey | 对齐 `opencode.rs` 现有读取路径 |

- 每个适配器都要实现：探测（agent 是否安装/已接线）、接线、还原、`Notice`（是否需重启 agent 生效）
- Qoder / WorkBuddy / Trae 不接线（Jolu 不代管其密钥，且订阅适配器未做），目录里不出现入口

### 8.3 接线与观测模式的互斥

同一 agent 的配置文件同时是观测接管和网关接管的写入目标。规则：**一个 agent 同一时间只处于一种模式**。`set_proxy_mode` 切换时先还原全部接线再建立新模式；`serialized_switch` 锁保证不并发。

## 9. 路由与额度

### 9.1 选连接（对一次新请求）

```
1. 解析模型 id → slug 命中连接（唯一命中即用）
2. 未命中 → gateway_default_connection 兜底，否则 400
3. protocol 匹配检查（§5.4）；auth 连接只接受其适配器声明的入口协议
4. 连接可用性：serve_enabled ∧ status='active' ∧ 不在冷却期
5. 同 slug 多连接（未来多账号）时：额度窗口余量多者优先
   （窗口数据 = connection_quota 命令的现有缓存，quota.rs / coding_plan.rs）
```

### 9.2 会话亲和（prompt cache 成本关键）

- 亲和键：可推导会话标识（Claude Code 的 `metadata.user_id`、Codex session id）；取不到按（agent 令牌 + 模型 slug）
- 规则：会话一旦由某连接答过，后续请求继续用该连接；窗口用量 ≥98% 才迁移；429 冷却的连接在会话内也不切换——直接报错给 agent（缓存迁移的代价高于偶发失败，对齐 magpie 的 Stays 语义）
- 亲和表在内存（`route.rs`），LRU 上限 1000 会话，进程重启清空（重启后首请求重新选择）

### 9.3 状态与冷却

| 上游信号 | 动作 |
| --- | --- |
| 429（带 Retry-After） | `cooldown_until_ms = now + Retry-After`（默认 60s），换连接重试 |
| 401/403（重试后仍失败） | `status='invalid'`，不自动恢复，界面标红 |
| 402 / 额度耗尽特征 | `status='exhausted'`，次日零点自动回 `active`（简单策略） |
| 连接失败 | 计入熔断计数（§10.4），单次请求内先换连接 |

### 9.4 熔断

连续失败达 `gateway_circuit_threshold`（默认 5）且 `gateway_auto_escape` 开启：还原全部接线 → 停网关 → 系统通知（`notify.rs`）「账号汇聚网关已自动切回直连：<原因>」→ `proxy_mode` 保留、`proxy_enabled=false`。**注意**：熔断后 agent 回到各自直连，若某 agent 本无订阅会不可用，通知文案必须写清楚是「切回直连」而非「服务故障」。

## 10. 逃生门（三层，全部必须）

1. **手动切回直连**：设置页按钮 → `gateway_wiring_set` 逐 agent 还原（按原值快照）→ 停网关 → Toast「已切回直连，各 agent 配置已还原」
2. **自动熔断**：§9.4
3. **启动自愈**：`proxy::startup()` 扩展——`proxy_mode=relay` 且启用时，校验各接线文件是否仍指向网关（`is_self_base` 同款判断）；是 → 重新接线；否（外部已改）→ 尊重外部、关闭网关并通知

## 11. 实施步骤（一次性交付，步骤仅表依赖顺序）

每步完成即 `cargo test` 全绿再进下一步。

1. **数据与设置**：`serve_enabled` 列 + §4.3 设置项 + 迁移测试（含旧设置文件加载）
2. **gateway 骨架**：`mod.rs` / `server.rs` 空路由 / `wire.rs` 雏形；`ProxyMode` 分发接入 `proxy.rs::start()`
3. **直通适配器**：`adapter/direct.rs`（anthropic + openai）；slug 索引与 `/v1/models`；mock 上游端到端测试（流式 + 非流式 + model 改写 + 头部替换断言，参考 `connections.rs` 的 `serve_models_once` 测试手法）
4. **路由与亲和**：`route.rs` 全分支单测（命中/兜底/协议不匹配/冷却/亲和）
5. **Codex 订阅适配器**：mock 后端测刷新（成功/失败/mtime 冲突）与流式转换
6. **MCP 桥助手**：`bin/gateway_mcp_bridge.rs` + 回调端点；用假 MCP 客户端测声明/调用/超时
7. **Claude 订阅适配器**：进程编排 + 会话复用 + 崩溃恢复；用假 claude 二进制（fixture 脚本）测全链路
8. **接线层**：框架 + claude/codex（改造现有写入路径）+ zcode/gemini/opencode；还原与「外部意图优先」测试
9. **归账落库**：网关请求带 `connection_id` 写 `requests`；与观测模式计时共存验证
10. **逃生门与自愈**：三层逐条实现 + 测试
11. **Tauri 命令与前端**：`gatewayApi.ts`、设置页分区（UI 归属待确认，§13）、`providerDraft` 式纯逻辑 + `.test.mjs`
12. **文档收尾**：`AGENTS.md` 模块 B/G「现有实现」列更新；`docs/todo/2026-10-22-unverified-integrations.md` 增补验证项

## 12. 验证要求

### 12.1 自动化

```bash
cargo test --manifest-path backend/Cargo.toml
pnpm --dir frontend typecheck
node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs
```

必测：直通适配器的头部/模型改写/流式透传；路由全分支；Codex 刷新与 mtime 冲突；MCP 桥调用/超时；Claude 桥进程复用与崩溃；接线还原与外部改动不覆盖；迁移（旧库旧设置）；熔断三条路径。

### 12.2 真机（隔离目录，`scripts/acceptance/README.md`）

1. Claude Code 接 Kimi 的 Anthropic 端点（`serve_enabled` 开启）→ 流式正常、`requests` 带连接 id
2. ZCode 接入网关 → rules 文件生效（长输出不被 32000 截断）、网关模型出现在其选择器
3. Codex 订阅经网关 → 非流式请求转流式成功、参数丢弃有诊断
4. Claude 订阅经网关（真实 claude 二进制已登录）→ 工具调用闭环、多轮会话复用同进程
5. 熔断：错误 base_url 连续失败 → 自动切回直连 + 通知
6. 逃生门：手动还原后逐 agent 检查配置文件与备份一致性
7. 降级：回退到旧版本 Jolu，各 agent 配置可手动恢复、应用不崩

### 12.3 界面

设置页改完用 Pencil MCP 截对应画板对照；本功能无既有画板，先等 §13 第 1 项结论。

## 13. 待确认（需鼠鼠决策）

1. **设置页 UI**：网关是独立分区还是并入「网络与代理」？`Jolu.pen` 无此界面，需补画板或给方向
2. **Gemini / Grok 订阅适配器**：本期不做（§3.2），确认是否接受；若要做 Gemini 需用户提供 Code Assist Standard/Enterprise 账号与 GCP 项目
3. **多 Claude 账号**：桥接前是否允许 Jolu 改写 `~/.claude` 凭证切换账号（与交互式 Claude Code 并存有风险）？本期默认只认当前登录
4. **claude 桥的工作目录**：spawn 真实 claude 时用什么 cwd（影响其文件工具的默认作用域）
5. **预置厂商模板**：是否内置 Kimi/GLM/DeepSeek/MiniMax 的 base_url 预填（减少手填）

## 14. 风险

| 风险 | 缓解 |
| --- | ---|
| claude 进程编排复杂（生命周期/流式交错/崩溃） | 进程粒度=会话 + 排队串行 + fixture 假二进制测试；实在不稳则该适配器单独回退为「待接入」，不影响其余 |
| Codex 后端协议漂移（非公开接口） | 适配器内聚在 `adapter/codex.rs`；失败返回明确错误不做静默降级 |
| 长上下文内存峰值（serde_json 往返） | `perf_bench.rs` 加 10MB 用例；超出即列 todo 优化 |
| 接线写坏用户配置 | 备份 + 原子写 + 只清自己的写入 + 真机验证第 6 项 |
| 用户误以为网关 = 官方服务，额度耗尽怪 Jolu | 通知与界面文案明确标注「流量经本机网关转发到各厂商」 |
| 无 Rust 工具链导致开发中途无法验证 | §2.2 前置条件不满足不开工 |

## 15. 交付标准

- [ ] §12.1 全绿（含新增测试全部条目）
- [ ] §12.2 第 1–7 项真机通过并记录到 `docs/verification/`
- [ ] 三层逃生门逐条手动验证
- [ ] 全库 grep 确认日志 / 导出 / DTO 无明文凭证；`x-jolu-proxy` 响应头保留
- [ ] 旧设置与旧库升级无损，回退旧版本可恢复
- [ ] `AGENTS.md` 现有实现列已更新
- [ ] §13 五项已获鼠鼠答复；未验证项（如有）明确标注「待确认」，不得含糊
