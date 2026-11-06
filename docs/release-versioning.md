# Jolu 版本与发行规则

版本号采用 `主版本.次版本.修订号`。修订号每次加 1；`100` 是当前次版本线的最后一个修订号，下一版次版本加 1，修订号归零。

```text
0.1.0 → 0.1.1 → 0.1.2 → … → 0.1.99 → 0.1.100 → 0.2.0 → 0.2.1
```

## 每版单独发行

发行由 `.github/workflows/release.yml` 完成。

正式发行仓库为 [meilule/Jolu](https://github.com/meilule/Jolu)。应用更新端点在 `backend/tauri.conf.json`，更新包下载地址由 `scripts/release/release-assets.mjs` 生成，并由 `scripts/release/verify-release.mjs` 限定为该仓库；迁移仓库时须同步修改这三处。工作流跟随当前检出的仓库发布，更新签名继续使用原有密钥。

- `main` 的每次推送使用一个新的 `vX.Y.Z` 标签和同名 GitHub Release。发行版只上传该版本的安装包、签名与 `latest.json`。
- 版本号取 `backend/tauri.conf.json`、已有 `vX.Y.Z` 标签与已有发行版（含草稿）中的最高版本，再按上述规则递增（`scripts/release/next-release-version.mjs`）。构建时以 `--config` 注入版本号，不改写仓库里的配置文件。
- 维护者手动推送 `v*` 标签时，标签必须与 `backend/tauri.conf.json` 的版本一致，工作流按该版本发行。也可以用 `gh workflow run release.yml --ref vX.Y.Z` 对已有标签重新触发。发行标签不应强制改指向或复用到别的提交。
- 只改文档或工具、无需重打安装包的提交，可以在提交信息中加 `[skip ci]` 跳过发行。

## 流程与校验

1. 计算版本号，按当前提交创建草稿发行版。
2. Windows（NSIS + Portable ZIP）、macOS arm64 / x86_64（app、dmg）、Linux x86_64（AppImage、deb）四路并行构建，使用仓库 Secret `TAURI_SIGNING_PRIVATE_KEY`（带密码的私钥）和 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 签名更新包；私钥必须与 `backend/tauri.conf.json` 中的公钥成对，换钥会让已安装版本无法验证更新。由 `scripts/release/release-assets.mjs collect` 统一重命名为 `Jolu-vX.Y.Z-<平台>` 后上传。
3. `release-assets.mjs manifest` 用四个平台的 `.sig` 生成 `latest.json`，下载地址指向 `releases/download/vX.Y.Z/…`。`verify-release.mjs` 核对标签、清单、安装包名称、签名文件和下载地址都属于这一版。
4. 校验通过后，草稿转为公开并标记为最新。应用的更新端点 `releases/latest/download/latest.json` 由此指向新版本。任一平台失败时发行版保持草稿，现有更新端点不受影响。
5. 构建失败留下的草稿不复用版本号。下一次构建取更高版本，避免旧包与新包混在一页。

macOS 安装包目前没有 Apple 开发者签名和公证，首次打开需要在系统设置中手动允许。

## Windows 免安装版与签名

- 每次发行同时提供 `Jolu-vX.Y.Z-Windows-x86_64-Setup.exe` 和 `Jolu-vX.Y.Z-Windows-x86_64-Portable.zip`；缺少 Portable 时校验失败，不公开发行版。
- Portable 解压运行，账号、设置和历史仍存当前用户目录，与安装版共用；需要系统已安装 WebView2 Runtime。ZIP 只包含 `Jolu.exe` 和使用说明，不携带构建机器的数据。
- 工作流先构建带更新功能的安装包，再用 `JOLU_UPDATES_ENABLED=0` / `VITE_JOLU_UPDATES_ENABLED=0` 和 `--no-bundle` 编译免安装版，由 `scripts/release/package-portable.ps1` 打包。免安装版需手动下载新版 ZIP 替换程序，避免应用内更新把它变成安装版。
- `Setup.exe.sig` 是安装包的独立更新签名，不是可执行程序，也不是另一种安装包。`latest.json` 内嵌该签名，供应用内更新验签使用；手动安装只需下载 `.exe`。
- Portable 不进入安装版更新清单，也不生成更新 `.sig`。Tauri 更新签名与 Windows Authenticode 代码签名不是一回事。
