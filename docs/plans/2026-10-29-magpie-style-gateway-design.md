# 多厂商反代网关 · 执行开发文档（2026-10-29）

**交给：鼠鼠**
**状态：待实施**
**参考实现：https://github.com/yetone/magpie（MIT，Go；仅作设计参考，不复制代码）**

## 0. 一句话说明

给 Jolu 加一个可选的**多厂商反代网关**：本机监听一个端口，把 Claude Code / Codex 等 CLI 的请求转发到用户配置的任意厂商（Kimi / GLM / DeepSeek / 自建网关…），支持 API Key 与官方订阅两类凭证、跨协议转换、额度耗尽自动换号，并保留「一键切回直连」的逃生门。

现有 `proxy.rs` 是**只读观测器**（透传 + 计时）。本功能在它旁边长出一个**控制平面**，两者共用监听端口与配置接管机制，但数据路径严格分离。

---

## 1. 实施前必读

### 1.1 铁律（本功能与现有纪律的冲突已备案，鼠鼠批准）

| 现有纪律 | 本功能的态度 | 处理方式 |
| --- | --- | --- |
| `proxy.rs` 的「不注入凭证」 | **废除（仅反代模式）** | 观测模式保持透传；反代模式必须改写凭证。用 `ProxyMode` 枚举在代码层面隔开，不共用 `forward()` |
| 「官方订阅直连一律不动」 | **放开** | 反代模式需接管官方订阅，见 §5 |
| `AGENTS.md` 密钥不入日志 | **继续遵守** | 反代模式下凭证只存在于内存与加密存储，任何日志、诊断、导出都不含明文 |
| `AGENTS.md` 数据结构变更需可恢复迁移 | **必须遵守** | 见 §6.2 |

### 1.2 前置条件（务必先确认，否则无法验证）

**这台机器当前没有 Rust 工具链**，`cargo` / `rustc` 均不在 PATH，VS 2022 Community 已安装但缺少 MSVC 编译器与 Windows SDK。开工前先装：

```bash
# 1. rustup（在线安装器，约 30MB）
#    https://rustup.rs/  安装时选 x86_64-pc-windows-msvc
# 2. VS 2022 Build Tools 的 C++ 工作负载
#    打开 Visual Studio Installer → 修改 → 勾选「使用 C++ 的桌面开发」
#    右侧组件至少保留：MSVC v143 编译器、Windows 10/11 SDK
```

装完自检（三条都要过）：

```bash
rustc -V                 # 需 >= 1.90（backend/Cargo.toml 的 rust-version）
cargo -V
cargo test --manifest-path backend/Cargo.toml   # 现有测试应全绿再开工
```

首次 `cargo build` 会编译 `rusqlite` 的 bundled SQLite 和整个 Tauri 依赖树，约 15–25 分钟，属正常。

### 1.3 开工前的基线验证

```bash
cd D:/develop/Jolu
pnpm --dir frontend typecheck
node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs   # 应 198 passed
cargo test --manifest-path backend/Cargo.toml                           # 应全绿
```

基线不绿先修，不要带着失败开工。

---

## 2. 目标与不做的事

### 2.1 本期要做

1. **厂商注册表**：新增/编辑/删除厂商，每家按协议分别存 base URL 与密钥
2. **同协议反代**：路径判定协议 → 选厂商 → 改写 URL / 凭证 / model → 流式回传
3. **跨协议转换**：Anthropic Messages ↔ OpenAI Chat Completions（双向）、Anthropic → OpenAI Responses（单向）
4. **凭证解析**：API Key + 官方订阅 OAuth（Claude / Codex 本机凭证读取与刷新）
5. **账号池与故障转移**：同厂商多账号，额度耗尽/429/5xx 时换号；换号失败再换厂商
6. **逃生门**：手动「切回直连」按钮 + 自动熔断（连续失败 N 次自动还原）
7. **用量归账**：反代请求写入现有 `requests` 表，按厂商 + 账号归账

### 2.2 本期明确不做

- 插件系统（magpie 的 Bun 运行时）
- 隐私脱敏（本地小模型）
- 局域网共享 / Docker / 远程 magpie / WebDAV 同步
- Ollama 兼容层（`/api/chat`、`/api/generate`）
- Gemini 协议的原生转换（只保留 base URL 直连能力，不做 generateContent ↔ Messages 互转）
- MCP / Skills / Prompts 注入（那是 magpie 的 Library，与网关无关）

**这一条边界是鼠鼠明确要求的「只做厂商反代」，越界的功能一律驳回。**

---

## 3. 架构

### 3.1 模块划分（新建 `backend/src/gateway/`）

```
backend/src/
  gateway/
    mod.rs            模块入口、ProxyMode 定义、对外 API
    registry.rs       厂商与账号的读写（providers.json 或 SQLite）
    protocol.rs       Protocol 枚举、请求/响应的规范化中间表示（Request / Part）
    route.rs          选厂商、选账号、故障转移决策
    credential.rs     凭证解析与注入（API Key / OAuth，含刷新）
    error.rs          错误类型与对外错误翻译
    convert/
      mod.rs
      anthropic.rs    Anthropic Messages 解析与生成
      chat.rs         OpenAI Chat Completions 解析与生成
      responses.rs    OpenAI Responses 解析与生成
      stream.rs       SSE 流事件映射
    server.rs         HTTP 路由与 handler
  proxy.rs            **保持现状**，仅增加 ProxyMode 分支入口
  proxy_config.rs     **保持现状**，增加反代模式的接管/还原
```

**关键约束：** `proxy.rs` 与 `gateway/` 不共享 `forward()`。`proxy.rs` 的 `forward` 保持零转换；`gateway/` 写自己的 handler。共用的是端口绑定、配置接管、`requests` 表落库三样基础设施。

### 3.2 数据流

```
CLI (Claude Code)
   │  POST http://127.0.0.1:<port>/v1/messages
   ▼
gateway/server.rs  ── 路径判定协议 = Anthropic
   │
   ▼
gateway/route.rs   ── 按 model 匹配厂商绑定 / 默认厂商
   │                  选出 (provider, account)
   ▼
gateway/convert/anthropic.rs  ── parse → 规范化 Request
   │
   ▼  (若厂商原生协议 != Anthropic)
gateway/convert/chat.rs       ── Request → OpenAI Chat body
   │
   ▼
gateway/credential.rs  ── 注入该账号凭证（替换 Authorization / x-api-key）
   │
   ▼
上游厂商 (Kimi / GLM / ...)
   │
   ▼  (若需转换)
gateway/convert/stream.rs ── Chat SSE → Anthropic SSE 事件映射
   │
   ▼
CLI 收到 Anthropic 格式流
```

### 3.3 与观测模式的关系

`ProxyMode` 两值：

```rust
pub enum ProxyMode {
    /// 现有行为：透传 + 计时，不碰凭证
    Observe,
    /// 新增行为：厂商反代，改写请求
    Relay,
}
```

`ProxyDeps` 增加 `mode: ProxyMode` 字段。`forward()` 首行按 mode 分发；`Relay` 走 `gateway::server::handle`。**不要让 `forward()` 内部长出第二套逻辑。**

---

## 4. 数据模型

### 4.1 厂商（Provider）

落库选择 **SQLite 新表**（与 `connections` 同库），理由：现有 `connections` 表语义是「检测到的连接」，与「用户配置的厂商」不同；混用会让所有 `connections` 查询都要加过滤条件。新表独立，互不影响。

```sql
CREATE TABLE IF NOT EXISTS relay_providers (
    id          TEXT PRIMARY KEY,          -- uuid
    name        TEXT NOT NULL,             -- 展示名，如 "Kimi"
    preset      TEXT,                      -- 预置模板 id，自定义为 NULL
    -- 按协议分别存 base URL；NULL 表示该厂商不支持此协议
    base_chat      TEXT,
    base_responses TEXT,
    base_anthropic TEXT,
    base_gemini    TEXT,
    -- 模型映射：JSON 数组，元素形如 {"from":"claude-sonnet-4","to":"kimi-k2"}
    model_map   TEXT NOT NULL DEFAULT '[]',
    -- 默认协议优先级：JSON 数组，如 ["anthropic","chat"]
    protocol_order TEXT NOT NULL DEFAULT '["anthropic","chat"]',
    enabled     INTEGER NOT NULL DEFAULT 1,
    created_ms  INTEGER NOT NULL,
    updated_ms  INTEGER NOT NULL
);
```

### 4.2 账号（Account）

```sql
CREATE TABLE IF NOT EXISTS relay_accounts (
    id           TEXT PRIMARY KEY,
    provider_id  TEXT NOT NULL REFERENCES relay_providers(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL CHECK (kind IN ('api','oauth')),
    name         TEXT NOT NULL,            -- 展示名
    -- 密钥：api 存用户填的 key；oauth 存 token bundle 的加密串
    -- 加密方式沿用 backend/src/creds.rs 的现有方案（DPAPI），不得新造
    secret_enc   BLOB,
    -- OAuth 专用
    oauth_kind   TEXT,                     -- 'claude' | 'codex'
    expires_ms   INTEGER,
    refresh_enc  BLOB,
    -- 状态
    status       TEXT NOT NULL DEFAULT 'active',   -- active | exhausted | invalid
    cooldown_until_ms INTEGER,
    last_used_ms INTEGER,
    weight       INTEGER NOT NULL DEFAULT 1,
    created_ms   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_relay_accounts_provider ON relay_accounts(provider_id);
```

### 4.3 设置项（`backend/src/settings.rs`）

在 `Settings` 结构新增，**全部带 `#[serde(default)]`**：

```rust
/// 本地代理工作模式：observe（现有观测）/ relay（厂商反代）
#[serde(default = "default_proxy_mode")]
pub proxy_mode: String,                    // "observe" | "relay"

/// 反代模式：模型名无法匹配任何厂商绑定时使用的默认厂商 id
#[serde(default)]
pub proxy_default_provider: Option<String>,

/// 反代模式：连续失败达到该次数自动切回直连（逃生门）
#[serde(default = "default_relay_circuit")]
pub proxy_relay_circuit_threshold: u32,    // 默认 5

/// 反代模式：是否允许自动切回直连（关闭后只能手动）
#[serde(default = "yes")]
pub proxy_relay_auto_escape: bool,

/// 反代模式：是否已接管官方订阅的 CLI 配置（还原时用）
#[serde(default)]
pub proxy_relay_took_official: bool,
```

**迁移要求：** 旧版本设置的 `proxy_claude_upstream` / `proxy_codex_upstream` **保留不动**，观测模式继续使用它们。反代模式不读这两个字段，读 `relay_providers` 表。这样两套配置互不干扰，降级回旧版本也不会坏。

### 4.4 `requests` 表扩展

现有 `requests` 表增加两列（`db.rs` 的迁移区，参考已有的 `ALTER TABLE connections ADD COLUMN account_email` 写法）：

```sql
ALTER TABLE requests ADD COLUMN relay_provider_id TEXT;   -- 反代记录归属的厂商
ALTER TABLE requests ADD COLUMN relay_account_id  TEXT;   -- 归属的账号
```

观测模式写入时这两列为 NULL，不受影响。

---

## 5. 凭证解析（官方订阅部分）

这是本期最需要小心的部分。magpie 对应代码是 `claude_subscription.go`（4,778 行），我们不抄，只做最小闭环。

### 5.1 Claude 官方订阅

本机凭证位置（`backend/src/creds.rs` 已有读取逻辑可复用）：

- `~/.claude/.credentials.json`（或 `$CLAUDE_CONFIG_DIR/.credentials.json`）
- 结构：`{"claudeAiOauth":{"accessToken":"...","refreshToken":"...","expiresAt":<ms>,"scopes":[...]}}`

**要做的事：**

1. 读取 `accessToken` 与 `expiresAt`
2. 过期前 5 分钟用 `refreshToken` 调 `https://api.anthropic.com/v1/oauth/token` 刷新
3. 刷新成功后**只更新内存与本表**，不回写 CLI 的凭证文件（回写会与 CLI 自己刷新冲突，见 §5.3）
4. 请求时注入 `Authorization: Bearer <accessToken>`
5. 官方端点固定 `https://api.anthropic.com`，**不允许用户改**，否则就是把凭证发给第三方

`backend/src/claude_auth.rs` 已有 `PROFILE_URL` 与预检逻辑，参考它的错误处理风格（缺字段时明确报错，不静默降级）。

### 5.2 Codex 官方订阅

- `~/.codex/auth.json`，含 `OPENAI_API_KEY`（可为 null）与 `tokens.access_token` / `refresh_token`
- 有 `chatgpt-account-id` 头需一并转发
- 刷新端点 `https://auth.openai.com/oauth/token`
- `backend/src/creds.rs` 的 `local_codex_auth()` 已能读出 token，复用

### 5.3 刷新冲突（必须处理）

CLI 自己也会刷新 token。如果 Jolu 刷了但没回写，CLI 手里的 refreshToken 可能已被 Jolu 用掉而失效（多数 OAuth 实现 refreshToken 是轮换的）。

**处理方式（二选一，文档要求选 A）：**

- **A（推荐）**：每次请求前**重新读一次** CLI 的凭证文件，只在文件里的 token 已过期时才用 Jolu 缓存的；Jolu 刷新后**回写**凭证文件（保持 CLI 可用），回写前比对文件 mtime，若外部已改则放弃回写、改用外部值。这与 `proxy_config.rs` 的「外部意图优先」原则一致。
- B：完全不刷新，token 过期就报错让用户去 CLI 重新登录。体验差，不推荐。

### 5.4 安全底线

- 凭证**只存加密后的 BLOB**，加密沿用现有 DPAPI 方案（`creds.rs`），不得新造算法
- 前端 DTO **绝不含**明文 secret，只返回 `masked`
- 任何 `eprintln!`、诊断、导出、日志都不得出现 token
- 官方端点 URL 硬编码，不可配置

---

## 6. 协议转换

### 6.1 规范化中间表示

参考 magpie 的 `Request` / `Part` 设计（`internal/gateway/chat.go`、`anthropic.go`），在 `gateway/protocol.rs` 定义：

```rust
/// 跨协议通用的请求表示
pub struct Request {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stream: bool,
    // 扩展参数原样保留，转换时不丢失
    pub extra: serde_json::Map<String, Value>,
}

pub struct Message {
    pub role: Role,          // System | User | Assistant
    pub parts: Vec<Part>,
}

pub enum Part {
    Text(String),
    Image { data_url: String, detail: Option<String> },
    ToolUse { id: String, name: String, input: Value },
    ToolResult { tool_use_id: String, content: Vec<Part>, is_error: bool },
    /// 思考内容：Anthropic 的 thinking block / OpenAI 的 reasoning_content
    Reasoning { text: String, signature: Option<String> },
}
```

**为什么必须有中间表示：** 直接写 Anthropic → Chat 的转换函数，将来加第三个协议就要写 6 个函数；有中间表示只需 N 个 parse + N 个 build。这是 magpie 8 万行里唯一值得抄的架构决策。

### 6.2 转换矩阵（本期范围）

| 下游（CLI） | 上游（厂商原生） | 实现 |
| --- | --- | --- |
| Anthropic | Anthropic | 直通（只换 URL/凭证/model） |
| Anthropic | Chat | 需要转换 |
| Anthropic | Responses | 需要转换 |
| Chat | Chat | 直通 |
| Chat | Anthropic | 需要转换 |
| Chat | Responses | **本期不做**，报明确错误 |
| Responses | Responses | 直通 |
| Responses | Chat | **本期不做**，报明确错误 |

不支持的组合必须返回清晰的 400 与中文说明，**不允许静默降级成别的协议**（会得到语义错误的结果）。

### 6.3 流式映射（最容易出错的地方）

`gateway/convert/stream.rs`。核心事件对应：

**Anthropic → Chat（下游 Anthropic，上游 Chat）：**

| 上游 Chat chunk | 下游 Anthropic 事件 |
| --- | --- |
| 首个 chunk | `message_start`（含 `message.id`、`usage.input_tokens`） |
| `delta.content` | `content_block_start`（首次）+ `content_block_delta`(`text_delta`) |
| `delta.reasoning_content` | `content_block_delta`(`thinking_delta`) |
| `delta.tool_calls[].function` | `content_block_start`(`tool_use`) + `content_block_delta`(`input_json_delta`) |
| `finish_reason` | `content_block_stop` + `message_delta`(`stop_reason`) |
| 流结束 | `message_stop` |

**必须遵守的细节：**

- `content_block_index` 全程单调递增，不能跳号
- `input_json_delta` 是**增量拼接**的 JSON 字符串，不能每块都是一个完整 JSON
- `stop_reason` 映射：Chat 的 `stop` → Anthropic 的 `end_turn`，`length` → `max_tokens`，`tool_calls` → `tool_use`
- `usage` 只在 `message_start` 与 `message_delta` 出现，中间不重复
- **thinking signature 必须原样透传**，改动会导致上游拒签（Claude 的 thinking block 有此要求）

### 6.4 请求侧的坑（逐条核对）

1. **`model` 字段替换**：按 `model_map` 映射，未命中且配置了默认厂商则用默认，否则报错
2. **工具定义**：Anthropic `input_schema` ↔ Chat `function.parameters`，字段名不同
3. **工具调用配对**：Chat 要求每个 `tool` 消息前面有对应的 `assistant.tool_calls`，否则上游 400。转换时要校验并成对；孤儿 tool 消息丢弃并记日志
4. **空 assistant 消息**：某些上游拒绝空 content，转换时过滤
5. **图片**：Chat 的 `image_url`（data URL）↔ Anthropic 的 `source.base64`
6. **`max_tokens` 必填**：Anthropic 要求必填，Chat 可选。反向转换时给默认值并从响应里读实际值
7. **`temperature` 范围**：Anthropic 0–1，OpenAI 0–2。跨协议时钳制并记日志

### 6.5 请求体处理（性能）

现有 `forward()` 用 `to_bytes(body, 512MB)` 全量缓冲。反代必须读 body 才能改 `model`，但要避免把 512MB JSON 建成 DOM：

- **同协议直通**：只做外层字段替换。用 `serde_json::Value` 解析后改 `model` 再序列化（简单可靠，长上下文时有内存峰值）；或做字节级定位替换（快但脆弱）。**本期选前者**，在文档里注明长上下文内存风险，二期再优化
- **跨协议转换**：必须完整 parse，无法避免。转换后立即释放原始 body
- 内存峰值监控：`perf_bench.rs` 已有基准框架，加一个 10MB 请求体的用例

---

## 7. 路由与故障转移

### 7.1 选厂商

```
1. 解析请求的 model 名（下游传入的）
2. 遍历 enabled 厂商，看其 model_map 是否命中该 from
3. 命中多个 → 按 protocol_order 偏好 + 最近成功优先
4. 未命中 → 用 proxy_default_provider
5. 仍无 → 返回 400，中文说明「模型 X 未绑定任何厂商，请在设置中配置」
```

### 7.2 选账号

```
1. 过滤 status = 'active' 且 cooldown_until_ms < now
2. 按 weight 加权，同权重按 last_used_ms 最久未用优先（打散负载）
3. 无可用账号 → 该厂商视为不可用，进 7.3
```

### 7.3 故障转移（关键：区分阶段）

**必须区分「响应首字节之前」和「之后」：**

- **首字节之前失败**（连接失败、DNS、5xx、429、401）：可以安全换账号/换厂商重试，最多试 3 个
- **首字节之后失败**（流中途断开）：**不能重试**，已发给 CLI 的内容无法撤回。必须原样结束流并让 CLI 自己报错

这条与 `proxy.rs` 现有的 `note_upstream_unreachable` 逻辑呼应，但阈值独立：反代用 `proxy_relay_circuit_threshold`（默认 5），观测用现有 `FALLBACK_THRESHOLD`（3）。

### 7.4 冷却与额度

- 收到 429 → 该账号 `cooldown_until_ms = now + Retry-After`（默认 60s），换下一个
- 收到 401/403 → 账号 `status = 'invalid'`，不自动恢复，界面标红提示重新授权
- 收到 402 或响应体含额度耗尽特征 → `status = 'exhausted'`，到下一个自然日重置为 `active`（简单策略，不做精确额度感知）

**不做**：magpie 的 `smart` / `pace` 这类额度感知路由（需要读各厂商额度接口，量太大）。本期只做「不可用就换下一个」。

---

## 8. 逃生门（鼠鼠明确要求）

三层，全部要实现：

### 8.1 手动切回直连

设置页「网络与代理」区新增按钮「切回直连」。点击后：

1. 调用 `proxy_config::restore_all_saved()`（含反代模式接管的部分）
2. `settings.proxy_enabled = false`，`proxy_mode` 保留
3. 停止监听
4. Toast：「已切回直连，CLI 配置已还原」

**关键：还原必须彻底。** 反代模式可能接管了官方订阅（`proxy_relay_took_official = true`），还原时要删掉 Jolu 写入的 `ANTHROPIC_BASE_URL` / Codex provider 字段，恢复成「无自定义 base URL」的官方直连状态，而不是写回一个错误的旧值。

### 8.2 自动熔断

连续失败达 `proxy_relay_circuit_threshold` 次（默认 5）：

1. 记录失败原因到 `FALLBACK_REASON`（现有机制，复用）
2. 自动还原 CLI 配置 + 停止监听
3. `proxy_mode` 保留为 `relay`，`proxy_enabled = false`
4. 发系统通知（`notify.rs` 现有能力）：「反代已自动切回直连：<原因>」
5. 主面板顶部显示红色提示条，带「重新启用」按钮

**与观测模式的差异：** 观测模式熔断后「回退直连」是回到用户原上游，天然可用；反代模式熔断后是回到「CLI 原本的官方直连」，如果用户本就没订阅，CLI 会直接不可用。所以通知文案必须说清楚，不能让人以为是 Jolu 的错。

### 8.3 崩溃恢复

`proxy::startup()` 现有逻辑会在启动时自愈「崩溃残留的代理地址」。反代模式要扩展：

- 启动时若 `proxy_enabled && proxy_mode == "relay"`，先检查配置是否还指向本机端口
- 是 → 重新接管（复用现有 `set_proxy` 流程）
- 否（外部已改）→ 尊重外部，关闭反代并通知

---

## 9. 后端改动清单

按依赖顺序，每步做完能编译通过再进下一步。

### 步骤 1：数据结构与迁移

- `backend/src/db.rs`：新增 `relay_providers` / `relay_accounts` 建表；`requests` 加两列；写迁移测试（参考 `db_breakdown_tests.rs` 风格）
- `backend/src/settings.rs`：加 §4.3 五个字段 + `default_*` 函数 + 默认值
- **验证**：`cargo test --manifest-path backend/Cargo.toml` 全绿；旧设置文件能正常反序列化（写一个用旧 JSON 的测试）

### 步骤 2：`gateway/` 骨架

- `gateway/mod.rs`：`ProxyMode` 枚举、模块导出
- `gateway/protocol.rs`：`Request` / `Message` / `Part` / `Tool` 定义 + 单测
- `gateway/error.rs`：错误类型，含「协议组合不支持」的明确载荷
- **验证**：`cargo test` 通过，此步无外部依赖

### 步骤 3：厂商与账号存储

- `gateway/registry.rs`：CRUD（`list_providers` / `save_provider` / `delete_provider` / `list_accounts` / `save_account` / `delete_account`）
- 密钥加密复用 `creds.rs` 的 DPAPI 方案
- **验证**：单元测试覆盖增删改查；加密后读回的明文与写入一致

### 步骤 4：协议解析与生成

- `gateway/convert/anthropic.rs`：`parse_anthropic` / `build_anthropic`
- `gateway/convert/chat.rs`：`parse_chat` / `build_chat`
- `gateway/convert/responses.rs`：`parse_responses` / `build_responses`
- **验证**：用真实报文做往返测试——解析后重新生成，语义等价。在 `backend/tests/fixtures/` 放样例 JSON（参考现有 `fixtures/` 结构）

### 步骤 5：流式转换

- `gateway/convert/stream.rs`：SSE 事件解析 + §6.3 的事件映射
- **验证**：喂入录制的上游 SSE 流，断言输出事件序列符合 §6.3 表格。这是最容易错的部分，测试要覆盖：纯文本、工具调用、思考内容、混合、异常中断

### 步骤 6：凭证解析

- `gateway/credential.rs`：API Key 直取；OAuth 读取、刷新、回写（§5.3 方案 A）
- **验证**：刷新逻辑用 mock server 测试；回写冲突用临时文件模拟外部修改

### 步骤 7：路由与故障转移

- `gateway/route.rs`：选厂商、选账号、冷却判定、首字节前重试
- **验证**：纯逻辑单测，覆盖 §7 每种情况

### 步骤 8：HTTP 服务

- `gateway/server.rs`：路由 `/v1/messages`、`/v1/chat/completions`、`/v1/responses`；`GET /v1/models` 返回已配置厂商的模型列表
- 接入共享端口：`proxy.rs` 的 `start()` 按 `ProxyMode` 挂不同 router
- **验证**：起本地 mock 上游，端到端跑通一次流式请求

### 步骤 9：落库归账

- 反代请求写入 `requests` 表，带 `relay_provider_id` / `relay_account_id`
- 复用现有 `apply_proxy_timing` 的计时逻辑（首字、总时长）
- **验证**：一次反代请求后，`requests` 表有对应行且字段正确

### 步骤 10：Tauri 命令

在 `lib.rs` 注册（参考现有 `set_proxy` / `proxy_status` 写法）：

```rust
#[tauri::command]
fn relay_list_providers(app: AppHandle) -> Result<Vec<ProviderDto>, String>;
#[tauri::command]
fn relay_save_provider(app: AppHandle, provider: ProviderInput) -> Result<ProviderDto, String>;
#[tauri::command]
fn relay_delete_provider(app: AppHandle, id: String) -> Result<(), String>;
#[tauri::command]
fn relay_list_accounts(app: AppHandle, provider_id: Option<String>) -> Result<Vec<AccountDto>, String>;
#[tauri::command]
fn relay_save_account(app: AppHandle, account: AccountInput) -> Result<AccountDto, String>;
#[tauri::command]
fn relay_delete_account(app: AppHandle, id: String) -> Result<(), String>;
#[tauri::command]
fn relay_set_mode(app: AppHandle, mode: String, default_provider: Option<String>) -> Result<Settings, String>;
#[tauri::command]
fn relay_escape(app: AppHandle) -> Result<Settings, String>;  // 逃生门
#[tauri::command]
fn relay_test_provider(app: AppHandle, id: String) -> Result<String, String>;  // 连通性测试
```

**`ProviderDto` / `AccountDto` 严格遵守 §5.4：不含明文 secret，只有 `masked`。**

### 步骤 11：前端

- `frontend/src/lib/relayApi.ts`：命令桥接
- `frontend/src/lib/providerDraft.ts`：草稿校验纯逻辑 + `.test.mjs`（遵循 AGENTS.md 的测试规范）
- `frontend/src/components/settings/RelaySection.tsx`：设置页新增「厂商反代」分区
  - 模式切换（观测 / 反代）
  - 厂商列表（增删改、启用开关、连通性测试）
  - 账号列表（按厂商分组）
  - 逃生门按钮 + 熔断状态提示
- `frontend/src/views/SettingsPage.tsx`：挂载新分区

**界面配色必须用 `Jolu.pen` 的 `jolu-*` 变量**；设计稿没有这个界面，按 AGENTS.md 标注「待确认」并先问鼠鼠，不要自行发明。

### 步骤 12：文档与收尾

- `docs/specs/2026-10-29-gateway-design.md`：把本文档的架构决策沉淀为设计说明
- `AGENTS.md` 的「现有实现」列更新（模块 B / G）
- `docs/todo/2026-10-22-unverified-integrations.md` 增加真机验证项

---

## 10. 验证要求

### 10.1 自动化

```bash
cargo test --manifest-path backend/Cargo.toml
pnpm --dir frontend typecheck
node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs
```

新增测试必须覆盖：

| 区域 | 必测内容 |
| --- | --- |
| 协议转换 | 每种组合的往返等价；§6.4 七个坑各一条 |
| 流式映射 | §6.3 五种场景 + 中途断开 |
| 路由 | §7 全部判定分支 |
| 凭证 | 刷新成功 / 刷新失败 / 回写冲突 / 过期边界 |
| 迁移 | 旧设置文件加载；旧 `requests` 行不受影响 |
| 逃生门 | 手动还原彻底性、熔断触发、崩溃恢复三条路径 |

### 10.2 真机验证（必须在隔离目录跑，见 `scripts/acceptance/README.md`）

1. Claude Code 配 API Key 厂商（如 Kimi 的 Anthropic 端点）→ 发一条消息 → 流式正常、`requests` 表有记录
2. 跨协议转换：Claude Code 配一个只有 OpenAI Chat 端点的厂商 → 消息正常、工具调用正常
3. 账号池：配两个账号，手动让第一个 401 → 自动切第二个，CLI 无感
4. 熔断：配一个错误 base URL → 连续失败 5 次 → 自动切回直连 + 通知
5. 逃生门：手动点「切回直连」→ `~/.claude/settings.json` 恢复原样
6. 官方订阅：「待确认」——见 §12

### 10.3 界面验证

改完界面用 Pencil MCP 截图对应画板对照（AGENTS.md §5 要求）。本功能是新界面，先确认画板是否存在。

---

## 11. 风险与已知限制

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| 协议转换语义不完整 | 某些工具调用/思考内容行为异常 | 本期只承诺矩阵内组合；不支持的明确报错 |
| 长上下文内存峰值 | 100MB+ 请求可能导致内存尖峰 | 本期接受，记入 todo；二期做流式改写 |
| OAuth 刷新与 CLI 冲突 | 用户被迫重新登录 | §5.3 方案 A（回写 + mtime 比对） |
| 官方订阅真机验证缺样本 | 可能上线即坏 | **必须**在发布前找真实订阅账号验证，未验证不得宣称支持 |
| 用户配置错误导致全断 | CLI 不可用 | 三层逃生门（§8） |
| 厂商协议漂移 | 某天突然不兼容 | 连通性测试命令 + 明确错误信息，不做自动降级 |

---

## 12. 待确认（需鼠鼠决策，开工前必须有答案）

1. **官方订阅反代的真机验证样本**：仓库里有可用的 Claude / Codex 订阅账号吗？没有的话这部分只能做到「代码完成 + mock 测试」，不得标记为已验证。
2. **设置页 UI 归属**：新增「厂商反代」是独立分区，还是并入现有「网络与代理」？`Jolu.pen` 里没有这个界面，需要鼠鼠给方向或补画板。
3. **预置厂商模板**：要不要内置 Kimi / GLM / DeepSeek / MiniMax 等常用厂商的 base URL 模板（减少用户填写）？如要，清单由鼠鼠确认。
4. **默认端口**：沿用现在的 12731，还是反代模式单独分配端口（避免与观测模式切换时冲突）？

---

## 13. 交付标准

全部满足才算完成：

- [ ] `cargo test` 全绿，新增测试覆盖 §10.1 全部条目
- [ ] `pnpm --dir frontend typecheck` 通过
- [ ] 前端测试全绿（含新增的 `providerDraft.test.mjs`）
- [ ] §10.2 前 5 条真机验证通过并记录到 `docs/verification/`
- [ ] 三层逃生门逐条手动验证
- [ ] 密钥不落明文：全库 grep 确认日志、导出、DTO 中无明文
- [ ] 旧设置文件升级后不丢配置，降级回旧版本不崩
- [ ] `AGENTS.md` 现有实现列已更新
- [ ] §12 四项已获得鼠鼠答复，未验证项明确标注「待确认」

**第 6 条真机验证（官方订阅）如果缺样本，必须在交付说明里写明「未验证」，不得含糊。**
