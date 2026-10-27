# Trae 国内签到与积分统计核实

结论：当前账号官方积分汇总明确返回总量 4700、已用 0，可以显示“可用额度（计算）4700”；官方没有直接返回剩余积分。签到状态接口仅能证明今日状态与今日奖励，不能补造周、月或终身累计签到历史。

## 官方依据与范围

先执行 Context7 库解析 `library Trae`，再查询 `/bytedance/trae-agent` 的 IDE 签到/积分协议；该库没有对应文档。随后核对本机 CN 3.4.4 官方发行文件与当前官网静态代码。

- `%LOCALAPPDATA%/Programs/Trae CN/resources/app/out/main.js`：每日签到使用 `/trae/api/v2/ug/checkin_credits/status` 与 `claim`。状态字段是 `enable`、`checked_in`、`did_checked_in`、`credits`；未发现普通签到历史读取接口。
- 同安装目录 `out/vs/workbench/workbench.desktop.main.js`：`V0r` 从每个权益包的 `entitlement_base_info.quota.credits_limit` 和 `usage.credits_amount` 计算余额，再汇总。官方实现缺失用量时默认零，Jolu 不沿用这一补零行为；逐包用量未知时保持未知。国内额度请求还带 `require_usage=true`、`req_source=1`。
- 当前官网 `dashboard/page.3b91976b9b.js`：官网同样以额度减已用计算积分余额；额度卡读取 `/trae/api/v2/pay/user_current_entitlement_list`，参数 `require_usage=true, full_data=true`。`GetExpiredEnts` 指向 `/trae/api/v2/pay/expired_ents`，界面明确为近期过期积分，响应 `expired_ent_list` 用于展示积分名称与到期时间，不是普通签到的全历史接口。
- 当前官网生成客户端 `58867.935d466a27.js`：普通签到只有状态、领取；还包含测试回滚、测试造历史及运维接口。本次未调用这些写入或测试接口，也不以其存在推断个人账号有读取权限。
- [官方订阅管理](https://docs.trae.cn/ide_subscription-management) 说明可在用量管理中查看总可用积分、积分类型和用量明细；[套餐与计费](https://docs.trae.cn/ide_plans-and-billing) 说明积分来源含注册、登录、签到、邀请与会员等，因此账号积分总量不能当成签到累计。每日奖励由实时状态接口决定，不硬编码网站缓存中的数值。

官网静态代码只保存到被 Git 忽略的 `output/research-trae-checkin/`。不把凭证、UID、设备值或原始接口响应写入该目录。

## 真实只读验证

`trae_quota::tests::cn_credit_fields_read_only_smoke` 使用本机已授权国内账号，分别发送官方额度接口支持的 `full_data=false` 与 `true` 查询；两次结果一致：

| 范围 | 官方总量 | 官方已用 | 官方直接剩余 | 可计算剩余 |
| --- | ---: | ---: | --- | ---: |
| `usage_summary` 积分汇总 | 4700 | 0 | 未返回 | 4700 |
| 原始权益包 1 | 4000 | 未返回 | 未返回 | 未知 |
| 原始权益包 2 | 非积分包 | 未返回 | 未返回 | 未知 |
| 原始权益包 3 | 500 | 未返回 | 未返回 | 未知 |
| 原始权益包 4 | 100 | 未返回 | 未返回 | 未知 |
| 原始权益包 5 | 100 | 未返回 | 未返回 | 未知 |

响应未含 `history` 或 `records`。两个 100 积分包带 `package_source_type=9` 和有效期字段，但未核实该枚举的完整官方定义，也没有证明当前权益包列表覆盖全部签到历史，因此不从包数反推签到天数，不由 100×天数补造奖励。

本次仅调用只读额度接口。没有领取、重试领取、切号、刷新或写回原工具登录，也没有调用测试和运维接口。脱敏输出仅为数值、包数量和字段是否存在。

## 数据合约与改动总结

`trae_quota.rs` 向 `AccountMetadata.amount_ranges` 提供独立的“积分汇总”及每个已知权益包范围，保留官方总量和已用数值；官方直接剩余 `remaining` 保持为空，计算值单独进入 `calculated_remaining`。只有总量、已用都存在、非负、有限且已用不超总量时才计算；真实零可以计算为零。汇总与包不得再次相加。

旧额度窗口仍保持有汇总时显示汇总的行为；新范围独立保留所有已知包，避免先前有汇总时丢弃包级信息。快速请求已用同时识别官方当前 `premium_model_fast_amount` 与此前 `premium_model_fast_request_usage` 字段。

`checkin_trae.rs` 的有效状态附带 `scope=today` 的官方统计说明，周、月、全部统计值均为空，明确当前签到接口的覆盖限制；Jolu 本地记录由公共统计层另行展示。

## 回归验证

- 新增两项额度范围回归先出现预期红灯：旧解析没有 `amount_ranges`，无法保留汇总、每个包及计算来源。
- 新增签到覆盖范围回归先出现预期红灯：旧状态没有官方历史范围说明；实现后转绿。
- `cargo test --manifest-path backend/Cargo.toml trae_quota::tests --lib`：10 通过，2 忽略，0 失败。
- `cargo test --manifest-path backend/Cargo.toml checkin_trae::tests --lib`：8 通过，2 忽略，0 失败。
- `cargo test --manifest-path backend/Cargo.toml trae_quota::tests::cn_credit_fields_read_only_smoke --lib -- --ignored --exact --nocapture`：1 通过，0 失败。最终解析真实响应得到 5 条数值范围（1 条汇总及 4 条积分包），汇总的计算可用额度为 4700，直接来源剩余仍为空；其余原始包的已用缺失，未推算余额。原响应另有 1 个没有可识别数值额度的包，不为它补零。
- `git diff --check -- backend/src/trae_quota.rs` 通过。未执行 Git 写操作和无关全量测试。

## 审查补充：不限量与账号标签快照

审查确认两个遗漏：新的原始范围会过滤掉总量为 `-1` 的不限量权益，尤其没有已用数值时会成为空列表；签到账号对象未携带同一份登录快照里的可见标签，公共刷新层无法在账号切换时更新名称。

`trae_quota.rs` 现把官方 `credits_limit` 或请求额度限制的 `-1` 显式保留为 `unlimited=true`，总量与剩余数值均为空、已用按来源保留，不做余额相减。有汇总时仍保留这些独立不限量包。

`checkin_trae.rs` 的 `Account` 现持有私有 `account_label`：本机来源与 UID、token 在同一次 `account_fields` 中读取邮箱，其次用户名；浏览器来源读取该 Session 的 `account`。统一修剪空白并拒绝控制字符及超长标签，缺失时返回 `None`，不回退到 UID。公共刷新层使用同一账号对象的 getter 更新标签，避免另读原工具账号而串号。

两项新增回归均先观察到预期断言失败，再实现修复。最终定向结果：`trae_quota::tests` 11 通过、2 忽略；`checkin_trae::tests` 9 通过、2 忽略；均 0 失败。此轮未重跑真实网络查询。
