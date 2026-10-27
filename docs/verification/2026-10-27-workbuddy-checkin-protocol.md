# WorkBuddy 每日签到协议核对

核对日期：2026-10-27。范围：国内 WorkBuddy 与国际 WorkBuddy AI。初始研究只检查官方静态发行代码和公开文档；后续在鼠鼠授权下通过现有后端封装完成当前国内账号的一次真实领取，见文末记录。未展示身份、凭证或原始响应，未新建或切换账号。

## 能力与官方依据

- 国内版支持活动型每日签到，是否开放以服务端 `active` 为准，奖励额度以实际响应为准。官方[系统设置](https://www.workbuddy.cn/docs/workbuddy/From-Beginner-to-Expert-Guide/Function-Description/Setting)说明「Buddy 加油站」用于参与当前活动获取积分。
- 国际版不提供此手动签到能力。本机官方 WorkBuddy AI 5.5.2 的 `cli/product.json` 明确 `features.DisableCheckin=true`；国内 5.8.3 为 `false`。国际版虽包含共享签到代码，不能据此调用相同路径。
- 国际版当前[订阅条款](https://www.workbuddy.ai/docs/workbuddy/Subscription)说明每日奖励由有效客户端 AI 对话或任务触发，系统自动结算、通常次日到账，无需手动领取。仅打开、登录或网页发起任务不满足该奖励条件；Jolu 不代造活跃任务。

本机官方发行代码来源：

1. `D:\WorkBuddy\resources\app.asar`，版本 5.8.3：`main/tar.js` 的 `AuthService.getCheckinStatus`、`claimDailyCheckin`、`buildHeadersWithTuringToken`；`main/account.js` 的 `mapCheckinStatus`；`renderer/assets/UserMenu-C4YeP3NP.js` 的 `useCheckinService`、`useCheckinDayRollover`。
2. `D:\WorkBuddy AI\WorkBuddyAI\resources\app.asar`，版本 5.5.2：`main/application-manifest.js` 包含未压缩的同名方法和说明，`DesktopAccountRepo.billingPrefix` 注释明确桌面端 `/v2` 使用 Bearer 认证，网页无此前缀使用 Cookie 认证。
3. 两版 `resources/app.asar.unpacked/cli/product.json` 的发行区、endpoint、`DisableCheckin` 配置。

## 国内桌面端协议

固定主机 `https://www.workbuddy.cn`，不按凭证中的域名构造地址；禁止重定向。

| 用途 | 方法与路径 | 请求体 |
|---|---|---|
| 状态预检 | `POST /v2/billing/meter/checkin-activity-status` | `{}` |
| 单次领取 | `POST /v2/billing/meter/daily-checkin` | `{}` |

认证头：`Authorization: Bearer <accessToken>`、`X-User-Id`、`Accept: application/json`、`Content-Type: application/json`，按原会话附带 `X-Domain`；企业会话包含 `X-Enterprise-Id` / `X-Tenant-Id`，但官方界面仅个人账号开放签到。现有本机 `sym-v1` 凭证解码逻辑原样复用，浏览器授权使用 Jolu 自己保存的已验证账号会话，均不刷新或写回原工具。

官方设备风控头 `X-Device-Token` / `X-Device-Token-Error` 为 provider 返回值可选注入；provider 未配置或获取异常时返回空对象。Jolu 不仿造设备身份或复制风控令牌；服务端拒绝时显示需到官方客户端处理，不绕过校验。

成功响应顶层为 `code: 0`、对象 `data`。状态字段：`active`、`today_checked_in`、`today_credit`，还可能有 `streak_days`、`daily_credit`、`is_streak_day`、`next_streak_day`、`streak_bonus_days`、`streak_bonus_credit`、`checkin_dates`、`total_credits`、`start_time`、`end_time`、`season` 等。领取响应 `data.credit` 表示实际领取积分，另有 `streak_days`、`is_streak_day`。未知奖励保留为空，真实 `0` 保留为零。

已核实业务码：`1001` 已领取、`1002` 不符合资格、`1003` 活动结束。其它业务码不猜含义；HTTP `401`、`403`、`429` 分别处理。界面或日志不透传服务端原始 `msg`、响应体、令牌或账号标识。

## 日期与执行边界

未查到签到服务端日期边界的官方时区定义。官方界面的本地日期仅触发重新查询，必须等状态接口返回 `today_checked_in=false` 才确认跨日。因此不按本机日期跳过请求，`period_key` / `reset_at` 保持未知；不得把积分包日期解析中的东八区规则挪用于签到。

适配器领取仅发起一次 POST，不自动重试。公共服务先检查状态、按真实用户 ID 去重，再决定领取；`active=false`、已签、未知或异常状态均不能继续发放请求。网络中断导致结果不明时，通过后续状态查询核实。

## 验证范围

Context7 按 `library` → `docs` 检索官方 WorkBuddy 文档索引 `/websites/codebuddy_cn_workbuddy`，未返回此私有桌面协议；协议依据上述官方发行代码。HTTP 读取上限为 1 MiB，使用应用联网配置、20 秒超时和禁重定向客户端。自动测试覆盖服务端状态、领取成功、重复签到、未知奖励、异常与缺字段。

合成执行记录：先以失败占位实现运行解析测试，5 项中 4 项因缺少正确状态分流而失败；稳定账号身份测试亦先失败。完成实现后执行 `cargo test --manifest-path backend/Cargo.toml checkin_workbuddy::tests --lib`，6 项通过、0 项失败。UTF-8 与空白检查通过。

## 当前国内账号的真实接口验收

仅在明确授权后显式运行以下忽略测试，常规 `cargo test` 不访问真实账号：

1. `cargo test --manifest-path backend/Cargo.toml checkin_workbuddy::tests::native_status_read_only_smoke --lib -- --ignored --exact --nocapture --test-threads=1`：1 项通过。输出 `state=Ready reward=Some(100.0) claimed=false`。
2. 使用仅本次测试进程生效的 `JOLU_RUN_REAL_CHECKIN=1`，显式执行 `checkin_workbuddy::tests::native_claim_once`：1 项通过。测试在预检为 `Ready` 后，于 POST 前重新读取本机当前账号并仅在内存比较真实 UID；最多调用一次领取，之后重新验证相同身份并查询状态，不自动重试。

实际脱敏结果：

```text
before state=Ready reward=Some(100.0) claimed=false
claim state=Claimed reward=Some(100.0) claimed=true
after state=Claimed reward=Some(100.0) claimed=true
```

此次领取端点以成功响应中的 `credit=100` 确认奖励 100 积分，并已由随后状态查询确认今日已签。此次请求未附带设备风控令牌且被服务端接受；不能据此保证其他账号、其他日期或后续风控策略始终接受。同日未再次发起领取，因此真实重复领取业务码、跨日日期边界、其它账号及国际版不在本次真实验收范围内。

## 官方返回统计的范围与字段形状

同日补充核对只调用 `POST /v2/billing/meter/checkin-activity-status`，未调用领取端点、切号或写回官方登录。两次查询分别验证字段类型与活动起止字符串的格式；输出只有字段存在性、类型和格式断言，不输出身份、凭证、设备信息、字段值或原始响应。

- 国内 5.8.3 `main/account.js` 的 `checkinStatus()` 直接映射 `checkin_dates`、`total_credits`、`start_time`、`end_time`、`season`，没有追加分页读取。
- `renderer/assets/UserMenu-C4YeP3NP.js` 的 `useCheckinService()` 优先使用 `total_credits`，缺失时客户端会退回 `streak_days * today_credit`。该乘法只能是客户端估算，Jolu 不采用它，也不使用 `daily_credit * 日期数` 补造周/月奖励。
- 当前真实状态的 `checkin_dates` 为字符串数组，元素均符合 `YYYY-MM-DD`，本次样本没有重复。每个元素只有日期，没有逐日奖励。
- `total_credits` 为数值；活动 `start_time` / `end_time` 为 `YYYY-MM-DD HH:mm:ss` 字符串，不含时区；`season` 为数值。活动边界和期数不能作为终身历史完整性的证明，也不能据此设置当日 `period_key` 或 `reset_at`。
- 未找到对 `checkin_dates` 完整覆盖范围的官方承诺。Jolu 的天数仅表示“官方返回日期”的去重计数：按官方 HTTP `Date` 对应的北京时间筛选本周、本月及已返回累计，并排除未来日期；任意日期无效、缺少数组或缺少可信服务端时间时，不补造天数。
- 官方奖励汇总显示为“本期活动”，只保留原始 `total_credits`，未知保留空、真实零保留零。周/月奖励和终身奖励仍未知；不能把本期累计标成终身累计。

统计随原有签到状态一起返回：无历史不会把今日已签改成失败；活动未开放也保留有效的官方统计。范围说明持续提示“天数仅按官方返回日期统计；官方未提供逐日奖励或完整历史范围”。

Context7 再次按 `library` → `docs` 查询 `/websites/codebuddy_cn_workbuddy` 的签到字段范围主题，没有匹配文档；以上结论来自官方发行代码与真实只读形状断言，没有依据第三方逆向实现推断统计含义。

本轮解析验证先运行 `cargo test --manifest-path backend/Cargo.toml checkin_workbuddy::tests --lib`：6 项旧测试通过，4 项新增测试因尚无官方统计快照而失败；实现后同一命令结果为 10 项通过、0 项失败、2 项真实测试忽略。新增回归覆盖返回日期去重、北京时间周/月窗口、未来日期排除、缺少服务端时间、非法日期整集合未知、真实零奖励、活动未开放仍保留统计，以及不估算逐日奖励。

随后显式执行只读 `native_status_read_only_smoke`，1 项通过。脱敏输出确认：当前状态为 `Claimed`，官方统计、周/月/本期返回天数、本期奖励与服务端日期均已知。此输出仅含状态枚举与“已知/存在”布尔值，不记录任何真实统计数值或身份；未执行真实领取测试。
