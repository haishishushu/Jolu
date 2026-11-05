# 胶囊灵动岛与经典灵动岛解耦（窗口容器分叉）

- 日期：2026-11-05
- 背景：`views/IslandWindow.tsx` 里经典样式与胶囊样式共用一个窗口容器，靠 `settings.island_style === "capsule"` 在同一个组件里分支；胶囊的探出、拖动、尺寸、右键菜单都和经典写在一起，改胶囊容易碰到经典。
- 鼠鼠选定方案：**B —— 连窗口容器也分叉**。渲染行为保持一个原生窗口（由 Rust 创建、加载 `index.html?window=island`），前端按设置分到两个窗口组件，互不 import。

## 目标结构

```
views/IslandWindow.tsx                 只做分流：capsule ? <CapsuleWindow/> : <ClassicIslandWindow/>
views/classic/ClassicIslandWindow.tsx  经典：停靠条 / 自由 / 展开 + 150ms 探出 · 500ms 收回
views/capsule/CapsuleWindow.tsx        胶囊：完整胶囊 / 迷你胶囊 / 展开卡片 + 400ms 悬停探出
hooks/useIslandWindowSize.ts           窗口尺寸同步（ResizeObserver → resize_island）
hooks/useIslandWindowModel.ts          数据装配（IslandData + 连接/额度/余额/用量/刷新事件）
hooks/useIslandWindowShell.ts          形态状态机 + 原生拖动 + 右键菜单 + 停靠事件 + 收起
components/island/PreviewControls.tsx  浏览器预览的两个演示控制条
```

## 约束

- 经典样式行为零回归：探出 150ms、收回 500ms、停靠条按 `island_shrink_scale` 缩放、`p-3.5` 内边距、预览控制条、双击解除停靠。
- 胶囊保留自己的语义：只有左右贴边缩成迷你胶囊，上下贴边保持完整胶囊；贴边不投影；卡片可拖动、展开时按停靠边锚定。
- 数据层只有一份，避免两套额度/余额逻辑漂移；两个窗口组件共用 hook，不共用 JSX。

## 内容对齐（同批）

收缩胶囊的内容按 `output/island-prototype/app.js` 的 `collapsedHTML()` / `quotaHTML()` 重排：

- 平台名 + 额度窗口芯片 `5h 10%` / `7d 59%`，`usedPercent ≥ 80` 转橙，满格换红字 `x 后重置`。
- 拿不到额度时只给破折号芯片（`5h —` / `7d —`，连窗口都不知道就一个 `—`），不再输出「剩余额度 —」这类卡片句式 —— 这是收缩胶囊读起来像「压扁的卡片」的直接原因。
- 内边距对齐画板 A3-1：bot 落在 (4,4)，右侧留白 14（`.cap-cl` 为 `0 13px 0 3px`）。
- 胶囊本体不投影（窗口内边距 20px，`0 8px 24px` 会被窗口边缘切出硬边）；只有展开卡片投影 `0 4px 16px`。

## 验证

- `pnpm --dir frontend typecheck`、`node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs`（基线 294 通过）。
- 真机：胶囊收缩（内容/宽度/无阴影）、展开卡片、左右贴边迷你 + 400ms 探出、上下贴边保持完整胶囊；经典样式自由态、停靠条、探出、展开各过一遍。
- Pencil：`Jolu.pen` 的 A3-1 逐行对照收缩态截图。
