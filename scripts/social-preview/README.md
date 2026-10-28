# 社交预览图

GitHub 仓库的社交预览图（链接分享时显示的卡片），由这里的 HTML 源生成，产物写入
`docs/assets/social-preview-1280x640.png`。

## 重新生成

```bash
pnpm social-preview        # 或 node scripts/social-preview/build.mjs
```

脚本会自检产物是否为 1280×640 PNG。生成后需要仓库所有者在 GitHub 仓库的
Settings → General → Social preview 中手动上传，脚本不会上传。

## 版面与素材

浅色、只用中文：左侧为应用图标、`Jolu` 字标、标语和三个卖点，右侧为主窗口截图，左下叠放灵动岛，人物从右下角探出挥手。

| 素材 | 来源 |
|---|---|
| 应用图标 | `frontend/public/app-icon.png` |
| 主窗口截图 | `docs/assets/readme/panel-usage.png` |
| 灵动岛 | `docs/assets/readme/island-expanded.png`（只截取卡片本体） |
| 人物 | `docs/logo/jolu-character-white-tights.png` |
| 字标 | `frontend/src/components/brand/Wordmark.tsx` 的路径 |

README 截图重拍或标语调整后，重跑一次即可。查找 Chromium 的逻辑与安装向导位图共用 `scripts/installer-art/find-chrome.mjs`。
