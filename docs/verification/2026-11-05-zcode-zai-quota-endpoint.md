# ZCode z.ai 额度端点只读验证（2026-11-05）

范围：只用本机 `~/.zcode/v2/provider_config.json` 里 `zai-api-2` 的 `zhipu-coding-plan-api-key`，向 `https://api.z.ai/api/monitor/usage/quota/limit` 发有限次真实只读 GET，确认该端点是否接受这把 Key、返回结构能不能映射成额度。没有做网关接管，没有读写 `.zcode` 下任何文件，没有重试。

## 调用记录

凭证来源：`provider_config.json` → `config.providerConfigRules.providerRules` 里 `providerId = "zai-api-2"`、`config.access.type = "zhipu-coding-plan-api-key"` 的 `apiKey`。该值长度 49、形如 `<id>.<secret>`（1 个点、无连字符）。**本文件不记录 Key 原文**。

两次请求均为 `GET https://api.z.ai/api/monitor/usage/quota/limit`，超时 20 秒，另外带上与现有实现相同的 `Content-Type: application/json`、`Accept-Language: en-US,en`。

| # | Authorization 形态 | HTTP 状态 | 响应体长度 |
|---|---|---|---|
| 1 | `<裸 Key>`（与 `backend/src/coding_plan.rs` 的智谱实现一致） | 200 | 59 字节 |
| 2 | `Bearer <Key>`（只为了排除鉴权头格式问题） | 200 | 59 字节 |

两次都没有重试；第 2 次不是第 1 次失败后的重复尝试，而是换一种鉴权头形态，否则无法区分「Key 不被接受」和「鉴权头写错」。

## 响应结构与结论

两次返回体完全相同：

```json
{ "code": 1000, "msg": "身份验证失败。", "success": false }
```

字段结构摘要（值本身不含账号信息）：

| 字段 | 类型 | 本次取值 | 说明 |
|---|---|---|---|
| `code` | number | `1000` | 业务码，非 0 表示失败 |
| `msg` | string | `身份验证失败。` | 固定中文错误文案 |
| `success` | boolean | `false` | 明确失败 |

结论：

1. **端点接受这把 Key 吗**：HTTP 层接受请求（200），业务层拒绝（`success: false` + `code 1000`）。
2. **能否映射「余额 / 剩余额度 / 到期」**：不能。响应里没有 `data`、`limits`、`used`/`remaining`、`nextResetTime`、`level` 等任何额度字段，只有 `code` / `msg` / `success` 三项。
3. 现有实现遇到这种响应会得到什么：`coding_plan::zhipu_state_from_body` 先看 `success == false`，业务码不是 500（它对应「当前用户不存在 coding plan」），因此走 `business_failure`，最终返回 `QuotaState::Failed { reason: "智谱套餐接口错误（业务码 1000）" }`。也就是说，直接把它接进 ZCode 的账号卡，用户看到的是一条失败，而不是额度。
4. **本轮处置**：按「成功才接线」的约定，`backend/src/zcode_quota.rs` 没有把国际站 Key 接到 `coding_plan.rs` 的额度查询上，而是走登录态分支：只返回「平台名 + 账号掩码」，额度窗口如实写「待联网查询」，不猜数值、不按套餐名推断。中国站（`bigmodel*`）Key 仍然复用 `coding_plan::query_plan_direct(PlanProvider::ZhipuCn, …)`，没有复制第二份实现。

## 尚未验证的替代路径（需要鼠鼠再拍板）

- `credentials.json` 的 `oauth:zai:access_token`（用户 OAuth 会话，非 Coding Plan Key）是否被同一端点接受。本轮范围只允许 Coding Plan Key，没有发送。
- `credentials.json` 的 `account-provider:coding-plan:…:api-key`（本机另有一把 49 字符的 team 套餐 Key，与 `provider_config.json` 这把不同）。没有发送。
- 同一把 Key 打中国站 `https://open.bigmodel.cn/api/monitor/usage/quota/limit`：站点不同，没有发送。
- 国际站是否另有监控端点（例如控制台同源的其他路径）：没有探测，不猜路径。

## 敏感信息处置

- 本文档不含 Key、JWT、邮箱、昵称、user_id 的任何原文，只保留长度与形态描述。
- 调用只在内存里组装 `Authorization` 头，没有把请求头写进日志、诊断或任何文件。
- 账号掩码由后端生成（`a***@d***.com`），前端只拿掩码；额度端点的失败原因不带凭证片段。

## 后续（同一日）

本文第 4 条「本轮处置：走登录态分支」已被后续实现取代：国际站的账号额度改由
`credentials.json` 的 `zcodejwttoken` 查询 `GET https://zcode.z.ai/api/v1/zcode-plan/billing/balance`
（`Bearer` + 一次性 `X-Device-Mid`），`login_state` 只作为拿不到该令牌时的兜底。
本文的实测结论仍然有效——`zai-api-2` 那把 49 字符 Key 本身失效（业务码 1000），
而能鉴权的个人 coding-plan 凭据在该监控端点是业务码 500「当前用户不存在coding plan」。
详见 `docs/verification/2026-11-05-zcode-account-balance.md`。
