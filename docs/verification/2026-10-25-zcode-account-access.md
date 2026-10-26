# ZCode 官方账号只读接入核查

日期：2026-10-25。范围：现有账号、个人 Coding Plan；不修改真实登录、不创建 Key、不兑换令牌、不发送模型请求。

## 结论与改动总结

- 原实现声称账号查询必须使用私有 appSecret，这个结论不成立。官方已公开客户端源码，可读取当前用户自己的登录缓存并正常请求对应服务。
- `backend/src/zcode_quota.rs` 新增账号路径：读取 `.zcode/v2/credentials.json`，按官方公开 AES-256-GCM 格式解码 `oauth:active_provider` 和其对应 access token。仅 BigModel 当前账号发起固定域名 GET；不把 Z.ai token 发给 BigModel。
- 先 GET `https://bigmodel.cn/api/biz/subscription/list`。明确成功的空数组返回实时 `inactive`、空额度窗口、文案“未开通个人编程套餐”。这不代表未登录、不代表真实额度为零，也不代表没有团队或 Start Plan。
- 有有效个人 Coding 订阅时，再 GET 同源 `/api/monitor/usage/quota/limit`，复用现有智谱额度解析。付费账号成功额度响应尚缺真实样本，不能宣称已真机通过。
- 原 BigModel Coding Plan API Key 查询保留；账号接口单独暴露 `local_zcode_account_quota`，避免账号页拿本机其他 Key 代替当前登录。提供 `validate_local_account_at(home)`，供无会话数据库时注册账号和隔离测试使用。
- 移除旧缓存权益回退。已复现旧 `any(not_entitled)` 会把混合未知状态误报为未开通；新路径从远端确认，不把旧缓存当实时状态。
- 网络、HTTP、业务异常与解密失败仅返回固定文案，不回显 URL 中身份信息、令牌、服务器正文；客户端禁止重定向。查询完成复读登录，变化时丢弃旧结果。

## 官方证据

先执行 Context7 library ZCode，解析到 `/zai-org/zcode`，再 docs 查询 OAuth/套餐额度。以下均为官方仓库：

- [额度协议](https://github.com/zai-org/zcode/blob/main/docs/coding-plan-quota-remaining-api.md)：GET quota/limit，裸 Authorization，返回值按实际窗口分类。
- [官方凭据加密实现](https://github.com/zai-org/zcode/blob/main/packages/services/src/credential/providers/credentialCipherProvider.ts)：`enc:v1:<iv>.<tag>.<cipher>`，AES-256-GCM，SHA256 派生；自定义 ZCODE_CREDENTIAL_SECRET 或当前用户平台、home、用户名构成公开 fallback。只读正常解码自己的已有缓存，不读取其他用户或猜测自定义密钥。
- [OAuth 凭据命名](https://github.com/zai-org/zcode/blob/main/packages/services/src/oauth/repo/oauthCredentialRepo.ts)：active provider 与 provider access token。
- [个人权益判定](https://github.com/zai-org/zcode/blob/main/packages/services/src/bigmodel/codingPlanEntitlement.ts)：个人订阅列表、Coding 商品、VALID、inCurrentPeriod；损坏数据不表示未开通。
- [业务鉴权](https://github.com/zai-org/zcode/blob/main/packages/services/src/coding-plan-subscription/bigmodelCodingPlanSubscriptionProvider.ts)：BigModel 业务登录令牌使用裸 Authorization。
- [项目令牌逻辑](https://github.com/zai-org/zcode/blob/main/packages/shared/src/project-access-token-client.ts)：本机 project-key-location 仅保存组织、项目、Key ID；真实模型访问令牌存在内存缓存，缺失时涉及 POST access_tokens，不能把 location 当 Key。未调用这些 POST。
- [Start Plan 余额实现](https://github.com/zai-org/zcode/blob/main/packages/services/src/model-provider/zaiStartPlanBilling.ts)：独立 balance 端点，不是 Coding monitor。

加密库先执行 Context7 library（aes-gcm 未命中后用 RustCrypto）与 `/rustcrypto/aeads` docs；版本用法又与 [aes-gcm 0.10.3](https://docs.rs/aes-gcm/0.10.3/aes_gcm/) 和 [base64 0.22.1](https://docs.rs/base64/0.22.1/base64/) 官方文档核对。依赖由主代理统一修改。

## 本机只读证据

仅记录状态与数量，不记录账号标识、Key、令牌、完整响应。

- 本机当前 BigModel OAuth 可按官方格式正常解码；未修改文件。
- 个人订阅 GET：HTTP 200、code 200、success true、data 为长度 0 的数组。
- quota/limit GET：HTTP 200、code 500、success false、没有额度数据。当前无个人套餐，未把它显示为零；正式实现对空订阅停止，不多发这一请求。
- Start Plan balance GET：HTTP 400、code 3001；包括携带本机观察到版本的请求仍失败，未尝试伪造签名、提取客户端私有 secret 或绕过认证。该路径未启用。
- 本机 project-key-location 只有位置字段，无可直接查询的现存派生 Key。不创建、不换取新令牌。

## 验证与未解条件

- 定向红测复现缓存混合未知状态错误；该缓存生产分支随后完全移除。
- 合成测试覆盖已有 API Key 选择、空白 Key、OAuth 不混入 Key、官方加密格式、错误密钥/坏密文/未知加密版本、个人订阅空数组/有效/损坏/过期。
- 最终 Rust 定向测试及只读 smoke 结果见执行补记。
- 仍需鼠鼠提供有效个人 Coding Plan 登录，才能验证账号额度成功、限流和到期样本。Z.ai 国际账号、团队套餐、Start Plan 保持明确未支持/待验证，不混成个人套餐额度。

执行补记：`cargo test --manifest-path backend/Cargo.toml zcode_quota::tests --lib -- --include-ignored --skip local_key_can_query_remaining_credits` 最终 6 项通过（其中 1 项真实账号只读 smoke）；真实 API Key 测试未运行。`git diff --check -- backend/src/zcode_quota.rs` 通过。原红测 1 项失败符合预期，失败原因是混合未知缓存被错误标记 NoneEntitled。编译仅出现当时尚未接线的新函数 dead_code 警告，由主代理整合账号注册后复核。
