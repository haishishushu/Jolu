# Trae 每日积分签到协议与验收（2026-10-27）

结论：Trae 国内个人账号存在真实的每日积分签到能力，已用当前原工具登录和官方实际 AHA 设备完成状态查询、一次真实领取与领取后状态确认。领取前返回可签到、100 积分，领取返回成功，随后状态为已签到。国际版官方客户端明确受 `providerCode === "cn"` 守卫限制，因此返回不支持，不把同包内的公共代码误当成全球开放能力。

## 官方依据

- [官方套餐与计费](https://docs.trae.cn/ide_plans-and-billing)：本次直接打开的当前页面列明免费用户每日 100 积分、会员每日 200 积分，签到积分有效期为 31 个自然日；头像菜单提供签到入口。搜索缓存仍有旧的 150 积分，因此产品不硬编码奖励，使用状态接口的数值。
- [官方更新日志](https://docs.trae.cn/ide_changelog)：3.4.1～3.4.4 支持账号菜单的每日签到与签到抽奖。普通每日积分领取与抽奖不是一个接口。
- [1024 官方活动规则](https://docs.trae.cn/ide_million-credits-giveaway-on-oct-24)：抽奖在北京时间 2026-10-24 18:00 截止，现已结束。本次实现不调用 `checkin_1024`、`activity/action` 等抽奖接口，不将普通签到包装成参与抽奖。
- Context7 已先执行 `library Trae`，只有 `/bytedance/trae-agent`；再执行 `docs` 查询，没有对应 IDE 每日签到与认证文档。随后使用官方安装发行代码和官网规则。HTTP 客户端配置另通过 Context7 `/seanmonstar/reqwest` 核对禁重定向与禁止重试接口。

本机官方发行代码位置：

| 版本 | 位置 | 核实内容 |
| --- | --- | --- |
| CN 3.4.4 | `%LOCALAPPDATA%/Programs/Trae CN/resources/app/out/main.js` | `iCubeEntitlementManagement._requestCheckinCredits`、`fetchCheckinCreditsStatus`、`claimCheckinCredits`、`_applyUgDeviceHeaders`、`mixAuthorization` |
| CN 3.4.4 | 同目录 `out/vs/workbench/workbench.desktop.main.js` | `isIdeCheckinAvailable`、`refreshCheckinCreditsStatus`、`claimIdeCheckinCredits`、`KCs` 错误码、北京时间日期函数 |
| CN 3.4.4 | 同目录 `out/nls.messages.json` | 错误码对应的官方提示（索引 2353～2359） |
| CN 3.4.4 | `%LOCALAPPDATA%/Programs/Trae CN/Trae CN.exe` | 官方 Electron AHA DeviceService 与 TinyStorage 绑定、缓存键及注册响应字段 |
| Global 3.6.4 | `D:/TraeAi/Trae/resources/app/out/main.js` 与 `out/vs/workbench/workbench.desktop.main.js` | 同样的签到代码要求 CN provider；`product.json` 实际 providerCode 为 `i18n` |

这些是官方客户端内部接口，不是公开稳定 OpenAPI；升级后需重新核对。

## 请求和认证

| 用途 | 方法与固定地址 | JSON 请求体 |
| --- | --- | --- |
| 读取状态 | `POST https://api.trae.cn/trae/api/v2/ug/checkin_credits/status` | `{"req_source":1}` |
| 领取每日积分 | `POST https://api.trae.cn/trae/api/v2/ug/checkin_credits/claim` | `{"req_source":1}` |

- `req_source=1` 是官方 IDE 来源；Work 为 2，本实现不冒充 Work。
- `Authorization: Cloud-IDE-JWT <当前凭证>`，`Content-Type: application/json`，`x-device-id: <官方 guaranteedDeviceId>`。
- 官方还按存在情况附带 `x-device-brand`、`x-device-type`、`x-os-version`、`x-app-version`。这些是可选系统描述，本实现没有编造它们。
- 国内普通登录须为 `host=https://api.trae.cn`、`account.scope=marscode`；SaaS、字节内部与国际区域不走该接口。浏览器凭证只能来自已有的国内 `ExchangeToken` 与 `/trae/GetUserInfo` 个人授权流程。
- 后端复用现有 `trae_quota::source_auth` 与 `decrypt_auth`，不读取或刷新 refresh token，不读设备私钥，不覆盖原工具配置。
- 浏览器流程产生的 `Pending.device` 是授权临时值，不作为实际签到设备。浏览器账号同样需要原工具的真实 AHA 缓存，缺失时明确提示安装并打开 Trae 国内版。

## 实际设备来源

官方主进程 `iCubeDeviceRegisterMainService.guaranteedDeviceId` 来自 Electron 的 `ahaDeviceService`。其设备注册服务缓存位于：

`%APPDATA%/Trae CN/aha/TinyStorage`

这是 JSON 文件，字段 `tiny_storage_data["aha.device.device_id"]` 保存 tc 信封。与现有登录 tc 相同的 `74 63 05 10 00 00` 格式经过 SHA512 完整性前缀和 AES128-CBC/PKCS7 解码后，对象包含：

```json
{
  "device_id_str": "官方注册的设备 ID",
  "install_id_str": "官方安装 ID",
  "uuid": "本机硬件 UUID",
  "version": "缓存版本"
}
```

上面仅展示字段语义，不是真实值。官方二进制内 `device_service.cc` 明确出现相同缓存键、`device_id_str` / `install_id_str` 注册响应字段与缓存硬件 UUID 不符时拒绝缓存的逻辑。`tiny_storage.cc` 包含 `MigrateFromPreferences`。

实现只读该缓存，严格检查 ID 为正数字符串、UUID 格式及本机硬件匹配。硬件 UUID 通过隐藏窗口的系统 PowerShell `Get-CimInstance Win32_ComputerSystemProduct` 读取，5 秒超时，不记录输出。没有以 `telemetry.devDeviceId`、随机 UUID 或机器摘要替代官方设备，没有发起设备注册，也没有尝试绕过每日设备限制。

## 状态、奖励与失败语义

状态响应直接读取顶层：

- `enable` 和 `checked_in` 必须是布尔值；缺失或错误类型为格式失败。
- `did_checked_in` 可选，存在时必须是布尔值。`checked_in=true` 才明确表示账号已领取；若前者为假而 `did_checked_in=true`，返回设备已签到的不可操作状态，不声称账号领取成功。
- `credits` 为有限正数时作为服务端签到奖励；不把缺失值补零，不硬编码 100/200，不将 `extra_credits` 抽奖相关奖励混加。
- `enable=false` 表示未开放；`enable=true && checked_in=false` 且设备未签到，表示可以领取。

领取仅在 HTTP 200、对象响应、业务 `code` 缺失或为整数 0 时认定成功，符合官方客户端的空对象成功兼容。存在非数字或非零业务码时不认定成功。

| 业务码 | 官方含义与本实现处理 |
| --- | --- |
| 1002（另兼容现有认证失效码 1001） | 账号验证失败，需重新登录或授权 |
| 2001 | 服务异常，失败 |
| 9004 / 4001 | 参数未通过校验，失败 |
| 9090 | 活动不可用，当前不可操作 |
| 9093 | 活动配置无效，失败 |
| 9095 | 该设备今天已签到；当前不可操作，不能伪称账号已领取 |
| 其他非零 | 固定提示服务异常，不向前端返回原始服务器文本 |

HTTP 401、403、429 分别表达登录失效、无权参与、请求频繁。未知网络结果提示先刷新状态，不自动重试领取。客户端通过 `net::blocking_builder` 遵守代理设置，20 秒超时、禁止重定向、禁止所有自动重试，文件和响应均限制 1 MiB。

## 日期与账号切换

官方代码按北京时间自然日计算签到日期。实现使用官方响应 HTTP `Date` 转换北京时间日期，并返回次日 00:00 的 Unix 秒时间戳。缺少可靠服务端日期时保留未知，不能用本机日期作为跳过请求的依据。

原工具账号来源每次请求前后重新核对其 UID、token、scope 与 AHA 缓存；账号切换、token 续期、设备变化或记录消失时拒绝迟到结果。后端对象仅在内存持有凭证、UID 和设备，不导出到普通日志、数据库历史或前端。

浏览器账号使用自己加密保存的 Session，不读取原工具当前登录的 UID、token 或 scope。此来源的前后守卫只核实自身有效期与本机实际 AHA 设备，因此更换原工具当前账号不会将另一个浏览器账号替换为当前账号。多个账号共享实际设备；官方设备每日限制仍按 `did_checked_in` / `9095` 表达，不以临时授权设备绕过。

## 同日浏览器授权链路复核

- 再次先执行 Context7 的 Trae 库解析及 `/bytedance/trae-agent` 文档查询，仍未获得 IDE 个人认证协议；核对 CN 3.4.4 与 Global 3.6.4 官方安装包以及当前国内官网的 `authorization/page.b9f91f9896.js`。
- 官网 PKCE 路径调用 `TraeGetPCAuthCode`，要求固定的 `http://127.0.0.1:<port>/authorize` 回调，以顶层导航回传 `authCodeInfo`、`loginTraceID` 等参数。Jolu 校验单次随机 `loginTraceID`，仅使用 AuthCode + 私有 CodeVerifier 换令牌，不信任回调中的账号、主机或直接令牌。
- 官方 `marscode/request` 的 `ExchangeToken`、`GetUserInfo` 是个人认证路径；身份请求为 `POST /cloudide/api/v3/trae/GetUserInfo`，`x-cloudide-token` 请求头，`ReqSource=IDE` 与 `IDEVersion` 请求体。`Result.UserID` 才是身份，邮箱或昵称不能代替；`ResponseMetadata.Error` 不能当作成功。
- 个人 `ide_user_ent_usage` 查询提供套餐或数值额度，不是身份认证接口。额度不支持或查询失败应继续表达原额度状态，不能据此否定已经实时验证过的个人身份。企业个人额度 OpenAPI 与此处的普通个人登录无关，不能混用权限或凭证。
- 官方 `marscode/util.xxe` 以毫秒解析 `TokenExpireAt`；当绝对期限落后且存在有效 `TokenExpireDuration` 时，使用收到响应时刻加该毫秒时长。修复原实现只接受未来绝对期限的差异；缺失期限、无效/非正时长和溢出仍失败。
- 新增 `trae_browser_auth::verify_session`：校验固定区域、凭证、期限，再实时请求官方 `GetUserInfo`，并要求返回 UID 与封存 Session 的 UID 相同。账号验证与额度查询分开处理；不借用原工具登录，不读取或刷新设备私钥，不修改历史凭证。
- Jolu 授权等待窗口仍为自身的 10 分钟；官方本机 OAuth server 的 30 分钟与旧中间页 5 分钟为客户端生命周期设置，不是访问令牌期限。访问令牌以官方响应期限为准，过期重新授权。

本轮先观察两个新增合成测试的预期红灯，再实现期限兜底与身份绑定。使用本轮构建产物定向执行：`trae_browser_auth::tests` 5 通过、1 忽略，`checkin_trae::tests` 7 通过、2 忽略，`trae_quota::tests` 8 通过、1 忽略。

显式执行 `trae_browser_auth::tests::saved_sessions_read_only_smoke`：只读正常数据库中已保存的 Trae 浏览器凭证，目前仅有一条国际版 Session；官方实时身份核验通过，返回 UID 与封存 UID 相同，独立额度查询为 `ok`，测试通过。没有国内版已保存浏览器 Session，因此不声称国内版网页回调、保存或浏览器来源签到已完成真实验证。真实探测仅执行身份、额度与 `status`，不再调用今天已领取过的 `claim`。

另显式执行 `trae_browser_auth::tests::cn_personal_session_read_only_smoke`：使用已授权本机国内个人凭证，仅在内存构造 Session，再执行同一实时身份接口和 `Account::from_browser` 的 `status` 分支；身份核验通过，状态为 `Claimed`，具有官方日期，测试 1 通过、0 失败。没有保存该 Session、修改原工具登录或再次领取。此项覆盖国内个人凭证在独立 Session 查询分支的兼容性，不能替代国内版真实网页授权与保存验收。

## 验证

1. 先写状态、业务码、设备同机验证、区域/账号守卫与日期边界测试及 stub；第一次因其他并行模块尚未创建而编译失败，文件齐备后第二次观察到 5 个预期红灯，再实现逻辑。
2. `cargo test --manifest-path backend/Cargo.toml checkin_trae::tests --lib`：7 passed，1 ignored，0 failed。覆盖严格布尔字段、未知奖励、设备已签到区别、业务错误、CN 个人资格、国际版提前拒绝、北京时间跨日、独立 Node crypto 合成 tc 固定向量与损坏缓存。
3. `cargo test --manifest-path backend/Cargo.toml checkin_trae::tests::native_status_read_only_smoke --lib -- --ignored --exact --nocapture`：1 passed，0 failed。只输出 `Ready`、奖励 100、具有官方日期三个脱敏结果；未执行 `claim`。
4. 新增独立忽略测试 `native_claim_once`，同时要求 `JOLU_RUN_REAL_CHECKIN=1`。先在不提供环境开关时显式执行，确认测试在访问真实登录、设备或网络之前断言失败，没有发生领取。
5. 提供显式开关后执行 `cargo test --manifest-path backend/Cargo.toml checkin_trae::tests::native_claim_once --lib -- --ignored --exact --nocapture`：1 passed，0 failed。实际脱敏结果为 `state=Ready reward=Some(100.0)` → 单次 `claim_state=Claimed reward=None` → `after_state=Claimed`。发送前 guard 重新读取并比对当前账号身份、凭证和实际 AHA 设备；没有重试领取，没有新建或切换账号，没有 AI 任务。领取接口未单独返回奖励数值，100 来自领取前的官方状态，不伪造领取响应中的奖励。
6. 未验证：付费账号、浏览器独立授权的真实签到、不同 Windows/PowerShell 条件。集成后的公共服务与 UI 验证由主任务继续完成。该真实测试改变了当前账号当日的签到状态，再次执行会先查询并在非 Ready 时直接结束，不会重复领取。

## 改动总结

新增 `backend/src/checkin_trae.rs`，实现按区域与来源绑定的账号对象、真实 AHA 设备读取、严格状态解析、一次领取、北京时间周期和登录切换守卫；真实领取测试额外受 ignore 与显式环境开关保护，并复查发送前账号和设备。`trae_quota.rs` 仅将两个既有内部函数开放为 crate 内可复用，复用解码与登录校验；没有改变原额度接口行为。
