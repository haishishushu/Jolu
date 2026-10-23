# 剩余开发任务执行记录

依据：`docs/plans/2026-10-22-jolu-remaining-work.md` 与 `docs/ui/Jolu.pen`。本次明确范围为 3.2、3.6、4.2–4.7、5.1。

## 执行约束与裁决

- 继续当前工作目录，保留此前隔离与通知修复；不暂存、不提交、不切分支、不推送。旧计划中的提交、worktree 和逐任务派发步骤不执行。
- 画板已可通过 Pencil 读取，48/50/52/54/57A 已存在；不重复补画，不修改设计稿。
- 3.6 真实评测需批准会话清单和约 80 次 CLI 调用；先完成工具、合成测试与操作说明，实际调用待最后统一批准。真实短会话压缩同理。
- ZCode 真实 Key、OpenCode 真机、正式发布签名与系统安装等外部条件统一收集，不阻断其他实现。
- 共享接口：3.2 消耗已有 3.1 命令；4.2 消耗 4.1 ToolSpec；4.3/4.4 共用连接协议字段；4.5 消耗 4.4 OpenCode 路径；5.1 验证所有改动。

## 进度

| 任务 | 状态 | 证据 / 待办 |
|---|---|---|
| 3.2 | 实现与合成UI验收完成 | 配置→首稿→第二稿→预览→源码编辑通过；真实CLI待批准 |
| 3.6 | 真实短会话探索评测完成 | 后续已授权并完成5样本、33次Claude调用；50轮以上正式评测缺样本，详见真实验证记录 |
| 4.2 | 实现完成，真机核对待条件 | 多平台扫描、配置启停、自定义只读目录；N-1/N-2/N-3已修复 |
| 4.3 | 实现完成，真实Key待条件 | 四种地址及协议、模型获取、套餐额度路由 |
| 4.4 | 实现完成，真机验证待条件 | Go/OpenAI订阅/兼容服务；OAuth只读检测 |
| 4.5 | 实现与合成测试完成 | JSON/SQLite V1/V2；未知值、幂等、暂停采集 |
| 4.6 | 完成 | 两家apiKey=none，专用说明与测试 |
| 4.7 | 检查/下载/签名验证完成 | 安装重启链路仍待Sandbox；正式密钥与发布待批准 |
| 5.1 | 测试、文档及安装包完成 | 全量测试、隔离7页冒烟通过；NSIS构建成功，安装/升级仍待隔离系统验证 |

## 执行补充
- 3.6：已添加仅 debug + handoff-eval feature 可用的评测二进制，3 项纯逻辑测试通过。未运行任何付费 CLI。待鼠鼠批准样本和调用预算。
- 4.2：WorkBuddy 按鼠鼠最新决定仅扫描 `.codebuddy/skills`，不扫描 `.workbuddy/skills`，暂只读。
- Ruling: Trae 中文官方文档确认全局 `.trae-cn/skills`、项目 `.trae/skills`；同时识别国际版 `.trae/skills`。https://docs.trae.cn/ide_skills
- Ruling: Gemini `.agents/skills` 与 Codex 共享，为避免路径命中不同工具的开关，Gemini 下只读；主 `.gemini/skills` 支持写入与名称级启停。
- Ruling: OpenCode JSONC 和通配符权限规则由原工具管理；解析失败绝不覆盖配置。代价是这些规则下不能在 Jolu 切换。

## 回归与审查
- 前端/发布脚本200项通过；类型检查通过。Rust第一轮最终回归427通过、11跳过，新增复审回归测试后的结果另追加。
- 隔离桌面profile：`output/acceptance/profile/ab6e765b364f4c1180af41047bf87610`。七页均通过，无读取失败、控制台error或未捕获异常。
- 压缩UI仅用合成IPC夹具，未发模型请求；截图 `output/acceptance/handoff-{config,chat,preview}.png`。已对照Pencil 48、50、52、54、57A。修正主按钮样式冲突和发送按钮挤压换行。
- 更新使用独立测试公钥与identifier，正常签名下载成功；篡改签名字节后返回 `The signature verification failed`，未执行安装。
- 一次独立复审发现3项：新增套餐连接额度被native提前拦截、OpenCode暂停id不匹配、项目Skills不继承全局deny；均已修复并补回归覆盖。
- 构建出现Tauri CLI的STATIC_VCRUNTIME弃用提示，以及MSVC创建导入库的linker stdout警告；不是测试失败，未隐去。
- 已纠正的旧判断：此前将上游AI.Usage.outputTokens（包含reasoning）误当作落盘TokenUsage.output。真实数据库与TokenUsage源码确认落盘output为可见输出，现按output+reasoning计Jolu输出，显式total优先。详见 `2026-10-23-real-validation.md`。
- Ruling: 无可核实启停配置的工具显示“启停待核对”，不把存在SKILL.md当成已启用。该状态文案需鼠鼠最终视觉确认。
- Ruling: 本机不安装测试更新或覆盖现用安装版；仅完成隔离检查/下载/签名测试，完整安装链路列入待批准条件。
- Git仅查看状态/差异，未暂存、提交、切分支或推送。

- 新增复审回归后Rust429通过、11跳过。补查发现ZCode/OpenCode手动保存还受旧fetch-only二次判断阻拦，新增prepare_new合成凭证回归，修正后重新构建，旧包不作为最终交付。
- WindowsSandbox.exe不存在。已生成 `output/acceptance/sandbox/install-check.wsb`，未在宿主安装，完整安装升级仍待具备隔离系统条件。

## 最终回归
- Rust：`cargo test --manifest-path backend/Cargo.toml --features handoff-eval`，430通过、0失败、11跳过；日志 `output/backend-tests-final3.txt`。新增连接回归覆盖准备、保存与列表读回，测试使用内存数据库和合成密钥。
- 测试初版误用私有数据库初始化函数，已改用现有公开数据库入口；不扩大生产函数可见性。
- 前端类型检查及200项前端/发布脚本测试通过；隔离验收使用的应用、网关、更新服务和5193预览服务已停止。
- 最终NSIS：`backend/target/release/bundle/nsis/Jolu_0.1.0_x64-setup.exe`，4,568,570字节，2026-10-23 15:46:54生成。构建退出码0，日志 `output/build-installer-final2.txt`；更新默认关闭，未生成正式更新签名产物，未执行宿主安装。
- 安装包SHA256：`A5D4ECBC1272BCD9CF05846D9B7ADB53AB73FE137580112B1B3B286BA71CD4B5`。

上述为首轮交付历史。鼠鼠后续授权后的Markdown修复、OpenCode真机问题修复、真实评测及16:24生成的新安装包，以 `2026-10-23-real-validation.md` 为准。
