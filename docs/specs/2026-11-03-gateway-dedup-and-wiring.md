# 模型入口：去重、唯一性与接线可靠性方案

> 适用模块：主窗口「模型入口」（原账号汇聚网关，`Jolu.pen` 模块 J）。
> 设计稿：`docs/ui/Jolu.pen` 的 `s1Bm7` / `Lz482` / `eMBlV` / `g1Uhz`。
> 本文取代早期计划里关于 slug、重复凭据与接线锁的零散描述；与 `Jolu.pen` 冲突时以画板为准。

## 1. 目标

同时保证两件事：

1. **分组模型一起路由时去重**：同一账号、同一模型在一次请求里最多尝试一次，一起冷却。
2. **接入 agent 时唯一**：每个 agent 只有一条接线记录、一个接管者，所有改配置的入口共用一把锁（跨进程也锁）。

做法是把「谁决定路由」和「谁决定排序」彻底分开：来源分组与 Agent 分组只管排序和界面，代码里不让它们碰路由、接线和 token。

## 2. 三个概念的边界

| 概念 | 管什么 | 不管什么 | 单归属 |
|---|---|---|---|
| 来源分组（模型分组） | `/v1/models` 的排序、Jolu 里来源的排列 | 菜单里有哪些条目、路由、冷却 | 一个连接最多在一个组；没有成员行就是「未分组」 |
| Agent 分组 | Jolu 里 Agent 的排列 | 接线、token、接管者、菜单 | 一个 agent 最多在一个组 |
| 路由组（以后做） | 一个 `route/<slug>` 名称对应多个 (连接, 模型)，可故障转移 | 排序 | 成员按 (connection_id, model) 去重 |

如果以后要「把来源分组当作路由入口」，就另外生成一个路由组。来源分组本身始终不参与路由。

## 3. 已拍板的八项决定

| # | 问题 | 决定 | 理由 |
|---|---|---|---|
| 1 | 暂停 / 认证失效的连接 | 留在菜单，条目带状态；请求按原因返回 401 / 429 / 503 并说明怎么恢复。只有用户主动关闭服务才移出菜单 | agent 配置里存的 id 不能因暂停失效 |
| 2 | 改过的旧 slug | 返回 404 并说明现在的 id 或「来源已删除」，不做别名 | 避免用户本意切走时又被悄悄转回去 |
| 3 | 存量重复凭据 | 每个 (account_fp, protocol) 只保留一条参与服务，优先最早创建；其余只关闭服务、写 `migration_log`、可一键恢复 | 不删数据，失败可回退 |
| 4 | 连接名撞保留字 | slug 分配时保留 `route / auto / jolu / group / default`，撞上时改名为 `<slug>-src` | 持久化 slug 上线前没有存量 id 要保护；`~` 等字符在 agent 模型字段里的行为未核对 |
| 5 | 路由组在菜单里 | 单独成段放在所有来源之后；成员默认不隐藏 | `route/x` 是主动入口，隐藏成员等于变相改 id |
| 6 | 回环地址上的未知 token | 放行并记 unknown；同时校验 Host、拒绝带 Origin 的浏览器请求 | 本机进程本就能读 `~/.claude`；要防的是浏览器 CSRF / DNS rebinding |
| 7 | 接线记录 | 迁到 `agent_wiring` 表，`settings.gateway_wiring` 双写保留一个版本 | pending 日志与 instance 标记在 settings 上做不了 |
| 8 | 整组接线 | 先不做；以后只做前端批量，逐个调用单个接线接口 | 分组一旦能触发接线，就违反「分组不碰接线」 |

## 4. 不变量清单

| 编号 | 规则 | 保证方式 |
|---|---|---|
| S1 | 每个连接有持久化 slug，全局唯一，改名不变 | 所有 INSERT 走 `identity::allocate_slug`；`idx_connections_slug` 唯一索引；调试构建的触发器拒绝空 slug，发布构建自动补 `legacy-<hex>` 并记 `migration_log`（保证回退旧版仍能添加连接）；`load_pool` 只读列，不再现算 |
| S2 | slug 不给别的连接复用；同一 connection_id 或同一 account_fp 重建时可复用 | `slug_history(slug PK, connection_id, account_fp, retired_ms)`；删除时同事务写入 |
| S3 | 字符集 `[a-z0-9-]`，不以 `-` 开头，不能是保留字 | `identity::valid_slug` + CHECK 等价的触发器；纯中文名兜底 `<platform>-<sha256(id)[..6]>` |
| S4 | 同一账号、同一协议只允许一个连接参与服务 | `account_fp` + 部分唯一索引 `(account_fp, IFNULL(protocol,'')) WHERE serve_enabled=1` |
| S5 | 订阅在「每个连接独立凭据目录」之前，每个 platform 只能有一个 auth 连接参与服务 | 部分唯一索引 `(platform) WHERE kind='auth' AND serve_enabled=1`；`set_serve` 先查冲突并给出名字 |
| G1 | 连接、agent 单归属 | 成员表以 connection_id / agent_id 为主键，移动分组用 UPSERT |
| G2 | 分组表不进入路由、接线、token、菜单成员资格 | 测试 grep `gateway/route.rs`、`wire.rs`、`wiring.rs`、`escape.rs`、`proxy.rs` 不得引用 `*_group` 表 |
| G3 | 删除分组不影响连接、模型、agent | `PRAGMA foreign_keys=ON`；成员表 `ON DELETE CASCADE` |
| M1 | `/v1/models` 的 id 不重复 | slug 全局唯一 + 路由组保留前缀；`models_catalog()` 用 HashSet 兜底并记诊断 |
| M2 | 菜单条目只由服务状态和上游模型列表决定；分组只改顺序 | 先算集合，再排序 |
| R1 | 路由组成员按 (connection_id, model) 去重 | 唯一索引 + 保存时规范化（待路由组上线） |
| R2 | 嵌套不成环，深度 ≤ 8 | 保存时 DFS 报路径；运行时 path 集合再截断（待路由组上线） |
| R3 | 一次请求里同一 account_fp 最多尝试一次，不看 model | attempt 循环维护 `exhausted_fps`（待故障转移上线） |
| R4 | 冷却和失效按 account_fp 计，单来源与路由组共用 | `Cooldowns` 用 `CredKey`；429 后按 fp 冷却 |
| R5 | 只在还没向调用方写出任何字节时转移 | `committed` 标志（待故障转移上线） |
| R6 | 用量只记真正应答的那次 | `requests` 只有采集器写；尝试记录以后放 `gateway_attempts` |
| R7 | 亲和用 CAS 写入，不来回跳 | 条目带 epoch（本次先保留现有绑定语义） |
| A1 | agent_id 只来自 `wiring::AGENTS`，保持 `gemini` | `agents` 种子表 + 测试断言三方一致 |
| A2 | 每个 agent 每个 Jolu 实例一个 token：`jolu-<agent>.<instance8>` | 写入 Claude / OpenCode / ZCode；Codex、Gemini 的登录来自 OAuth / 环境变量，本版不写 token（见 §7） |
| A3 | 每个 agent 只有一个接管者；全局 mode 是唯一来源 | `agent_wiring.owner`；切换在一个流程里改完 |
| A4 | 接线幂等；部分一致按漂移处理；重新应用不重新快照 | `agent_wiring.applied_json` + 快照过滤 |
| A5 | 快照不收任何网关值；发现别的实例标记就拒绝接线 | `is_gateway_value` 过滤；`foreign_marker` 检查 |
| A6 | 所有读写 agent 配置的入口持同一把锁，进程内 + 跨进程 | `cli_switch::with_switch`；Windows 命名互斥量 `Global\jolu-wiring` |
| A7 | 先记 pending 再写文件，写完改成 wired；启动时修复 | `agent_wiring.state` |
| A8 | 状态和断开按文件里实际写下的值判断，不按 settings 判断 | 各 agent 的 `*_current_*` 解析函数；端口被占时中转模式拒绝启动 |

## 5. 数据模型

### 5.1 connections 新增列

- `slug TEXT`：持久化菜单 id 前缀。迁移时回填，之后改名不变。
- `account_fp TEXT`：账号指纹，`sha256(kind \n platform \n secret)` 的前 32 位十六进制。没有凭证的行为 NULL。

### 5.2 新表

```sql
slug_history(slug TEXT PRIMARY KEY, connection_id TEXT NOT NULL, account_fp TEXT, retired_ms INTEGER NOT NULL)
agents(id TEXT PRIMARY KEY)                       -- 种子：AGENTS 五项
agent_wiring(agent_id TEXT PRIMARY KEY REFERENCES agents(id),
             instance_id TEXT NOT NULL, owner TEXT NOT NULL DEFAULT 'none',
             state TEXT NOT NULL DEFAULT 'idle',  -- idle | pending | wired
             original_json TEXT, applied_json TEXT, token TEXT UNIQUE, wired_ms INTEGER)
source_groups(id TEXT PRIMARY KEY, name TEXT NOT NULL, sort INTEGER NOT NULL, created_ms INTEGER NOT NULL)
source_group_members(connection_id TEXT PRIMARY KEY REFERENCES connections(id) ON DELETE CASCADE,
                     group_id TEXT NOT NULL REFERENCES source_groups(id) ON DELETE CASCADE,
                     sort INTEGER NOT NULL DEFAULT 0)
agent_groups(id TEXT PRIMARY KEY, name TEXT NOT NULL, sort INTEGER NOT NULL, created_ms INTEGER NOT NULL)
agent_group_members(agent_id TEXT PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
                    group_id TEXT NOT NULL REFERENCES agent_groups(id) ON DELETE CASCADE,
                    sort INTEGER NOT NULL DEFAULT 0)
migration_log(id INTEGER PRIMARY KEY, at_ms INTEGER NOT NULL, kind TEXT NOT NULL, detail TEXT NOT NULL)
```

### 5.3 迁移顺序

1. 打开数据库时 `PRAGMA foreign_keys=ON`。
2. 补列 `slug` / `account_fp`。
3. 按 `created_ms, id` 回填 slug：`slugify(name)` → 保留字加 `-src` → 重名加 `-2`；纯中文等空结果用 `<platform>-<hash6>`。
4. 回填 account_fp。
5. 存量重复服务：按 `(account_fp, IFNULL(protocol,''))` 和 `(platform WHERE kind='auth')` 分组，每组保留最早创建的，其余 `serve_enabled=0` 并写 `migration_log`。
6. 建部分唯一索引（先回填再建；失败只记诊断，不阻止启动）。
7. 建 `slug_history`、`agents`、`agent_wiring`、分组表、`migration_log`。
8. 建触发器：connection 的 slug 插入 / 更新不得为空；agent_wiring 的 agent_id 必须存在于 agents。
9. 写 `slug_history`：`connections::remove` 在同一事务里记录 slug、connection_id、account_fp。

## 6. 路由

### 6.1 模型解析

1. 在第一个 `/` 处切开。前缀是 `route` → 查路由组（以后）；否则按 `connections.slug` 精确匹配。
2. 未命中时查 `slug_history`：
   - 旧 slug 属于仍存在的连接 → 404「x 现在叫 y」；
   - 记录存在但连接已删除 → 404「来源已删除」。
3. 命中但 `status='paused'` → 503「连接已暂停，请在供应商页恢复」。
4. 命中但 `status='invalid'` → 401「登录已失效，请重新登录」。
5. 命中但 `status='exhausted'` → 402「额度已用尽，次日恢复」。
6. 冷却中 → 429 + `retry-after`，单来源不换别家。

### 6.2 去重与冷却（本次落地部分）

- 冷却键从 connection_id 换成 `CredKey`（account_fp 优先，缺失时 `id:<connection_id>`），429 后按账号冷却。
- 会话亲和保持「换 slug = 用户故意换模型」的语义；会话绑定连接在冷却中时仍拒绝而不是偷偷换连接。
- 同账号多连接（同一把 Key 的双协议端点）共享冷却，避免一个端点被限流后另一个继续撞。

### 6.3 模型菜单

- 集合：所有 `serve_enabled=1` 的连接展开模型，得到 `<slug>/<model>`。slug 唯一，集合天然不重复；用 HashSet 兜底并记诊断。
- 排序：来源分组按 sort、组内按 sort、未分组最后（created_ms 升序）。分组读取失败不影响菜单。
- `display_name` = `<来源名> · <model>`；来源名重复时追加 `(<slug>)`。
- 暂停 / 失效的连接仍在列表里，条目带 `status`，`display_name` 追加「（已暂停）」「（登录已失效）」。
- 每次成功拉取模型清单都写入 `connections.models_json`；来源失效或重启后优先用这份清单，
  菜单 id 不会因为一次拉取失败消失。只有从未成功拉取且没有默认模型的来源才会进入 `unavailable`。

## 7. 接线

### 7.1 流程

```
持锁 → 读原文件 → 拒绝别的实例标记 → 算快照（过滤网关值）→ 写 pending → 备份 → 原子写 → 写 applied → 标记 wired
```

- `applied_json` 记录我们实际写下的值：`{"base": "...", "token": "...", "provider": "jolu-gateway", "agent": "..."}`。
- 还原只清理自己的写入：当前值已不指向本机网关时跳过，保留外部修改。
- 启动时修复 pending：文件里是我们写下的值 → 直接标 wired；不是 → 标 idle，不自动重写。

### 7.2 token

- token 格式 `jolu-<agent>.<instance8>`，`instance_id` 首次启动生成并存入设置。
- 已写入 token 的 agent：Claude Code、OpenCode、ZCode、Codex（只在其未登录 ChatGPT 时）、Gemini。
- **Codex**：`wire_api` 必须是 `responses`（当前 Codex 只接受这个值，`chat` 会让它拒绝加载整份配置）。
  - 已登录 ChatGPT（`auth.json` 有 OAuth tokens）：保留内置 openai provider，删掉 `model_provider`，
    只写顶层 `openai_base_url = <base>/backend-api/codex`；网关新增同名路由。
    这条路不写 token，靠 `originator` / `User-Agent` 识别 agent。
  - 未登录 ChatGPT：写 `[model_providers.jolu-gateway]` +
    `experimental_bearer_token = "jolu-codex.<实例>"`，`wire_api = "responses"`。
  - 还原时恢复原 `model_provider` / `openai_base_url`，并删掉 Jolu 的 provider 段。
  - 已知限制：Codex 发 Responses 请求，上游必须支持 `/v1/responses`；只支持 Chat Completions
    的第三方来源需要后续做 Responses → Chat 翻译，当前会由上游返回 404。
- **Gemini**：事务式接管，同时写两个文件：
  - `~/.gemini/.env`：`GOOGLE_GEMINI_BASE_URL` + `GEMINI_API_KEY=jolu-gemini.<实例>`；
  - `~/.gemini/settings.json`：`security.auth.selectedType = "gemini-api-key"`；
  - 快照保存原 `GOOGLE_GEMINI_BASE_URL` / `GEMINI_API_KEY` / `selectedType`，断开时逐项还原；
    OAuth 凭据文件不碰。
  - 接线期间 Gemini 不再用 Google 账号登录；`WiringStatus.notice` 已明确告知。
- **Gemini 原生端点**：网关新增
  `/v1beta/models/{model}:generateContent` 与 `:streamGenerateContent`，把 Gemini 请求翻成
  OpenAI Chat Completions，再把响应翻回 Gemini 形状（文本、图片、函数调用、用量、流式文本）。
  只服务 `protocol = openai` 的 API 连接；`countTokens` 按文本字符数本地估算（不冒充真实用量），
  `googleSearch` 等内置工具与多候选明确报错。
- **会话键**：优先级
  `X-Jolu-Session` → Claude `metadata.user_id`（JSON 里取 `session_id`）→ Codex
  `session-id` / `thread-id` / `x-client-request-id` → 通用会话头 →
  请求体 `prompt_cache_key` → 调用方 token + slug。
  Gemini 没有官方会话 id，目前只按来源选路，不做会话亲和。

### 7.3 状态判断

- Claude：解析 `env.ANTHROPIC_BASE_URL` 是否指向本机端口。
- Codex：解析 `model_provider` 与 `[model_providers.<provider>].base_url`，不看子串。
- Gemini：解析 `.env` 的 `GOOGLE_GEMINI_BASE_URL`。
- OpenCode / ZCode：解析 JSON `provider.jolu.options.baseURL`。
- 漂移（文件与 applied 不一致）分四种：已移除、被改写、被覆盖、网关不可达；本次先能识别「指向 / 不指向」，界面细分的文案待画板确认。

### 7.4 模式与开关

- 切到中转模式：先撤销观测接管（两种模式共用同一个端口），把 `proxy_enabled` 置为 true 并启动监听；
  端口被占直接报错并回滚模式，不再悄悄换端口。
- 中转模式下关闭「本地代理」开关：先逐 agent 还原接线，再停监听，最后关开关；不允许留下指向死端口的工具。
- 中转模式启动不做观测上游探测（`set_proxy` 的 relay 分支直接 `start()`），
  这样只用 API Key、没有安装 Claude / Codex 订阅工具的用户也能启动。
- 切回观测模式：`escape::manual` 还原接线并停网关，之后按观测流程重新接管。

### 7.5 跨进程锁

- 进程内 `Mutex` + Windows 命名互斥量 `Global\jolu-wiring`（创建失败回退 `Local\`）。
- 等待上限 15 秒，超时报「另一个 Jolu 正在修改工具配置」，不硬闯。
- `cli_apply::serialized_switch` 改为调用同一个入口，观测接管与网关接线互斥。

## 8. 安全

- 只监听 `127.0.0.1`。
- 网关拒绝带 `Origin` / `Referer` 的请求（浏览器跨站），并校验 `Host` 必须是 `127.0.0.1` / `localhost` / `[::1]`（可带端口），防 DNS rebinding。
- 未知 token 在回环地址上放行；以后开局域网共享时默认关闭，开启后每台设备一个 token、可吊销，监听地址单独配置。
- 中转模式下端口被占直接报错，不再悄悄改用随机端口（随机端口会让已接线的 agent 全部连不上）。
- 观测模式保留原来的随机端口回退：它不依赖固定端口，CLI 配置按实际端口写入。

## 9. 界面状态

以下状态已补到 `Jolu.pen` 的 `g1Uhz`（V3 延展（二）），浅色版：

| 画板 | 覆盖状态 |
|---|---|
| E4 · 重复凭据 | 新建 / 开启服务时被已有来源阻止；卡片上的「与 X 重复、未参与服务」；「为什么不自动合并」 |
| E5 · 菜单 id 与改名 | 显示名与菜单 id 分开；改名不改 id；改 id 的失效警告与「同时更新接线」；保留字报错与建议值 |
| E6 · Agent 接线漂移 | 已移除 / 被改写 / 被覆盖 / 网关不可达，各自的「重新应用 / 接受现状」；「已被另一个 Jolu 接管」 |
| E7 · 移动反馈与刷新 | 「已从 A 移到 B」与撤销；「Agent 分组不影响接线」；「分组已变化，请刷新」；同一工具第二份安装「暂不支持」 |
| E8 · Codex 两种接法 | 检测到 ChatGPT 登录时推荐保留 OAuth；未登录时才用网关 Key；两条路各自写什么、Responses 上游限制 |
| E9 · Gemini 登录方式切换 | 接线前告知 OAuth → API Key；当前 / 接线期间 / 断开时三行对照；确认勾选 |
| E10 · Gemini 还原失败 | 原值、当前值、配置文件路径；重试还原与手动修改说明 |

仍待确认：

- E4–E10 的**深色版**（目前全部是浅色稿）。
- 订阅「同平台只能有一个订阅参与服务」的专用文案（目前沿用 E4 的重复提示）。
- 分组管理页（重命名、删除成员、拖拽排序）本身，以及来源 / Agent 分组的后端命令。
- 路由组（以后）：成环路径、重复成员、跳过原因、全部不可用。

## 10. 实施顺序与状态

| 阶段 | 内容 | 状态 |
|---|---|---|
| 1. 身份 | slug、account_fp、slug_history、保留字、唯一索引、触发器、迁移回填与重复清理 | 已完成 |
| 2. 接线 | 统一锁（进程内 + `Global\jolu-wiring`）、`agent_wiring`、pending / 启动修复、快照过滤、状态解析、instance token、端口回退与 Host / Origin | 已完成 |
| 3. 菜单 | 分组读取与排序、状态标注、来源 / Agent 分组命令与界面 | 已实现（三栏总览、分组命令、拖拽排序 + Alt+↑/↓、保存视图、E1–E10 状态弹窗、OX4b7 动效） |
| 4. 路由组 | `route/<slug>`、故障转移、`gateway_attempts` | 未开始 |
| 5. Agent 登录方式 | Codex `openai_base_url` / 静态 token 两条路；Gemini 原生端点 + 事务式接管；会话头 | 已实现，真机验证待做 |

阶段 1、2 对应的主要文件：

- `gateway/identity.rs`：slug、指纹、迁移回填、去重、索引与触发器
- `gateway/wiring_store.rs`：`agent_wiring` 记录
- `gateway/wiring.rs`：快照过滤、状态解析、别的实例标记
- `gateway/route.rs`：持久化 slug、404 说明、按账号冷却
- `gateway/server.rs`：菜单状态、`models_json`、Host / Origin
- `cli_switch.rs`：进程内 + 跨进程锁
- `db.rs` / `connections.rs` / `native_sources.rs` / `provider_browser_auth.rs`：表结构、迁移与所有写入路径

## 11. 验证

- `cargo test --manifest-path backend/Cargo.toml`：774 通过（含新增的迁移、slug、指纹、接线记录、Host / Origin、暂停来源菜单、Codex 两种接法、Gemini 翻译与流式转换、分组排序测试）。
- `pnpm --dir frontend typecheck`、`pnpm --dir frontend build` 与
  `node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs`：通过。
- 新增单测：slug 分配与保留字、slug 历史复用、触发器的空 / 非法 slug 拒绝、迁移回填幂等、重复服务清理与唯一索引、旧 slug 404 说明、接线 pending 修复、快照过滤网关值、别的实例标记、Host / Origin 拒绝、暂停来源仍在菜单、Codex OAuth / provider 两种往返、Gemini 事务式接管与还原、Gemini 请求 / 响应 / SSE 翻译。
- 真机待验证：Codex 已登录 ChatGPT 时 `openai_base_url` 是否真的带 OAuth 头；Codex 的
  `session-id` / `thread-id` 实际取值；Gemini `selectedType = gemini-api-key` 下是否真的走网关；
  Gemini CLI 的流式与工具调用；断开后 `selectedType` 改回原值是否免重登。
- 中转模式端口占用拒绝启动没有单测（依赖真实监听端口），由 `proxy::start` 的分支校验。
- 改界面时再按 `AGENTS.md` 用 Pencil 截图对照。
