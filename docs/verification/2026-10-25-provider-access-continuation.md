# 供应商账号接入续轮验收

日期：2026-10-25。本记录承接 account-project-audit，区分真实账号查询、合成接口与未验证条件；未执行 Git 写操作。

## 改动总结

- Trae：新增国内与国际独立来源，复用本机正常登录，只读查询官方额度接口。国内采用 v2，避免 v1 全零旧数据误导；不把额度期限当订阅到期。真实查询两版均成功，详细证据见 `2026-10-25-trae-account-access.md`。
- ZCode：新增独立账号来源，不依赖会话数据库；复用官方客户端保存的 BigModel 登录查询个人订阅，有效个人套餐继续查询额度。真实服务返回个人套餐未开通；界面区分未开通、有效、已到期。国际 ZAI、团队与 Start Plan 不猜接口，详细证据见 `2026-10-25-zcode-account-access.md`。
- 自定义余额：接通卡片、单卡刷新和后端；支持 sub2api、new-api/one-api 及 DeepSeek、硅基流动国内、OpenRouter 的精确官方路由。地址保持同源，错误与未知值不当零，失败保留旧值并标记，不支持清旧值。OpenRouter 管理 Key 权限与模型 Key 权限独立。
- 账号归属修复：WorkBuddy 国内/国际严格按来源选择，Qoder 国内不借国际数据；Trae、WorkBuddy、ZCode 以本地登录指纹绑定缓存，缓存命中也复核身份，退出或换号立即失效，迟到响应无法写回。无法可靠取得身份的旧本机查询不再跨请求缓存；有身份的查询保留合并请求与失败退避。
- 统计导入修复：损坏的指标行连带排除对应请求，防止部分导入造成错误统计；覆盖行序变化。其他会话、设置和存储审查见 `2026-10-25-continuing-project-review.md`。
- 新增 aes、aes-gcm、cbc、base64 依赖，用于只读解码原工具已保存的登录，不写回授权。凭证仅留后端，未输出到列表、日志或报告。

## 验证结果

| 验证 | 结果与证据 |
|---|---|
| 前端纯逻辑、hooks、发布脚本 | 217 通过，0 失败；`output/provider-continuation-frontend-tests.log` |
| 后端（含 handoff-eval） | 555 通过，0 失败，21 忽略；`output/provider-continuation-backend-tests.log` |
| 类型检查、前端生产构建、桌面 debug 构建 | 通过；对应 `output/provider-continuation-*.log` |
| 缓存身份回归 | 新用例先失败后通过，连同请求去重/退避 4 项通过；`output/native-identity-cache-green.log` |
| 账号真实组件 | 额度与余额并存、真实零、刷新失败旧值、不支持清值、单卡范围通过；`output/custom-balance-components.log` |
| 隔离桌面自定义余额 | 五类余额状态、列表不泄露凭证、本机账号隔离保护通过；`output/custom-balance-desktop.log` |
| 隔离桌面主窗口 | 七页加载通过，未发现脚本检查的异常；`output/provider-continuation-desktop-smoke.log` |
| UI 对照 | Pencil 只读查看账号与能力画板，实际浅深组件截图已检查；沿用余额卡片布局。自定义余额业务规则依据鼠鼠新要求覆盖旧 29A，不修改设计稿 |

首轮全量测试发现合成 TCP 服务 accepted socket 的 WouldBlock，显式设为阻塞并放宽读超时后重跑全量通过；不是把失败忽略。21 项 ignored 不算通过，真实只读 Trae / ZCode 验证单独记录。官方余额仅做解析与合成 HTTP 验证，未声称真实 Key 已验。

## 剩余条件

- 缺登录/样本：Gemini、Qoder、OpenCode 订阅、Claude 完整订阅授权、Trae 付费/企业、ZCode 付费个人/团队/Start Plan/国际账号。具体逐项见未验证清单。
- 自定义并非任意服务自动可查；未适配协议明确不支持。DeepSeek 多币种不合并为单一金额；OpenRouter 需管理 Key；硅基流动只确认国内官方路由。
- 正式更新仍待签名与 Release/latest.json；默认构建停用更新。Windows Sandbox 未安装，跨版本安装/重启/回滚不视作已验证。
- 本轮未修改真实登录、未使用重置卡、未发模型请求，未删除历史数据。验收使用带标记的隔离目录，测试证据保留在 output。
