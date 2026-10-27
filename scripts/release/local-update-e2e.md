# 本地更新验证

两种做法：只验证检查、下载与签名（不安装）；或用两个不同版本的真实 NSIS 包做真机安装与更新。真机安装的最近一次结果见 `docs/verification/2026-10-27-update-install-e2e.md`。

## 共同准备

1. 在 `output/` 下单独建目录，生成专用测试密钥：`pnpm tauri signer generate --ci --password <测试密码> --write-keys <目录>/test.key`。不要使用正式私钥，不输出密钥内容。
2. 覆盖配置写独立 JSON 文件，禁止修改生产 `tauri.conf.json`：端点为 `http://127.0.0.1:<端口>/latest.json`，公钥为测试公钥，`dangerousInsecureTransportProtocol: true`。
3. 检查更新默认启用，无需额外环境变量；`CARGO_TARGET_DIR` 指向 output 下独立目录，不覆盖开发用的 `backend/target`。

## 只验证检查、下载与签名

1. identifier 使用 `dev.ningz.jolu.acceptance`，编译 debug 版，按 `scripts/acceptance/start.ps1` 用带标记的隔离 profile 运行。
2. 用 `pnpm tauri signer sign --private-key-path ... --password ... <文件>` 签署测试文件，`.sig` 内容写入测试 latest.json 的 `windows-x86_64.signature`。
3. 在 127.0.0.1 启动静态服务器。调用 `check_app_update_available` 应看到新版本，调用 `download_app_update` 验证下载与签名。
4. 修改 sig 内的签名字节（保持两层 Base64 与 minisign 文本格式有效），再次下载必须返回 `The signature verification failed`，随后恢复原清单。

## 真机安装与更新

本机没有 Windows Sandbox（家庭版）时，在宿主机上用改名测试包执行，避免与正式 Jolu 共用目录、注册表和进程名：

1. 覆盖配置另设 `productName: "JoluE2E"`、`mainBinaryName: "jolu-e2e"`、`identifier: "dev.ningz.jolu.e2e"`，分别以 `version` 0.1.0 和 0.1.1 构建：设置 `TAURI_SIGNING_PRIVATE_KEY`（测试私钥路径）与 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`，执行 `pnpm tauri build --debug --bundles nsis --config <覆盖配置>`。debug 构建才认 `JOLU_TEST_DIR` 隔离目录。
2. 先备份 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 的 `Jolu` 值：卸载钩子按固定名称删除它，应用在非隔离模式启动时也可能改写它。
3. 运行 0.1.0 安装包，逐页截图，确认没有选择目录页，安装到 `%LOCALAPPDATA%\JoluE2E`。安装窗口出现在桌面上，测试期间不要手动点击。NSIS 按钮在 UI 自动化中显示为 Pane，需要用 Win32 `BM_CLICK` 驱动。
4. 用 0.1.1 安装包和签名生成 latest.json，在本机启动静态服务器。把 `JOLU_TEST_DIR` 临时设为用户级环境变量，确保安装器重新拉起的应用也处于隔离模式。启动已安装的 0.1.0，并带上 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<端口>`。
5. 通过 CDP 等待页头出现「安装 v0.1.1」绿色按钮，截图后点击，记录被动安装窗口。应用应以 0.1.1 自动重启，卸载注册表项中的 `DisplayVersion` 为 0.1.1。
6. 清理：结束测试进程，执行 `uninstall.exe /P`，恢复 `Run\Jolu`，删除用户级 `JOLU_TEST_DIR` 和 `dev.ningz.jolu.e2e` 数据目录，关闭服务器，逐项核对。
