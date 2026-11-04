# 订阅账号绑定与 API 调用

## 本轮范围

鼠鼠确认先完成 Claude / Codex，其他订阅逐个平台验证。不构建安装包。真实账号验收仍另做，不以合成测试替代真实可用性结论。

## 已实现

- 选中来源后按连接 ID 读取完整、已加密保存的授权。缺失或身份不匹配时明确失败，绝不退回本机当前登录账号。
- Codex：使用所选账号请求上游；刷新同账号串行执行，以旧凭据作比较条件，事务更新该连接的授权快照、掩码和指纹。不读写原工具 auth.json。
- Claude：每次在一次性配置中写入所选账号授权；进程不继承 API Key、OAuth、端点覆盖等环境，禁用内置工具与外部 MCP，结束后清理配置。完整对话逐次传入，不恢复原工具会话；隔离配置中的授权轮换只回写原连接。
- 隔离请求不再依赖原工具日志扫描：保存不含对话正文的请求统计，绑定所选账号；用量未知与真实零分别保留。CLI 生成失败时也保全已轮换的授权。
- 修复 Claude 默认命令遗漏可执行文件、缺少 verbose、错误输出误报成功；system 作为系统提示词参数，max_tokens 传入 CLI 输出限制，真实 usage 存在时返回，否则不编造。
- 修复 Codex 模型名没有去掉来源前缀；Chat 流式转换为 Chat chunks，Responses 保持原生 SSE；解析支持 event 行、CRLF、跨块 UTF-8。Chat 用量字段转换为 prompt_tokens / completion_tokens。
- 来源卡片从后端读取能力；缺少完整授权提示重新登录，所有官方订阅可开启来源选择，未适配的平台显示“已选择 · 转发适配尚未就绪”，不表示实际调用通过。已经开启的来源即使授权失效也可关闭。模型池提供模型名复制。

## 使用与边界

1. 在账号管理中保存完整授权；旧连接若只保存了单个 token，需重新登录并保存。
2. 开启来源与模型转发。地址和 Key 使用页面第二行的复制按钮；模型名使用模型池中的 `<来源>/<模型>`。
3. Codex：OpenAI 客户端 base URL 可填 `http://127.0.0.1:<端口>/v1`；支持 Chat 文本与 Responses。Chat 工具和图片不做隐式转换，明确报错，需使用 Responses。
4. Claude：Anthropic Messages，base URL 填 `http://127.0.0.1:<端口>`，`stream=false`，仅文本。流式、图片、工具及未支持的生成选项明确报错。此路径是 CLI 文本桥接，不是完整 Anthropic API 的等价实现；历史消息按角色转成文本上下文，CLI 可能依模型上限调整输出长度。
5. 订阅模型菜单仍含预置候选，不能视为该账号模型权限的实测结果。真实模型权限、额度和调用结果须在真实验收中确认。
6. Claude 需要安装官方 Claude Code。完整授权保护沿用 Windows DPAPI；本轮验证环境为 Windows。
7. 从原工具同步得到的授权可能与原工具共用 refresh token；官方轮换规则及原工具并发刷新影响仍需真实验收。优先使用 Jolu 内独立浏览器登录保存的授权。

## 其他订阅：逐平台核查

| 平台 | 本轮证据 | 尚需验证；当前状态 |
| --- | --- | --- |
| Gemini | 官方 CLI 有非交互 JSON 输出与缓存 OAuth 复用，见 [headless](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/headless.md)、[authentication](https://github.com/google-gemini/gemini-cli/blob/main/docs/get-started/authentication.mdx) | Jolu 保存的授权是否完整、账号/项目隔离、刷新、模型权限与错误映射；尚未开放订阅转发 |
| Qoder | 官方 [脚本模式](https://docs.qoder.com/cli/run-in-scripts)支持 print、JSON/stream-json，文档有 Personal Access Token 的脚本认证方式 | 当前保存的网页登录凭据不能直接当作 PAT；国内/国际版分别验证授权、非交互执行与隔离；尚未开放 |
| WorkBuddy | 本轮查到的官方[模型配置](https://www.codebuddy.cn/docs/workbuddy/From-Beginner-to-Expert-Guide/Function-Description/Model)介绍外部套餐 Key 的接入，不能证明 WorkBuddy 登录授权可反向调用模型 | 需进一步确认 WorkBuddy 自身的生成通道、账号身份与国内/国际版差异；不能把签到/额度凭据当成推理凭据；尚未开放 |
| Trae | 官方开源 [Trae Agent](https://github.com/bytedance/trae-agent/blob/main/README.md)配置的是外部模型 API Key，不能据此认定可以消耗 Trae IDE 订阅 | IDE 订阅生成通道、模型清单和授权隔离尚需核实；国内/国际版分开；尚未开放 |
| ZCode | 官方源码 [provider-runtime-headers.ts](https://github.com/zai-org/zcode/blob/main/apps/zcode-cli/packages/bootstrap/src/zcode-protocol/provider-runtime-headers.ts)描述每次请求向 Host 获取运行时授权 | 需验证 Host 的短期凭据交换、账号作用域、当前已保存登录及个人订阅的对应关系；不直接把网页登录 token 当模型 Key；尚未开放 |

以上为 Context7 官方文档/源码核查与本项目代码核查，不代表真实生成验收。未执行第三方代理项目脚本。下一批优先验证 Gemini、Qoder 的隔离非交互通道。

## 验证记录

- 先运行失败测试，确认 Claude 程序名错误、Codex 非文本内容被静默变成空串；随后修复。
- 合成 A/B 账号：选中账号加载、缺失快照拒绝、跨账号刷新拒绝、并发旧凭据不覆盖新登录、另一个账号不变。
- 假 Claude CLI：两个账号分离配置、敏感环境不继承、完成后目录清理、返回真实提供的 usage。
- 本地模拟 Codex：同账号并发刷新只请求一次；选中账号的 Bearer/account-id 与去前缀模型名；SSE 字节级分块与错误终止。
- 实际前端组件 + 合成响应：能力状态、缺失授权和未支持平台仍可选择、单独显示调用状态、按连接 ID 开启、模型名复制；截图 `output/acceptance/subscription-api/`。
- 后端完整测试 789 通过、36 按条件跳过；后续 Claude 失败路径修复后，最终网关回归 109 项全部通过。前端类型检查、7 项网关纯逻辑测试及真实组件合成验收通过。
- 后端完整测试日志：`output/acceptance/subscription-backend-tests.txt`；最终网关回归日志：`output/acceptance/subscription-gateway-tests.txt`。最终计数以日志为准。
- Pencil MCP 连接失败，尚未完成画板对照，未读取或修改 `.pen` 文件。
- 尚未进行真实账号生成、官方 Claude 二进制实际加载隔离凭据、真实刷新或真实额度归账验收；不将这些标为通过。


## 续轮：所有订阅开关与账号隔离

### 已实施

- 同时移除前端禁用条件、Tauri 能力开关拦截、connections 的仅 Claude/Codex 白名单。所有已知官方订阅可以选择开启/关闭；缺少授权或尚未适配不会显示为调用成功。
- 撤销同平台订阅只能开启一个的旧约束，改为按账号身份去重。浏览器账号指纹包含平台、地区、账号和企业/组织身份，不再用随机密文作为稳定身份。迁移保留所有账号及历史，重复来源仅关闭服务并留迁移记录。
- Claude/Codex 同时支持本机同步快照及浏览器 DPAPI 密文。刷新保持原加密存储形式，以旧密文作比较条件；不覆盖新登录。Claude 新的浏览器登录保存经官方身份端点核实的完整账号/组织信息；旧的不完整授权需要重新登录。
- OpenCode 通过 OpenAI 登录保存的账号复用 Codex 转发路径，但使用自己的连接 ID；不读取当前 Codex 登录。
- Grok：官方 cli-chat-proxy 的 Chat/Responses 转发；刷新后核实 userinfo.sub，仅写回选中连接。
- Gemini：官方 Code Assist 非流式 generateContent；按选中账号读取项目及模型额度，刷新核对邮箱。当前不支持订阅流式及 Chat/Responses 转换；尚未开通项目时明确要求先完成官方开通。
- WorkBuddy：官方客户端 ModelProvider 的 /v2/chat/completions；账号、企业头和地区来自本连接。后续已移除目录中的 default/default-model 固定值，按官方云端及企业配置查询可用模型，见 [目录修复记录](2026-11-04-workbuddy-model-catalog.md)。过期授权要求重新登录，不借用另一账号。
- ZCode：BigModel 个人项目 → 项目 Key → 短期调用 token → Anthropic Messages。排除团队项目，多个个人项目无法唯一识别时拒绝猜测；缺少官方 zcode-api-key 时遵循官方流程创建。短期 token 只在内存持有，每次调用按连接取用。浏览模型菜单也会取得调用凭据；生产操作尚未执行。

### 前阶段未完成项（Qoder / Trae 后续进展见本页末尾）

- Qoder 双版：官方 SDK 的 accessToken 是 PAT；当前保存的是桌面 OAuth。已检查本机官方 CLI，模型请求依赖其 WASM 签名上下文及加密凭据存储。尚未完成选中桌面账号到隔离 CLI 的可靠桥接，不能把 token 直接当 PAT。
- Trae 双版：现有登录/额度协议不等于模型调用协议。官方安装包的 AI 模块尚未完成请求、会话及流式适配，实际转发未实现。
- 因此“全部订阅转 API”尚未完成；Qoder/Trae 开关可选，但实际请求明确返回未接入。不能把开关已打开或合成上游成功等同于官方模型权限验证。
- Grok、WorkBuddy、ZCode 复用直通响应路径；直接用量入库仍需补齐，不将缺失用量伪装为零。

### 主要协议依据

- Grok 官方 [endpoint 配置](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-config/src/endpoints.rs)、[生产地址](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-env/src/lib.rs)。
- Gemini 官方 [server](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/code_assist/server.ts)、[converter](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/code_assist/converter.ts)。
- ZCode 官方 [项目短期令牌](https://github.com/zai-org/zcode/blob/main/packages/shared/src/project-access-token-client.ts)、[账号调用凭据](https://github.com/zai-org/zcode/blob/main/packages/services/src/model-provider/accountProviderRequestAuthService.ts)。
- WorkBuddy 官方本机安装包 resources/app.asar.unpacked/cli/product.json 与 dist/codebuddy-lite-wb.mjs；国内 5.8.3，国际 5.5.2。只读程序代码，没有读取原工具账号文件。
- 公开源码/文档副本位于忽略目录 output/subscription-research；未执行下载的第三方代码。

### 隔离验证

- 覆盖所有订阅的连接 ID 绑定、跨账号/跨地区拒绝、迟到刷新拒绝。
- 本地模拟 Grok token/userinfo、Gemini 项目/生成/归账、WorkBuddy 账号/企业头、ZCode 个人项目换证；不调用真实端点。
- 真实前端组件验证 Qoder 开关从关到开，显示尚未就绪；其他来源选择及模型名复制通过。
- Pencil MCP 仍无法连接 VS Code，因此未完成画板对照；未读取或修改 .pen。
- 未构建安装包，未提交/推送，未进行真实账号验收。最终测试数见后续日志记录。

最终回归：后端 796 通过、36 跳过，日志 output/acceptance/subscription-backend-tests-final.txt；前端 typecheck、7 项 gatewayPage 测试和实际组件合成验收通过。一次并发全量运行中原有 handoff_facts 文本脱敏性能测试耗时 1.61 秒超过 1 秒阈值，未修改该测试或实现，最终全量重跑通过。

## Qoder / Trae 实际转发续轮

- Qoder 已接入所选桌面 OAuth → 官方 jobToken → 独立临时目录中的官方 CLI → 非流式 Chat 响应；不把 OAuth 冒充 PAT。仅公开 `来源/auto`，要求安装官方 Qoder CLI 和 Node.js。详见 [协议与隔离记录](2026-11-04-qoder-subscription-adapter.md)。
- Trae 已接入官方随包 SDK 中的 `/api/ide/v1/chat_completion`：按地区选择模型域名，实时校验所选授权身份，以官方 JWT 头发出请求。仅接受已核实形状的非流式 Chat 响应；不推测私有流式包络。真实订阅是否获准使用该 SDK 端点仍待验收。
- Trae 模型目录响应尚未核实：不编造模型清单。调用方显式提供 `来源/模型ID`；本地模型池和 `/v1/models` 仅展示用户已保存的模型。Qoder 目录使用官方 `auto`，不表示账号拥有所有模型权限。
- 两平台已进入实际请求分发和按连接归账，不再返回通用“适配器尚未接入”。缺少完整授权、过期、身份不符、重新登录竞态或未知响应会明确失败，不借用其他账号。
- 修复共享路由旧缓存：每次发送前复查来源开关、身份指纹、状态及 API 凭据/地址；配置变化拒绝旧快照并使缓存失效。Chat/Messages/Responses 与 Gemini 原生入口共用检查。
- 独立审查发现并补齐排队窗口：Qoder / Trae 取得订阅账号锁后、实际模型请求前再次复查来源。新增两个测试先确认旧代码在关闭来源后仍发授权请求 / 返回模型结果（`output/acceptance/qoder-trae-queued-red.txt`），再验证关闭后的排队请求返回 503 且不访问上游；独立定点复核确认修复覆盖问题。
- 本轮只使用合成账号、回环 HTTP 和假 CLI 做隔离验证。没有真实账号验收、安装包构建或 Git 写操作。
- 验证：先复现来源关闭后仍访问上游的失败测试（`output/acceptance/source-disable-red.txt`）；修复后首轮网关 128 项通过（`qoder-trae-gateway-tests.txt`）。补齐 Trae 网络策略、模型目录和两个排队隔离测试后，最终 `cargo test --manifest-path backend/Cargo.toml --lib` 为 **812 通过、36 忽略、0 失败**，日志 `output/acceptance/qoder-trae-backend-final.txt`。中间一次原有会话脱敏性能测试耗时 1.076 秒超过 1 秒阈值，未改其代码；保留 `qoder-trae-backend-timing-failure.txt`，随后完整复跑通过。`git diff --check` 通过。本轮未改前端组件，不重复前阶段的前端验收。
