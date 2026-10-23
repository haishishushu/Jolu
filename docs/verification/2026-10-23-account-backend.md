# 账号额度、订阅信息与完整授权切换验收

## 改动总结

### 额度与订阅元数据

- `QuotaState::Ok` 新增可选 `account`。Codex 重置卡数量随原有额度 GET 一起解析，不另发重复额度请求，不兑换重置卡。
- 额度按 `limit_window_seconds` 区分 5 小时与周窗口，缺少窗口不填零。卡数的真实零、字段缺失与无效数据分别保留为 `0`、`null`、未提供；无效可选元数据不影响有效额度。
- 到期仅读取与连接 access token 完全匹配的本机 ID token 中 `chatgpt_subscription_active_until`；`chatgpt_subscription_last_checked` 提供观察时间。不使用 JWT `exp`，不把历史记录当实时状态。
- 历史记录返回 `subscription_source=local_snapshot`、`subscription_status=unknown`。不从套餐名称、额度查询成功或历史到期记录推断实时续费、欠费或取消状态。

### Codex 设置为使用

- 新增 `auth_snapshots` 表，保存 Windows DPAPI 当前用户加密的完整授权；连接 ID 作为额外熵，无法把另一行密文移植为本行授权。外键与删除触发器保证删连接时清理其快照，原连接和统计不重建。
- 添加连接和同步凭证时，仅捕获 access token 完全匹配且 access、refresh、ID token、account ID 完整的本机授权。新增连接与快照用 SAVEPOINT 保证失败一起回滚。
- 从订阅切换到 API 前，保全当前注册账号。CLI 已轮换 token 时，以工作区 ID 与 ID token 中用户 ID 共同确认身份，更新该账号完整快照和查询凭证。缺少可核实身份时拒绝覆盖并引导重新同步。
- 切回订阅恢复整组授权，禁止与另一账号混合 access/refresh/ID token。旧连接无完整快照时明确说明重新登录并同步，不假装切换成功。
- 多窗口共用进程切换锁；单个 blocking 任务内持有数据库锁到 CLI 文件写入及连接状态更新结束，防止另一窗口在启用过程中删除或替换连接。
- Codex 未创建 `config.toml` 或 `auth.json` 时可创建。写入前保留最初读取字节并比较，检测到外部变更就拒绝覆盖；第二份写入失败时回滚第一份，原本不存在的文件恢复为不存在。
- CLI 新备份为当前 Windows 用户加密的 `.bak.dpapi`，旧备份不删除。提供恢复脚本，只解密到用户指定的新文件，不覆盖 CLI 配置。
- 隔离验收入口直接拒绝写外部 CLI 配置。实际功能测试只用合成数据与独立临时目录。

### 同步身份保护

- 单个订阅账号也不能被不同本机登录静默覆盖。
- Codex 只有完整快照与新授权的工作区 ID、用户 ID 都匹配才允许 token 轮换；同工作区的不同用户拒绝。未知身份保持原连接和邮箱。
- Claude OAuth token 无已核实的跨文件身份绑定，本阶段保留切换当前完整授权、清除 API 覆盖的能力；不把另一个账号的凭证与 `.claude.json` 资料拼接成多账号切换。
- 读取、检测与写入统一遵守 `CODEX_HOME` / `CLAUDE_CONFIG_DIR`。设置后 Claude 的 `.claude.json` 在配置目录内，默认仍在用户主目录。

## 来源核实

先使用 Context7 解析 `openai codex` 为 `/openai/codex`，查询额度与订阅字段；Windows DPAPI 使用 `/microsoft/windows-rs` 查询，再核对本机 `windows-sys 0.61.2` 声明。使用了 `security-review` 技能审查凭证边界。

官方资料：

- https://github.com/openai/codex/blob/main/codex-rs/backend-client/src/client/rate_limit_resets.rs
- https://github.com/openai/codex/blob/main/codex-rs/backend-client/src/types.rs
- https://github.com/openai/codex/blob/main/codex-rs/login/src/token_data.rs
- https://github.com/openai/codex/blob/main/codex-rs/model-provider/src/auth.rs
- https://code.claude.com/docs/en/mcp-quickstart
- https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata

## 本机只读核实

- 当前已登录 Codex GET `/backend-api/wham/usage` 成功，响应确有 `rate_limit_reset_credits.available_count` 和 `applicable_available_count`，真实零保留。未记录完整响应、令牌、邮箱或账号 ID。
- 本机 ID token 存在明确订阅起止及上次检查字段。
- GET `/backend-api/subscriptions?account_id=…` 返回 HTTP 403，未加入生产刷新链路。
- 没有兑换重置卡、发送模型任务、充值/付款或改真实 CLI 登录配置。

## 验证结果

- 重置卡新回归先红（旧实现返回 null），修复后 Codex 专项 4 通过、1 个联网测试默认忽略。
- 订阅到期快照身份匹配与不使用 JWT exp：1 通过。
- 首轮授权加密/导出排除/删除清理：2 通过。
- 首轮 CLI 临时目录往返、无快照拒绝、缺失文件回滚、多窗口串行化：6 通过。
- 连接回归首次 31 通过、1 失败：旧测试期望跨身份同步清空邮箱，已按新要求改为拒绝覆盖并保留原身份。
- Windows PowerShell 5.1 `-File scripts/restore-cli-backup.ps1` 使用合成 DPAPI 备份恢复成功；UTF-8 BOM 中文正常；PowerShell 7 恢复及拒绝覆盖已有目标验证通过。中途写失败仅清理本次 CreateNew 创建的文件。
- 后续独立复审补充的轮换身份、跨用户拒绝、读取后变更保护、空 DPAPI 内容与隔离拒绝测试由最终联合专项及全量验证确认，最终结果追加于下方。

导出已核实为 `db::export_backup` 的请求/统计 JSON 白名单，不是整库文件复制；加密授权快照不会进入导出。数据库迁移仅增加表和触发器。

## 同用户恢复加密 CLI 备份

使用创建备份时的 Windows 用户，在仓库根目录运行（替换示例路径）：

```powershell
powershell.exe -NoProfile -File .\scripts\restore-cli-backup.ps1 -BackupPath 'C:\path\cli-backups\auth.json.123456.bak.dpapi' -DestinationPath 'C:\path\auth.restored.json'
```

目标必须不存在。脚本只恢复到新文件，确认内容后再由用户决定是否替换 CLI 配置；不会打印凭证内容。额外熵来自原文件名，备份文件名应保持原样。恢复后的文件含凭证，不能放进仓库、日志或诊断报告。

## 整合验证补充

- 最终账号切换与当前连接判定整合后，主代理执行 `cargo test --manifest-path backend/Cargo.toml --lib --quiet`：460 通过、0 失败、12 忽略。忽略项需要真实账号或外部环境，不计作通过。
- 使用独立应用标识、5193 开发地址与独立 target 构建桌面成功；链接器输出创建 `.dll.lib` 的提示，不影响退出码。
- 新隔离目录中七页桌面冒烟全部通过，无读取/扫描/刷新失败、无控制台 error、无未捕获异常。这只覆盖基础导航与隔离数据，完整授权切换使用上述合成文件测试，不写真实登录。
- 真实桌面视觉复核时再次观察到 Windows 锁屏，已暂停原生桌面操作并请求解锁；不把此部分记为通过。
- 上述全量后，独立复审发现 Codex 活跃 profile 和官方 provider 覆写可能使“设置为使用”仍连接旧服务，已重新进入修复；最终版本需追加相应回归结果。

## 尚存能力边界

- 实时订阅接口对当前 OAuth 凭证返回 403；只能展示明确的未获取状态和带来源的历史到期记录。
- 未确认的其他供应商重置卡/到期能力保持未提供。
- Claude 跨账号完整切换缺少可验证身份来源，保持拒绝，不能冒充已实现。
- Windows 用户保护的快照不能在另一 Windows 用户下直接解密；用户变更后需要重新同步。

## 最终供应商配置优先级复审

发现并修复：只修改顶层 `model_provider` 时，当前默认 profile 的旧供应商仍可生效；切回 `openai` 也可能残留自定义端点或鉴权覆盖。

- 同步更新顶层和当前默认 profile 的供应商选择，保留模型、推理参数和其他 profiles。
- 切回官方时删除 `model_providers.openai` 中端点、环境 Key、固定 bearer、命令授权、网关 OAuth、签名和请求头等覆盖，保留非鉴权设置，恢复 Responses 协议和官方授权来源。
- 当前 profile 缺失、包含无法确认的嵌套供应商配置，或采用 keyring/auto/ephemeral 凭据存储时，在写 `auth.json` 之前明确拒绝。
- 检测到 `CODEX_API_KEY` 环境覆盖，或官方切换存在 `OPENAI_BASE_URL` 环境覆盖时拒绝，避免宣称目标配置已生效。
- 用户另行传入的 CLI 参数、受管要求和项目层配置仍由 Codex 处理；本应用只切换本机默认用户配置。

Context7 核对官方配置层级、`ModelProviderInfo` 和凭据存储枚举：`/openai/codex`，官方源码 `codex-rs/config/src/loader/README.md`、`codex-rs/model-provider-info/src/lib.rs`、`codex-rs/config/src/types.rs`。

profile 回归先红（实际仍为 old，期望 jolu），修复后 `cargo test --manifest-path backend/Cargo.toml cli_apply:: --lib` **11 项通过**，包括错误配置下 auth/config/备份均未变化的磁盘测试。未改真实 CLI 文件。

当前账号识别补充同源约束：有效 `cli_auth_credentials_store` 非 `file` 时，不认定 `auth.json` 的遗留凭证为当前账号；遵守 profile 覆盖全局的顺序。新回归先红后绿，1 项通过，覆盖 keyring/auto/ephemeral、profile 覆盖及显式 file 恢复识别。

最后整合：`cargo test --manifest-path backend/Cargo.toml --features handoff-eval --quiet` 471 通过、0 失败、12 忽略，退出码 0。此结果覆盖上述最终复审修复及评测工具，替代前面的中间版本测试统计。
