# Trae 账号与额度接入验收（2026-10-25）

结论：本机 Trae CN 与国际版均存在可复用的原工具登录记录。两版官方额度查询均已正常返回 HTTP 200；「没有登录记录」的前提不成立。国内版新版积分必须查 v2，不能用 v1 的零值占位字段替代。

## 证据与查询路径

- 国内安装包 `%LOCALAPPDATA%/Programs/Trae CN/resources/app/out/main.js`：`UserStorage.getUserInfoFromLocalStorage` 读取 `iCubeAuthInfo://icube.cloudide`，经过 `byteCrypto` 解码。记录不是 Electron safeStorage/DPAPI，而是普通 `tc` 信封：Base64、版本头、随机种子、SHA512 派生与完整性前缀、AES128-CBC/PKCS7。
- 本机两版 `User/globalStorage/storage.json` 的该键均成功只读解码。只取 token、host、expiredAt、account.email；不读取 `icube-dc:*` 设备密钥，不使用 refreshToken，不保存任何解密结果。
- 国内 `workbench.desktop.main.js` 的 `_doFetchCnEntitlement...` 路径：`POST /trae/api/v2/pay/ide_user_ent_usage`，`Authorization: Cloud-IDE-JWT ...`，请求体 `require_usage: true`。官方 `product.json` 的 `bootConfig.ug.trae.normal` 为 `https://api.trae.cn`。
- 国际安装包 `D:/TraeAi/Trae/resources/app/product.json` 核实 ug 主机：`grow-normal.trae.ai`、`growsg-normal.trae.ai`、`growva-normal.trae.ai`、`grow-normal.traeapi.us`。国际额度为相同路径的 v1。本机登录主机为最后一种，正常返回 HTTP 200。
- 此为原客户端使用的接口，非公开稳定 OpenAPI 合约；上游版本变化需重新核对。代码仅允许精确 HTTPS 主机白名单，禁止重定向和跨区域回退。

## 实际脱敏结果

| 来源 | 接口 | 结果 |
|---|---|---|
| Trae CN | v2 | HTTP 200；套餐标签「免费」；`usage_summary.consumed_amount=0`、`total_amount=4600`、`consumption_ratio=0` |
| Trae CN | v1（仅研究对照） | HTTP 200；旧额度字段全部为 0，不能代表新版积分 |
| Trae 国际版 | v1 | HTTP 200；套餐标签 `Free plan`；`basic_usage_limit=1`、`basic_usage_amount=0`；请求次数上限存在但当前响应没有对应 usage，不补成 0 |

国内分包返回福利、月度赠送、签到奖励等不同权益；本次只使用服务端 `usage_summary` 汇总，避免自行按未确认的 status 或权益期筛选、加总。`entitlement_base_info.end_time` 是权益周期端点，不能认定为订阅到期或额度重置，因此二者均不推导。

## 实现与集成

新增 `backend/src/trae_quota.rs`：

- `source_quota("native:trae:ide")` 只读取 `%APPDATA%/Trae` 并查询国际版；`source_quota("native:trae:cn")` 只读取 `%APPDATA%/Trae CN` 并查询国内版。
- `validate_source(root)` 接受变体用户数据根，纯本地验证信封、受支持主机、token 字段和明确的本地到期时间；不联网、不扫描会话。
- `local_trae_quota()` 保留为国际版便捷入口；产品应调用按 source ID 区分的入口。
- 真零可以显示 0；缺失已用量保留未知，0 分母不生成百分比，-1 不限量不生成百分比。百分比仅从响应中同时存在的有限有效已用/总量计算，不依赖未确认量纲的 `consumption_ratio`。
- 套餐仅使用含 `subscription_extra` 的服务端 `display_desc`。不把积分包名称、token 有效期、权益周期终点冒充订阅状态或到期时间。
- HTTP 401/403/429 与解析/网络失败分别返回固定中文错误，业务 code 1001 表示登录失效；不返回原始服务器文本。文件/响应限制 2 MiB，凭据只在后端短时使用。
- 复用 `quota::client()`，遵守 Jolu 网络与代理策略；请求结束后重读原来源，仅比较 host+token，登录切换/令牌续期/记录消失时拒绝迟到结果，不因邮箱等资料更新误拒绝。

## 文档核对与许可证

先执行 Context7 `library Trae`，唯一相关返回 `/bytedance/trae-agent`；`docs` 仅包含开源 Agent 的模型 API Key 配置，并非 Trae IDE 账号接口。随后以官方本机安装源码与实机只读响应为准。

RustCrypto 使用先查 Context7 `library cbc`（无相关项），再 `library RustCrypto` / `docs /rustcrypto/utils`，核对 PKCS7 严格填充校验；具体 API 另以 [cbc 0.1.2 官方文档](https://docs.rs/cbc/0.1.2/cbc/) 的 `Decryptor<Aes128>` / `decrypt_padded_mut` 为准。

普通 tc 格式的公开盐表与独立 Node 验证参考 [CreditDaddy traeLocal.js](https://github.com/techysy/CreditDaddy/blob/main/src/traeLocal.js)，[MIT 许可证](https://github.com/techysy/CreditDaddy/blob/main/LICENSE)已保留在模块注释。没有提取私有 appSecret；没有采用该项目的账号切换、快照覆盖或进程终止功能。

企业路线也已核对：[官方成员额度查询](https://docs.trae.cn/enterprise_query-usage-limits-for-members?lang=zh)需要 `quota:read` 权限及企业 OpenAPI access token；[官方鉴权](https://docs.trae.cn/enterprise_authentication?lang=zh)与个人 IDE JWT 不同，不能用它替代本机个人登录。[CLI 登录令牌](https://docs.trae.cn/cli_login-token)属于企业旗舰版个人 PAT；本次无需创建新 PAT、启动登录或变更原工具状态。

## 验证

- TDD 红灯已执行：新增 CN v2 summary 测试在 stub 上按预期失败。
- 离线验证覆盖 CN v2 实际响应形状、国际美元计费形状、真实零/未知/零分母、不限量、Node 独立生成的合成 tc 固定向量、完整性损坏、未知信封头、主机白名单与过期登录。
- `cargo test --manifest-path backend/Cargo.toml trae_quota::tests --lib`：7 passed，1 ignored（需显式联网的 smoke），0 failed。
- `cargo test --manifest-path backend/Cargo.toml trae_quota::tests::local_variants_read_only_smoke --lib -- --ignored --exact --nocapture`：1 passed，0 failed；国内版 1 个额度窗口、国际版 2 个额度窗口，均有服务端套餐标签。只打印来源、窗口数、是否存在套餐，不打印凭据/邮箱/原始响应。
- 登录切换守卫测试覆盖 host/token 变更拒绝，账号资料更新允许。独立 Node 查询与正式 Rust 查询均已验证两版正常读取。
- 尚未验证：付费国际套餐、企业私有域、不同地区未实际安装版本、未来 tc 格式。对未支持域名明确返回 Unsupported，不猜测请求地址。

## 改动总结

新增严格按来源读取的 Trae 本地登录解析与只读额度模块，将国内 v2 积分及国际 v1 计费结果转换为既有 `QuotaState`。核心逻辑是复用当前原工具账号、固定官方主机、根据服务端数值构建额度窗口，并保留所有未知状态；不写入原客户端数据、不发送模型请求。

## 同轮审查：WorkBuddy / Qoder 来源绑定

原因：原 `local_workbuddy_quota()` 是国内版优先的通用发现入口；如果用它查询所有账号卡，国际版卡可能拿到国内版结果，国内版过期时又可能回退国际版。Qoder 的现有 CLI 只覆盖国际版，国内版卡不能复用该查询。

修复：

- 新增 `workbuddy_quota::source_quota(source_id)`：`native:workbuddy:main` 只读取 `workbuddy-desktop.info`；`native:workbuddy:ai` 只读取 `workbuddy-desktop-ai.info`。缺失、过期、加密或损坏直接返回当前来源状态，不读取另一个文件。沿用既有认证解析、域名白名单、额度查询及网络策略。
- 新增 `qoder_quota::source_quota(source_id)`：`native:qoder:global` 复用国际 CLI 入口；`native:qoder:cn` 明确返回「国内版在线额度暂未接入」，未知 source ID 同样 Unsupported，不启动 CLI。
- 两个旧无参本地入口保持不变；产品账号卡应使用新增的按来源入口。没有修改用户的任何真实登录记录。

验证：先添加 WorkBuddy 来源绑定测试与 stub，确认红灯；随后针对新来源绑定执行 `cargo test --manifest-path backend/Cargo.toml source_binding --lib`：2 passed，0 failed。合成临时目录同时包含「过期国内版、有效国际版」，验证国内版过期/文件缺失仍不回退，国际版始终读取自己的登录；Qoder 国内版及未知来源直接返回 Unsupported。

## 缓存身份与隔离环境补修

增加两个仅供后端缓存使用的 `source_identity(source_id) -> Result<String, QuotaState>`：Trae 按所选版本读取并验证账号，用 host+token 的带长度边界 SHA256 摘要标识身份；WorkBuddy 用经既有解析验证的 base+token+header 名值生成摘要。邮箱等资料字段不参与，令牌、主机、用户/组织请求头变化会使身份改变。摘要不记录日志、不返回前端，读取错误保留对应来源状态。

Trae 身份查询与额度查询共用 `source_auth`，在文件访问前拒绝隔离环境；WorkBuddy 两个入口均有相同保护。缓存调用方可在查询前后比较摘要并拒绝过期账号结果，Qoder 无可靠独立身份读取，因此由调用方停用跨请求缓存。

合成身份测试先红后绿，`cargo test --manifest-path backend/Cargo.toml source_identity_ --lib`：2 passed，0 failed，验证资料修改不影响摘要，token/host/base/用户请求头修改会改变摘要。本轮未重新请求真实网络额度。
