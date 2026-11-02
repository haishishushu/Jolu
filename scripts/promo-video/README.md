# 宣传片

Jolu 宣传片（1920×1080 · 60fps · 61.55 秒 · H.264 + AAC），深色科技风，成片写入 `output/promo-video/jolu-promo.mp4`（已被 Git 忽略）。片长与帧率写在 `promo.html` 的 `promo-duration` / `promo-fps` 里。

## 重新生成

```bash
pnpm dev                                                       # 另开终端启动浏览器预览（演示数据）
JOLU_PREVIEW_URL=http://localhost:5173 node scripts/promo-video/shoot.mjs   # 补拍深色主题截图，只需拍一次
pnpm promo-video --music <配乐文件>                              # 或 node scripts/promo-video/build.mjs --music …
```

- `--music` 可以是音乐文件，也可以是带音轨的视频。`fit-music.py` 在曲子中段找听感最接近的两处位置无缝拼接：原曲偏长时跳过一段，偏短时循环一段（每段 8–20 秒），开头与自然结尾保持原曲。
- 当前成片配乐取自参考视频 `output/promo-video/4af9f7bf22bf25a91e93bca08a047446_raw.mp4` 的音轨（128 BPM，31.55 秒），原曲 9.567–24.567 秒（整 8 小节）循环两遍延长到 61.55 秒。配乐版权由仓库所有者自行处理，文件不入库。
- 只检查画面：`node scripts/promo-video/build.mjs --still 14,27,41.5`，把这些时刻的 PNG 导出到 `output/promo-video/stills/`，不需要配乐，也不编码视频。
- 在浏览器里直接看动画：打开 `output/promo-video/promo.build.html`（构建时生成），加 `?t=秒` 从指定时刻开始。

## 分镜

配乐 128 BPM，1 小节 = 1.875 秒，小节线 b(k) = 7.24 + 1.875k，转场都落在小节线上。

| 时间 | 段落 | 画面 |
|---|---|---|
| 0–6.7s | 开场隐喻 | 「你的 AI 编程工具」节点被六边形墙挡住（2.77s 重音变红：额度看不见 · 任务会中断），5.9s 冲破墙、连上 Jolu |
| 6.6–11s | 品牌 | 头像图标发光放大 → 字标描线、「把 AI 编程工具，管在一处」、英文副标题乱码解码 |
| 11–12.9s | 闪切 | 全屏绿色「5 大能力」 |
| 12.9–20.4s | 01 灵动岛 | 90% 大数字与 5h / 7d 额度条；真实灵动岛双击展开，再开出 3 个分身 |
| 20.4–27.9s | 02 自动化 | 任务日志逐行打出：执行 → 额度用完 → 时钟走到 18:30 → 原会话续跑；倾斜的等待额度截图加标注 |
| 27.9–35.4s | 03 账号管理 | 4 张账号卡片额度条依次填满、点击隐藏敏感信息；10 个供应商卡片按十六分音符弹出 |
| 35.4–42.9s | 04 压缩会话 | 3D 消息墙与压缩进度 → 交接 MD 逐行生成，从 Codex 飞到 Claude Code |
| 42.9–50.4s | 05 一键签到 | 点击一键签到，4 个账号依次领到奖励；倾斜的签到页截图加标注 |
| 50.4–57.9s | 更多 | 用量统计截图加标注；会话线程、Skills 管理、设置三联 |
| 57.9–61.55s | 片尾 | 图标、字标、标语、「立即下载 →」与「GitHub 搜索 Jolu」，鼠标点击 |

## 素材与数据

画面里的账号、项目和数值全部是演示数据：截图只来自浏览器预览（`docs/assets/readme/` 与 `shoot.mjs` 补拍的 `output/promo-video/shots/`），`shoot.mjs` 检测到真实桌面接口会直接退出；账号、签到、消息墙等卡片在 `promo.html` 里重绘，只用 `example.com` 之类的假数据。片中不出现仓库所有者的用户名或本机路径，右下角标注「演示数据」。

| 文件 | 作用 |
|---|---|
| `promo.html` | 画面源。CSS 关键帧的 `animation-delay` 写绝对时间；打字、计数、时钟、乱码解码由页内脚本按时间计算 |
| `build.mjs` | 填入素材路径，无头 Edge 逐帧截图，经管道送入 ffmpeg 与配乐合成 |
| `fit-music.py` | 把配乐无缝剪短或循环延长到片长：比较频谱找接缝，按波形互相关对齐到采样级 |
| `shoot.mjs` | 从浏览器预览补拍深色主题截图 |

依赖：`playwright-core`（与 README 截图脚本一样从 `output/acceptance/node` 解析）、本机 Microsoft Edge、Python + numpy / scipy、ffmpeg（依次查找 `JOLU_FFMPEG`、PATH、Python 包 `imageio-ffmpeg` 自带的可执行文件）。中文标题字体用系统里的 Noto Sans SC 可变字体（`NotoSansSC-VF.ttf`，可用 `JOLU_CJK_FONT` 指定），找不到时回退到微软雅黑。平台标识路径构建时直接读取 `frontend/src/components/brand/PlatformLogo.tsx`。
