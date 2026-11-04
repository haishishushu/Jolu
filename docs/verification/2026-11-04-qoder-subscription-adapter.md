# Qoder 国内 / 国际订阅转发适配

## 实现边界

`backend/src/gateway/adapter/qoder.rs` 从所选连接 ID 解封 Qoder 浏览器授权，按 `cn` / `global` 选择官方 OpenAPI。该实现没有读取原工具账号文件、执行真实账号验证或调用真实模型。

入口为 OpenAI Chat Completions 非流式文本桥接；模型菜单只开放官方 `auto` 自动选模，不表示已枚举账号可用的全部模型。工具调用、图片、流式及未实现的采样 / 输出控制参数显式拒绝。完整对话作为有角色标记的文本送入一次性 CLI 会话，因此它不是原生多角色 OpenAI 协议透传。

## 协议依据

Context7 先执行 `library qoder`，再查询 `/websites/qoder`：

- https://docs.qoder.com/cli/sdk/authentication ：SDK `accessToken` 是 PAT，不能拿桌面 OAuth 冒充。
- https://docs.qoder.com/cli/model ：支持 `--model auto`。
- https://docs.qoder.com/cli/cli-reference 与 https://docs.qoder.com/cli/run-in-scripts ：`--print`、`stream-json`、`--max-turns`、`--no-session-persistence`。
- https://docs.qoder.com/cli/permissions ：`--tools` 限制内置工具集。

只读核对本机官方程序源码（非账号数据）：

- `D:/Qoder/resources/app.asar` 与 `D:/Qoder CN/resources/app.asar` 中 `out/main/index.js` 的 `Sje`：`POST /api/v1/me/jobToken`，OAuth Bearer，JSON `clientId`。两版 clientId 均为 `732aef47-9cf2-46a2-95fe-4cebb5d0d1fa`。
- 同文件授权刷新使用 `POST /api/v1/deviceToken/refresh`，JSON `refresh_token`；身份接口为 `GET /api/v1/userinfo`。
- npm `@qoder-ai/qodercli` 1.1.78 的 `bundle/qodercli.js`：`QODERCLI_SITE` 选择地区；环境前缀为 `QODER_` / `QODERCN_`；`JOB_TOKEN` 解析 JSON 并进入 `loginWithJobToken`，从传入任务令牌查询身份并创建 WASM 签名上下文。它不是 PAT 路径。

这些未公开的桌面换票细节可能随官方版本改变；代码拒绝无效响应，不回退其他账号或 PAT。

## 隔离与凭据处理

- 每连接授权锁保护刷新及换票。刷新返回的新 OAuth 必须经官方身份接口核对 UID，随后 compare-and-swap 保存，防止覆盖更新的登录。
- JobToken 也核对 UID，并在换票完成及生成结果返回时检查连接密文快照。中途切换账号会丢弃旧结果。
- JobToken 每轮重新换取，不持久化；不向 CLI 传 OAuth 或 refresh token。任务令牌仅进入子进程环境，不进入命令行、提示词、返回错误或普通日志。
- HTTP 禁止重定向，有 30 秒请求超时和 1 MiB 授权响应上限；代理跟随 Jolu 网络设置。
- CLI 使用新临时目录并清空环境；额外隔离 HOME、USERPROFILE、APPDATA、LOCALAPPDATA、XDG、TEMP；禁用会话持久化、内置 Skills、MCP、工具和 Hooks；关闭 stderr。进程非零退出、超时或取消均清理临时配置，子进程设置 kill-on-drop。
- 真实返回的用量归到所选连接，不编造缺失 usage。
- 排队取得账号锁后以及启动 CLI 前复查来源开关、状态及身份指纹；等待期间关闭或暂停来源返回 503，不继续换票或生成。

## 合成验证

初始未实现测试的 RED 编译结果由主任务保存于忽略目录 `output/acceptance/qoder-adapter-red.txt`。

定向测试覆盖请求能力边界、地区和 UID、错误结果、真实 usage 字段、两地区干净进程环境、无令牌命令行、独立目录及清理、非零退出错误脱敏、超时、取消；回环 mock 覆盖所选账号 OAuth 刷新、JobToken 换取、另一个账号不变、身份不符拒绝、中途登录变化丢弃，完整 forward 流程及账号用量归账。

真实 Qoder 国内 / 国际套餐调用、官方版本兼容及实际响应样本仍未验收；不得把合成测试描述为真实模型请求成功。

最终验证：本模块 7 项均通过，含取消后清理及排队期间关闭来源；统一后端回归 812 通过、36 忽略、0 失败，日志 `output/acceptance/qoder-trae-backend-final.txt`。

## 2026-11-04 重写：按 magpie 直连 Qoder 模型接口

上文的 CLI 文本桥接已被取代。按 magpie-community/plugins `packages/qoder`（MIT；协议源自 CLIProxyAPI，MIT）改为直连：

- **模型列表**：`GET {api}/algo/api/v2/model/list?Encode=1`，COSY 签名。取 app 场景、再补 chat 场景中 app 已不列出的模型，只要启用且有具体模型键的（排除 auto / default）。条目按授权密文摘要缓存 5 分钟，`api_key` / `url` 字段不落库。读取失败时沿用上次列表，再没有则用 magpie 内置清单（国际 17 个、国内 4 个）。
- **对话**：`POST {api}/algo/api/v2/service/pro/sse/agent_chat_generation`，请求体按 magpie `qoderBody` 组装（系统提示、工具、思考档位、上下文档、任务与会话 ID、提示缓存标记、`model_config`），经乱序 Base64 编码；SSE 回复转回 OpenAI Chat，支持流式与非流式、原生工具调用和文本写出的工具调用（`text_calls.rs`）。
- **主机**：国际 `openapi.qoder.sh` / `api3.qoder.sh`，国内 `openapi.qoder.com.cn` / `gateway.qoder.com.cn`。
- **令牌**：沿用设备令牌刷新与 `/api/v1/me/jobToken` 换票，任务令牌仅存内存并在到期前 5 分钟重换；国内版换票返回 4xx 时改用设备令牌签名。模型接口返回登录失效时换一次票重发。
- **设备标识**：登录时上报的 `machine_id` 现在保存到授权中。旧授权没有保存，按连接与账号派生一个稳定值；如果 Qoder 因此拒绝，需重新登录该账号。
- **错误**：排队（10605）返回 503 并带 Retry-After（不在网关内等待）；额度用尽返回 429，请求被拒返回 400，上游失败返回 502。

新增依赖 `rsa` 0.9（仅用公钥加密，不涉及私钥解密的时序问题）与 `md-5` 0.10；Context7 已核对 `DecodePublicKey::from_public_key_pem` 用法。

验证：编码与签名测试向量由 magpie 原实现在 node 中生成，比对一致；网关模块 166 项通过（回环 mock 覆盖设备令牌刷新、换票、国内版设备令牌回退、身份不符、中途切号、模型列表、流式与非流式对话、排队映射、关闭来源）。真实 Qoder 账号的列表与对话尚未验收。
