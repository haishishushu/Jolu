# 费用估算 · 价目与字号修复 执行文档（2026-11-03）

**交给：鼠鼠**
**状态：部分已实现；2026-11-04 已按代码校正，详见下方现状。**

## 2026-11-04 当前状态（优先于下方原计划）

- 模块拆分、扩充价目、峰谷与长上下文计算、单次费用拆解及来源展示已在 `0fd68a4` 实现，并随 v0.1.6 发布。原文“待实施”不再适用于这些项目。
- 本轮补齐未计价第三类 `unpriced_partial`：有任一原始用量字段（包括真实零或只有总量）但不能计价时，显示“分项不完整”；全无字段才算“无用量”。模型无价目仍独立分类。未改计价金额或历史数据。
- 已补 `scripts/pricing/sync-price.mjs`。明确选择精确模型名后，从第三方镜像生成 `output/pricing/<时间>/table.candidate.rs` 和 `review.json`；不直接改产品价目。复核记录带原表与上游 SHA-256、完整选中条目，保留阶梯和促销信息供核对。
- 与原步骤 1 的区别：不自动按前缀整批覆盖价目。平台归属、私有别名、长上下文及峰谷规则必须人工确认；候选生成保留本地复杂规则，不表示它们已与新上游规则一致。新增模型仍需先明确映射与规则。
- 网络、JSON、缺模型、缺来源、缺缓存读或非法价格均令命令失败。缓存写缺失保持未知，不猜倍率。全部校验完成才生成文件。说明与命令见 `scripts/pricing/README.md`。
- 字号仅已有语义名，目前仍为 10px/11px；放大与画板变更未实施，继续待鼠鼠决定。
- 本轮没有逐项重新核验下面价格数字，不把旧文档的“已核实”当成本轮结论。原文 `gpt-6.1-sol` 同时写“已核实”和“查不到”存在矛盾，不能直接作为维护输入。

### 另发现的独立费用风险（本轮未修改金额算法）

`usage_stats.rs::load` 计算 cost 调用 `estimate_usage`（内部用当前时间），而展示单价调用
`applied_price` 时使用请求自身 ts。历史请求跨峰谷时可能导致金额与拆解使用不同时段。
此为读码发现，尚未用固定时钟回归复现；应单独补测试并统一时间口径，不能宣称现有金额已全面核验。

### 本轮验证与限制

- 响应头与未计价后端回归均先观察到预期失败；前端缺少“分项不完整”的断言也先失败，修复后通过。
- 前端纯逻辑、hooks 与价目脚本共 272 项通过，前端类型检查通过。真实概览组件合成验收通过，截图 `output/acceptance/usage-overview/unpriced-reasons.png`；原验收脚本补入已新增的 fx 参数。
- Node 直接下载上游在当前网络下返回 fetch failed，没有生成候选；通过系统网络下载同一 JSON 后用 `--input` 成功生成 claude-sonnet-4-5 的候选，生产价格未改变。
- 后端首次并行全量：774 通过、1 失败、36 忽略；失败为既有脱敏性能测试超过单例 1 秒上限，独立复跑通过。串行复跑发现网关两个测试复用 c-kimi，命中进程级模型缓存，修正暂停菜单测试连接 ID 隔离，未改网关产品行为。
- 修正测试 ID 后，最终串行后端全量 775 通过、0 失败、36 忽略（真实账号等外部条件测试）；日志 `output/acceptance/backend-maintenance-tests.txt`。类型检查与 `git diff --check` 通过。
- 鼠鼠已明确选择“先完成本轮修复，真实账号验收另做”；本轮未调用真实模型、未构建安装包、未提交或发布。
- Pencil 仍无法连接，未完成设计稿截图对照。本轮只追加既有费用说明中的分类文案，不改变布局。
- 网关只读预检：Claude/Codex/Gemini 命令可发现，相关配置路径存在；这些不证明登录有效。Windows Sandbox 不存在。没有读出凭据、修改真实 CLI 配置或发送模型请求；真实 OAuth、工具调用与还原链路仍未验收。

**关联：`backend/src/pricing.rs`、`frontend/src/index.css`**

---

## 第一部分 · 价目表补全

### 1. 问题现状

用量页的「估算费用」长期显示 `已计价 0 / N 次`。根因已定位：

**`pricing.rs` 的 `PRICES` 表只覆盖 Claude 4.x 及以下、Gemini、少量 Codex，共 44 条。** 而实际在用的模型全部不在表内：

| 实际在用的模型 | 价目表 | 结果 |
| --- | --- | --- |
| `claude-opus-5-5` | ❌ 最高只到 `claude-opus-4-8` | 未计价 |
| `claude-sonnet-5-5` | ❌ 最高只到 `claude-sonnet-4-6` | 未计价 |
| `GLM-5.3` / `GLM-5.3-Flash` | ❌ 完全没有 GLM | 未计价 |
| `deepseek-v4.1-flash-1017` | ❌ 没有 DeepSeek | 未计价 |
| `gpt-6.1-sol` / `gpt-6-sol` | ❌ 没有 gpt-6 系列 | 未计价 |

`price_of()` 找不到就返回 `None` → `estimate_usage()` 返回 `None` → `priced_requests` 为 0。

### 2. 参照对象：sub2api 的做法

`Wei-Shaw/model-price-repo`（sub2api 和 CRS 共用的价目仓库）的架构：

1. **上游同步**：GitHub Actions 每 10 分钟从 LiteLLM 的 `model_prices_and_context_window.json` 拉取
2. **前缀筛选**：`claude-` / `gpt-` / `gemini-` / `deepseek-`
3. **自定义补充**：LiteLLM 没有的（GLM / Kimi / MiniMax / Grok）手工维护在 `config.json` 的 `custom_models`
4. **每条带 `source`**：指向官方定价页 URL（如 `https://bigmodel.cn/pricing`）
5. **别名映射**：同价变体（`claude-opus-4-8` → `claude-opus-4-7`）显式声明
6. **哈希校验**：输出 SHA-256，内容不变则不提交

**核心思想：不自己编价，每个数字都能追溯到官方定价页。**

### 3. 已核实的价目数据

从 sub2api 的价目文件取到的实际数值（每百万 token，USD）：

| 模型 | 输入 | 输出 | 缓存读 | 缓存写 | 官方来源 |
| --- | --- | --- | --- | --- | --- |
| `claude-opus-5-5` | 4.00 | 20.00 | 0.20 | 5.00 | platform.claude.com |
| `claude-opus-5` | 5.00 | 25.00 | 0.50 | 6.25 | platform.claude.com |
| `claude-sonnet-5-5` | 2.00 | 10.00 | 0.10 | 2.50 | platform.claude.com |
| `claude-sonnet-5` | 2.00 | 10.00 | 0.20 | 2.50 | platform.claude.com |
| `claude-haiku-5-5` | 0.10 | 0.50 | 0.01 | 0.125 | platform.claude.com |
| `glm-5.3` | 1.40 | 4.40 | 0.26 | — | docs.z.ai/guides/overview/pricing |
| `glm-5.2` | 1.18 | 4.13 | 0.29 | 1.18 | bigmodel.cn/pricing |
| `deepseek-v4-flash` | 0.30 | 1.20 | 0.006 | 0 | api-docs.deepseek.com |
| `kimi-k3` | 2.95 | 14.74 | 0.29 | 2.95 | platform.kimi.com |
| `gpt-6.1-sol` | 2.00 | 10.00 | 0.10 | 2.50 | developers.openai.com |
| `gpt-6-sol` | 2.00 | 10.00 | 0.10 | 2.50 | developers.openai.com（与 6.1 同价） |
| `gpt-5.6-sol` | 5.00 | 30.00 | 0.50 | 6.25 | developers.openai.com |

**`gpt-6.1-sol` 已核实**（2026-11-03 复核）：OpenAI 官方定价页与多个独立来源一致，标准价 $2/$10，缓存读 $0.10、缓存写 $2.50。**初稿误标「未收录」，已更正。**

**`claude-sonnet-5` 的重要说明**（原文件带注释）：官方已取消 2026-09-01 涨到 $3/$15 的计划，**$2/$10 是永久标准价**。按旧数据录会多算 50%。

**`gpt-5.6-sol` 存在促销价**：OpenAI 于 2026-08 起做降价（$5/$30 → $4/$20），至少持续到 2026-11-21。表内录标准价，促销期会**高估**约 20%。

### 3.1 长上下文加价：三家共有的通用规则

这是初稿低估的部分。不是 GLM 的特例，而是**主流厂商的通用机制**：

| 厂商 | 阈值 | 加价规则 |
| --- | --- | --- |
| GLM-5 | 输入 > 32K | 输入 ×1.5、输出 ×1.22 |
| GPT-6.1 Sol | 输入 > 272K | 输入/缓存 ×2、输出 ×1.5 |
| GPT-6 Sol | 输入 > 272K | 同上 |
| Gemini 2.5 Pro | 输入 > 200K | 现有 `GEMINI_LONG` 已实现 |

**关键细节：整条请求按新价算，不是只对超出部分加价。** 官方原文（GPT-6.1 Sol）：

> For long-context requests (>272K input tokens), pricing increases: 2× input/cache rates and 1.5× output rates **for the full request**.

现有 `Price` 结构完全表达不了这个规则 → 长会话（如大项目 refactor）费用会被**严重低估**。这是必须解决的。

### 4. 需要扩展的数据结构

现结构表达不了三个真实计费特性：

```rust
struct Price {
    platform: &'static str,
    model: &'static str,
    input: f64,
    cached_input: f64,
    output: f64,
    cache_write: Option<f64>,
}
```

**缺失项：**

| 缺失 | 真实场景 | 影响 |
| --- | --- | --- |
| 峰谷价 | DeepSeek：谷时输入 $0.15（半价），工作日特定时段 | 全按峰价算会高估约 50% |
| 长上下文阶梯 | GLM-5：输入 >32K 时 ×1.5；GPT-5.6：>272K 时翻倍 | 长会话费用严重低估 |
| 官方来源 | 无法追溯 | 价格错了查不到依据 |

**建议结构：**

```rust
struct Price {
    platform: &'static str,
    model: &'static str,
    input: f64,
    cached_input: f64,
    output: f64,
    cache_write: Option<f64>,
    /// 官方定价页，便于追溯与人工复核
    source: &'static str,
    /// 峰谷定价：谷时倍率与时段（UTC 小时 + 星期），None 表示无峰谷
    off_peak: Option<OffPeak>,
    /// 长上下文阶梯：超过阈值后的倍率
    long_context: Option<LongContext>,
}
```

### 5. 模块分层（不拆大模块，只拆价目表）

研究 sub2api 后的结论：**计价逻辑不该拆，价目表必须拆**。

现状 `pricing.rs`（199 行）把两件事混在一起：

```
pricing.rs
├─ 计价逻辑（estimate_usage、Gemini 长上下文、Codex 缓存扣除）  ← 稳定，几十行
└─ PRICES 表（44 条硬编码价目）                              ← 会膨胀到几百条
```

sub2api 的价目文件已经证明了膨胀速度：带 `source` / 峰谷价 / 长上下文阶梯 / 别名映射之后，单条记录就有十几个字段。

**拆成：**

```
backend/src/pricing/
├─ mod.rs      计价逻辑 + Price / Usage 类型（对外 API 不变）
└─ table.rs    价目数据（脚本生成，文件头注明「不要手改」）
```

**关键约束：`pricing` 的 4 个对外符号保持不变**

```rust
pub const GEMINI_LONG_PROMPT_TOKENS: i64 = 200_000;
pub struct Usage { ... }
pub fn estimate_usage(...) -> Option<f64>
pub fn estimate(...) -> Option<f64>
```

这样 5 个调用方（`db.rs`、`usage_stats.rs`、`session_threads.rs`、`db_breakdown_tests.rs`）**一行都不用改**。这是判断拆得对不对的标准：拆完调用方无感知。

**不做的拆解**：不做独立 crate、不做独立进程、不与 `fx.rs` 合并。`fx.rs`（汇率）保持独立是对的——它只管 USD→CNY 换算，有缓存和兜底，与计价是两条正交的职责。

### 6. 明细可解释性（新增）

研究 sub2api 后补的第三块。它有一条原则值得照搬：**未定价模型显示 `unknown`，绝不显示 `$0.00`**，理由是「避免『未知』被误读成『免费』」。

Jolu 现在顶部显示「已计价 0 / 396 次」方向对了，但用户看不出「为什么是 0」。补三点：

**6.1 单次费用拆解**

调用明细点开某一行时，从「估算 ¥0.0234（按内置价目表）」改成展示计算过程：

```
输入 1.2K × $2.00/M   = $0.00240
输出 500  × $10.00/M  = $0.00500
缓存读 800K × $0.20/M = $0.16000
─────────────────────────────
合计                   = $0.16740（按 platform.claude.com 2026-11-02 价目）
```

实现要点：后端 `CallRow` 需要带上该次请求命中的单价（`input_price` / `output_price` / `cached_price` / `cache_write_price`）与 `source`，前端才能画这张表。金额仍以最终 `cost` 为准，拆解只是展示。

**6.2 未计价原因细分**

现在只有一个「已计价 0 / N 次」。改成区分三种原因：

| 原因 | 展示 |
| --- | --- |
| 模型不在价目表 | `模型未收录价目` + 列出是哪些模型 |
| 用量字段缺失（上游没返回 token） | `上游未返回用量` |
| 只有部分字段缺失 | `分项不完整` |

后端在 `CostSummary` 里加 `unpriced_reasons: { no_price: Vec<String>, no_usage: i64, partial: i64 }`，前端按实际情况显示。这样用户知道是「Jolu 不认识这个模型」还是「这个工具没写 token」。

**6.3 价目来源标注**

模型行的「估算费用」悬停显示该模型的价目来源与核对日期：

```
claude-opus-5-5 · $4.00/$20.00 per 1M
来源：platform.claude.com（2026-11-02 同步）
```

数据来自 `Price.source` 字段（见 §4）。

### 7. 实施步骤

**步骤 1 · 写同步脚本**（`scripts/pricing/sync-price.mjs`）

- 输入：`https://raw.githubusercontent.com/Wei-Shaw/model-price-repo/main/model_prices_and_context_window.json`
- 筛选：只保留 Jolu 需要的平台前缀（claude / gpt / gemini / deepseek / glm / kimi / minimax / grok）
- 输出：`backend/src/pricing/table.rs`（Rust 静态数组，文件头注明「由脚本生成，不要手改」）
- **网络不可达时必须失败退出，不生成半份表**（避免静默缺价）

**步骤 2 · 拆分模块**

- `pricing.rs` → `pricing/mod.rs` + `pricing/table.rs`
- 4 个对外符号保持不变，确认 5 个调用方零改动

**步骤 3 · 扩展 `Price` 结构**

- 加 `source` / `off_peak` / `long_context` 三个字段
- `estimate_usage()` 相应处理：按请求时间戳判定峰谷、按 input token 数判定阶梯

**步骤 4 · 补全模型**

- 用步骤 1 的脚本生成为主
- 脚本覆盖不到的（如 ZCode 的私有模型别名 `deepseek-v4.1-flash-1017`）**显式标注「未收录」，不猜**

**步骤 5 · 未计价原因细分**

- `CostSummary` 加 `unpriced_reasons: { no_price: Vec<String>, no_usage: i64, partial: i64 }`
- 前端按原因分别展示（§6.2）

**步骤 6 · 单次费用拆解**

- `CallRow` 增加 `price_source` / 四项单价字段（只用于展示，金额仍以 `cost` 为准）
- 调用详情展示计算过程（§6.1）

**步骤 7 · 界面如实标注**

- 已计价：「估算费用 · CNY/USD」
- 部分未计价：「已计价 N / M 次 · 其余模型未收录价目」
- 点击可展开「哪些模型未收录」
- 模型行费用悬停显示价目来源与核对日期（§6.3）

**步骤 8 · 验证**

- 单测：峰谷边界（时段内/外）、长上下文阶梯边界
- 单测：未收录模型返回 `None` 而不是 0
- 单测：`unpriced_reasons` 三种原因各自命中
- 单测：拆解展示的单价之和与 `cost` 一致（浮点容差内）
- 回归：现有 730 个测试全绿

### 8. 不做的事

- **不自动联网拉取**（保持离线可用，脚本在发版前手动跑）
- **不猜价格**：官方页查不到就标「未收录」
- **不做用户自定义单价**（本期不做，记入待办；套餐用户按 API 单价估算本来就不准）
- **不把未计价模型算作 0 元**：总额只含已计价部分，未计价单独计数
- 不改历史数据：已有记录的 `cost` 是快照，不回算
- 不引入独立 crate 或独立进程：`pricing` 只做文件拆分，对外 API 不变

### 9. 关键设计约束

**为什么"顶部总额 ≠ 各模型费用之和"是可以接受的**

未收录价目的请求既不出现在模型行费用里，也不计入总额。界面通过「已计价 N / M 次」明确告知覆盖范围。若把未收录算作 0 元加进总额，总额会偏低且看起来像真花少了——比显示「—」更有害。

**为什么金额以 `cost` 为准而不是前端重算**

拆解展示只用来看"怎么算的"，最终金额由后端一次算定。前端重算会因浮点、单位、倍率理解差异产生分歧，导致"明细加起来不等于总额"的投诉。

---

## 第二部分 · 灰色小写字号

### 1. 现状

统计结果：

| 字号 | 出现次数 | 典型用途 |
| --- | --- | --- |
| `10px` | **152 处** | 卡片说明、表格辅助列、状态提示 |
| `11px` | **264 处** | 次要标签、行内提示 |
| `9px` | 6 处 | 极小的角标 |

其中「灰色小字」（`text-jolu-muted` + 10/11px）在 1080p 及以下屏幕上确实偏小——11px 中文在高 DPI 缩放下可读性差。

### 2. 方案：集中定义语义化字号，而不是逐处改

逐处改 400+ 个位置风险高、容易漏、后续新增代码又会退回旧值。

**推荐做法**：在 `index.css` 用 Tailwind 的 `@theme` 定义字号 token，然后批量替换：

```css
@theme {
  /* 辅助文字：原 10px → 11px */
  --font-size-note: 11px;
  --font-size-note--line-height: 1.45;
  /* 次要文字：原 11px → 12px */
  --font-size-meta: 12px;
  --font-size-meta--line-height: 1.5;
}
```

对应的类名变成 `text-note` / `text-meta`，替换规则：

| 原 | 新 | 提升 |
| --- | --- | --- |
| `text-[9px]` | `text-note`（11px） | +2px |
| `text-[10px]` | `text-note`（11px） | +1px |
| `text-[11px]` | `text-meta`（12px） | +1px |

### 3. 为什么用 token 而不是直接改数字

1. **可回退**：觉得太大只改 `@theme` 一处
2. **一致性**：后续新代码写 `text-note` 而不是随手 `text-[10px]`
3. **可审计**：`grep text-\[1[01]px\]` 应该归零

### 4. 风险与边界

- **会改变布局**：字号变大会挤压横向空间，表格列、卡片行高需要复查
- **画板对照**：`Jolu.pen` 里这些字号是设计定的。按 AGENTS.md「冲突时以 `Jolu.pen` 为准」，**这一步需要鼠鼠明确批准偏离设计稿**，或者同步更新画板里的字号
- **灵动岛不受影响**：`components/island/` 的尺寸是像素级精调的浮层，本次不纳入

### 5. 实施步骤

1. `index.css` 加 `@theme` 字号 token
2. 批量替换主窗口（`views/` 除 `IslandWindow`、`components/` 除 `island/`）
3. `pnpm typecheck` + 逐页目视复查（重点是表格列宽与卡片高度）
4. 用 Pencil 截图对照 `Jolu.pen` 画板 30–39（用量统计），确认视觉可接受
5. 如鼠鼠要求同步设计稿，再用 Pencil 更新画板字号

---

## 待鼠鼠确认

1. **价目数据**：上表列的数字直接采用，还是你要逐条对官方页核对？
2. **`gpt-6.1-sol` 等查不到的模型**：同意按「未收录」处理吗？
3. **字号偏离设计稿**：10px→11px、11px→12px 这个幅度可以吗？要不要同步更新 `Jolu.pen`？
4. **峰谷价 / 长上下文阶梯**：这两项本期就做，还是先只做基础价（简单但 DeepSeek 谷时和 GLM 长上下文会偏）？
5. **明细可解释性**（§6 三项：单次拆解 / 原因细分 / 来源标注）：本期一起做，还是先做价目补全、明细留到下一期？
6. **模块拆分**（§5）：同意拆成 `pricing/mod.rs` + `pricing/table.rs`（对外 API 不变、调用方零改动）吗？

---

## 附：为什么不做成「独立功能模块」

**结论：不做。** 理由：

| 考虑 | 判断 |
| --- | --- |
| 计价逻辑本身 | 约 40 行纯函数，输入用量输出金额，无状态无 IO——拆成独立模块只会增加跳转成本 |
| 真正会膨胀的 | 价目表（几百条、带 source/峰谷/阶梯/别名）——这部分拆文件即可 |
| 与 `fx.rs` 的关系 | 正交：一个算金额、一个换币种。保持独立，不合并 |
| 与 `usage_stats.rs` 的关系 | 统计层依赖计价层是正当依赖，不是耦合问题 |
| 独立 crate / 进程 | 收益为零，只增加构建与调试复杂度 |

**判断标准**：拆完之后，5 个调用方（`db.rs`、`usage_stats.rs`、`session_threads.rs`、`db_breakdown_tests.rs`）一行都不用改——这就是拆对了。
