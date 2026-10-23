# 安装包 Windows Sandbox 检查（2026-10-23）

任务 1.3 步骤 4 未测：开发机没有安装 Windows Sandbox（`C:\Windows\System32\WindowsSandbox.exe` 不存在），按控制方裁决不在本机安装。

- [ ] 在已安装 Windows Sandbox 的电脑上运行 `scripts/acceptance/prepare-sandbox.ps1`，打开生成的 `output/acceptance/sandbox/install-check.wsb`
- [ ] 检查安装界面图片（`scripts/installer-art/` 的产物）显示正常
- [ ] 检查桌面快捷方式图标正确
- [ ] 默认卸载后应用数据目录保留
- [ ] 结果写进 `output/acceptance/sandbox/sandbox-result.json`，并把结论补到 `docs/verification/2026-10-22-desktop-acceptance.md` 步骤 4
