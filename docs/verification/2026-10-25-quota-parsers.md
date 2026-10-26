# 额度解析审查与定向验证

日期：2026-10-25。范围为 `backend/src/workbuddy_quota.rs`、`gemini_quota.rs`、`gateway_balance.rs` 的现有未提交实现，以及真实联调后 `coding_plan.rs` 的业务错误分类与脱敏。解析修复阶段未运行真实账号查询；后续授权的只读 smoke 结果单列如下。未修改真实登录或配置，未进行 Git 写操作。

## 改动总结

### WorkBuddy：完整资源包集合才能汇总

根因：只有总量时把缺失的已用量默认成零；负数用 `max(0)` 变为零；无法解析的资源包被跳过，造成部分合计看起来像完整余额。

修复：总量、剩余、已用至少有两项有效，才允许推导第三项。优先精确字段存在但无效时直接失败，不用低优先级字段掩盖错误。拒绝负数、超界值、三项不一致、汇总溢出，以及无法解析的非空周期结束时间。任何有效期内资源包缺数据都会让整项查询失败。真实零、明确过期的资源包、空列表仍分别保留原语义。重置时间加一秒采用检查过的日期运算，避免极端日期溢出。

未改认证路径、域名白名单、加密凭证处理、资源包接口或其到期时间解释。

### Gemini：缺省与损坏响应分开

根因：任意 JSON 和错误类型的 `buckets` 都会因为 Option 迭代得到空列表，误报查询成功。

修复：顶层必须是对象；显式 `buckets` 必须是对象数组。官方类型允许省略 `buckets`，因此 `{}` 仍保留为空窗口，不编造零额度。现有越界比例保留未知、剩余次数真实零的处理不变。

### 网关：真实换算比例、账号上限与失败状态

根因：new-api 固定以 500000 换算美元，忽略站点 `quota_per_unit` 与 `user_quota`；负余额被夹成零；业务 `code=false` 一律误认登录失效并原样展示上游 message；HTTP 500 和网络失败被吞成“不匹配网关”。

修复：

- 从公开 `/api/status` 取得每美元额度，公开请求不附带 API Key。比例缺失、非正、非有限或换算溢出时明确查询失败，不使用默认猜测值。
- 有账号额度时取令牌额度和账号额度的较小值；不限额令牌仍能使用明确的账号额度，无两者上限则不支持余额。
- 保留源负余额；拒绝负已用量。one-api 余额小于零时保留负值。
- 已识别的 new-api 成功响应若字段不完整，返回失败，避免继续套用另一种网关协议得到错误解释。
- `code=false` 返回固定的查询失败信息，不透传上游自由文本。HTTP 鉴权、权限、限流继续分别表达；服务器和网络错误保留查询失败，404 或无法识别的非 JSON 才允许尝试其他来源。

## 官方依据

按照 Context7 顺序先 `library` 确定库，再 `docs` 查询具体主题：

- `/google-gemini/gemini-cli`：[types.ts](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/code_assist/types.ts) 中 `RetrieveUserQuotaResponse.buckets?: BucketInfo[]`，支持“缺省不等于损坏”的判断。
- `/quantumnous/new-api`：[channel-billing.go](https://github.com/quantumnous/new-api/blob/main/controller/channel-billing.go) 的 `updateChannelNewAPIBalance` 使用 `min(token,user_quota)` 和 `/api/status.data.quota_per_unit`。
- `/quantumnous/new-api`：[token.go](https://github.com/quantumnous/new-api/blob/main/controller/token.go) 的 `GetTokenUsage` 返回原始额度，未声明固定换算常数。

WorkBuddy 原实现基于第三方源码，官方接口未公开，本轮只修正通用数值逻辑，没有把第三方描述当作认证扩展依据。

## 测试结果

解析修复阶段的最终定向命令：

```powershell
cargo test --manifest-path backend/Cargo.toml --lib -- workbuddy_quota::tests gemini_quota::tests gateway_balance::tests
```

当时结果：**18 passed / 0 failed / 2 ignored**。当时未执行 Gemini、WorkBuddy 的两个真实本机登录 smoke。其余测试使用固定 JSON、临时测试目录以及一次仅绑定 `127.0.0.1` 的 HTTP 响应；后续新增 ignored smoke 不包含在该历史计数内。

修复前已实际观察到以下测试失败，修复后均通过：

- `summary_rejects_missing_invalid_and_partial_capacities`
- `buckets_reject_malformed_response_structure`
- `gateway_does_not_turn_negative_or_invalid_values_into_zero`
- `new_api_business_error_does_not_expose_upstream_message_or_assume_auth_failure`
- `new_api_uses_site_conversion_and_account_cap`
- `new_api_requires_valid_conversion_and_complete_balance`
- `server_failure_is_not_a_missing_gateway_and_public_status_has_no_credentials`

最后一个本机 HTTP 测试还验证 `/api/status` 请求不携带 Authorization。未运行无关全量测试。

## 授权后的真实只读查询

同轮主代理回传：WorkBuddy `ok`，返回 1 个额度窗口；Gemini、Qoder 均为 `unauthorized`。这些是查询状态，不据此推断具体订阅到期日期或所有版本兼容性。

Claude 的三项 smoke 结果分别记录，未修改旧断言，也未把套餐 Key 强行当作钱包 Key：

| 测试 | 执行结果 | 可确认结论 |
|---|---|---|
| `quota::official_connection_tests::local_claude_gateway_read_only_smoke` | 主代理执行失败，`HTTP/parse status 404` | 旧测试只直接查询 sub2api；当前配置不满足“sub2api 网关”前提。此结果不能证明产品余额回退链路有缺陷。 |
| `gateway_balance::tests::local_claude_gateway_balance_read_only_smoke` | 主代理执行失败，`state=unsupported currency=unknown balance_present=false used_present=false`；在“配置属于编程套餐”前提检查处退出 | 产品根据已配置地址识别为编程套餐，因此不走 sub2api / new-api / one-api 钱包回退；该余额测试同样不适用。 |
| `gateway_balance::tests::local_claude_coding_plan_read_only_smoke`（分类修复前） | 本代理实际执行失败（退出码 101），`state=failed windows=0 resets_present=false` | 套餐路由的适用条件成立，但真实套餐查询未返回额度窗口；不能计入已验证成功。后续已继续定位，见下文。 |
| `coding_plan::tests::local_zhipu_response_classification` | HTTP 200，数字业务码 500，`success=false`，无 data；完整固定文本白名单匹配后仅输出 `message_class=no_coding_plan` | 官方服务明确表示当前配置凭证对应账号没有关联 Coding Plan；不是网络异常、HTTP 500 或钱包回退缺陷。 |
| `gateway_balance::tests::local_claude_coding_plan_read_only_smoke`（分类修复后） | 本代理实际执行仍失败（退出码 101），`state=unsupported windows=0 resets_present=false` | 产品已正确表达当前凭证不具备套餐额度，严格成功断言仍保留，因此 smoke 不被伪装成成功。 |

套餐 smoke 执行命令：

```powershell
cargo test --manifest-path backend/Cargo.toml --lib gateway_balance::tests::local_claude_coding_plan_read_only_smoke -- --ignored --nocapture
```

该测试按 `detect_provider` → `coding_plan_quota` 调用产品套餐路由，只发只读 GET；首次输出仅包含状态、窗口数、重置字段是否存在。后续授权诊断增加了固定供应商类别、HTTP 状态、数字业务码和白名单错误分类，没有输出 Key、地址、百分比或原始响应。保留“状态必须为 ok 且窗口非空”的断言，未为了通过测试放宽判定。

诊断先按 Context7 查询国内官方库和 GLM Coding Plan 库，未获得该监控接口的完整错误码文档；随后核对 [zai-org 官方查询脚本](https://github.com/zai-org/zai-coding-plugins/blob/main/plugins/glm-plan-usage/skills/usage-query-skill/scripts/query-usage.mjs)，确认产品的 GET 路径、裸 Authorization 头和请求头与官方脚本一致。没有换 Key、跨供应商试探或发送模型请求。

### 真实联调后的生产修复与验证

- 只对智谱 `success=false`、业务码 500、完整固定无套餐文本白名单同时命中的响应，返回 `Unsupported` 和固定文案“当前凭证未关联 Coding Plan，无法查询套餐额度”。不是将所有业务码 500 都归为无套餐。
- 智谱、MiniMax、ZenMux 其余业务错误保持 `Failed`，只返回固定文案和数字业务码，不再透传上游 `msg` / `status_msg` / `message`。非数字码不输出，避免上游回显凭证。
- MiniMax、ZenMux 的既有 JSON 解析提取为独立函数，合成测试直接覆盖真实生产分支。
- 新增 `verified_no_plan_is_unsupported_but_other_business_failures_stay_failed`、`business_error_messages_never_echo_credentials_and_only_keep_numeric_codes`。先得到 **19 passed / 2 failed**，修复后运行 `cargo test --manifest-path backend/Cargo.toml --lib coding_plan::tests:: -- --skip local_` 得到 **21 passed / 0 failed**。
- 最后再运行真实套餐 smoke，确认产品返回 `unsupported`，其严格成功断言仍失败并记录。没有重复主代理已完成的后端全套测试。

上述结果仅说明当前配置凭证的套餐关联状态，不能推断用户其他账号是否已购买套餐，也不构成全供应商真实查询成功的结论。

## 未验证边界

- WorkBuddy 当前账号已返回 1 个窗口；各版本资源包字段与到期/重置语义仍需进一步验证，未宣称所有版本均已通过。
- Gemini 本轮真实查询为 `unauthorized`。解析修复未扩展其认证刷新流程，不能把离线解析测试通过等同于真实账号查询成功。
- 当前 Claude 凭证被官方服务明确判定未关联 Coding Plan，产品现返回 `unsupported`；两项钱包 smoke 的适用条件不成立，不能用于判断真实网关支持情况。要验证真实额度成功，需有对应服务已关联套餐的凭证；本轮没有更换凭证。
- new-api 的不同部署版本可能不返回 `user_quota` 或不公开 `quota_per_unit`；缺换算比例会明确失败。one-api dashboard 的部署兼容性仍待真机验证。
- 额度周期结束时间不是订阅到期时间。本轮不把周期结束或 OAuth 到期日期冒充订阅到期日期，也未新增缺乏来源的订阅到期字段。
- 未修改界面或设计稿，无新增 UI 状态；前端对失败、未知与负余额的展示由总验收继续核对。
