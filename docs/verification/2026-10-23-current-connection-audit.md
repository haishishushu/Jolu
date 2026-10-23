# 当前使用账号识别复审

## 原因与修复

此前界面和自动化分别枚举 API、Auth，找得到哪条已保存连接就使用哪条。若 CLI 实际优先使用的 API Key 没有登记，代码会继续命中残留 OAuth，把未实际使用的订阅账号标为当前账号；同一 Key 配多个网关时还会误选最早保存的网关。

本轮新增 `current_connection.rs`：先判断实际生效来源，只返回一个平台的一种来源，再以 kind + secret + 归一 Base URL 匹配数据库。界面 `current_connections` 和自动化账号检查共用这一条路径。

Codex 识别读取默认 profile 选择的供应商，显式 env_key 缺失时停止；匿名供应商及无法解析的命令型授权不冒认磁盘 OAuth。环境和文件读取在隔离模式入口之前被阻止。原值不进入 DTO。

## 依据

Context7 已解析 `/openai/codex`，具体查询遇到网络失败，随后核对官方源码：[供应商认证选择](https://github.com/openai/codex/blob/main/codex-rs/model-provider/src/auth.rs)。该实现先处理供应商 bearer 凭证，再依据 requires_openai_auth 选择匿名或 OpenAI 授权。

另向账号实现代理提交两项独立身份问题：Codex account_id 是工作区而非用户，轮换应同时绑定用户身份；CLI 轮换后直接切换必须保全新 refresh token。依据：[官方 token 类型定义](https://github.com/openai/codex/blob/main/codex-rs/login/src/token_data.rs)。这两项修复归账号实现代理，不能由本报告的当前来源测试代替验证。

## 验证

`cargo test --manifest-path backend/Cargo.toml current_ --lib` 最终 10 项通过（含本轮新增 7 项），0 失败。覆盖缺失 env_key 不回退、匿名供应商不冒认 OAuth、默认 profile 优先、官方 API/Auth 来源选择、kind/密钥/地址联合匹配、隔离入口，以及登录输入环境不覆盖已保存 OAuth。

测试全部使用合成密钥或隔离配置，没有切换实际账号、写真实 CLI 配置或使用真实授权。

最后复审补充有效授权存储判定：keyring、auto、ephemeral 不从遗留 auth.json 冒认当前账号；profile 的存储设置优先于顶层。回归先红后绿。最终包含此修复和评测工具的整合命令 `cargo test --manifest-path backend/Cargo.toml --features handoff-eval --quiet`：471 通过、0 失败、12 忽略。

补充来源核对：[官方授权加载](https://github.com/openai/codex/blob/main/codex-rs/login/src/auth/manager.rs) 的默认运行时不直接使用 OPENAI_API_KEY 环境变量；此变量用于登录输入。因此当前识别仅在供应商显式 env_key 指向它时读取它。CODEX_API_KEY 是否启用取决于执行入口，环境存在时保守地不认定为某个已保存交互账号。
