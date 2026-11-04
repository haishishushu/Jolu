# Trae 订阅模型适配：静态依据与隔离验证范围

当前对话实现已由旧 SDK `ChatCompletion` 迁至参考 magpie 插件的 `llm_utils_chat`，支持账号目录选路、Chat / Messages 流式与非流式转换。尚未发送真实账号、真实模型请求；接口权限、套餐覆盖和模型可用性留待鼠鼠另行授权真实验收。下文旧 SDK 与目录阶段保留为历史记录，当前行为以末尾「对话重写阶段」为准。

## 官方程序静态依据

仅只读检查以下安装目录中的程序文件，没有读取用户数据目录、登录数据库、Cookie 或原工具真实凭据：

- 国内：Trae CN 安装目录下的 `resources/app`。
- 国际：Trae 国际版安装目录下的 `resources/app`。

两版共同的 SDK 相对路径为 `node_modules/@byted-icube/ai-modules-chat/dist/index.mjs`。定位采用符号及下列 UTF-8 解码后的字符偏移；偏移只适用于本次核对的程序，不作为运行时依赖。

| 定位 | 已核实内容 |
| --- | --- |
| `ChatCompletion(e,t)`，字符偏移 420820 | `genBaseURL("/api/ide/v1/chat_completion")`，`method:"POST"`，参数直接作为 JSON body |
| `function g(e,t={},r)`，国内字符偏移 552866 | 创建通用请求 wrapper，使用 `Content-Type: application/json`，成功后直接 `response.json()` |
| `Authorization:`，字符偏移 553799 | `Authorization: \`Cloud-IDE-JWT ${e}\``，这是 **Authorization 请求头的值**，不是名为 `Cloud-IDE-JWT` 的请求头 |
| wrapper 后的 `createServices` | `copilotService` 使用上述 wrapper，并将 `jwt` 作为 wrapper 第一个参数 |
| `product.json` → `bootConfig.agent.trae`、`bootConfig.account.trae` | 登录域名和模型域名不同，分别按地区配置 |

SDK 的 ChatCompletion 请求字段包括 `model`、`messages`、`max_tokens`、`temperature`、`top_p`、`n`、`stream`、`stop`、`presence_penalty`、`frequency_penalty`、`functions`、`function_call`、`tools`、`tool_choice`、`session_id`、`Request`、`disable_antidirt`。Jolu 只透传其中公开调用参数，自己生成 `session_id`；拒绝调用方提供 `Request`、`session_id`、`disable_antidirt` 或其他未知顶层字段。

国内 SDK SHA-256：`7d198baedf880bbb36eefb9aeb70aa6697b76f8c952d4d60db1e30f609bccc7e`。国内 `product.json` 的 commit 为 `871bca3bb55e7a37af74c1eef2a9696f29aa7b26`。其中 `version: 1.107.1` 是该文件的编辑器版本字段，不应误写成 Trae 产品版本。

国际 SDK SHA-256：`6b1749b10e1fded8f2c5c70ae5688d9bb3ea89aa210947ff47b4e803ed88dce8`；国际 `product.json` 的 commit 为 `77f41624cc0c27d6d65bafb87a3150742bcc77cc`。

静态 SDK 未提供可核实的返回类型声明或该方法的业务消费代码。因此实现没有假定未知包络等价于 OpenAI 响应：只接受非空 `choices` 数组及合法 `message.role=assistant`，正文或结构完整的函数工具调用；未知包络、流式 delta 等返回明确 502。该运行时形状检查不是对真实服务响应结构的证明。

## 地区与授权隔离

| 保存授权的 edition / host | 固定模型域名 |
| --- | --- |
| `cn` / `https://api.trae.cn` | `https://trae-api-cn.mchost.guru` |
| `global` / `https://growsg-normal.trae.ai` | `https://coresg-normal.trae.ai` |
| `global` / `https://grow-normal.traeapi.us` | `https://core-normal.traeapi.us` |

不接受跨地区 host、任意 host、尾斜杠变体或由调用方提供的模型地址。账号身份检查沿用已核对的 Trae 登录协议：在所选登录 host 上 POST `/cloudide/api/v3/trae/GetUserInfo`，使用 `x-cloudide-token`，核对 `Result.UserID` 与保存的 `session.user_id`。模型调用则使用前述 `Authorization: Cloud-IDE-JWT <token>`。

授权仅按 `connection_id` 解封；验证有效期及身份字段；请求前、响应头到达后、完整响应体读取后复查密文快照。授权变化则丢弃旧响应，不刷新覆盖新登录。每次请求创建独立会话 ID，不复用其他账号的会话。

取得订阅账号锁后以及模型请求发送前复查来源开关、状态和指纹。排队期间停用来源的请求返回 503，不能继续访问身份接口或模型端点；已用新增测试先复现旧实现继续返回 200 的问题。

客户端遵循 Jolu 当前联网方式：Direct 关闭代理，Manual 使用配置代理，System 使用系统代理解析结果，未发现显式系统代理时保留 reqwest 系统行为。所有模式均禁止重定向。隔离测试的本机服务器强制不走代理。

## 明确限制

- 模型必须由用户明确填写该账号的模型 ID；不生成固定模型清单，不将未知目录响应当作可用模型。
- 仅支持 OpenAI Chat Completions 非流式调用；`stream=true` 明确拒绝。未确认私有 `llm_raw_chat` 的 SSE 事件和增量语义，不做猜测转换。
- 连接超时 15 秒，身份检查 30 秒，模型请求总超时 180 秒；模型 JSON 响应上限 8 MiB。
- 上游非成功状态保留 HTTP 状态及有效数值 Retry-After，使用安全错误说明，不透出原始错误正文、令牌或内部 JSON。
- 仅将上游实际返回的用量字段记入所选连接，不推算未知 token 数。

## 验证记录

先写 6 项定向测试和失败骨架，执行 `cargo test --manifest-path backend/Cargo.toml --lib gateway::adapter::trae::tests -- --nocapture`，确认 6 项均因缺少实现失败，再替换实现。

测试覆盖地区映射、加密连接身份隔离、过期授权、请求白名单、不同请求独立会话、未知响应包络、仅向本机 fixture 发送完整 HTTP 请求、账号不匹配、请求期间重新登录、重定向拒绝、429 状态和错误脱敏。fixture 成功仅证明代码按指定请求/响应契约工作，不证明真实官方服务可用。

最终 GREEN / 回归由主任务统一执行：本模块 7 项均通过，包含新增排队隔离测试；统一后端回归 812 通过、36 忽略、0 失败，日志 `output/acceptance/qoder-trae-backend-final.txt`。没有构建安装包，没有执行 Git 写操作。

辅助提取脚本和原始静态片段保留在 `output/trae-protocol-evidence.py`、`output/trae-protocol-evidence.txt`、`output/trae-sdk-request-wrapper.txt`；本文件独立记录关键依据，不依赖被忽略的 output 目录来说明实现边界。

## magpie 目录重写阶段

本节取代前文「模型必须手填」的目录限制，不取代对话转发限制。对话仍为旧 SDK 非流式路径，不能将目录查询成功解释为新模型均已完成真实调用验收。

- 依据：magpie 主仓库已将 Trae 交给 `magpie-community/plugins/packages/trae/index.mjs`；核对 `batchLists`、`listOf`、`chatModel` 与 `liveModels`。MIT 许可保留在 `docs/licenses/magpie-community-plugins.md`。
- 新目录：所选账号 JWT 请求固定地区模型主机的 `/api/ide/v1/batch_get_detail_param`。按 `chat_v3`、`solo_work_lite`、`solo_agent`、`solo_agent_lite`、`solo_coder` 顺序合并；批量目录不可用且非授权错误时，兼容逐函数 `/api/ide/v1/get_detail_param`。
- 筛选：排除非对话用途、停用、自定义供应商槽位、无效 ID；国际版排除 `is_invisible_to_user`，按 ID 去重；不加入固定模型或未经实现的 Max 派生项。
- 缓存：只保存公共模型 ID、获取时间、地区和授权密文摘要，五分钟有效；不保存原始目录条目中的潜在认证字段。换号不能复用旧缓存。
- 失败：授权拒绝立即终止，不继续逐函数查询；网络、无效 JSON、业务错误、空列表明确失败并保留历史，不把旧缓存伪装成刷新成功。
- 隔离：账号锁、来源状态复查与密文条件更新；查询期间换号或关闭来源时丢弃旧目录。遵循代理设置、禁止重定向，单次请求 15 秒、目录阶段总计 45 秒、JSON 上限 2 MiB。
- 接线：`/v1/models` 使用新目录；模型池只读当前授权绑定的新鲜缓存，仅在当前授权有效时允许显示用户明确保存的模型。未查询前的旧裸数组不视为可信目录。
- 边界：未移植设备身份伪装或自动绕过绑定，也未重写登录、签到和用量业务；未调用真实账号或模型。完整 `llm_utils_chat` 对话、流式和工具调用仍待后续阶段。

验证：目录模块四项测试先因缺实现失败；模型菜单与模型池测试分别复现旧代码返回手填模型、忽略新目录，再改为通过。补充筛选、重定向和空目录保留历史测试后，本轮网关回归 157 通过、0 失败。此数字是当前工作区测试集合结果，不能与前轮不同集合直接比较覆盖率。后端全量执行结果：837 通过、1 失败、36 忽略；失败为未改动模块的 `handoff_facts::tests::redact_skips_values_it_does_not_mask`，21 万字符处理耗时 1.0653332 秒超过测试时限；单独重跑通过（整项 0.46 秒），根因尚未确定，不能将全量表述为通过。完整日志为 `output/trae-model-catalog-backend-tests.txt`。参考插件 commit：`3e24e228377ed17a9e5991c4e715dfde0244a1ea`。

独立审查确认并修复两项问题：

1. 失效授权下模型池仍显示手填模型。新增断言先复现失败，再要求当前授权有效才允许手填模型回退。
2. 串行逐函数查询可因慢接口触发整体超时并丢掉成功目录。新增函数间通知依赖测试先复现串行阻塞，再改为固定最多五项并发、按完成顺序读取，输出仍按函数顺序合并；授权错误立即终止并丢弃剩余请求。

修复后网关回归 158 通过、0 失败。未进行二次独立审查；保留的边界是 45 秒预算不含账号锁等待，长对话占锁时端到端目录等待可能更长。SOLO-only 模型的旧 SDK 调用兼容性仍未验证，不把可列出当成可调用。修复后的串行后端最终回归：`cargo test --manifest-path backend/Cargo.toml -- --test-threads=1`，839 通过、0 失败、36 忽略，其他测试目标及 doc-tests 亦通过；日志 `output/trae-model-catalog-backend-final.txt`。仍有既有 `pricing::estimate` 未使用及链接器输出警告。串行通过不证明前次性能时限失败的根因已解决。

## 对话重写阶段（当前实现）

### 请求、目录和授权

- 参考同一插件 commit 的 `chatBody`、`nativeTools`、`NativeCalls`、SSE 及用量处理；许可范围已更新。未移植设备伪装、反滥用绕过或自动绑定规避。
- 目录缓存额外保存白名单路由：function、首个有效 `__dev` 名称、token 上限、图片能力和思考档位。请求固定为 `/api/agent/v3/llm_utils_chat`，function / model_name 不接受调用方注入。
- 优先 __dev、其次思考档位的简化评分尚不等价参考插件完整名称多数票 / misnamed 算法。未公开 Max 派生模型。
- 模型池只读当前授权绑定的新鲜目录，不再以手填模型兜底。旧 ID 缓存可用于展示账号目录，调用前必须重新获取完整路由。
- 所选连接身份检查、固定地区 host、禁重定向、代理设置和密文复查保留；每次读取流事件前复查来源与授权。没有刷新授权、读取原工具真实登录文件或创建远端 Key。
- 会话 ID 绑定进程随机盐、所选连接和密文；真实会话头优先，否则取首个用户内容。相同账号、相同首条内容且没有显式会话键的独立对话可能共享会话，重启后会话盐变化。

### 转换与失败语义

- 下游 Chat / Messages 均有流式和非流式转换；Responses 明确拒绝。Messages 回环测试直接组合实际请求转换、Trae HTTP 适配器和回复转换，不包含完整网关监听及 wire 选路。
- 支持 UTF-8 跨分片、CRLF、多行 data、思考文本、原生工具快照及明确参数 delta。文本工具只解析既有 Splitter 支持的部分格式，不等价完整 GLM / DSML / 宽松 JSON 模板。
- 原生和文本工具均缓冲到完成检查后释放，混合工具协议、非法 JSON 参数、截断或过滤明确失败。仅转换工具数据，不执行工具。
- EOF 无明确完成事件返回错误；Messages 公共转换也不再把无 `[DONE]` 的 EOF 变成成功 stop。下游丢弃响应体会释放上游及账号锁，无脱离请求的后台生成任务。
- SSE 单事件 2 MiB、整流 16 MiB；连接 15 秒、模型请求 180 秒。账号锁等待不计入目录 45 秒预算。归账计时从 Reader 建立起算，不包含目录、身份检查及等待上游响应头。
- 思考历史暂用 `<think>` 文本兼容表示，未经真实服务验证；显式禁用思考在目录未声明该能力时准确拒绝。图片仅对目录明确允许的用户图片块开放，其他不支持形态不静默丢弃。
- 缓存嵌套字段保留其为 prompt 子集的语义，转 Messages / 归账时扣除已知缓存；平铺独立缓存字段保持原口径。已知零保留，未知字段不补入账号统计，不推算未知 total。
- Messages SDK 的 input_tokens / output_tokens 是必填字段，实时 message_start 当前保留占位零；末尾完全未知的 output_tokens 也有协议占位零。此值不能作为真实计量，客户端区分未知与零仍有兼容边界；本阶段没有靠缓冲整段回答换取完整初始用量，也未把占位数写入后端统计。

### 本轮验证与审查

- 独立审查发现思考历史白名单拒绝、禁用思考错误提示及平铺缓存丢失，均先以失败测试复现后修复；未进行二次独立审查。
- 追加测试先复现：嵌套缓存被扁平化、文本工具提前泄出、归账缓存子集重复计数、公共 Messages EOF 误报成功、模型池手填 ID 兜底。修复后网关回归 172 通过、0 失败。
- 本轮仅使用合成凭据和本地模拟上游，不代表真实 JWT 权限、套餐模型、思考历史格式或真实流式工具调用已验收。最终后端验证结果在执行后追加。
- 未提交、推送、构建安装包；旧 SDK 阶段的测试与全量数字只代表当时状态，不作为当前代码最终证据。
- Trae 对话阶段全量命令 `cargo test --manifest-path backend/Cargo.toml -- --test-threads=1`：853 通过、0 失败、36 忽略，其余测试目标及 doc-tests 通过；日志 `output/trae-chat-backend-final.txt`。输出含本轮测试专用 `Body` import 警告（随后改为 cfg(test)）、既有 `pricing::estimate` 未使用及链接器输出警告，不将结果表述为无警告。
- 随后加入 ZCode「不隐式创建长期 Key」「凭据交换不跟随重定向」两项失败复现与修复，再执行相同串行后端全量：855 通过、0 失败、36 忽略，其余测试目标及 doc-tests 通过，日志 `output/trae-zcode-backend-final.txt`。`Body` 警告已消失，仍有既有 `pricing::estimate` 未使用和链接器输出警告。定向检查 11 个阶段文件的严格 UTF-8、替换字符和含用户名本机路径均无异常；已跟踪阶段文件 `git diff --check` 无空白错误（存在 Git 行尾转换提示）。
