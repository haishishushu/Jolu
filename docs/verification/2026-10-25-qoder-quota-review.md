# Qoder 本机登录额度接入

日期：2026-10-25。

## 改动总结

- 新增 `backend/src/qoder_quota.rs`。复用已安装的 Qoder CLI，通过官方 SDK 同款控制协议执行 `initialize` 和 `get_usage_info`，不发 `user` 帧，不实现或猜测私有 HTTP 端点。
- 解析套餐 `userType`、总用量百分比、套餐额度、加购额度、组织额度。真实零保留；缺字段和越界数字保持未知；会话 `session.total_credits` 不能冒充账号额度。
- `expiresAt` 不当作额度重置时间。本轮没有将缺少明确单位证据的期限数字转换成展示日期。
- 仅原工具登录可用时查询。旧版 CLI、缺失运行时或不识别的包装器明确返回不支持；登录失败、查询失败、限流和权限不足单独表达。
- 路由、能力目录和前端整合由主任务完成，本文不预先宣称整体界面验收完成。

## 官方依据与实现边界

- Context7 先 `library qoder sdk`，再 `docs /websites/qoder` 查询 `getUsageInfo` 和相关配置。
- 官方文档：[Cost and Usage](https://docs.qoder.com/cli/sdk/cost-usage)、[TypeScript Reference](https://docs.qoder.com/cli/sdk/references-typescript)。明确可在无用户提示、无 Agent 回合时直接读取额度，账号额度与会话用量彼此独立。
- 核对本机官方 npm CLI 1.1.78，以及官方 SDK 1.0.56 发布包源码和类型。SDK 的本地登录认证类型 `qodercli` 标注为只读复用本机登录。
- `RuntimeLaunchOptionsBuilder.buildEnv` 会写 `QODER_AGENT_SDK_ENTRYPOINT=sdk-ts`、`QODER_AGENT_SDK_VERSION=1.0.56`；本地登录通过一次性文件 `{ "type": "qodercli" }` 和 `QODER_SDK_AUTH_PAYLOAD_FILE` 选择，不复制 token。
- SDK 序列化控制请求时保留 `request.type` 并补同值 `request.subtype`。本实现按此发帧，以 `request_id` 匹配回应；收到 assistant/user/反向控制请求立即终止，不继续代理回合。
- 先检查 CLI 版本，低于 1.1.78 不启动控制查询。查询运行在本次临时目录，空设置来源、禁用 hooks、空工具、严格空 MCP 配置、空插件和 skills、关闭记忆/技能生成、不保存会话。
- 不引入 SDK 或 Node 打包依赖：原生 CLI 直接调用；Windows npm 安装复用它已依赖的本机 Node，绕过 `.cmd` 包装器，不经 shell 拼接命令。
- 版本读取最多 10 秒，控制查询最多 30 秒，输出总量最多 2 MiB。退出各路径均终止/回收自建子进程并清理专属临时目录。原始 stdout/stderr 不写日志或文件，错误仅返回固定脱敏说明。

## 本轮验证

- 最初 4 项解析/协议回归在占位实现上全部失败，再实现后全部通过。
- `cargo test --manifest-path backend/Cargo.toml qoder_quota::tests --lib -- --include-ignored --nocapture`：6 项全部通过，包含 4 项纯测试、1 项合成子进程协议往返和 1 项授权本机只读查询。
- 合成子进程核对实际 SDK 环境变量、认证选择文件、空设置来源、空工具/MCP、禁用 hooks、空插件/skills、无记忆生成、无会话持久化；仅接受两个 control_request，返回合成零额度，验证真实 Rust 进程传输与解析。
- 本机只读结果：`state=unauthorized`，`windows=0`。本机 Qoder CLI 登录不可用，未登录或修改认证，不能声称已验证真实账号成功额度。
- 编译仅显示既有 `coding_plan.rs` 测试模块未使用导入警告，本模块没有警告。

## 探索阶段异常与限制

- 首次一次性探针只设置 stream-json 参数，遗漏 SDK ENTRYPOINT，CLI 返回了 system / assistant / result，没有 control_response。探针结束后停止继续请求；没有保存该次 result 内的数字，因此**无法证明这次是否产生模型用量**。该探针未写入产品实现。
- 只读检查 CLI 源码后证实：仅流格式参数不足以进入 SDK 模式，ENTRYPOINT 是必要条件。补 ENTRYPOINT 后、尚未提供 auth payload 的探针直接失败退出；完整本地认证选择配置的后续探针返回登录错误，明确 `num_turns=0`。
- 产品代码固定完整 SDK 启动条件，并用合成子进程回归防止遗漏。成功账号响应目前只有官方类型与合成协议验证，仍需有有效本机 Qoder 登录的环境验证真实成功数据。
- 未进行 Git 写操作、自动登录或实际模型任务。
