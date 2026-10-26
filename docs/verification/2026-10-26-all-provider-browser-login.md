# 全供应商浏览器登录与六版本目录

日期：2026-10-26。鼠鼠确认范围为全部现有订阅供应商，并要求六个国内/国际版本分别显示。此记录区分已实现、合成验证、真实初始化与完整真实登录。

## 改动总结

- 目录共 13 张卡片：自定义供应商，以及 Claude Code、Codex、Gemini、Grok、OpenCode、ZCode、Qoder CN、Qoder、WorkBuddy、WorkBuddy AI、Trae CN、Trae。12 个订阅入口均接入浏览器登录；自定义供应商保持用户配置的 API Key 方式，没有为任意地址伪造统一 OAuth。
- 新增展示 ProviderId；保留平台族 PlatformId、连接 ID、统计与历史。六版本的卡片计数、账号筛选、同名检查、弹窗选择与已保存账号标题分别处理。
- 后端 DTO 新增非敏感 edition；本机连接从固定 source ID 得出，旧浏览器连接仅在后端解封自身 DPAPI 授权取版本。不删账号、不重置状态、不改表或统计关联；未知区域仍保留为未知。
- 修复 ProvidersView 保存本地账号的 wrapper 丢弃 edition；Trae 的本地检测与登记也严格限定所选版本。探测与网络工作均在数据库锁外。
- 授权生命周期统一处理准备、默认浏览器、设备码提示、轮询、账号与真实额度验证、取消、超时和加密保存。取消清除未保存凭证，迟到结果不继续查询或保存。
- CLI 登录使用独立临时配置；仅运行认证，不发送模型请求。保留系统浏览器的用户配置；取消只结束直接认证进程，不关闭浏览器。子进程网络跟随 Jolu 的直连、系统或手动代理设置，loopback 不走代理。
- 独立浏览器账号查询、恢复与获取均用自己的封存凭证。Claude/Codex 浏览器账号不进入原工具补邮箱或写配置路径。
- 已验证但名称为空时显示明确的保存提示。

## 登录实现与来源

| 供应商 | 实现与来源 | 限制 |
|---|---|---|
| WorkBuddy / WorkBuddy AI | 既有官方 state → token → account 轮询，系统默认浏览器打开官方 authUrl | 会话到期需重新授权 |
| Qoder CN / Qoder | 既有官方设备 PKCE、nonce 和 userinfo 核验 | 会话到期需重新授权 |
| Trae CN / Trae | 官方安装包及当前官网的 PKCE、固定 loopback `/authorize`、强制 `loginTraceID`、ExchangeToken、GetUserInfo | 不接受回调直接提供的令牌；不申请刷新设备权限 |
| Grok | xai-org/grok-build 官方公共客户端的 RFC 8628 设备流及 userinfo | 请求身份和计费访问 scope，不请求会话或工作区写权限 |
| ZCode | zai-org/zcode 与本机官方 CLI 的 BigModel 设备轮询 broker | 32 字节随机 poll token 转 64 位 hex；不用 zcode:// 抢占原工具 |
| Claude Code | 官方 `claude auth login --claudeai`，独立 CLAUDE_CONFIG_DIR | 需要已安装官方 Claude Code |
| Codex | 官方原生 `codex login`，独立 CODEX_HOME，文件授权存储 | 需要已安装官方 Codex CLI |
| OpenCode | 独立 OpenAI 订阅授权复用 Codex 官方登录 | 先选择 OpenAI 订阅；不写入原 OpenCode 配置 |
| Gemini | 已核实的官方 CLI 导出 getOauthClient，独立 GEMINI_CLI_HOME；只调用 OAuth 函数 | 需要受支持的官方 Gemini CLI；账号若未完成 Code Assist 初始化会明确失败，不伪报额度 |

涉及 SDK / CLI / 认证协议先查询 Context7，再核对当前官方源代码或本机官方安装包。Grok 官方源：`xai-org/grok-build` 的 `xai-grok-login/config.rs`、`device_code.rs`；ZCode 官方源：`zai-org/zcode` 的 `oauthService.ts`、`bigmodelProviderConfig.ts`。

Trae 当前双版官网回调为顶层 GET 导航；官网和官方安装包均没有要求此新 PKCE 流补 PNA 响应头。已按公开代码验证固定路径与 loginTraceID，不为假设风险放宽来源或状态校验。

## 验证结果

- Rust 全量：576 通过，0 失败，23 忽略。随后 CLI 联网调整的 6 项定向测试通过；忽略的真实授权探测单独显式执行。
- 前端纯逻辑：207 通过；TypeScript 类型检查与生产构建通过。
- Node 认证 helper：4 项通过，覆盖 Gemini 导出/路径与 WorkBuddy 加密凭证；最后 Gemini helper 定向 2 项再次通过。
- 真实 React 连接弹窗 + 合成桥接：12 个订阅入口、6 个版本的 family/edition、授权等待不可保存、授权通过后需名称、独立授权保存、本地版本传递、卸载迟到取消，全部通过。
- 真实供应商页预览：13 张卡片与六版本名称已检查；浅深色截图在 `output/acceptance/provider-auth/all-provider-cards-*.png`。Pencil MCP 仍提示设计文件未打开，未完成 Jolu.pen 对照，未修改设计文件。
- 真实 Grok 设备授权初始化成功，返回 accounts.x.ai；最小 scope 请求成功。ZCode 官方初始化 HTTP 200 / code 0，返回 bigmodel.cn。两者的 Rust 初始化探测通过，未打开浏览器或输出 device_code、state、token。
- Trae CN 的真实 Pending 初始化及临时 CNG 公钥/loopback 创建通过；这不等于官网登录、换令牌或额度已通过。
- 之前 WorkBuddy 与 Qoder CN 已由鼠鼠在默认浏览器完成授权、保存，并核实保存后真实同步；本轮不将其结论扩展到其他账号或国际版本。

## 人工登录反馈与剩余复核

- 鼠鼠反馈 WorkBuddy AI、Trae CN、Trae、Qoder 四个版本均已保存；其他已有账号的平台也已保存。记录为鼠鼠的人工操作确认，不扩大到没有账号的平台。
- 随后只读核对正常用户数据库的非敏感元数据：Qoder 2 条、WorkBuddy 2 条、Trae 1 条浏览器连接，均 connected 且已有同步时间；另有原 Codex 订阅连接和 Claude API 连接。自动复核的范围以这些记录为准，不据此推定其他平台或两个 Trae 版本都已覆盖。
- 平台/版本逐项明细、没有有效账号的平台、整应用重启、企业和特殊套餐样本继续单列；不能因组件测试或初始化成功宣称未核实的完整链路成功。

没有执行 Git 写操作；未读取或改写原工具的登录来替代新浏览器账号，未发送模型请求。

## 账号卡片精简：移除订阅状态

鼠鼠要求评估后移除统一的订阅状态行。检查发现多数来源没有可靠状态，原显示还将授权过期混入其中；ZCode 的真实开通状态仍是有效后端数据，因此仅移除统一展示及已无调用的状态格式化分支，保留接口字段与来源解析。

全部卡片保留等级/套餐、到期时间、额度、重置卡与原有认证失败提示。12 项 accountDisplay 测试、类型检查与真实账号组件合成验证通过；新增断言确保全部卡片无“订阅状态”，且授权失效仍明确提示。实际浅深色截图位于 output/playwright/account-sources-*.png；Pencil 仍无法读取未打开的设计文件，未修改 .pen。
