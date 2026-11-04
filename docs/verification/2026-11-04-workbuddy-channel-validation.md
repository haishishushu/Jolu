# WorkBuddy 渠道校验 11128 修复

## 结论与依据

Claude Code 经 Jolu 调用 WorkBuddy `hy3` 返回 `400 / 11128 / Illegal API invocation from an unapproved channel`。当前适配器已具备客户端请求头，但只移植了 magpie 的头部处理，没有移植请求体的 `FLAGGED` / `unflag` / `systemOn`。不能仅凭“渠道校验”文案认定缺少认证头。

按鼠鼠指定的 [yetone/magpie](https://github.com/yetone/magpie/tree/8a807968059b63561bab80a027ec922b1f21c3df) 核对：`internal/provider/workbuddy.go` 明确 WorkBuddy 生成已由社区插件承接。实际对照 [插件源码](https://github.com/magpie-community/plugins/blob/9bba5fedc2a9aa3aeeaa5571f0052ea1c89dac9c/packages/workbuddy/index.mjs) 与同版本 `REFUSED.md`，不是只看已退役的内置实现。插件记录了特定 Agent 提示语句与此错误的对应关系。

Context7 按 library → docs 查询 `/websites/codebuddy_cn_workbuddy`，未找到 11128 的官方公开规则。另只读检查本机 WorkBuddy 5.8.3 的 `codebuddy-lite-wb.mjs` 中 ModelProvider、AuthenticationHttpInterceptor、UserAgentHttpInterceptor；这些客户端代码本身不能证明服务器拒绝的具体条件。最终原因由下面的真实对照确认。

## 改动总结

- 新增 `workbuddy_request.rs`，只处理发往 WorkBuddy 的请求副本；按插件的窄匹配规则移除 Anthropic 计费标记、改写已知提示语句，保留 Agent 身份与其他内容。不修改原始会话文件。
- 支持字符串和文本块；保留图片、文本块其他字段、工具调用参数、工具定义和消息顺序。只由计费标记构成的空 system 消息移除后补默认 system。
- 按 `systemOn` 统一 developer → system、补首条 system、max_completion_tokens → max_tokens（已有值优先）及 assistant reasoning 字段。
- `workbuddy.rs` 在发往直通适配器前调用规范化；不改变其他供应商的协议转换。MIT 来源已补入 `docs/licenses/magpie-community-plugins.md`。

## 验证

1. 先修改真实转发路径的回环测试，使用 Anthropic → Chat 转换后的 Claude Code 请求；观察到计费标记和身份原句仍在上游请求中，测试失败。
2. 修复后 WorkBuddy 相关 8 项测试通过，包括所选账号隔离、实际发出的请求、文本块/图片/工具参数保留、中文 UTF-8、重复处理稳定性及已有目录测试。
3. 手动运行忽略测试 `local_channel_prompt_probe`：只读打开鼠鼠截图对应连接的数据库，密文只在内存中解封，测试归账使用内存数据库，没有改动真实账号和统计。两次请求均为同一账号、`hy3`、`stream: true`、`max_tokens: 32`，只发送合成的身份提示和 `Reply with OK.`，不发送用户会话。
   - 原始 Claude Code 身份原句：HTTP **400**，业务码 **11128**。
   - 经过生产适配器请求规范化：HTTP **200**，收到 **10 个含 choices 的 SSE 块**。
   - 测试 **1 项通过**；输出只有状态码、业务码和块数量，不输出授权或回复内容。
4. `git diff --check` 通过（工作区原有换行提示不影响结果）。

真实对照只证明此次国内版 hy3 的提示语句问题已解决，不代表国际版、所有模型、完整 Claude Code 工具任务都完成真实验收。未移植插件的全部工具转换、非流式适配或 systemAsUser 重试策略，后续不同错误需分别定位。没有构建安装包，没有 Git 暂存、提交或推送。已运行的旧二进制需重新启动开发构建后才会使用修复。
