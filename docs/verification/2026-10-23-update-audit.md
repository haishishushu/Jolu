# 检查更新审查与修复

日期：2026-10-23。范围：检查、下载、取消、签名门禁、安装准备、失败恢复与交互。

## 修复内容

- 下载和安装共享互斥保护。旧下载取消/失败后释放锁，新请求不能在旧请求结束前覆盖准备安装的包。
- 下载支持取消：oneshot 通知配合 `tokio::select!` 丢弃网络 future；界面回到可重试状态。
- 检查请求 30 秒超时，下载请求 600 秒超时，避免无限等待。下载进度最多每 100 毫秒推送一次，最后一段立即推送，减少跨进程消息和重绘。
- 安装操作及等待数据库锁移到阻塞线程池。持有写数据库锁检查运行中的自动化任务，整个安装接管期间阻止调度器认领新任务；有运行任务时保留已校验安装包并提示先处理任务。
- Windows 安装器解包成功、准备接管时才隐藏托盘，不提前销毁托盘或窗口资源。安装器启动失败时恢复托盘，并保留安装包供重试。
- 对照 Pencil 的「更新过程状态」画板，补齐「取消下载」「稍后安装」「查看任务」。稍后安装后再打开使用已下载包；失败可关闭或重试；查看任务会退出遮罩并导航到自动化。
- 自动检查更新偏好接入 Rust 设置存储和跨窗口广播。关闭后不启动静默检查，手动检查继续可用；总开关关闭时仍可保存偏好。
- 发布清单验证新增官方仓库 HTTPS 地址、版本路径、签名封装和 `.sig` 文件检查。签名格式检查不等于密码学验签；真实验签由 updater 内置公钥执行。

## 设计与 API 依据

- Pencil：通过名称定位更新相关画板，读取并截图「更新过程状态」（本次节点 `x4HO06`），没有修改设计稿。
- Context7：先解析 Tauri/Tokio 库，再查询 `on_before_exit`、独立下载/安装与 oneshot/select 取消机制。
- 核对本机锁定的 tauri-plugin-updater 2.13.3 源码：默认退出钩子在 ShellExecute 之前清理界面资源；本轮改用可恢复的隐藏托盘操作，避免启动安装器失败后界面失效。

## 验证记录

- 前端 `pnpm --dir frontend typecheck` 通过。
- `node --test scripts/release/*.test.mjs`：12 项通过；新增缺签名、错误下载域名、HTTP 降级用例先失败后通过。
- `cargo test --manifest-path backend/Cargo.toml updater::tests --lib`：3 项通过，覆盖互斥/失败释放、取消后重试、运行任务阻止安装。超时与进度节流调整后的最终针对性执行同样 3 项通过。
- `node scripts/acceptance/update-flow.cjs`：在独立 Vite 端口注入合成 API，验证取消、离线重试、运行任务导航、任务完成、稍后安装、安装包复用、安装失败重试和事件监听清理；无页面异常。另验证偏好关闭不请求、手动检查有效、开启后只检查不下载。脚本生成的临时页面结束即清理。
- `cargo test --manifest-path backend/Cargo.toml auto_update_preference --lib`：1 项通过，真实临时目录写入/重载确认偏好持久化，旧配置默认开启。
- 独立浏览器预览完成取消→重新下载→稍后安装→再次打开→模拟安装→关闭，截图 `output/update-ui-downloaded.png` 与画板状态对照。
- 已有 `output/update-e2e/result.json` 记录真实插件正常签名下载通过、篡改签名拒绝；本轮审查读取该结果，没有将其冒充为本轮重新执行，也没有执行合成 payload 安装。

## 尚未证明 / 发布条件

- 完整真实 NSIS 版本升级、重启、新旧版本数据/设置保留尚未验证。当前机器没有 Windows Sandbox；需要隔离虚拟机或受控测试环境，不能覆盖真实 Jolu 做验证。
- 正式签名私钥与 Release/latest.json 发布仍需鼠鼠最终决定。本轮没有生成生产密钥、替换生产公钥或发布 Release。
- 生产构建按项目规范默认关闭更新，发布时必须同时传 `JOLU_UPDATES_ENABLED=1` 与 `VITE_JOLU_UPDATES_ENABLED=1`。已检查两端都仅把字符串 `1` 作为开启值。


## 重跑隔离前端验收

在单独终端运行 `$env:VITE_JOLU_UPDATES_ENABLED='1'; pnpm --dir frontend dev --host 127.0.0.1 --port 5197`，然后运行 `node scripts/acceptance/update-flow.cjs`。脚本仅注入合成 API，结束后关闭这一个测试服务即可。也可通过 `JOLU_PREVIEW_URL` 指定其他独立端口。

本轮没有 Git 写操作，改动留给主代理按完整阶段统一提交。
