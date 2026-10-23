# Jolu 剩余功能实施计划

> **给执行代理：** 鼠鼠已选定 `superpowers:subagent-driven-development`：每个任务由一个子代理实现，再由另一个子代理审查，阶段 2 中互不依赖的任务可以并行。步骤用复选框（`- [ ]`）记录进度。
>
> 编写日期：2026-10-22。本文是计划，不代表功能已实现或已通过测试。取代 `2026-10-21-jolu-development-roadmap.md` 中尚未完成的部分。

**目标：** 补齐主窗口七个模块中标「待接入」的功能，在真机上跑通已写好的功能；依赖外部条件的项也写好代码，能测的测，不能测的写进 `docs/todo/`。

**架构：** 沿用现有分层：后端 Rust 负责凭证、文件、进程、网络和数据库，前端只做展示与交互，经 `lib/*Api.ts` 调用 Tauri 命令。新增后端模块平铺在 `backend/src/`，前端纯逻辑放 `frontend/src/lib/`，与测试同名同目录。

**技术栈：** React 19、TypeScript、Vite、Tailwind CSS 4、Tauri 2.11.5、Rust、rusqlite、reqwest 0.12.28；新增依赖只有 `tauri-plugin-notification`（任务 2.5）。

**依据：** `docs/ui/Jolu.pen`（只能用 Pencil MCP 读取）、根目录 `AGENTS.md`、2026-10-22 六份调研报告（结论已并入第二节「决策表」和各任务）。

## 一、全局约束

- 所有代码、配置和文档使用 UTF-8。
- 未经鼠鼠授权不执行 `git add`、提交、推送、切换分支或重置；每个阶段结束时请鼠鼠决定是否提交。
- API Key 和认证信息只在后端处理；前端只拿掩码；不写入日志、导出、诊断结果和交接文档。
- 未知、真实的零、不支持、失败、过期分别表达；桌面运行出错时不回退到 `mock/`。
- 不从 UI、图标或套餐名称推断供应商能力；每项外部能力都要有第二节列出的来源。
- 不删除旧账号、历史统计和原始会话文件；数据库加列用 `db.rs` `init` 里的幂等 `ALTER TABLE ... ADD COLUMN`，自建表用各模块的 `ensure_schema`。
- 修改外部工具的配置文件（Codex `config.toml`、Gemini `settings.json`、OpenCode `opencode.json`）前先备份；解析失败就不写入。
- 浮动灵动岛窗口的展示保持现状，本计划的任何开关都不作用于灵动岛。
- 改了界面要用 Pencil 截取对应画板与运行效果对照；画板没画的状态按第二节决策实现，并在代码注释里标「待确认」。
- 只有以下任务修改 `Jolu.pen`，均已获鼠鼠授权：
  - 任务 3.2：补画压缩对话弹窗，以及画板 48、50 的两处提示；
  - 任务 3.4：补画自定义供应商弹窗；
  - 任务 4.6：修改画板 29 中两行文案。

  其他任务不改设计稿。
- 多个代理并行时，每个任务只在 `lib.rs` 里自己模块的锚点注释处加 `mod` 和注册命令（格式：`// 任务 2.3 网络`），不改别人的锚点区域。
- 测试命令：
  - 前端单测：`node --test frontend/src/lib/<文件>.test.mjs`
  - 类型检查：`pnpm --dir frontend typecheck`
  - 后端：`cargo test --manifest-path backend/Cargo.toml <过滤词>`
  - 阶段末全量：前端 `node --test frontend/src/lib/*.test.mjs` 加后端全量 `cargo test`。

## 二、决策表

D-01 到 D-07 鼠鼠已回复「按推荐」。2026-10-22 鼠鼠审阅时又确认了：D-14、D-21、D-22、D-26 按推荐；D-09 由小的优化（已改为下表的版本）；D-05 先参考五家 harness 的做法，鼠鼠随后授权小的不受原方案限制自行优化，鼠鼠最终选定「分段纪要 + 合成」，压缩过程做成用户与 AI 对话的弹窗，并加上「预计发送约 N token」和「以下路径未在会话中出现」两处提示（见第三节 3.3、3.4）；其余细节由小的决定。执行方式：每个任务由一个子代理实现，再由另一个子代理审查。其余各条按推荐值实现。

| 编号 | 事项 | 决定 | 依据 |
|---|---|---|---|
| D-01 | 压缩会话用什么调模型 | 只用 Claude Code / Codex 命令行**当前登录**的账号，非交互、不落会话、不续接原会话：Claude `-p --no-session-persistence --output-format stream-json --verbose`；Codex `exec --json --ephemeral --sandbox read-only --skip-git-repo-check -`。工作目录用一个空的临时目录。其他连接在下拉里禁用并注明原因 | 调研 1；Context7 核对过参数 |
| D-02 | 交接文档存到哪里 | 摘要存 SQLite；MD 用系统保存框保存，默认目录是应用数据目录下的 `handoffs/`，弹窗提供「项目目录」快捷切换 | 鼠鼠按推荐 |
| D-03 | 原工具自带的压缩记录能否当摘要来源 | 只作备用：默认读原文；遮蔽旧工具输出后大多数会话能一次读完。只有压缩前的原文读不到时，才把原工具摘要当作早期部分的条目来源，并注明「来自原工具压缩摘要，未经 Jolu 复核」。Claude Code 取最后一份 `isCompactSummary`；Codex 只认本地压缩且正文非空的记录，远程压缩的加密记录跳过。这样仍符合画板 54-02 的「摘要来源可为原工具的压缩记录」 | 压缩调研；本机 93 条 Claude 原生摘要可读，64 条 Codex 记录全部加密 |
| D-04 | 压缩消耗的 Token | 记在交接记录里并显示在「本次压缩记录」；不并入用量统计 | 调研 1 |
| D-05 | 压缩怎么做 | 分段纪要 + 合成：事实区由程序生成；先遮蔽旧工具输出；较早的消息生成段落纪要，生成一次后一直复用；最近原文直接使用；按交接目标合成 8 节叙述。完整方案和依据见第三节 | 五家 harness、Amp / Factory / OpenHands / Cline / Anthropic 等产品，以及相关论文 |
| D-32 | 压缩过程的界面 | 对话弹窗：显示 Jolu 的步骤、AI 的草稿，用户可以继续提修改意见，最多 10 轮。修改请求不带会话状态，不在 CLI 里留下临时会话；补画进 `Jolu.pen` | 鼠鼠要求 |
| D-33 | 评测的裁判 | 用同一家服务商，会话内容不发给第三方；报告里注明存在自评偏差 | 小的决定（隐私优先） |
| D-34 | 在原会话通道里直接压缩（`--resume`） | 这一版不采用：会写入原会话；Codex 能否分叉还没有核实；压缩过的会话在原通道里也只剩摘要。记进 todo | 小的决定 |
| D-31 | 交接文档用什么语言 | 正文用中文；文件路径、命令、报错、标识符保持原样。为防止多次合并后越写越长（DeepSeek 因此强制英文），叙述区限制在约 2,500 字以内，事实区由程序生成、不经模型改写 | 画板 50 示例为中文、约 1,280 字；本机 Claude 原生摘要也有中文的 |
| D-06 | 画板 50 的预览怎么呈现 | 会话页内的覆盖层子视图；「返回会话」关闭预览，不保留 | 小的推荐 |
| D-07 | 「交接给」有哪些选项 | 九个平台加「其他 AI」；只影响文案和文件名 | 画板 52 |
| D-08 | 自动化选 Skills | 弹出多选，可搜索。列出目标工具的全局 Skills，加上当前工作目录下的项目 Skills，只列已启用的，最多选 10 个。执行时在提示词前加说明：Codex 写成 `$名称` 列表，Claude 写成「本任务请使用以下 Skills：名称、名称」。执行前和续跑前再检查一次，不在了就把任务转为「需要处理」 | 调研 2、5：Claude `-p` 一次只加载第一个 `/skill`，所以不用斜杠写法 |
| D-09 | 关闭同账号排队 | 默认开启。关闭后这个任务可以和同账号的其他任务并行，但有三条保护：① 同一工作目录始终串行，避免两个进程同时改同一批文件；② 每个账号同时运行的任务最多 3 个；③ 任何一个任务报额度耗尽时，同账号正在运行的任务各自跑完当前这一轮，之后不再启动新任务，额度检查按账号合并成一次。额度恢复后，先放行排队任务的队首，再放行并行任务，仍受 3 个的上限。关闭开关时下方提示「同账号任务可能同时运行并争用额度；同一工作目录仍按顺序执行。」 | 画板 60、68 未画关闭态；鼠鼠授权小的优化 |
| D-10 | 任务提醒 | 「需要处理」和「等待超时」归入失败类通知。Windows 没有运行时通知权限，画板上的「引导入口」不做。低余额提醒一起补上：低于阈值时提醒一次，回到阈值以上后重新计数。免打扰期间不弹 | 画板 94；调研 2、6 |
| D-11 | 自定义供应商 | 新平台 id 为 `custom`。弹窗字段：名称、协议（OpenAI 兼容 / Anthropic 兼容）、Base URL、API Key、获取模型；模型接口不可用时改为手填模型 ID。保存时以「获取模型成功」或「手填模型」作为接口可达的依据，不查余额（显示「不支持查询」）。先在 `Jolu.pen` 补画弹窗 | 鼠鼠按推荐；画板 29、27 |
| D-12 | OpenCode 连接 | 按画板 19、21、27 的三种模型服务：OpenCode Go（套餐 Key，地址固定）、OpenAI（官方账号登录，检测 OpenCode `auth.json` 里的 `openai` 条目，只读字段名，不读凭证值）、自定义兼容服务（API Key）。连接表新增 `service` 列 | 调研 4、6 |
| D-13 | ZCode 的 API Key | 地址四选一，默认国内 Anthropic 兼容：`https://open.bigmodel.cn/api/anthropic`；另外三个是 `https://open.bigmodel.cn/api/coding/paas/v4`、`https://api.z.ai/api/anthropic`、`https://api.z.ai/api/coding/paas/v4`。先尝试 `GET <base>/models`，失败就改为手填模型 ID | 调研 6（智谱官方 FAQ） |
| D-14 | Qoder / WorkBuddy 的 API Key | 两家都没有自己的 API Key 服务，只支持在客户端里填别家的 Key（BYOK）。改为 `apiKey: "none"`，说明文案为「Qoder 的自定义模型在 Qoder 客户端内配置，Jolu 不代管密钥。」（WorkBuddy 同理）。鼠鼠已确认 | 调研 6 |
| D-15 | 登录指引和切换账号的链接 | 新建 `lib/providerLinks.ts`，只填执行时在官方网站核实过的 URL，并在注释里写核实日期；未核实的按钮保持禁用 | 鼠鼠按推荐 |
| D-16 | 从 Git 仓库导入 | 来源切到「Git 仓库」后显示「仓库地址」（只接受 `https://`）和「分支（可选）」，再点「获取」。后端用本机 `git clone --depth 1` 拉到临时目录，复用现有的导入扫描和冲突检查；本机没有 git 时提示「未找到 git，请先安装 Git」 | 画板 82 只画了入口 |
| D-17 | 查看差异 | 弹窗按行对比「磁盘版本」和「当前草稿」，按钮为「使用磁盘版本」「保留草稿」 | 画板 84 · 05 只画了入口 |
| D-18 | 更换全局目录 | 只改 Jolu 扫描和写入的目录（存在 `skills.json` 配置里），不改工具本身的环境变量。界面提示「工具是否读取该目录取决于其配置（如 CLAUDE_CONFIG_DIR / CODEX_HOME）」。Codex 的 `~/.agents/skills` 不受 `CODEX_HOME` 影响，作为第二个全局目录照常扫描 | 调研 5 |
| D-19 | 其他工具的 Skills | 改成数据驱动的 `ToolSpec` 表，按工具决定能力（见下表）；「自定义」只扫描，不提供启停 | 调研 5 |
| D-20 | 默认打开页面 | 选项为「上次访问的页面」加七个页面，默认「上次访问的页面」。托盘左键、托盘菜单「打开主窗口」、第二个实例启动都按这个设置打开 | 画板 86 |
| D-21 | 界面缩放档位 | 80%、90%、100%、110%、125%、150%；后端对 `main` 窗口调用 `set_zoom` | 画板 88 |
| D-22 | 紧凑模式行高 | 会话树 36px 改 32px；调用明细 54px 改 44px；Skills 表格 55px 改 44px；设置页预览示例 40px 改 32px | 画板 88 未画紧凑态 |
| D-23 | 减少动效 | 只作用于主窗口：开关打开或系统开启「减少动画」时都生效 | AGENTS：灵动岛保持现状 |
| D-24 | 应用联网方式 | 三种：跟随系统、直连、手动代理；代理只支持 `http://` / `https://`，不加 socks 依赖。直连的状态行文案为「不使用代理，直接连接」。代理地址格式错误时提示「请输入 http:// 或 https:// 开头、带端口的代理地址」。跟随系统时补上按协议分开写的 `ProxyServer` 解析（复用 `updater.rs` 的逻辑）。CLI 本地代理的上游（`proxy.rs`）不跟随 | 画板 90、102；调研 3、6 |
| D-25 | 连接诊断查什么 | 手动代理时先查「代理可达」（与代理建立 TCP 连接，超时 3 秒）；再对每个已保存连接的额度或模型接口查一次「可达」（GET 请求，不发模型请求），显示延迟毫秒；失败显示原因 | 画板 90、102 |
| D-26 | 导入的语义 | 按画板 100 · 05 改为：先预览，显示有效、重复、无效三个数字；确认后重复项跳过（不再合并更新）、无效项跳过（列出前 20 条原因），其余导入 | 画板 100 |
| D-27 | 按日期导出 | 导出弹窗复用 `DateRangeDialog` 选日期；会话标题只导出区间内有请求的会话；修掉其他供应商导出时报错的 bug | 画板 92；调研 3 |
| D-28 | 清理时的「预计释放」 | 这一轮不做，保持「—」 | 鼠鼠按推荐 |
| D-29 | 检查更新 | 把 `UPDATES_ENABLED` 改成构建时开关：后端 `option_env!("JOLU_UPDATES_ENABLED")`，前端 `import.meta.env.VITE_JOLU_UPDATES_ENABLED`。用本地 HTTP 服务器和测试密钥做端到端测试；发布正式 Release 留作 todo | 调研 6 |
| D-30 | 会话项目名 | 不改用量表：`top_sessions` 通过 `session_threads::source_files` + `read_cwd` 取工作目录 | 调研 1 |

D-19 中各工具的能力，均于 2026-10-22 核实：

| 工具 | 全局目录 | 项目目录 | 启停方式 | 写入 |
|---|---|---|---|---|
| Claude Code | `$CLAUDE_CONFIG_DIR` 或 `~/.claude/skills` | `.claude/skills` | 现有停用区（移到应用数据目录） | 可以 |
| Codex | `~/.agents/skills`、`$CODEX_HOME/skills` | `.agents/skills`、`.codex/skills` | 现有 `config.toml` 的 `[[skills.config]]` | 可以 |
| Gemini CLI | `~/.gemini/skills`（也读 `~/.agents/skills`） | `.gemini/skills` | `~/.gemini/settings.json` 的 `skills.disabled[]` | 可以 |
| OpenCode | `~/.config/opencode/skills` | `.opencode/skills` | `opencode.json` 的 `permission.skill.<名称> = "deny"` | 可以 |
| Qoder | `~/.qoder/skills`、`~/.qoder-cn/skills` | `.qoder/skills` | 未核实，不提供启停 | 可以 |
| Trae | `~/.trae/skills` | `.trae/skills` | 只能在客户端界面里开关，不提供启停 | 可以 |
| ZCode | `~/.zcode/skills` | `.zcode/skills` | 只能在客户端界面里开关，不提供启停 | 可以 |
| WorkBuddy | `~/.workbuddy/skills`（只有社区来源） | `.workbuddy/skills` | 不提供 | 只读，标「目录未核实」 |
| Grok | — | — | — | 保持「待接入」 |

## 三、会话压缩方案（D-05）

### 3.1 五家做法对比（2026-10-22 核实）

| | Claude Code | Codex（本地压缩） | DeepSeek harness | ZCode harness | pi |
|---|---|---|---|---|---|
| 来源 | 官方文档、本机 93 条记录；提示词闭源 | `openai/codex` @61f34c3 `core/src/compact.rs` | `deepseek-ai/deepseek-harness` @687f282 `packages/compaction/` | `zai-org/ZCode` `packages/core/src/compact/` | `earendil-works/pi` @ac80dd8 `core/compaction/` |
| 不调模型的预处理 | 先清旧工具输出 | 无 | 单条工具结果超过 8,192 字符时留头 4,096、尾 1,024 | 旧工具结果整条清空，保留最近 5 组 | 工具结果截到 2,000 字符，省略思考过程 |
| 保留原文 | 最近一段原样保留 | 最近的用户消息，共 20k token | 最近 16% 窗口 | 最后一轮 | 最近 20k token，不在工具调用和结果之间切开 |
| 摘要结构 | 9 节（含「所有用户消息」） | 4 条要点的简短提示词 | 8 节固定 Markdown，空节写 (none) | 9 节，先写分析再写摘要，保存时删掉分析 | 6 节（目标、约束、进度分已完成/进行中/受阻、决策、下一步、关键上下文） |
| 再次压缩 | 旧摘要作为历史一起压缩 | 同左 | 同左，并明确要求「合并、不照抄、删掉过时内容」 | 同左 | 单独的「更新摘要」提示词，旧摘要放进 `<previous-summary>` |
| 输入超长 | 未公开 | 删掉最旧的一条后重试 | 不处理，直接失败 | 先扩大保留段，再丢弃最旧的轮并加截断标记，最多 3 次 | 不处理 |
| 质量保护 | — | — | 摘要没有变短、输出被截断时都判失败 | 摘要里出现工具调用时报错；连续失败 3 次熔断 | 输出被截断时判失败；文件清单由程序从工具调用里抽取 |
| 存储 | 写入边界记录和摘要记录，原文保留 | `compacted` 记录 | 只追加的事件日志，原文永久保留 | 写入边界元数据：覆盖条数、保留区间、压缩前后 token | 只追加，用 `firstKeptEntryId` 标出从哪条开始保留原文 |

### 3.2 更大范围的证据（2026-10-22 调研）

鼠鼠授权小的推翻原方案、寻找最好的方法，所以调研范围扩大到专门做交接的产品和有实测数据的研究。WebFetch 被拦截，部分数字来自搜索摘要，标「二手」的没有逐字核对原文。

| 结论 | 依据 | 强弱 |
|---|---|---|
| 「改了哪些文件」是所有压缩方法的共同短板，要由程序单独记录 | Factory 在 36,611 条生产消息上做探针评测，文件轨迹一项最高只有 2.45/5；Cline 新版、pi 都改为程序注入文件清单 | 强（厂商自评，但各家做法一致） |
| 先遮蔽或清理旧工具输出，效果不输让模型总结，而且便宜得多 | JetBrains《The Complexity Trap》（NeurIPS 2025 DL4Code）：SWE-bench Verified 上成本减半，成功率持平或更高，环境输出约占每轮 84%；Anthropic：清理工具结果 +29% | 强 |
| 让模型总结会抹掉失败信号；失败尝试必须保留 | 同上，摘要让轨迹变长 13–15%；Manus 的经验「keep the wrong stuff in」 | 中 |
| 固定章节、每节必填（空节写「（无）」）能防遗漏 | Factory 结构化摘要 3.70，对比 Anthropic 3.44、OpenAI 3.35；Cline、Roo、Kilo、OpenHands、Claude Code 都用固定章节 | 中（缺少学术上的受控研究） |
| 按下游目标定制的摘要更好 | LongLLMLingua 去掉问题感知后约 77 降到约 42；RECOMP、ACON；Amp Handoff 的定性理由 | 中强（证据都来自问答或在线 agent） |
| 多层反复改写会累积失真；滚动更新式摘要更容易漏因果 | BooookScore（ICLR 2024）；《LLM as a Broken Telephone》（ACL 2025） | 中 |
| 增量合并比每次全量重写更好 | Factory 的锚定迭代摘要胜出 | 弱（自评，而且和上一条冲突） |
| 能回查原文，才补得回被摘要丢掉的细节 | MemGPT：只看递归摘要 32.1%，可以检索原文 92.5%（二手）；Cursor 摘要时附历史文件引用；Manus「压缩必须可恢复」 | 中 |
| 关键信息放在开头和结尾，避免埋在中间 | 《Lost in the Middle》；Chroma《Context Rot》；Manus 用 todo.md 把目标复述到末尾 | 中 |
| 交接要明确写出计划和下一步 | 《Plans Don't Persist》：计划被逐出上下文后成功率降 34.7 个百分点；Anthropic 长时运行 agent：只靠压缩不够，用进度文件加冒烟测试 | 中 |
| 先自由分析，再填模板；不要用 JSON 约束解码 | 《Let Me Speak Freely?》（EMNLP 2024） | 中 |

两点要注意：

1. Amp 在 2026-05 撤掉了 Handoff，改回自动压缩，理由是前沿模型做压缩已经够好。但那是「同一线程内续跑」的场景。Jolu 是跨工具、跨 agent 交接，没有原会话可续，仍然需要交接文档。
2. 「增量合并」和「一次写完」谁更好，现有证据互相冲突。鼠鼠最终选定「分段纪要 + 合成」：每份纪要只从原文生成一次，不做多层改写，同时保留增量处理的低成本。

### 3.3 定稿方案：分段纪要 + 合成（鼠鼠 2026-10-22 选定）

鼠鼠选定「分段纪要 + 合成」作为生成方式，其余细节由小的按 3.2 的证据决定：事实由程序抽取、先遮蔽旧工具输出、按交接目标写、引用来源、关键信息放两头。压缩过程放在一个用户与 AI 对话的弹窗里进行（见 3.4）。

```
原始会话 ─┬─▶ ① 事实区：程序抽取，不调用模型，100% 原样
          │
          └─▶ ② 精简：旧工具输出遮蔽，报错和最近 5 组保留
                    │
       ┌────────────┴────────────────────────────┐
       │ 较早的消息（最近约 20k token 之前）        │ 最近约 20k token 的消息
       │ 按轮次切段，每段 ≤ 40k token              │ 原文直接使用
       │ 已有纪要且校验通过 → 直接复用，不调模型     │
       │ 新离开最近窗口的消息 → 生成新的段落纪要      │
       └────────────┬────────────────────────────┘
                    ▼
     ③ 合成：全部段落纪要 + 最近原文 + 事实区 + 交接目标 → 8 节叙述（1 次调用）
                    ▼
     ④ 检查：章节齐全 · 没有被截断 · 下一步引用了 #n · 路径都在会话里出现过
                    ▼
     ⑤ 对话弹窗里展示草稿 → 用户可以继续提修改意见 → 确认后排版成 MD → 预览与保存
```

**1. 事实区（程序生成，模型不碰）**

| 内容 | 抽取规则 |
|---|---|
| 用户原话时间线 | 只取用户亲自输入的消息，最近 30 条，每条最多 300 字，标注 `#n`；更早的只计条数 |
| 改过的文件 / 读过的文件 | Edit / Write / MultiEdit / NotebookEdit / apply_patch 记为改过；Read / Glob / Grep 记为读过；改过的从读过里去掉；每个文件标出最后一次涉及它的 `#n` |
| 执行过的命令 | Bash 类调用，保留命令原文和退出状态（取不到写「未知」），最多 20 条，测试、构建、检查类排在前面 |
| 最后一次验证 | 最后一条测试、构建或检查类命令的输出，保留最后 15 行原文 |
| 失败的尝试 | 退出码不为 0，或工具结果标记为错误的调用，保留命令和报错的第一行，最多 10 条 |
| Git 快照 | 工作目录是 Git 仓库时，只读获取分支、HEAD 和未提交的文件（最多 50 项） |
| 回查线索 | 原始会话文件路径，以及「需要原文细节时，按消息序号 #n 查阅」 |

**2. 精简输入**：最近 5 组工具输出保留，每条截到 2,000 字；更早的输出替换为 `[输出已省略 · #n · N 字]`；失败的工具输出保留报错的前 20 行；思考过程省略。每条消息前面带 `#序号`，整体包在 `<conversation>` 里，作为一条用户消息发出。

**3. 段落纪要**
- 每份只从精简后的原文生成一次，并绑定消息范围和校验值。
- 以后校验通过就一直复用，所以再次压缩时只处理新消息，鼠鼠最初要的「从上次之后接着压」仍然成立。
- 某条消息被改写时，只重做覆盖它的那一份纪要；原文被截断时，丢弃超出范围的纪要。
- 纪要格式固定，每份最多约 1,500 字，每条都带 `#n`。内容包括：用户的新要求（原话）、完成的改动、决策与理由、失败的尝试、验证（命令与结果）、遗留问题。
- 纪要总量超过 60,000 token 时，把最早的若干份归并成一份「早期纪要」，原文不动。

**4. 合成**
- 输入：全部纪要、最近原文、事实区、交接目标与交接对象。
- 输出 8 节叙述，每节必填，空节写「（无）」，用中文写，路径、命令、报错原样保留，总长约 2,500 字以内。
- 前后冲突时以后面的为准，旧结论标「已失效」。

```
## 交接目标        接手后要做什么（来自「交接主题」；未填写则按「继续当前开发」写）
## 当前状态与卡点  正在做什么、做到哪一步、卡在哪里
## 下一步          编号清单；第一步与用户最近一次明确的要求一致，并引用原话（#n）
## 目标与约束      用户目标；用户提出的约束和安全要求，用原话（#n）
## 已完成          做了什么、结果如何（#n）
## 关键决策与原因  决策：理由；已推翻的标「已失效」
## 失败的尝试      试过什么、为什么不行
## 待确认          未验证的结论标「未验证」；需要用户决定的事项
```

**5. 排版**
- 交接目标、当前状态与卡点、下一步放在最前面。
- 后面依次是其余叙述章节和事实区。
- 文档末尾再重复一次「下一步」的第一条，并给出接手第一步建议：「先核对 Git 快照，运行『最后一次验证』里的命令确认状态」。

**6. 预算与降级**
- 三项输入都有上限：纪要 60k、最近原文 20k、事实区约 5k，所以合成的输入稳定在约 90k token 以内，低于预算 167,000。
- 预算的算法：窗口 − 20,000 − 13,000，窗口拿不到时按 200,000 计。token 按「字符数 ÷ 3」估算。
- 模型仍报上下文超限时，依次降级：最近原文窗口减半，再归并纪要；第 3 次仍超限时报「会话过长，无法压缩」。

**7. 质量检查**（不通过的不进入草稿）
- 检查项：输出被截断、输出里有工具调用、缺少章节、「下一步」里没有引用 `#n`。
- 不通过时自动重试 1 次。
- 叙述里出现的文件路径，如果在会话里从未出现过，在预览中提示「以下路径未在会话中出现：…」（鼠鼠要求加上），但不阻止保存。

**8. 复用**：上次之后没有新消息时，不调用模型，直接复用上一版叙述，只重新包装（画板 52）。

**9. 原工具的压缩摘要与原会话通道**
- 默认读原文，所以不用原工具的压缩摘要。只有压缩前的原文已经读不到时，才把原工具摘要当作早期部分的纪要来源，并注明「来自原工具压缩摘要，未经 Jolu 复核」。
- 「在原会话通道里直接压缩」（`--resume`）这一版不采用，原因有三：
  - 会往原会话里写入内容；
  - Codex 在非交互模式下能否分叉还没有核实；
  - 压缩过的会话在原通道里也只剩摘要。
- 这个思路记进 todo，作为以后的候选。

### 3.4 压缩对话弹窗（鼠鼠 2026-10-22 要求）

压缩过程放在一个弹窗里，弹窗就是用户和 AI 交互的窗口：用户能看到 AI 每一步在做什么、草稿写成什么样，并可以继续提修改意见，满意后再生成 MD。

```
┌ 压缩会话 · 优化 Jolu 的会话界面 ─────────────────────────── [×] ┐
│ Codex · jolu · 共 24 条 · 执行：Claude Code · Momo · Claude Sonnet │
├────────────────────────────────────────────────────────────────┤
│ ● Jolu   已抽取事实：改过 6 个文件 · 12 条命令 · 最后一次验证失败   │
│ ● Jolu   复用 2 份纪要（#1–#16），整理新增 #17–#24 …               │
│ ◆ AI     交接草稿 v1（约 1,280 字）                    [展开全文]   │
│          ## 交接目标 …  ## 当前状态与卡点 …  ## 下一步 …            │
│          ⚠ 以下路径未在会话中出现：src/x.ts                        │
│ ● 你     下一步再写细一点，补上测试失败的原因                       │
│ ◆ AI     交接草稿 v2（约 1,410 字）                    [展开全文]   │
├────────────────────────────────────────────────────────────────┤
│ [ 对草稿提修改意见…                                    ] [发送]  │
│ 本轮预计发送约 3.2k token · 已用 41.5k / 6.1k token     │
│                         [取消压缩]  [用 v2 生成 MD →]            │
└────────────────────────────────────────────────────────────────┘
```

- **入口**：先打开画板 48 的配置弹窗。提示行加一句「预计发送约 N token」（鼠鼠要求加上）。点「压缩并生成 MD」后，切换到这个对话弹窗；配置弹窗关闭。
- **消息类型**：
  - Jolu 的步骤消息：抽取事实、复用或生成纪要（第 n/m 段）、合成、检查；
  - AI 的草稿消息：可以折叠，「展开全文」看完整 Markdown；路径提示紧跟在草稿下面；
  - 用户的修改意见；
  - 错误消息：带「重试」「更换连接」两个按钮（画板 54-08）。
- **修改意见**：每次发送都是一次不带会话状态的调用，输入是「当前草稿 + 事实区 + 用户意见 + 与意见相关的纪要」。之所以不带状态，是为了不在 Claude Code 或 Codex 里留下临时会话。
  - 相关纪要的判断方法：意见里提到的文件或关键词出现在哪份纪要里，就带上哪份；找不到就不带。
  - 每次修改都会产出新版本 v2、v3……，同样要过质量检查。最多 10 轮。
- **底部状态行**：显示本轮预计发送的 token，以及累计的输入和输出 token。
- **结束**：点「用 vN 生成 MD」，用选定的版本加事实区排版，进入画板 50 的预览（「Markdown 源码」页签可以编辑）。这时才写入新的交接版本；纪要在生成时就已经保存。
- **关闭与取消**：生成 MD 之前关闭弹窗，先确认「关闭将丢弃未生成的草稿，已整理的纪要会保留」。「取消压缩」会结束当前进程，已发出的请求可能已经产生用量（画板 54-03 原文）。
- **画板**：鼠鼠这次明确要求了这个弹窗，所以任务 3.2 先用 Pencil 在模块 D 末尾补画浅色和深色两张，再按画板实现。上面两处提示文案也一并补进画板 48 和 50。

### 3.5 本机评测（调提示词用）

- **目标**：生成方式已经定了，评测只用来检验和调整提示词：比较「单次通用总结」（基线）和本方案。
- **出题与打分**：方法同前。
  - 程序题：改过的文件、最后一个报错、最后一条用户指令、测试是否通过。
  - 模型题：每个会话 8 题，覆盖事实、文件轨迹、接下来做什么、为什么这样决定。
  - 编造检测：每个会话加 2 道答案不在会话里的题。
  - 作答：接收模型只读交接文档作答。
  - 打分：裁判看不到答案出自哪个候选。
- **规模**：开发集 4 个会话、测试集 8 个会话，约 80 次 CLI 调用。
- **服务商**：裁判用同一家服务商，会话内容不发给第三方；报告里注明存在自评偏差。
- **执行前确认**：执行前征得鼠鼠同意，包括会话清单和调用量。
- **结果用法**：本方案在任何一类题上明显落后于基线时，改提示词并把版本号升到 `handoff-v2`。

## 四、重点审查

以下是各任务的测试最容易漏掉、用户最可能碰到的问题；对应测试已写进负责的任务：

1. 已覆盖的原文被改写或截断：只重做受影响的纪要，丢弃超出范围的纪要，其余照常复用（任务 3.1 `changed_message_redoes_one_digest`、`truncation_drops_out_of_range_digests`）。另一类问题是模型编造会话里没有的文件、结论或进度：事实区一律由程序生成；叙述区要求引用 `#n` 来源，出现会话里没有的路径时在预览中提示（任务 3.1 `check_output_rules`、`unknown_paths_flags_invented`；任务 3.6 用「答案不在会话里的题」统计编造率）。
2. 切换联网方式后，额度查询仍在用旧客户端（`quota::client()` 每 120 秒才重建）：切换应立即生效（任务 2.3 `client_rebuilds_after_network_change`）。
3. 导入文件里重复、无效、有效记录混在一起：预览的数字必须和实际导入的结果一致（任务 2.4 `preview_counts_match_import`）。
4. 自动化任务选中的 Skill 在运行前被停用或删除：任务转为「需要处理」，不静默执行（任务 3.3 `run_blocks_when_skill_missing`）。
5. 外部工具的配置文件格式异常或正在被别的程序修改：不写入，原文件保持不变（任务 4.2 `toggle_refuses_malformed_config`）。

---

## 阶段 1：真机跑通已写好的功能

阶段目标：先确认已经写好的部分在真实桌面上能用，发现的问题当场修。凡是会消耗鼠鼠真实额度的操作（任务 1.2），执行前先问鼠鼠。

### 任务 1.1：修复其他供应商导出时报「导出范围无效」

**文件：**
- 修改：`backend/src/data_files.rs:43-57`
- 测试：`backend/src/data_files.rs` 的 `mod tests`

**接口：**
- 产出：`export_data_file` 的平台校验与 `lib.rs` 里 `export_data` 一致，所有已知平台都放行。

- [ ] **步骤 1：写失败测试** `export_scope_accepts_every_known_platform`：对目录中的每个平台 id，校验函数返回 `Ok`；对 `"unknown"` 返回错误「导出范围无效」。
- [ ] **步骤 2：运行** `cargo test --manifest-path backend/Cargo.toml export_scope`，预期失败（gemini 被拒）。
- [ ] **步骤 3：实现**：把白名单换成 `export_data` 用的那套平台校验，抽成 `fn valid_export_scope(platform: Option<&str>) -> Result<(), String>`，两处共用。
- [ ] **步骤 4：再次运行**，预期通过。

### 任务 1.2：自动化真实执行验收

**文件：**
- 新建：`docs/verification/2026-10-22-automation-real-run.md`（记录结果）
- 修改：发现问题时修改 `backend/src/automation.rs` 和相关前端文件，并先补一个能复现问题的测试。

- [ ] **步骤 1：征得同意**：说明会用 Claude Code 和 Codex 当前登录的账号各跑一次很短的任务（提示词「只回复 OK」），会消耗少量额度，等鼠鼠同意。
- [ ] **步骤 2：用 Claude Code 跑三种启动方式**：
  - 手动启动：「立即运行」，预期运行记录依次出现排队、运行、完成，结果里有「OK」，Token 不为空。
  - 定时：设为两分钟后，预期按时启动。
  - 停止：运行中点「停止」，预期进程退出（任务管理器里没有残留的 `claude` 进程），状态为「已取消」。
- [ ] **步骤 3：用 Codex 重复步骤 2。**
- [ ] **步骤 4：续接原会话**：选一个已有会话执行，预期在原会话里继续，没有新建会话。
- [ ] **步骤 5：额度耗尽后续跑**：真实耗尽额度无法人为触发，只验证代码路径。补测试 `classify_quota_exhausted_enters_waiting`：把 Claude 和 Codex 各一段额度耗尽的输出喂给 `classify`，预期任务进入等待，并记下重置时间。真机部分写进 todo。
- [ ] **步骤 6：记录结果**：每项写明通过、失败或未测，以及原因。

### 任务 1.3：Skills、深色主题、灵动岛分身、安装包验收

**文件：**
- 新建：`docs/verification/2026-10-22-desktop-acceptance.md`

- [ ] **步骤 1：Skills 读写**：用 `scripts/acceptance/start.ps1` 启动隔离环境（`JOLU_TEST_DIR`，Skills 目录在 `<隔离目录>/skills-home`）。依次做：新建、编辑、Claude 停用后再启用、Codex 停用后再启用、从本地目录导入出现同名冲突时选「跳过」和「覆盖」。每一步核对磁盘上的文件和 `config.toml`。跨盘停用要把隔离目录放在另一个盘符上再做一次。
- [ ] **步骤 2：深色主题**：把系统主题切到深色，逐页截图主窗口的七个页面，和 Pencil 中对应的偶数画板（02、06、30、40、58、74、86 等）对照，记录差异并修复。
- [ ] **步骤 3：灵动岛分身**：用两个不同连接各开一个分身，确认两边额度各自显示，补上旧待办 V-4。
- [ ] **步骤 4：安装包**：运行 `scripts/acceptance/prepare-sandbox.ps1`，在 Windows Sandbox 中检查安装界面图片、桌面快捷方式图标、卸载后数据是否保留。本机没有 Windows Sandbox 时写进 todo。

### 任务 1.4：主窗口冒烟验收脚本

**文件：**
- 新建：`scripts/acceptance/main-window-smoke.e2e.mjs`
- 修改：`scripts/acceptance/README.md`（第 4 步那句「主窗口各模块……验收脚本待补充」）

**接口：**
- 消耗：隔离应用启动后 9337 端口的 CDP；主窗口 URL 以 `http://localhost:5173/` 开头，并且不含 `window=island`。

- [ ] **步骤 1：写脚本**：用 Playwright 的 `connectOverCDP('http://127.0.0.1:9337')` 连接，依次点击侧栏的七个页面。每页检查：没有出现「读取失败」，「正在读取」在 10 秒内消失，控制台没有 error。截图存到 `output/acceptance/main-window/<页面>.png`。
- [ ] **步骤 2：在隔离环境中运行**，预期七页全部通过。
- [ ] **步骤 3：更新 README**：写明运行方式，删掉「待补充」那句。

**阶段 1 结束：** 跑前后端全量测试和类型检查，把结果汇报给鼠鼠，请鼠鼠决定是否提交。

---

## 阶段 2：规则明确、可以直接写的功能

本阶段各任务之间没有依赖，可以多个代理并行。任务 2.1 和 2.2 都改 `settings.rs` 和设置页，放给同一个代理按顺序做。

### 任务 2.1：默认打开页面

**文件：**
- 修改：`backend/src/settings.rs`（`Settings` 结构体）
- 修改：`backend/src/lib.rs`（新增命令；`MainIntent::Overview` 已经带着设置快照）
- 修改：`frontend/src/lib/mainNavigation.ts`
- 修改：`frontend/src/views/MainPanelWindow.tsx:27-34, 83`
- 修改：`frontend/src/components/settings/GeneralSection.tsx:69-73`
- 修改：`frontend/src/lib/settingsApi.ts`
- 测试：`frontend/src/lib/mainNavigation.test.mjs`，`settings.rs` 的 `mod tests`

**接口：**
- 产出（Rust）：
  - 新字段 `default_page: String`，默认 `"last"`；`last_page: Option<String>`。两者都加 `#[serde(default)]`。
  - 命令 `set_default_page(app, page: String) -> Result<Settings, String>`；命令 `remember_page(app, page: String) -> Result<(), String>`，只写 `last_page`。
- 产出（TS）：
  - `export type DefaultPage = "last" | MainPage`
  - `export function resolveOpenPage(defaultPage: string, lastPage: string | null): MainPage`
  - `settingsApi.setDefaultPage(page: DefaultPage)`，`settingsApi.rememberPage(page: MainPage)`

- [ ] **步骤 1：写失败测试**（`mainNavigation.test.mjs`）：
  ```js
  test("resolveOpenPage", () => {
    assert.equal(resolveOpenPage("last", "skills"), "skills")
    assert.equal(resolveOpenPage("last", null), "usage")
    assert.equal(resolveOpenPage("last", "bogus"), "usage")
    assert.equal(resolveOpenPage("sessions", "skills"), "sessions")
    assert.equal(resolveOpenPage("bogus", null), "usage")
  })
  ```
  Rust 测试 `default_page_rejects_unknown`：`set_default_page` 的校验函数拒绝 `"bogus"`，接受 `"last"` 和七个页面。
- [ ] **步骤 2：运行两边测试**，预期失败。
- [ ] **步骤 3：实现**：
  - `pageForIntent` 的 overview 分支改用 `resolveOpenPage(settings.default_page, settings.last_page)`；首次挂载时同样处理。
  - 切换页面时调用 `rememberPage`，切换后延迟 1 秒再写，期间再切换就重新计时。
  - 下拉框可选「上次访问的页面」加七个页面，去掉 `PendingBadge`。
- [ ] **步骤 4：运行测试和类型检查**，预期通过；在桌面上关闭主窗口再从托盘打开，确认打开的是设置的页面。

### 任务 2.2：界面缩放、列表密度、减少动效

**文件：**
- 修改：`backend/src/settings.rs`、`backend/src/lib.rs`（命令；主窗口创建后应用缩放。缩放由后端调用，不需要给前端加 `set-webview-zoom` 权限）
- 新建：`frontend/src/lib/appearance.ts`
- 修改：`frontend/src/components/settings/AppearanceSection.tsx:61-91`
- 修改：`frontend/src/lib/motion.ts`（`reducedMotion()`）
- 修改：`frontend/src/views/MainPanelWindow.tsx`（根节点加密度 class）
- 修改：`frontend/src/components/sessions/SessionTree.tsx:93`、`components/usage/CallsPanel.tsx:77`、`components/skills/SkillsTable.tsx:54`
- 测试：`frontend/src/lib/appearance.test.mjs`，`settings.rs` 的 `mod tests`

**接口：**
- 产出（Rust）：
  - 字段 `main_zoom: u16`（默认 100）、`list_density: String`（默认 `"comfortable"`）、`reduce_motion: bool`（默认 false）。
  - 命令 `set_appearance_preferences(app, zoom: u16, density: String, reduce_motion: bool) -> Result<Settings, String>`。命令内部对 `main` 窗口调用 `set_zoom(zoom as f64 / 100.0)`，`show_main` 创建窗口后也调用一次。
- 产出（TS）：
  - `export const ZOOM_STEPS = [80, 90, 100, 110, 125, 150] as const`
  - `export type Density = "comfortable" | "compact"`
  - `export function applyAppearance(root: HTMLElement, density: Density, reduceMotion: boolean): void`：在 `<html>` 上切换 `density-compact` 和 `reduce-motion` 两个 class，只在非 `island-window` 时调用。
- 行高用 Tailwind 的 `[.density-compact_&]:` 变体写，数值见 D-22。

- [ ] **步骤 1：写失败测试**：
  - `appearance.test.mjs`：`ZOOM_STEPS` 包含 100；`applyAppearance` 在一个假的 classList 上加、去两个 class 正确。
  - Rust `appearance_validation`：缩放值不在档位里时报错；密度只接受两个值。
- [ ] **步骤 2：运行**，预期失败。
- [ ] **步骤 3：实现**：
  - 设置页三个控件接上命令；预览区右侧显示「<缩放>% · 舒适/紧凑」，表头去掉「· 示例」，与画板一致。
  - `reducedMotion()` 改为系统偏好或 `<html>` 上有 `reduce-motion` 时返回 true。
  - 全局兜底的 CSS 规则（`index.css:561`）同时匹配 `.reduce-motion`。
- [ ] **步骤 4：运行测试和类型检查**；在桌面上切换 125%，确认灵动岛大小不变、主窗口弹窗位置正确。

### 任务 2.3：应用联网方式与连接诊断

**文件：**
- 新建：`backend/src/net.rs`
- 修改：`backend/src/quota.rs:364-385`（`client()`）、`connections.rs:529`、`provider_key.rs:118`、`updater.rs:23-29, 114`
- 修改：`backend/src/settings.rs`、`backend/src/lib.rs`
- 修改：`frontend/src/components/settings/NetworkSection.tsx:20-47`、`frontend/src/lib/settingsApi.ts`
- 测试：`net.rs` 的 `mod tests`

**接口：**
- 产出（Rust，`net.rs`）：
  - `pub enum NetMode { System, Direct, Manual(String) }`
  - `pub fn current_mode() -> NetMode`：从设置读，结果缓存在一个 `RwLock` 里，由 `set_mode` 更新。
  - `pub fn generation() -> u64`：每次修改联网方式加一。
  - `pub fn blocking_builder() -> reqwest::blocking::ClientBuilder`：三种模式分别用系统代理、`no_proxy()`、`proxy(Proxy::all(url))`；跟随系统且注册表 `ProxyServer` 按协议分开写时，取 `https=` 那一项，没有就取 `http=` 那一项。
  - `pub fn parse_proxy_server(raw: &str) -> Option<String>`：从 `updater.rs:114` 搬过来，两处共用。
  - `pub fn validate_proxy_url(raw: &str) -> Result<String, String>`：错误文案见 D-24。
  - 命令 `set_network(app, mode: String, proxy_url: Option<String>) -> Result<Settings, String>`
  - 命令 `diagnose_network(app, mode: String, proxy_url: Option<String>) -> Result<NetworkDiagnosis, String>`
  - `NetworkDiagnosis { proxy: Option<Probe>, endpoints: Vec<EndpointProbe> }`
  - `Probe { ok: bool, latency_ms: Option<u64>, error: Option<String> }`
  - `EndpointProbe { connection_name: String, platform: String, probe: Probe }`
- 设置字段：`network_mode: String`（默认 `"system"`）、`network_proxy_url: Option<String>`。
- `quota::client()` 的缓存同时记录 `net::generation()`，值变了就立即重建。

- [ ] **步骤 1：写失败测试**：
  - `parse_proxy_server_handles_per_protocol`：`"http=a:1;https=b:2"` 得到 `"http://b:2"`；`"c:3"` 得到 `"http://c:3"`；空串得到 `None`。
  - `validate_proxy_url_rules`：接受 `http://127.0.0.1:7890`；拒绝 `socks5://x:1`、`127.0.0.1`、`http://host`（没有端口）。
  - `client_rebuilds_after_network_change`：改模式后 `generation()` 加一，并且 `quota` 的缓存判断函数 `fn cache_valid(built_at, built_gen, now, gen) -> bool` 在代数不同时返回 false。
- [ ] **步骤 2：运行** `cargo test --manifest-path backend/Cargo.toml net::`，预期失败。
- [ ] **步骤 3：实现** `net.rs`，把 `quota.rs`、`connections.rs`、`provider_key.rs` 的客户端都改为用 `blocking_builder()`。`updater.rs` 的代理改为从 `net` 取：手动代理用设置里的地址，直连不设代理，跟随系统沿用原来的逻辑。`proxy.rs` 不动。
- [ ] **步骤 4：实现诊断**：手动模式先对代理做 TCP 连接（3 秒超时）；再对每个已保存连接的额度或模型地址发一个 GET（5 秒超时），只记状态和延迟，不读取响应内容。用草稿参数检测，不保存设置。
- [ ] **步骤 5：前端**：三段选择各自对应的状态行文案；手动代理时显示输入框和「保存配置」；草稿未保存时提示「代理地址草稿 · 点击保存后生效」；诊断结果每行一个连接，缺延迟显示「—」。
- [ ] **步骤 6：运行测试和类型检查**；桌面上手动填一个不存在的代理端口，点「测试连接」，预期显示「代理不可达」，设置没有被保存。

### 任务 2.4：导入前预览与按日期导出

**文件：**
- 修改：`backend/src/db.rs:679-720`（`import_backup`），新增预览函数
- 修改：`backend/src/data_files.rs`、`backend/src/lib.rs`（命令）
- 修改：`backend/src/source_store.rs:67, 90`（导入时重复项跳过，不再合并）
- 修改：`frontend/src/components/settings/DataSection.tsx`、`DataDialogs.tsx:34-69`、`frontend/src/lib/api.ts:376-377`
- 测试：`db.rs` 的测试模块，`data_files.rs` 的 `mod tests`

**接口：**
- 产出（Rust）：
  - `pub struct ImportPreview { pub token: String, pub file_name: String, pub valid: usize, pub duplicate: usize, pub invalid: usize, pub invalid_reasons: Vec<String> }`，原因最多 20 条。
  - `pub fn preview_backup(conn: &Connection, json: &str) -> Result<(ImportPreview, ParsedBackup), String>`：只读，不写库。
  - `pub fn import_parsed(conn: &mut Connection, parsed: &ParsedBackup) -> Result<ImportSummary, String>`：只插入有效且不重复的记录。
  - 命令 `preview_import_file(app, db) -> Result<Option<ImportPreview>, String>`：弹出选择文件框，解析后把 `ParsedBackup` 按 token 暂存在内存里，最多保留 1 份，10 分钟后过期。
  - 命令 `confirm_import(app, db, token: String) -> Result<ImportSummary, String>`；token 过期时报「预览已过期，请重新选择文件」。
  - `ImportSummary` 增加 `skipped: usize`。
  - 命令 `export_data_file(app, db, platform: Option<String>, start_ms: Option<i64>, end_ms: Option<i64>)`：区间为左闭右开；会话标题只导出区间内有请求的会话。
- 产出（TS）：
  - `api.previewImportFile()`、`api.confirmImport(token)`
  - `api.exportDataFile(platform, startMs, endMs)`，日期换算复用 `usageStats.ts` 里的 `rangeToMillis`。

- [ ] **步骤 1：写失败测试**：
  - `preview_counts_match_import`：库里先有 2 条，文件里有 5 条，其中 2 条和库里重复、1 条缺 `dedup_key`。预期预览为有效 2、重复 2、无效 1；导入后新增 2 条、跳过 3 条；库里原来那 2 条的 Token 值不变。
  - `preview_writes_nothing`：预览前后库里的行数相同。
  - `export_range_is_half_open`：记录时间为 start、end-1、end，导出结果只包含前两条。
- [ ] **步骤 2：运行**，预期失败。
- [ ] **步骤 3：实现**：
  - 把逐条校验从「遇到一条错就整体失败」改为收集原因。
  - 判断重复用 `SELECT 1 FROM requests WHERE source=? AND dedup_key=?`，原生来源用对应的表。
  - 导出时给 requests、metrics、attributions 加时间条件，`source_store::contexts` 也加。
- [ ] **步骤 4：前端**：导入流程改为选文件、显示预览卡片（文案见画板 100 · 05）、点「导入 N 条」、提示「完成：已导入 N 条，跳过 M 条」。导出弹窗加日期范围行，点击打开 `DateRangeDialog`。去掉两处 `PendingBadge` 和「待接入」文案。
- [ ] **步骤 5：运行测试和类型检查**。

### 任务 2.5：系统通知：任务提醒与低余额提醒

**文件：**
- 修改：`backend/Cargo.toml`（加 `tauri-plugin-notification = "2"`）、`backend/capabilities/default.json`（只有 Rust 端发通知，不加前端权限）
- 新建：`backend/src/notify.rs`
- 修改：`backend/src/lib.rs`（注册插件；`MainIntent` 增加 `Automation { task_id: i64 }`；`set_task_notify` 命令）
- 修改：`backend/src/automation.rs:781-788, 1426-1433, 1476-1480, 1173`（在这些位置挂通知）
- 修改：`backend/src/quota.rs`（额度刷新后检查低余额）
- 修改：`backend/src/settings.rs`（字段）
- 修改：`frontend/src/components/settings/NotificationSection.tsx:96-107`、`frontend/src/lib/mainNavigation.ts`（`NavigationIntent` 增加 automation 分支）、`frontend/src/views/AutomationView.tsx`（按 task_id 打开详情）
- 测试：`notify.rs` 的 `mod tests`

**接口：**
- 产出（Rust）：
  - 字段 `notify_task_resumed: bool`、`notify_task_finished: bool`，默认都为 true，用 `#[serde(default = "yes")]`。
  - `pub enum TaskNotice { Resumed, Completed, Failed { reason: String } }`
  - `pub fn task_message(kind: &TaskNotice, platform: &str, task_name: &str) -> (String, String)`：返回标题和正文，纯函数。
  - `pub fn should_notify(settings: &Settings, kind: &TaskNotice) -> bool`：免打扰时返回 false；按两个开关分别判断。
  - `pub fn notify_task(app: &AppHandle, task_id: i64, kind: TaskNotice, platform: &str, task_name: &str)`
  - `pub fn balance_crossed(prev_remaining: Option<f64>, now_remaining: f64, threshold: f64) -> bool`：只在从阈值以上（或第一次读到）跌到阈值以下时返回 true。
  - 命令 `set_task_notify(app, resumed: Option<bool>, finished: Option<bool>) -> Result<Settings, String>`
- 文案：
  - 恢复：标题「任务已继续」，正文「{平台}额度已恢复，「{任务名}」正在原会话中继续。」
  - 完成：「任务已完成」，「「{任务名}」已运行完成。」
  - 失败：「任务失败」，「「{任务名}」：{原因}」，原因截断到 60 字。
  - 低余额：「余额偏低」，「{连接名}剩余 {金额} {币种}，低于提醒阈值。」
  - 币种和阈值的币种不同时不比较。
- 点击通知后调用 `show_main_with_intent(MainIntent::Automation { task_id })`。Windows 通知的点击回调如果插件不支持，就退一步只在通知正文里写任务名。执行时用 Context7 核对插件的点击回调能力。

- [ ] **步骤 1：Context7 核对** `tauri-plugin-notification` v2 的 Rust 用法和 Windows 点击行为（最多 3 条命令）。
- [ ] **步骤 2：写失败测试**：
  - `should_notify_respects_dnd_and_switches`：免打扰时三种通知都返回 false；关掉 resumed 只拦恢复通知。
  - `task_message_texts`：断言上面三段文案；原因超过 60 字时截断并以「…」结尾。
  - `balance_crossed_once`：(None, 5, 10) 为 true；(Some(5), 4, 10) 为 false；(Some(12), 9, 10) 为 true；币种不同时，调用方不调用这个函数，在调用处测。
- [ ] **步骤 3：运行**，预期失败。
- [ ] **步骤 4：实现**并在 automation 的四个位置挂上通知；「需要处理」和「等待超时」按 D-10 归入 Failed。
- [ ] **步骤 5：前端**：两个开关接上命令，去掉 `PendingBadge`。
- [ ] **步骤 6：运行测试**。开发模式下通知显示的是 PowerShell 的名字和图标，这是已知限制；安装版的验收放进阶段 5。

### 任务 2.6：关闭同账号排队

**文件：**
- 修改：`backend/src/automation.rs`：`ensure_schema` 加列；`TaskInput` / `Task` 加字段；`startable()`（1127 行）；`queue_ahead()`（531 行）
- 修改：`frontend/src/lib/automationApi.ts:35,51`、`frontend/src/components/automation/TaskDialog.tsx:440-442`
- 测试：`automation.rs` 的 `mod tests`

**接口：**
- 产出：表加列 `serial_queue INTEGER NOT NULL DEFAULT 1`；Rust 字段 `serial_queue: bool`（`#[serde(default = "yes")]`）；TS 字段 `serial_queue: boolean`。

- 常量 `const MAX_PARALLEL_PER_ACCOUNT: usize = 3`。
- 账号进入额度等待的判断：同账号下任意一个任务处于等待额度状态。

- [ ] **步骤 1：写失败测试**（规则见 D-09）：
  - `startable_skips_serial_for_opt_out`：同一连接下 A 运行中且 serial，B 排队且 serial，C 排队且 non-serial，工作目录各不相同。预期 `startable` 返回 [C]；A 完成后返回 [B]。
  - `same_cwd_always_serial`：C 与正在运行的 A 工作目录相同（大小写和斜杠不同也算同一目录），预期 C 不被放行。
  - `parallel_cap_per_account`：同账号已有 3 个在运行，第 4 个 non-serial 任务不放行。
  - `quota_wait_blocks_whole_account`：同账号有一个任务处于等待额度，预期该账号下 serial 和 non-serial 任务都不放行；其他账号的任务不受影响。
  - `recovery_releases_serial_head_first`：额度恢复后同时有 serial 队首和 non-serial 任务时，按顺序放行，先 serial 队首，总数不超过上限。
- [ ] **步骤 2：运行** `cargo test --manifest-path backend/Cargo.toml startable`，预期失败。
- [ ] **步骤 3：实现**：在 `startable` 里依次应用四条规则；工作目录比较复用会话模块已有的规范化方式（小写、`\` 换成 `/`、去掉末尾 `/`）；`queue_ahead` 只统计 serial 任务；额度检查按账号去重。
- [ ] **步骤 4：前端**：复选框可以点了；关闭时显示 D-09 的提示文案。
- [ ] **步骤 5：运行测试和类型检查**。

### 任务 2.7：Skills：从 Git 仓库导入、查看差异

**文件：**
- 修改：`backend/src/skills.rs`（新增 Git 拉取，复用 `import_scan_with`，1200 行）
- 新建：`frontend/src/lib/textDiff.ts`
- 修改：`frontend/src/components/skills/skillDialogs.tsx:118`、`SkillEditor.tsx:190`、`frontend/src/lib/skillsApi.ts`
- 测试：`frontend/src/lib/textDiff.test.mjs`，`skills.rs` 的 `mod tests`

**接口：**
- 产出（Rust）：
  - `pub fn validate_git_url(url: &str) -> Result<(), String>`：只接受 `https://`，不能带用户名或密码（`https://user:pw@` 要拒绝）。
  - 命令 `skills_import_git(app, url: String, branch: Option<String>, platform: String, scope: String, project: Option<String>) -> Result<GitImportScan, String>`
  - `GitImportScan { checkout_dir: String, scan: ImportScan }`：执行 `git clone --depth 1 [--branch B] <url> <临时目录>`，60 秒超时，用 `CREATE_NO_WINDOW` 隐藏窗口。之后复用现有的 `skills_import(source = checkout_dir, ...)`。
  - 导入完成或取消后删除临时目录：命令 `skills_import_git_cleanup(checkout_dir: String)`，只能删除应用临时目录下以 `jolu-git-` 开头的目录。
- 产出（TS）：
  - `export type DiffLine = { kind: "same" | "add" | "del"; text: string }`
  - `export function lineDiff(a: string, b: string): DiffLine[]`：用 LCS 算法，超过 5,000 行时退化为整段替换。

- [ ] **步骤 1：写失败测试**：
  - `textDiff.test.mjs`：`lineDiff("a\nb\nc", "a\nx\nc")` 得到 `[same a, del b, add x, same c]`；两段相同时全部是 same；空串对 `"a"` 得到 `[add a]`。
  - Rust `git_url_rules`：接受 `https://github.com/o/r.git`；拒绝 `git@github.com:o/r`、`http://x`、`https://u:p@x/r`、`file:///c:/x`。
  - Rust `cleanup_only_removes_own_temp`：传入其他目录时报错，目录保持不变。
- [ ] **步骤 2：运行**，预期失败。
- [ ] **步骤 3：实现**。本机没有 git 时报「未找到 git，请先安装 Git」。
- [ ] **步骤 4：前端**：
  - 导入弹窗的来源切换按 D-16；获取成功后进入现有的冲突列表。
  - 差异弹窗按 D-17：删除行用 `jolu-danger` 浅底，新增行用 `jolu-on` 浅底。
- [ ] **步骤 5：运行测试和类型检查**；在隔离环境里导入一个公开的 Skills 仓库，例如 `https://github.com/anthropics/skills`，执行时确认它仍然存在。

### 任务 2.8：用量概览显示会话项目名

**文件：**
- 修改：`backend/src/usage_stats.rs:304-310, 445-471`
- 修改：`backend/src/session_threads.rs`：把 `source_files` 和 `read_cwd` 改为 `pub(crate)`
- 修改：`frontend/src/lib/api.ts:598-603`、`frontend/src/components/usage/OverviewPanel.tsx:137-138`
- 测试：`usage_stats.rs` 的测试模块

**接口：**
- 产出：`SessionRow` 和 `StatsSessionRowDto` 增加 `cwd: Option<String>`；前端显示 `{projectName(cwd)} · {供应商}`，没有 cwd 时只显示供应商。

- [ ] **步骤 1：写失败测试** `top_sessions_fills_cwd_from_source`：造一个临时的 Claude jsonl 文件，首行带 `cwd`，并登记为来源文件；预期 `top_sessions` 返回的那一行 `cwd` 等于该路径。来源文件不存在时 `cwd` 为 `None`。
- [ ] **步骤 2：运行**，预期失败。
- [ ] **步骤 3：实现**并更新前端。
- [ ] **步骤 4：运行测试和类型检查**。

### 任务 2.9：自动化删除任务（鼠鼠 2026-10-23 要求）

任务 1.2 验收时发现：后端没有删除任务的命令，界面也删不掉任务；画板 68「08 任务更多操作」里只有编辑任务、暂停等待（恢复 / 停止运行），没有删除。鼠鼠确认要补上。

**文件：**
- 修改：`docs/ui/Jolu.pen`：画板 68 / 69 的「08 任务更多操作」菜单末尾加一项「删除任务」（危险色，前面加分隔线）；同区补一个删除确认弹窗状态。浅色和深色都要改。
- 修改：`backend/src/automation.rs`（新命令；调度器和运行中进程的处理）、`backend/src/lib.rs`（注册命令）
- 修改：`frontend/src/lib/automationApi.ts`、自动化页的任务更多操作菜单所在组件（`frontend/src/components/automation/` 下，以实际文件为准）、`frontend/src/views/AutomationView.tsx`
- 测试：`automation.rs` 的 `mod tests`；`frontend/src/lib/automation.test.mjs`

**接口：**
- 产出（Rust）：
  - `pub fn delete_task(conn: &mut Connection, task_id: i64) -> Result<(), String>`：在一个事务里删除任务及其运行记录和事件。用量统计（`requests` 等表）不动。
  - 任务有运行中的实例时报「任务正在运行，请先停止」；处于排队或等待额度时，先取消等待再删除。
  - 命令 `automation_delete(app, task_id: i64) -> Result<(), String>`：成功后发出 `automation-changed` 事件。
- 产出（TS）：
  - `automationApi.deleteTask(taskId: number)`
  - `export function canDeleteTask(status: string): boolean`：运行中返回 false，其余返回 true。
- 界面：
  - 菜单项「删除任务」放在最后，用危险色；运行中时禁用，提示「运行中的任务需先停止」。
  - 点击后弹出确认框：
    - 标题「删除任务「{任务名}」？」
    - 正文「任务和它的运行记录会一起删除，已产生的用量统计保留。此操作无法撤销。」
    - 按钮「取消」「删除」，其中「删除」用危险色。
  - 删除成功后关闭详情并刷新列表；失败时在弹窗内显示原因。

- [ ] **步骤 1：补画画板**（Pencil），截图对照。
- [ ] **步骤 2：写失败测试**：
  - `delete_task_removes_runs_keeps_usage`：建一个任务、两条运行、若干事件，再往 `requests` 表插一条记录；删除后任务、运行、事件都查不到，`requests` 的记录还在。
  - `delete_running_task_rejected`：有运行中实例时报「任务正在运行，请先停止」，数据不变。
  - `delete_waiting_task_cancels_wait`：等待额度的任务可以删除，`startable` 和等待检查列表里都不再出现它。
  - TS：`canDeleteTask("running")` 为 false，`canDeleteTask("paused")` 为 true。
- [ ] **步骤 3：运行**，预期失败。
- [ ] **步骤 4：实现**前后端。
- [ ] **步骤 5：运行测试和类型检查**；在隔离环境里新建一个任务，删除它，确认列表刷新、界面不报错。

**阶段 2 结束：** 跑全量测试和类型检查，用 `main-window-smoke.e2e.mjs` 冒烟，对照画板 86、88、90、92、94、100、102、82、84、30、68 截图；请鼠鼠决定是否提交。

---

## 阶段 3：按决策开发

任务 3.2 和 3.6 都依赖 3.1，两者之间可以并行；3.3、3.4、3.5 之间相互独立。

### 任务 3.1：会话交接后端

按第三节 3.3 实现生成流程，按 3.4 实现对话弹窗用到的命令。

**文件：**
- 新建：`backend/src/handoff_facts.rs`（事实区，纯函数）
- 新建：`backend/src/handoff_input.rs`（精简、序列化、token 估算、切段）
- 新建：`backend/src/handoff_digest.rs`（段落纪要的复用、失效与归并）
- 新建：`backend/src/handoff_baseline.rs`（原文缺失时读取原工具的压缩摘要）
- 新建：`backend/src/session_handoff.rs`（提示词、生成与修改流程、质量检查、存储、命令）
- 修改：`backend/src/automation.rs`：`Progress`、`classify`、`current_connection_ids`（1312 行）改为 `pub(crate)`
- 修改：`backend/src/session_threads.rs`：`messages`（463 行）、`source_files`、`ToolCall`（233 行）供新模块使用；`ToolCall` 增加 `failed: Option<bool>`，从工具结果的错误标记或退出码取得，取不到时为 None
- 修改：`backend/src/lib.rs`（注册命令）
- 测试：各新文件的 `mod tests`；样例放 `backend/tests/fixtures/handoff/`，只保留结构，正文替换为占位文字

**接口：**
- 表 `handoff_digests`，约束 `UNIQUE(platform, session_id, level, from_idx)`。字段：
  - 定位：`id INTEGER PK`、`platform TEXT`、`session_id TEXT`、`level INTEGER`（1 表示段落纪要，2 表示早期纪要）
  - 范围与校验：`from_idx INTEGER`、`to_idx INTEGER`、`range_hash TEXT`
  - 内容：`digest_md TEXT`、`tokens INTEGER`、`source TEXT`（`raw` / `claude-native` / `codex-native`）
  - 归并：`rolled_into INTEGER`（被归并时指向早期纪要的 id，否则为空）
  - 版本：`prompt_version TEXT`、`created_ms INTEGER`
- 表 `session_handoffs`，约束 `UNIQUE(platform, session_id, version)`。字段：
  - 基本信息：`id INTEGER PK`、`platform TEXT`、`session_id TEXT`、`version INTEGER`
  - 覆盖范围：`covered_to INTEGER`
  - 内容：`narrative_md TEXT`、`facts_json TEXT`、`focus TEXT`、`target TEXT`
  - 草稿：`draft_rounds INTEGER`、`path_warnings TEXT`
  - 执行：`executor_platform TEXT`、`executor_connection_id TEXT`、`model TEXT`、`input_tokens INTEGER`、`output_tokens INTEGER`、`calls INTEGER`
  - 版本与时间：`prompt_version TEXT`、`created_ms INTEGER`
- 常量：
  - 预算类：`INPUT_BUDGET = 167_000`、`RECENT_RAW_TOKENS = 20_000`、`DIGEST_SEGMENT_TOKENS = 40_000`、`DIGEST_ROLLUP_TOKENS = 60_000`
  - 精简类：`KEEP_RECENT_TOOL_GROUPS = 5`、`TOOL_OUTPUT_MAX = 2_000`
  - 其他：`MAX_REFINE_ROUNDS = 10`、`PROMPT_VERSION = "handoff-v1"`
- 产出（`handoff_facts.rs`）：
  - `pub struct Facts`，字段：
    - 用户原话：`user_timeline: Vec<(usize, String)>`、`earlier_user_count: usize`
    - 文件：`files_modified: Vec<(String, usize)>`、`files_read: Vec<(String, usize)>`
    - 命令：`commands: Vec<CommandFact>`、`last_verify: Option<CommandFact>`、`failures: Vec<CommandFact>`
    - 其他：`git: Option<GitSnapshot>`、`transcript_path: Option<String>`
  - `CommandFact { idx: usize, command: String, exit: Option<i32>, tail: String }`
  - `pub fn extract_facts(messages: &[Message], cwd: Option<&Path>, transcript: Option<&Path>) -> Facts`：规则见 3.3 第 1 条。
  - `pub fn is_verify_command(cmd: &str) -> bool`：匹配 `test|build|check|lint|typecheck|tsc|pytest|jest|vitest|cargo (test|build|check|clippy)`。
  - `pub fn git_snapshot(cwd: &Path) -> Option<GitSnapshot>`：只读，5 秒超时，`CREATE_NO_WINDOW`。
  - `pub fn render_facts(f: &Facts) -> String`
  - `pub fn summary_line(f: &Facts) -> String`：对话弹窗第一条步骤消息，例如「改过 6 个文件 · 12 条命令 · 最后一次验证失败」。
- 产出（`handoff_input.rs`）：
  - `pub fn estimate_tokens(text: &str) -> usize`：字符数除以 3，向上取整。
  - `pub fn condense(messages: &[Message]) -> Vec<CondensedMsg>`：规则见 3.3 第 2 条。
  - `pub fn serialize(msgs: &[CondensedMsg], from: usize, to: usize) -> String`
  - `pub fn recent_start(msgs: &[CondensedMsg], recent_tokens: usize) -> usize`：退到轮次边界。
  - `pub fn plan_segments(msgs: &[CondensedMsg], from: usize, to: usize, max_tokens: usize) -> Vec<(usize, usize)>`
- 产出（`handoff_digest.rs`）：
  - `pub fn range_hash(msgs: &[CondensedMsg], from: usize, to: usize) -> String`
  - `pub fn reusable(stored: &[DigestRow], msgs: &[CondensedMsg], upto: usize) -> (Vec<DigestRow>, Vec<(usize, usize)>)`：返回可复用的纪要，以及需要新建或重做的范围。
  - `pub fn needs_rollup(active: &[DigestRow], limit: usize) -> Option<Vec<i64>>`
  - `pub fn relevant_digests(instruction: &str, digests: &[DigestRow]) -> Vec<i64>`：按意见里出现的路径和长度不少于 2 个字的关键词去匹配纪要正文，最多返回 3 份。
- 产出（`handoff_baseline.rs`）：
  - `pub fn claude_baseline(jsonl: &str) -> Option<NativeBaseline>`
  - `pub fn codex_baseline(jsonl: &str) -> Option<NativeBaseline>`：加密的记录返回 None。
  - `NativeBaseline { source: &'static str, summary: String, boundary_idx: usize }`
- 产出（`session_handoff.rs`）：
  - 提示词常量：`DIGEST_PROMPT`、`ROLLUP_PROMPT`、`SYNTH_PROMPT`、`REFINE_PROMPT`、`NO_TOOLS_PREAMBLE`、`NO_TOOLS_REMINDER`。
  - `pub const SECTIONS: [&str; 8]`：3.3 第 4 条的 8 个标题。
  - `pub fn check_output(raw: &str, stop: StopReason) -> Result<String, String>`：删掉 `<analysis>` 取 `<summary>`，检查项见 3.3 第 7 条。
  - `pub fn unknown_paths(narrative: &str, messages: &[Message]) -> Vec<String>`
  - `pub fn executor_args(platform: &str, model: Option<&str>) -> Result<Vec<String>, String>`：参数按 D-01，绝不带 `--resume`。
  - `pub trait Executor { fn run(&self, prompt: &str) -> Result<ExecOutput, String>; }`
  - `ExecOutput { text, stop: StopReason, input_tokens, output_tokens, context_exceeded: bool }`
  - `pub fn generate(exec: &dyn Executor, req: GenerateRequest, emit: &dyn Fn(ChatEvent)) -> Result<Draft, String>`：纯流程，依次执行：复用或补齐纪要（每生成一份就通过 `ChatEvent::DigestSaved` 交给调用方落库）、按需归并、合成、检查、降级。
  - `pub fn refine(exec: &dyn Executor, draft: &Draft, facts: &Facts, instruction: &str, digests: &[DigestRow], emit: &dyn Fn(ChatEvent)) -> Result<Draft, String>`
  - `Draft { round: u32, narrative_md: String, path_warnings: Vec<String>, input_tokens: Option<u64>, output_tokens: Option<u64> }`
  - `pub enum ChatEvent { Step { text: String, done: usize, total: usize }, DigestSaved(DigestRow), Draft(Draft), Error { text: String, retryable: bool } }`
  - `pub fn render_markdown(narrative: &str, facts: &Facts, meta: &RenderMeta) -> String`：排版见 3.3 第 5 条。
- 命令（对话弹窗用）：
  - `handoff_state(app, platform, session_id) -> Result<HandoffState, String>`
    - `HandoffState { readable, total, latest: Option<HandoffMeta>, digest_covered_to: usize, reusable: bool, estimate_tokens: usize }`
  - `handoff_start(app, platform, session_id, connection_id, model: Option<String>, focus: String, target: String) -> Result<String, String>`：返回本次弹窗的 `job_id`。草稿保存在内存中，按 `job_id` 索引，每次只有一个活动任务；过程通过事件 `handoff-chat` 推送 `{ job_id, event: ChatEvent }`。
  - `handoff_refine(app, job_id, instruction: String) -> Result<(), String>`：超过 10 轮时报「已达到修改上限」。
  - `handoff_estimate_refine(app, job_id, instruction: String) -> Result<usize, String>`：给底部状态行用。
  - `handoff_finish(app, job_id, round: u32, project: String) -> Result<HandoffResult, String>`：用选定的版本写入新的交接版本，返回排好版的 Markdown。
  - `handoff_cancel(app, job_id)`：结束进程树，丢弃草稿，已保存的纪要保留。
  - `handoff_render(app, platform, session_id, project, topic, target) -> Result<String, String>`：不调用模型（画板 52）。
  - `handoff_save(app, markdown, file_name, prefer_project_dir) -> Result<Option<String>, String>`
  - `validate_file_name`：规则同画板 54-09。

- [ ] **步骤 1：核对前提**
  - Context7 核对 `--no-session-persistence`，以及 Codex `exec` 的 `--ephemeral` / `--sandbox read-only` / `--skip-git-repo-check`，再用本机 `--help` 佐证。
  - 用本机样本核对：工具结果能否取到失败标记或退出码；压缩过的会话文件里是否仍保留压缩前的原文。第二件不成立时向鼠鼠汇报。
- [ ] **步骤 2：定稿提示词**：`DIGEST_PROMPT`、`ROLLUP_PROMPT`、`SYNTH_PROMPT`、`REFINE_PROMPT`，贴给鼠鼠过目。
  - 四套都以「只输出文字，不调用工具」开头和结尾，都要求先写 `<analysis>` 再写 `<summary>`。
  - 都用中文写，标识符保持原样，内容要带 `#n` 来源。
  - `REFINE_PROMPT` 要求只按用户意见修改，没被提到的章节保持原样，八节都不能删。
- [ ] **步骤 3：写失败测试**：
  - 事实区：
    - `facts_files_modified_and_read`：Read a(#1)、Edit a(#3)、Read b(#4)，预期改过 = [(a,3)]，读过 = [(b,4)]。
    - `facts_user_timeline_excludes_tool_results`
    - `facts_last_verify_and_failures`：`pnpm test` 输出 30 行、退出码 1，预期 tail 只有 15 行，failures 里有这条命令。
  - 精简与切段：
    - `condense_masks_old_outputs_keeps_errors`
    - `recent_start_respects_turn_boundary`
  - 纪要：
    - `only_new_ranges_get_digested`：已有纪要覆盖 #0–#15，共 40 条，最近窗口从 #30 开始。用会记录输入的假执行器跑，预期只为 #16–#29 生成纪要，#0–#15 的原文一个字都不出现在任何请求里。
    - `small_increment_single_call`：新消息都在最近窗口里时，只调用 1 次（合成）。
    - `changed_message_redoes_one_digest`
    - `truncation_drops_out_of_range_digests`
    - `rollup_when_digests_exceed_limit`
    - `digest_saved_before_synthesis_failure`：合成失败时，已生成的纪要仍然通过 `DigestSaved` 交出，但不产生草稿。
  - 修改：
    - `refine_sends_draft_facts_instruction_only`：修改请求里有当前草稿、事实区和意见；关键词命中的纪要不超过 3 份；不包含会话原文。
    - `refine_round_limit`：第 11 次时报「已达到修改上限」。
    - `relevant_digests_by_path`：意见里提到 `src/a.ts` 时，返回包含这个路径的纪要。
  - 检查与降级：
    - `check_output_rules`：截断、缺章节、下一步没有 `#n` 三种情况各报对应的原因。
    - `unknown_paths_flags_invented`
    - `context_exceeded_degrades`
    - `no_new_messages_skips_model`
  - 其他：`render_order_and_repeat`、`codex_baseline_skips_encrypted`、`claude_baseline_reads_last_summary`、`executor_args_never_resume`、`file_name_rules`
- [ ] **步骤 4：运行** `cargo test --manifest-path backend/Cargo.toml handoff`，预期失败。
- [ ] **步骤 5：实现**：
  - `handoff_start` 一开始就固定快照 `snapshot_to = total`；
  - 纪要生成一份保存一份；
  - 交接版本只在 `handoff_finish` 时，在一个事务里写入。
- [ ] **步骤 6：运行测试**，预期通过。

### 任务 3.2：会话压缩前端（含对话弹窗）

**文件：**
- 修改：`docs/ui/Jolu.pen`：模块 D 末尾补画「压缩对话弹窗」浅色、深色各一张（布局见 3.4）；画板 48 的提示行补「预计发送约 N token」；画板 50 补「以下路径未在会话中出现」提示
- 新建：`frontend/src/lib/sessionHandoff.ts`、`frontend/src/components/sessions/CompressDialog.tsx`（画板 48 / 52 / 54 的配置与状态）、`frontend/src/components/sessions/CompressChat.tsx`（对话弹窗）、`frontend/src/components/sessions/HandoffPreview.tsx`（画板 50）
- 修改：`frontend/src/components/sessions/SessionDetail.tsx:171-179`、`frontend/src/views/SessionsView.tsx:29, 48-49, 247-267`、`frontend/src/lib/api.ts`（绑定命令）
- 测试：`frontend/src/lib/sessionHandoff.test.mjs`

**接口：**
- 消耗：任务 3.1 的命令、`HandoffState`、`ChatEvent`、`Draft`、`HandoffResult`，以及 `handoff-chat` 事件。
- 产出（TS）：
  - `export function handoffFileName(project: string, topic: string, target: string, at: Date): string`：格式为 `项目_主题_交接-目标AI_YYYYMMDD-HHmm.md`。
  - `export function dedupeFileName(name: string, exists: (n: string) => boolean): string`
  - `export function dialogMode(state: HandoffState): "first" | "incremental" | "reuse" | "unreadable"`
  - `export type ChatItem = { kind: "step" | "draft" | "user" | "error"; text: string; round?: number; warnings?: string[]; retryable?: boolean }`
  - `export function reduceChat(items: ChatItem[], event: ChatEventDto): ChatItem[]`：同一阶段的步骤消息原地更新进度，不新增一行。
  - `export function formatTokens(n: number): string`：1234 显示为 `1.2k`，小于 1000 原样显示。
  - `export const HANDOFF_PROMPT`：画板 50「可一起发送的指令」原文。
- 界面：
  - 画板 48：配置弹窗。提示行加「预计发送约 {formatTokens(estimate_tokens)} token」；「交接主题」的提示文字为「接手后要做什么（可留空）」。
  - 对话弹窗：按 3.4。最新消息出现时自动滚到底部，但用户往上翻看时不强制滚动；生成中禁用输入框；草稿消息默认折叠，只显示前三节的开头。
  - 画板 52、54-01、54-04、54-06、54-07、54-08、54-09：按原文实现，其中 54-03 的进行中状态由对话弹窗取代。
  - 画板 50：在覆盖层里预览；「Markdown 源码」页签可以编辑，保存编辑后的内容；有 `path_warnings` 时，在提示条下方列出「以下路径未在会话中出现：…」。
  - 「执行压缩的 AI」下拉：来源是 `automation_accounts`，不是 CLI 当前登录账号的项禁用，并注明「需为 CLI 当前登录账号」。

- [ ] **步骤 1：补画画板**（Pencil）：对话弹窗浅色和深色各一张，以及画板 48、50 的两处提示，截图给鼠鼠看。
- [ ] **步骤 2：写失败测试**：
  - `handoffFileName("jolu", "会话界面优化", "Claude", new Date(2026,9,21,16,48))` 等于 `"jolu_会话界面优化_交接-Claude_20261021-1648.md"`；主题里的非法字符替换为 `-`。
  - `dedupeFileName("a.md", n => n === "a.md")` 得到 `"a-2.md"`。
  - `dialogMode` 四种状态各一个用例。
  - `reduceChat`：连续两个同阶段的 Step 只留一行，并且进度被更新；Draft 追加一行，带上 round 和 warnings。
  - `formatTokens(1234)` 为 `"1.2k"`，`formatTokens(999)` 为 `"999"`。
- [ ] **步骤 3：运行** `node --test frontend/src/lib/sessionHandoff.test.mjs`，预期失败。
- [ ] **步骤 4：实现**：纯逻辑和四个组件；文案取画板原文；去掉「压缩会话待接入」。
- [ ] **步骤 5：运行测试和类型检查**；用 Pencil 截图画板 48、50、52、54 和新补画的弹窗，与运行效果对照。
- [ ] **步骤 6：真机**：征得鼠鼠同意后，对一个短会话做一次真实压缩，提一次修改意见，生成并保存 MD；再追加几条消息，压缩一次，确认只处理了新消息。

### 任务 3.6：交接提示词评测（第三节 3.5）

依赖任务 3.1，可以和 3.2 并行。

**文件：**
- 新建：`scripts/handoff-eval/README.md`
- 新建：`backend/src/bin/handoff_eval.rs`：复用 3.1 的模块；只在 debug 构建中编译，不进入安装包
- 新建：`docs/verification/2026-10-xx-handoff-eval.md`（评测结果）

**接口：**
- 命令行：
  ```
  handoff_eval select       # 列出 50 轮以上的会话，由鼠鼠勾选，写入 output/handoff-eval/selection.json
  handoff_eval questions    # 为勾选的会话出题，结果缓存
  handoff_eval run          # 生成两份交接文档（基线、本方案），作答并打分
  handoff_eval report       # 输出报告，含按会话的 bootstrap 95% 置信区间
  ```
- 输出目录：`output/handoff-eval/`，已被 Git 忽略。

- [ ] **步骤 1：写测试**：
  - `program_questions_from_jsonl`：改过的文件集合、最后一个报错、最后一条用户指令，与预期一致。
  - `set_f1_normalizes_paths`：斜杠方向、大小写不同的路径视为同一个文件。
  - `bootstrap_ci_by_session`：固定随机种子时，结果可以复现。
- [ ] **步骤 2：实现**：出题只读原始会话；裁判看不到候选名称；裁判与生成文档用同一家服务商。
- [ ] **步骤 3：征得鼠鼠同意**：确认会话清单和调用量，约 80 次 CLI 调用。
- [ ] **步骤 4：先用开发集 4 个会话调提示词**，再只跑一次测试集 8 个会话。
- [ ] **步骤 5：处理结果**：本方案在某一类题上明显落后时，修改提示词，并把版本号升到 `handoff-v2`。结果写进验证文档，汇报给鼠鼠。

### 任务 3.3：自动化选择 Skills

**文件：**
- 修改：`backend/src/automation.rs`：`validate`（383 行）、`start_run`（提示词前缀）、续跑提示词、新命令
- 修改：`backend/src/skills.rs`：新增一个公开的轻量查询函数，复用 `overview_with`（823 行）
- 新建：`frontend/src/components/automation/SkillPicker.tsx`
- 修改：`frontend/src/components/automation/TaskDialog.tsx:212, 337-344`、`frontend/src/lib/automationApi.ts`
- 测试：`automation.rs` 的 `mod tests`、`frontend/src/lib/automation.test.mjs`

**接口：**
- 产出（Rust）：
  - `pub fn enabled_skills(platform: &str, cwd: Option<&Path>) -> Result<Vec<SkillOption>, String>`，放在 `skills.rs`
  - `SkillOption { name: String, description: Option<String>, scope: String, project: Option<String> }`
  - 命令 `automation_skills(app, platform: String, cwd: Option<String>) -> Result<Vec<SkillOption>, String>`
  - `pub fn skill_prefix(platform: &str, names: &[String]) -> String`：Codex 写成 `"$a $b\n\n"`；Claude 写成 `"本任务请使用以下 Skills：a、b\n\n"`；列表为空时返回空串。
  - `validate`：最多 10 个；每个名称都要通过 `skills::valid_name`。
- 产出（TS）：`automationApi.skills(platform, cwd)`；`export function skillSummary(names: string[]): string`，规则为：0 个返回 `""`；否则返回 `已选择 N 项 · a、b`，超过 2 个时写 `a、b 等`。

- [ ] **步骤 1：写失败测试**：
  - `skill_prefix_formats`：断言上面的三种输出。
  - `validate_rejects_too_many_skills`：11 个时报错。
  - `run_blocks_when_skill_missing`：任务选了 `x`，但 `enabled_skills` 的结果里没有它。预期运行转为「需要处理」，原因为「Skill「x」已停用或不存在」，并且没有启动进程（用测试用的检查函数 `fn missing_skills(selected, available) -> Vec<String>` 验证）。
  - `skillSummary`：`[]` 得到 `""`；`["项目约定","测试检查"]` 得到 `"已选择 2 项 · 项目约定、测试检查"`。
- [ ] **步骤 2：运行**，预期失败。
- [ ] **步骤 3：实现**：首次运行和续跑都加前缀，执行前用 `missing_skills` 检查。
- [ ] **步骤 4：前端** `SkillPicker`：弹出层里有搜索框、每项一个复选框，项目级的项后面标项目名。切换工具或工作目录时清掉已经无效的选择。
- [ ] **步骤 5：运行测试和类型检查**；对照画板 60、72。

### 任务 3.4：自定义供应商

**文件：**
- 修改：`docs/ui/Jolu.pen`：通过 Pencil 补画「自定义供应商 · 连接」弹窗，浅色和深色各一张，放在模块 B 末尾，编号按现有最后一个画板顺延
- 修改：`frontend/src/types.ts:2`（`PlatformId` 加 `"custom"`）、`frontend/src/lib/platformCatalog.json`、`frontend/src/lib/providerCatalog.ts`、`frontend/src/components/brand/PlatformLogo.tsx`（复用 `skillParts.tsx:127-131` 的六边形标识）、`frontend/src/views/ProvidersView.tsx:162-181`、`frontend/src/components/providers/ConnectDialog.tsx`
- 修改：`backend/src/platforms.rs:24, 41`、`backend/src/connections.rs:446, 587`、`backend/src/lib.rs:1504`（`fetch_remote_models` 白名单）、`backend/src/quota.rs:742-783`（`validate_connection`）
- 修改：`backend/src/db.rs` 的 `init`：`connections` 表加 `protocol TEXT`，值为 `openai` 或 `anthropic`，可空
- 测试：`providerCatalog.test.mjs`、`connections.rs` / `quota.rs` 的测试模块

**接口：**
- 产出（Rust）：
  - `NewConnection` 增加 `protocol: Option<String>`。
  - `apply_auth`：`protocol == "anthropic"` 时用 `x-api-key` 加 `anthropic-version`，否则用 Bearer。
  - `validate_connection`：平台为 `custom` 时，有模型列表或手填模型就算通过；不调用额度接口。
  - 余额显示为「不支持查询」。
- 产出（TS）：`PROVIDERS` 增加 `{ id: "custom", name: "自定义供应商", access: "名称 / Base URL / API Key", subscription: "none", apiKey: "ready" }`，卡片放在最前（画板 05 的顺序）。

- [ ] **步骤 1：补画画板**（Pencil）：弹窗字段按 D-11，复用画板 15/17 的 API Key 弹窗结构；截图给鼠鼠看。
- [ ] **步骤 2：写失败测试**：
  - Rust `custom_validation_uses_models`：custom 连接带 `model` 时校验通过，并且没有调用额度接口（把额度函数注入成一个 panic 实现）。
  - Rust `anthropic_protocol_headers`：`apply_auth` 得到的请求头里有 `x-api-key`，没有 `Authorization`。
  - TS：`canConnect(providerEntry("custom"))` 为 true；`showsModeTabs` 为 false。
  - 更新 `platforms.rs` 里目录长度的断言（9 改 10）。
- [ ] **步骤 3：运行**，预期失败。
- [ ] **步骤 4：实现**前后端，去掉「自定义供应商待接入」。
- [ ] **步骤 5：运行测试和类型检查**；征得鼠鼠同意后，用一个真实的 OpenAI 兼容服务验证获取模型和保存。

### 任务 3.5：登录指引与切换账号链接

**文件：**
- 新建：`frontend/src/lib/providerLinks.ts`
- 修改：`frontend/src/components/providers/ConnectDialog.tsx:364-366`
- 测试：`frontend/src/lib/providerLinks.test.mjs`

**接口：**
- 产出：`export const PROVIDER_LINKS: Partial<Record<PlatformId, { loginGuide?: string; switchAccount?: string; verified: string }>>`；`export function linkFor(platform: PlatformId, kind: "loginGuide" | "switchAccount"): string | null`

- [ ] **步骤 1：核实链接**：用 WebSearch / WebFetch 找每个平台官方的登录文档页，只收录官方域名的 HTTPS 地址，`verified` 填核实日期。
- [ ] **步骤 2：写测试**：每个收录的 URL 都以 `https://` 开头；未收录的平台 `linkFor` 返回 null。
- [ ] **步骤 3：实现**：有链接时按钮可用，调用 `openUrl`；没有链接时保持禁用，提示「暂无官方指引链接」。
- [ ] **步骤 4：运行测试和类型检查**。

**阶段 3 结束：** 跑全量测试和类型检查，冒烟，对照画板截图；请鼠鼠决定是否提交。

---

## 阶段 4：依赖外部条件的功能

写代码时用合成数据测试；需要真实账号、真实安装或真实发布的部分，写进 `docs/todo/2026-10-22-unverified-integrations.md`（任务 4.7）。

### 任务 4.1：Skills 改为数据驱动，加上更换全局目录

**文件：**
- 修改：`backend/src/skills.rs`：`PLATFORMS`（34 行）、`Homes`（69-85 行）、`homes()`、`roots()`（91 行）、`static_platform`（87 行）、`set_enabled_impl`（647-703 行）、`validate_front`（352 行）、`ProjectInfo`（186-194 行）
- 修改：`frontend/src/components/skills/skillDialogs.tsx:424`、`frontend/src/lib/skillsManager.ts:6-16`、`frontend/src/lib/skillsApi.ts`
- 测试：`skills.rs` 的 `mod tests`

**接口：**
- 产出（Rust）：
  ```rust
  pub enum Toggle { Park, CodexToml, GeminiSettings, OpenCodePermission, None }
  pub struct ToolSpec {
      pub id: &'static str,
      pub global: &'static [&'static str],   // 相对用户目录，如 ".claude/skills"
      pub project: &'static [&'static str],  // 相对项目根，如 ".claude/skills"
      pub home_env: Option<(&'static str, &'static str)>, // (环境变量, 其下子目录)，如 ("CLAUDE_CONFIG_DIR","skills")
      pub toggle: Toggle,
      pub writable: bool,
      pub require_name: bool,
      pub verified: bool,
  }
  pub const SPECS: &[ToolSpec];
  pub fn spec(id: &str) -> Option<&'static ToolSpec>;
  ```
  - `Homes` 改为 `{ user: PathBuf, env: HashMap<String, PathBuf> }`，隔离环境的分支保留。
  - `Config` 增加 `global_override: HashMap<String, String>`，按工具 id 存储；`custom_dirs: Vec<String>`。
  - 命令 `skills_set_global_dir(platform: String, path: Option<String>) -> Result<Overview, String>`：传 None 表示恢复默认；路径必须存在并且是目录。
- 本任务只把 Claude Code 和 Codex 两项填进 `SPECS`，行为与现在完全一致；其他工具在任务 4.2 加。

- [ ] **步骤 1：写失败测试**：
  - `specs_preserve_claude_codex_roots`：用重构前后两套 `roots` 的结果比对同一组目录，结果完全相同（先把重构前的输出写成测试里的固定值）。
  - `global_override_replaces_default`：设置覆盖后 `roots` 用新路径；路径不存在时报「目录不存在」。
  - `codex_agents_dir_ignores_codex_home`：设了 `CODEX_HOME` 时，`~/.agents/skills` 仍在根目录列表里。
- [ ] **步骤 2：运行**，预期失败（`spec` 还不存在）。
- [ ] **步骤 3：重构**，跑 `skills` 的全部现有测试，预期全部通过。
- [ ] **步骤 4：前端**：「更换」按钮打开选择目录框；旁边显示「恢复默认」；说明文案见 D-18。
- [ ] **步骤 5：运行测试和类型检查**。

### 任务 4.2：其他工具的 Skills

**文件：**
- 修改：`backend/src/skills.rs`（`SPECS` 增加条目，新增 `Toggle::GeminiSettings` 和 `Toggle::OpenCodePermission` 的实现）
- 修改：`frontend/src/lib/skillsManager.ts`（`SKILL_PLATFORMS`、`PENDING_SKILL_PROVIDERS` 只留 grok）、`frontend/src/components/skills/skillParts.tsx:125-133`（「自定义：添加扫描目录」）
- 测试：`skills.rs` 的 `mod tests`

**接口：**
- 消耗：任务 4.1 的 `ToolSpec`、`SPECS`、`Config.custom_dirs`。
- 产出：`SPECS` 加入 gemini、opencode、qoder、trae、zcode、workbuddy，取值见第二节 D-19 的表。另有：
  - `fn toggle_gemini(settings_path: &Path, name: &str, enabled: bool) -> Result<(), String>`：读写 `skills.disabled` 数组，保留其他字段。
  - `fn toggle_opencode(config_path: &Path, name: &str, enabled: bool) -> Result<(), String>`：读写 `permission.skill.<名称>`；启用时删掉这个键，不写 `"allow"`。
  - 两个函数写入前都备份成 `<文件>.jolu-bak`；文件不存在时新建，只写最小结构。
- 画板 76 菜单上写的是「Wordbuddy」「Zcode」，代码沿用正确名称 WorkBuddy、ZCode，这一处差异记进 todo。

- [ ] **步骤 1：写失败测试**：
  - `gemini_toggle_roundtrip`：已有 `{"theme":"x"}` 时，停用 a 得到 `{"theme":"x","skills":{"disabled":["a"]}}`，再启用 a 后数组为空，`theme` 保留。
  - `opencode_toggle_roundtrip`：同理，断言 `permission.skill.a == "deny"`，启用后这个键消失。
  - `toggle_refuses_malformed_config`：文件内容是 `{bad json` 时报「配置文件格式异常，未修改」，文件内容不变，也没有生成备份。
  - `readonly_tool_rejects_write`：workbuddy 新建 Skill 时报「该工具目录未核实，暂不支持写入」。
  - `custom_dirs_scan_only`：自定义目录里的 Skill 出现在列表里，`set_enabled` 报「自定义目录不支持启停」。
- [ ] **步骤 2：运行**，预期失败。
- [ ] **步骤 3：实现**。
- [ ] **步骤 4：运行测试和类型检查**；在隔离环境的 `skills-home` 下为每个工具建一个示例 Skill，确认能列出。真实安装各工具后的验证写进 todo。

### 任务 4.3：ZCode 的 API Key

**文件：**
- 修改：`frontend/src/lib/providerCatalog.ts:29`（`apiKey: "ready"`，增加 `baseUrlChoices`）、`frontend/src/components/providers/ConnectDialog.tsx`（地址下拉）
- 修改：`backend/src/lib.rs:1504`（白名单加 zcode）、`backend/src/connections.rs`（zcode 用 Anthropic 兼容端点时用 `x-api-key`，复用任务 3.4 的 `protocol`）、`backend/src/quota.rs`（API 连接的额度识别复用 `coding_plan::query_zhipu`）
- 测试：`providerCatalog.test.mjs`、`connections.rs` 的测试模块

**接口：**
- 消耗：任务 3.4 的 `protocol` 字段和 `apply_auth`。
- 产出（TS）：`ProviderEntry` 增加 `baseUrlChoices?: { label: string; value: string; protocol: "openai" | "anthropic" }[]`，ZCode 的四个选项见 D-13，标签分别为「国内 · Anthropic 兼容」「国内 · OpenAI 兼容」「国际 · Anthropic 兼容」「国际 · OpenAI 兼容」。

- [ ] **步骤 1：写失败测试**：ZCode 条目有 4 个地址选项，默认是第一个；`showsModeTabs(zcode)` 为 true。
- [ ] **步骤 2：运行**，预期失败。
- [ ] **步骤 3：实现**；模型列表请求失败时界面改为「模型 ID *」加「验证模型」（画板 27 的状态，先提示「验证可能产生少量 API 用量」）。
- [ ] **步骤 4：运行测试和类型检查**。鼠鼠提供 ZCode Key 后实测获取模型、保存、额度显示；没有 Key 时写进 todo，包括「额度接口要不要加 Bearer 前缀」这个未决点。

### 任务 4.4：OpenCode 连接

**文件：**
- 修改：`backend/src/db.rs` 的 `init`：`connections` 加 `service TEXT`
- 新建：`backend/src/opencode.rs`（本机路径、`auth.json` 字段检测）
- 修改：`backend/src/connections.rs`、`backend/src/quota.rs:744`（OpenCode Go 额度识别已有）、`backend/src/lib.rs:1504`
- 修改：`frontend/src/lib/platformCatalog.json`（opencode 改为 available，按实现填 capabilities）、`backend/src/platforms.rs:58`（更新断言）、`frontend/src/lib/providerCatalog.ts`（OpenCode 的 `services`）、`frontend/src/components/providers/ConnectDialog.tsx`（模型服务 / 订阅服务下拉，按画板 19、21）
- 测试：`opencode.rs` 的 `mod tests`、`providerCatalog.test.mjs`

**接口：**
- 产出（Rust）：
  - `pub fn opencode_paths(user: &Path) -> OpenCodePaths`
    - `OpenCodePaths { config: PathBuf, auth: PathBuf, db: PathBuf }`，分别是 `~/.config/opencode/opencode.json`、`~/.local/share/opencode/auth.json`、`~/.local/share/opencode/opencode.db`
  - `pub fn auth_providers(auth_json: &str) -> Vec<(String, String)>`：返回 (provider id, type)，只读键名和 `type` 字段，**不返回、不记录任何凭证值**。
  - `NewConnection` 增加 `service: Option<String>`，取值 `"opencode-go"`、`"openai"`、`"openai-compatible"`。
- 产出（TS）：
  - `ProviderEntry` 增加 `services?: { id: string; name: string; mode: ConnectMode; baseUrl?: { value: string; locked: boolean } }[]`。
  - OpenCode 的三项：
    - `opencode-go`：套餐 Key，`https://opencode.ai/zen/go/v1`，锁定。执行时对照 `coding_plan.rs:10` 和官方文档核实。
    - `openai`：订阅。
    - `openai-compatible`：API Key，地址可编辑。

- [ ] **步骤 1：写失败测试**：
  - `auth_providers_never_returns_secrets`：输入 `{"openai":{"type":"oauth","access":"SECRET","refresh":"R","expires":1},"x":{"type":"api","key":"K"}}`，预期结果为 `[("openai","oauth"),("x","api")]`，并且结果的 Debug 字符串里不含 `SECRET`、`R`、`K`。
  - `auth_providers_tolerates_bad_json`：输入非法 JSON 时返回空列表。
  - TS：OpenCode 有 3 个服务；`opencode-go` 的地址锁定。
- [ ] **步骤 2：运行**，预期失败。
- [ ] **步骤 3：实现**连接流程：订阅服务 OpenAI 时，检测 `auth.json` 里是否有 `openai` 条目，有就显示「已在 OpenCode 中登录 OpenAI」。
- [ ] **步骤 4：运行测试和类型检查**。本机没有安装 OpenCode，真机验证写进 todo。

### 任务 4.5：OpenCode 用量采集

**文件：**
- 新建：`backend/src/opencode_usage.rs`
- 修改：采集入口：照 `native_sources.rs` 现有平台的注册方式接入
- 测试：`opencode_usage.rs` 的 `mod tests`

**接口：**
- 产出：
  - `pub enum OpenCodeLayout { JsonFiles, SqliteV1, SqliteV2 }`
  - `pub fn detect_layout(conn: &Connection) -> OpenCodeLayout`：按表名判断。有 `session_message` 判为 V2，有 `message` 判为 V1。
  - `pub fn read_usage(conn: &Connection, layout: OpenCodeLayout, since_ms: i64) -> Result<Vec<RequestRecord>, String>`：从 `tokens.{input,output,reasoning,cache.read,cache.write}` 映射到 fresh_input、output、cache_read、cache_write；`reasoning` 计入 output，并在注释里说明口径；`modelID` 取模型，`providerID` 写进来源备注。
  - 数据库用只读、不改动 WAL 的方式打开：`SQLITE_OPEN_READ_ONLY` 加 `immutable=0`。
- 来源均为第三方（ccusage#966 等），所以 V2 的表结构以本机实测为准；没有实测前，`platformCatalog.json` 里 OpenCode 的 limitation 写「用量采集依据社区资料，待真机核对」。

- [ ] **步骤 1：写失败测试**：用内存 SQLite 分别建 V1 表（`message` 表，`data` 列是含 tokens 的 JSON）和 V2 表，各插两条。
  - `detect_layout_v1_v2`：各自判断正确。
  - `read_usage_maps_tokens`：input=100、cache.read=20、output=30、reasoning=5 时，记录为 fresh_input 100、cache_read 20、output 35。
  - `read_usage_skips_missing_tokens`：tokens 字段缺失的消息不产生记录，也不当作 0。
- [ ] **步骤 2：运行**，预期失败。
- [ ] **步骤 3：实现并接入采集**。
- [ ] **步骤 4：运行测试**；真机验证写进 todo。

### 任务 4.6：落实 Qoder / WorkBuddy 的 API Key 决策（D-14）

**文件：**
- 修改：`frontend/src/lib/providerCatalog.ts:30-31`、`providerCatalog.test.mjs`
- 修改：`docs/ui/Jolu.pen` 画板 29 中这两行的文案（鼠鼠已确认 D-14），改为与 D-14 一致的说明

- [ ] **步骤 1：用 Pencil 改画板 29 的两行文案**，截图对照。
- [ ] **步骤 2：写测试**：`singleModeReason(providerEntry("qoder"))` 等于 D-14 的文案；WorkBuddy 同理。
- [ ] **步骤 3：实现**：`apiKey` 改为 `"none"`，`singleModeReason` 对这两家返回专门的说明。
- [ ] **步骤 4：运行测试**。

### 任务 4.7：检查更新本地端到端，以及未验证项清单

**文件：**
- 修改：`backend/src/updater.rs:202`、`frontend/src/lib/updateState.ts`（`UPDATES_ENABLED` 改为构建时开关，见 D-29）
- 新建：`scripts/release/local-update-e2e.md`（本地端到端步骤）
- 新建：`docs/todo/2026-10-22-unverified-integrations.md`
- 修改：`AGENTS.md` 第 2 节「检查更新目前停用」一段

**接口：**
- 后端：`const UPDATES_ENABLED: bool = matches!(option_env!("JOLU_UPDATES_ENABLED"), Some("1"));`
- 前端：`export const UPDATES_ENABLED = import.meta.env.VITE_JOLU_UPDATES_ENABLED === "1"`

- [ ] **步骤 1：改开关**，默认构建仍然停用；跑 `cargo test` 和类型检查。
- [ ] **步骤 2：本地端到端**（Context7 核对 `UpdaterBuilder` 能否覆盖 endpoints 和 pubkey；不能就用 `--config` 覆盖）：
  1. `pnpm tauri signer generate` 生成一对测试密钥，放在 `output/update-e2e/`，这个目录已被 Git 忽略。
  2. 设 `JOLU_UPDATES_ENABLED=1`、`VITE_JOLU_UPDATES_ENABLED=1`，用 `--config` 覆盖 pubkey、`endpoints=["http://127.0.0.1:18766/latest.json"]` 和 `dangerousInsecureTransportProtocol: true`，构建 0.1.0 并安装。
  3. 把版本号改成 0.1.1 再构建，用 `.sig` 的内容写 `latest.json`，用本地静态服务器提供文件。
  4. 在 0.1.0 里点「检查更新」，依次下载、安装、重启，预期版本变为 0.1.1。
  5. 篡改 `.sig` 再测一次，预期停止安装并保留 0.1.0。
  6. 测完把 `tauri.conf.json` 和版本号恢复原样。
- [ ] **步骤 3：写 todo 文档**，列出所有未能实测的项，每项写明缺什么条件、怎么验证：
  - 正式 Release 和签名私钥；改开关的时机
  - 额度真实耗尽后续跑
  - ZCode 模型列表接口，以及额度接口的鉴权写法
  - OpenCode 连接和用量（三种数据格式）
  - Gemini、Qoder、Trae、ZCode、WorkBuddy 安装后的 Skills 目录核对
  - WorkBuddy 的 Skills 目录来源冲突（`.workbuddy` 和 `.codebuddy` 两种说法）
  - 安装版的系统通知外观
  - Windows Sandbox 安装检查（如果任务 1.3 没做成）
  - 画板 76 上「Wordbuddy / Zcode」的拼写
- [ ] **步骤 4：更新 AGENTS.md** 的检查更新说明，改为「首个带 latest.json 的 Release 发布时，用 `JOLU_UPDATES_ENABLED=1` / `VITE_JOLU_UPDATES_ENABLED=1` 构建」。

---

## 阶段 5：收尾

### 任务 5.1：全量验证、文档与安装包

**文件：**
- 修改：`AGENTS.md`（第 1 节表格的「现有实现」列：模块 B、C、D、E、F、G 去掉已完成的「待接入」；D 行删掉「创建自动化目前只跳转到自动化页」，这句已经过时）
- 修改：`docs/plans/2026-10-21-jolu-development-roadmap.md`（文首加一行「已由 2026-10-22-jolu-remaining-work.md 取代」）
- 修改：`README.md`、`README.en.md`（新能力：会话交接、自定义供应商、OpenCode 等；用词按实际完成情况写）

- [ ] **步骤 1：全量测试**：`pnpm --dir frontend typecheck`、`node --test frontend/src/lib/*.test.mjs`、`node --test scripts/release/*.test.mjs`、`cargo test --manifest-path backend/Cargo.toml`，预期全部通过，并且没有编译警告。
- [ ] **步骤 2：冒烟**：在隔离环境运行 `main-window-smoke.e2e.mjs`。
- [ ] **步骤 3：打包**：运行 `pnpm tauri build --config '{"bundle":{"createUpdaterArtifacts":false}}'`，安装后检查通知外观、缩放、默认打开页面和联网方式。
- [ ] **步骤 4：更新文档**，然后向鼠鼠汇报完成情况和未验证项，请鼠鼠决定是否提交。
