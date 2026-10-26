# 全供应商账号查询验收

日期：2026-10-26。鼠鼠已确认范围为额度、余额、套餐、重置卡全部查询。本轮不执行 Git 写操作、不改设计稿、不消费重置卡、不发送模型请求。

## 改动总结

- 取消前后端对自定义服务额度查询的提前拦截；沿用编程套餐与同源网关适配器。
- 修复 Gemini、Grok 等 API 连接被归入本机订阅查询的问题：只有 `auth` 连接使用原工具登录，API 连接按自身地址和密钥查询，不借用原工具账号。
- 10 个供应商的账号卡片统一提供重置卡只读查询。返回次数时区分真实零、总次数和适用次数；没有字段时显示“服务未返回重置卡信息”，失败说明原因，保留旧值并标记。
- 余额不再因同时取得套餐额度而隐藏“不支持查询”状态；自定义服务可以同时显示套餐和余额。
- WorkBuddy 支持新版 sym-v1 加密登录：使用对应国内/国际官方客户端运行时在内存中解码，通过 stdin 传递密文，10 秒超时，输入和输出各限 64 KiB，stderr 不回显，退出清理子进程，不写回登录。支持标准安装路径、只读安装注册表及 `JOLU_WORKBUDDY_EXE` / `JOLU_WORKBUDDY_AI_EXE` 指定对应客户端。
- 修复 WorkBuddy 套餐名称遗漏：读取真实响应 `data.SubscriptionPackageName`，不根据名称推断有效状态或到期日。

## 验证结果

| 项目 | 结果 |
|---|---|
| 前端纯逻辑、hooks、WorkBuddy 信封解码 | 205 通过，0 失败；`output/all-provider-frontend-tests.log` |
| Rust `cargo test --manifest-path backend/Cargo.toml --lib` | 550 通过，0 失败，21 忽略；忽略项不算通过 |
| 类型检查与前端生产构建 | 通过；构建保留既有动态/静态导入拆包提示 |
| 真实账号组件、hook、桥接的合成验证 | 全部 10 个供应商查询入口、重置卡缺失字段、单卡归属、额度与余额并存、真实零、失败旧值、余额不支持状态通过；`scripts/acceptance/account-quota-sources.cjs` |
| WorkBuddy 凭证真实读取 | 本机 5.8.3 官方客户端成功读取新版加密登录；只输出成功布尔值，未输出密钥、令牌或账号 |
| WorkBuddy 官方额度真实请求 | 返回 HTTP 401，Rust 显示 `unauthorized`，没有取得额度；令牌尚未到期不能证明服务端认可授权 |
| Gemini 真实来源 | `unauthorized`，本机没有可用登录，未取得额度 |
| Qoder 真实 CLI 来源 | `unauthorized`，需在原工具登录，未取得额度 |
| Trae 国内/国际真实来源 | 两版均成功，分别返回 1 / 2 个真实额度窗口与套餐 |
| ZCode BigModel 真实账号 | 成功返回 `inactive`（未开通个人套餐），0 个额度窗口；不当作零余额 |
| `git diff --check` | 通过 |
| 独立只读复核 | 未发现本轮新增的明确阻断性回归；核对凭证输出、子进程清理、账号缓存、暂停查询和 API 路由，重置卡数据边界保持明确 |

先观测到自定义查询用例失败（quota 为 null）、WorkBuddy 顶层套餐字段用例失败、全平台重置卡按钮断言失败，再修复并通过。解码测试覆盖篡改、未知格式和不同客户端密钥拒绝；合成数据不能替代真实服务联调。

## 仍需真实条件与未完成验收

- WorkBuddy 当前服务端拒绝授权，需要在原工具检查登录后再次查询；不能据凭证成功解码声称积分查询成功。
- Gemini / Qoder 缺有效登录；Qoder 国内版在线接口仍未接入，不能借用国际版账号。Claude 完整订阅、Grok、OpenCode 订阅及各官方 API 余额缺真实样本，本轮不声称真实通过。
- 普通模型 API Key 不一定有账户余额、订阅或管理权限；未知服务也不保证存在统一接口。重置卡已确认的数据源仍为 Codex / OpenCode OpenAI 授权，其他服务没有返回该信息时保持未知。
- Pencil MCP 多次读取返回“文件需要在编辑器打开”；通过电脑操作启动 Pen 也未出现可操作窗口。已检查实际浅深账号组件截图，但未完成与设计稿画板的视觉对照，不修改 `.pen`。

## 依据

- WorkBuddy 官方文档：[套餐与用量](https://www.workbuddy.cn/docs/workbuddy/Usage)、[Open API](https://open.workbuddy.cn/docs/openapi)。官方开放 API 未列出全部所需账单字段。
- Context7 先解析官方文档库 `/websites/codebuddy_cn_workbuddy`，再查询本机登录凭证资料，未取得加密信封协议文档；随后核对本机官方客户端 `main/app-lifetime-job.js`、`main/account.js`、`main/tar.js` 的信封格式和只读资源查询代码。
- Electron 执行方式通过 Context7 `/electron/electron` 核对：[ELECTRON_RUN_AS_NODE](https://github.com/electron/electron/blob/main/docs/api/environment-variables.md)。未把客户端原生密钥复制到应用、日志或前端。
