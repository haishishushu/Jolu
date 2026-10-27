# Claude 本机配置读取优先级修复

## 原因

只读核对确认：用户 `.claude/settings.json` 的服务地址为 `https://bpi.kuajingengine.com`，而 Jolu 启动时继承的环境仍是 BigModel。原实现先读继承环境，再读配置文件，所以保存地址与密钥均停留在旧来源，并错误地标为当前连接。

官方资料经 Context7 查询核对：普通 CLI 会话中，settings.env 覆盖同名 shell 变量，空字符串可以取消旧变量；认证选择优先 ANTHROPIC_AUTH_TOKEN，再选 ANTHROPIC_API_KEY。Claude Desktop 或自托管 runner 会话的启动环境另有规则，本轮修复针对本机 CLI 用户配置读取，不声称能识别所有项目或托管会话的独立覆盖。

参考：[环境变量优先级](https://code.claude.com/docs/en/env-vars)、[env 设置](https://code.claude.com/docs/en/settings-reference#how-env-values-interact-with-your-shell)、[认证优先级](https://code.claude.com/docs/en/authentication)。

## 改动总结

- Claude 用户配置只读取一次快照，再统一解析地址与凭证；同名设置覆盖继承变量，显式空值不会回退到旧变量。
- 读取失败、JSON 损坏或 env 结构错误时停止本次读取，保留已保存连接，不冒认旧环境。
- 本机检测、发现、读取、新增与同步复用同一解析逻辑；无显式密钥的新连接准备也采用同一凭证/地址快照。显式填写的凭证不触发本机读取。
- 沿用现有单连接自动同步，更新原连接的地址及凭证，保留连接 ID、账号名及历史数据。未修改 Claude 配置、系统环境或 Git 状态。

## 验证

- 新的子进程隔离回归先复现失败：旧启动器凭证/地址遮蔽设置文件。修复后覆盖配置优先、逐变量继承、无文件、空值取消和损坏文件，所有读取入口保持一致。
- creds 模块 4 项与 connections 模块 41 项测试通过。
- 真实只读余额烟测通过，返回 ok、有效 USD 余额字段；未调用模型、未输出密钥或余额金额。
- 桌面开发程序已重新编译并启动。只读检查持久化连接：Claude API 地址已同步为 bpi.kuajingengine.com，保存凭证与当前 settings.json 一致，状态 connected。
- 差异空白检查通过。仅检查并输出必要的来源主机、布尔匹配结果和查询状态，不保存凭证诊断副本。
