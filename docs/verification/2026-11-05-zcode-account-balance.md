# ZCode 账号额度只读接入与真机冒烟（2026-11-05）

范围：把「ZCode 账号额度」接到 `backend/src/zcode_quota.rs`。只读：不向 `~/.zcode` 写任何文件、不刷新登录、不创建 Key、不兑换令牌；`zcodejwttoken` 只在内存里用于当次请求。

## 端点与请求头

| 项 | 取值 |
|---|---|
| 端点 | `GET https://zcode.z.ai/api/v1/zcode-plan/billing/balance` |
| 凭证 | `credentials.json` 里的 `zcodejwttoken`（JWT，claims 只有 `iat` / `sub` / `token_version` / `user_id`，无 `exp`） |
| 鉴权头 | `Authorization: Bearer <token>` |
| 设备标识 | `X-Device-Mid: <uuidv4>`，**内存现生成、每次请求一个**，不落盘、不进日志 |
| 客户端标识 | `User-Agent: ZCode/3.14.3` |

同一份登录里的 `oauth:zai:access_token` 打这个端点返回 HTTP 401 空 body，所以它只用于表达登录态，不能当额度凭证。

## 为什么不再用另外两条路

| 路径 | 实测结论 |
|---|---|
| `provider_config.json` 的 `zai-api-2` Key 打 `api.z.ai` / `open.bigmodel.cn` 的 `/api/monitor/usage/quota/limit` | HTTP 200 + 业务码 1000「身份验证失败」——**这把 49 字符 Key 失效**，不是站点问题 |
| `credentials.json` 的 `*:coding-plan:*:api-key`（能鉴权的个人账号）打同一端点 | 业务码 500「当前用户不存在coding plan」；带团队头 + `?type=2` 仍 500；不带头时 `code 200` + `data:{}`，`data.limits[]` 对该账号不通 |
| `zcodejwttoken` → `/billing/balance` | `200 / code 0`，`data.plans[]` + `data.balances[]` 齐全（本文件记录的就是这条） |
| `/billing/current` | `200 / code 0` 但没有 `data`：按「成功但没有额度数据」处理，不当解析失败 |

旁证：`project-key-location` 末位 `non-team`、`setting.json` 的 `providerFamilyConnectionSelections.zai.kind = "individual-coding-plan"`、`oauth:zai:access_token` 的 `user_type = "PERSONAL"`。

## 字段映射（`coding_plan::parse_zcode_plan_balance`）

| 来源字段 | 映射到 |
|---|---|
| `balances[].show_name` | 额度窗口名（条目没有时回填 `plans[].entitlements[].show_name`，都没有才用「额度窗口 N」） |
| `used_units / total_units * 100` | `used_percent`；`total_units == 0` 或缺失 → `None`（未知，不 clamp 成 0） |
| `remaining_units` / `available_units` | `remaining_text`（两者不同时写「剩余 X（可用 Y）」） |
| `used_units` + `total_units` | `amount_text`（`已用 / 总量 + unit_type`） |
| `period_end`（unix 秒） | `resets_at`（RFC3339） |
| `bucket_id`（缺则 `entitlement_id`） | 窗口 `key` 与去重键：同一 plan 的多个 bucket 各自成行，重复条目只留第一条 |
| `plans[].name` / `ends_at` / `status` | `plan` / `account.expires_at` / `account.subscription_status` |
| `data.server_time` | `account.observed_at` |

## 状态表达（四态不合并）

| 情形 | 表达 |
|---|---|
| 正常返回 `balances[]` | `Ok`；`used_units = 0` 是**真实 0%**（会画进度条） |
| `total_units == 0` / 缺总量 | 窗口存在但 `used_percent = None`（**未知**，不画条、不写 0） |
| `code 0` 但 `data` 缺失 / null / 空对象 | `Ok` + 空窗口 + `plan = "账号未返回额度数据"`（**成功但没有额度数据**，不报错） |
| `data` / `plans` / `balances` 类型不认识 | `Failed`（**查询失败**） |
| HTTP 401 或业务码 401 | `Unauthorized`「需要重新登录」，不混成查询失败 |
| 业务码 500 / 1000 等 | `Failed`，只保留数字业务码，不回显上游 `msg`（可能含凭证） |
| 有登录态但拿不到 `zcodejwttoken` | `Ok` + 未知额度窗口（`login_state` 兜底） |

## 隐私

- 令牌与设备标识只在内存里；`X-Device-Mid` 每次请求新生成一个 v4 UUID，**不写 `zcode-device-mid` 之类文件**。
- 前端只拿账号掩码（`a***@z***.ai`）与额度字段；日志、错误文案、诊断与本文档都不含令牌、邮箱、`user_id` 原文。
- 缓存身份 = 家族 + 令牌种类 + 实际使用的令牌的 SHA-256：换账号或换令牌就换身份，不会显示上一个账号的额度。

## 真机只读冒烟（`cargo test --manifest-path backend/Cargo.toml zcode_quota::tests::local -- --ignored --nocapture`）

```
ZCode balance smoke: token=eyJh…（255 字符）
  window=GLM-5.3 used=Some(0.0) amount=Some("0 / 3000000 token") remaining=Some("剩余 3000000 token") has_reset=true
  window=GLM-5.3-Flash used=Some(0.0) amount=Some("0 / 5000000 token") remaining=Some("剩余 5000000 token") has_reset=true
ZCode balance smoke: windows=2, plan=Some("ZCode Start Plan"), status=Some("active"), expires=true, observed=true
local quota: windows=2, masked_account=true
ZCode account read-only: windows=2, plan=Some("ZCode Start Plan"), status=Some("active"), masked_account=true
test result: ok. 3 passed; 0 failed; 0 ignored
```

## 尚未决定（需要鼠鼠拍板）

- 「`ZCode Start Plan` 是免费档、额度按日重置」这类产品说明要不要显示在账号卡上（当前只透传 `plans[].name` 与 `status`，不解释档位）。
- 窗口 `key` 现在是 bucket id（例如 `bkt-…`），前端 `planCoverage.ts` 只把 `5h` / `7d` 当套餐窗口，`dockQuotaView` 因此不会把 ZCode 账号额度画进灵动岛额度环；要不要给日额度一个 `1d` 之类的稳定 key，需要先确认显示口径。
- `frontend/src/lib/remainingDisplay.ts` 的 ZCode 文案仍是「BigModel 套餐剩余 / 本机 ZCode BigModel Coding Plan API Key」，与现在 zai 走官方客户端令牌的事实不符，是否改文案待确认。
