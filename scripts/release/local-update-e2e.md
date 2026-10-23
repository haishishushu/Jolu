# 本地更新验证

本轮已在隔离debug应用验证检查、下载、签名验证，未安装合成payload。结果：`output/update-e2e/result.json`。

1. 在 `output/update-e2e/` 生成专用测试密钥：`pnpm tauri signer generate --ci --password <测试密码> --write-keys output/update-e2e/test.key`。不要使用正式私钥，不输出密钥内容。
2. 用 `pnpm tauri signer sign --private-key-path ... --password ... <文件>` 签署测试文件。`.sig` 内容写入测试 latest.json 的 windows-x86_64.signature。
3. 覆盖配置写独立JSON文件：identifier使用 `dev.ningz.jolu.acceptance`，端点为 `http://127.0.0.1:18766/latest.json`，公钥为测试公钥，`dangerousInsecureTransportProtocol:true`。禁止修改生产 tauri.conf.json。
4. 设 `JOLU_UPDATES_ENABLED=1`、`VITE_JOLU_UPDATES_ENABLED=1`；`TAURI_CONFIG` 设置JSON内容，`CARGO_TARGET_DIR`设置独立目录后编译debug版；按 acceptance/start.ps1 使用带标记隔离profile运行。
5. 在127.0.0.1启动静态服务器。调用 `check_app_update_available`，检查0.1.1可用；调用 `download_app_update`验证下载与签名。
6. 修改sig内的签名字节（保持两层Base64与minisign文本格式有效），再次检查和下载，必须返回 `The signature verification failed`。恢复原清单。
7. 不调用安装命令执行合成文件。完整安装验收另用两个不同版本的真实NSIS包，在Windows Sandbox中执行安装、更新、重启和篡改签名场景，确认数据与设置保留。
8. 结束本次创建的应用/服务器进程，清除当前终端测试环境变量。正式构建不传这些变量，更新默认停用。

本轮范围：签名下载通过；签名篡改被拒绝；未安装、未重启更新、未验证旧版本保留。测试密钥仅在被忽略的output下。
