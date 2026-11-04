# 胶囊灵动岛 · 执行文档（2026-11-04）

**交给：鼠鼠**
**状态：待确认后实施（先完成设计说明 §11 的确认项）**
**设计说明：`docs/specs/2026-11-04-capsule-island-design.md`**
**设计稿：`Jolu.pen` 模块 A3（A3-1 至 A3-10）；原型：`output/island-prototype/`**

---

## 0. 开工前

1. 鼠鼠确认设计说明 §11 的 5 项，尤其是「以 A3 取代 A2」和「迷你胶囊是否取代停靠条」。
2. 修改 `AGENTS.md` §1：浮动灵动岛的例外改为以 A3 为准，模块表新增 A3 一行，A2 标为被取代。
3. 原型只作行为参考，不直接搬代码：原型是原生 DOM + 字符串拼接，正式实现按项目的 React + Tailwind 写。

## 1. 分阶段

每个阶段都能单独验收、单独提交。阶段 1–3 只改前端，阶段 4 需要后端。

### 阶段 1 · 纯逻辑与 bot 组件

新增：

| 文件 | 内容 |
|---|---|
| `frontend/src/lib/botExpression.ts` | 由状态推出表情与原因：`botExpression(input) → { expr, reason }`，实现设计说明 §6.1 的优先级表 |
| `frontend/src/lib/botGaze.ts` | 视线目标：`gazeTarget(faceRect, pointer, { base, amp, reduced })`，以及插值一步 `gazeStep(cur, target, k)` |
| `frontend/src/lib/tokenTicker.ts` | 轮播节拍状态机：`first 2000 / tick 3000 / eyes 4000`；暂停与恢复；短格式 `formatShort(n)` |
| `frontend/src/components/island/Bot.tsx` | SVG bot：脸、眼睛（16 种形状）、额度环（0–2 圈）、屏幕数值、Token 面；props 只收数据，不读全局状态 |
| `frontend/src/hooks/useGaze.ts` | 订阅指针位置（阶段 4 前先用窗口内 `pointermove`），逐帧插值，写到眼睛的 `transform`，不触发 React 重渲染 |

要点：

- 眼睛插值与 Token 轮播都在 bot 内部用 ref 直接改 DOM，避免灵动岛每帧重渲染（现有 `LiveCountsContext` 已采用同样思路）。
- 额度环颜色阈值复用 `lib/quota.ts` 与 `authQuotaRows`，不另写一套。
- 「减少动效」用 `lib/motion.ts` 的 `reducedMotion()`。
- 动画帧用 `requestAnimationFrame`，并以 20ms 定时器兜底（窗口在后台时浏览器不出帧，原型里已验证这是必要的）。

测试（`node --test`，与源文件同名同目录）：

- `botExpression.test.mjs`：优先级表逐行覆盖；「一次性表情不回放」；API 连接与未连接的分支。
- `botGaze.test.mjs`：方向正确（夹角余弦 > 0.9）、上限不超过 13% / 10%、睡着幅度乘 0.3、减少动效时直接对准。
- `tokenTicker.test.mjs`：节拍顺序、暂停后恢复从眼睛开始、`formatShort` 不超过 4 个字符。

### 阶段 2 · 收缩胶囊与展开卡片

改动：

| 文件 | 改动 |
|---|---|
| `components/island/UsageIsland.tsx` | `IslandCollapsed` 换成胶囊布局；`IslandExpanded` 换成卡片布局，头部 + bot 140 + 图例 + 会话 + 底部。`IslandData` 结构不变，只改展示 |
| `components/island/IslandMotion.tsx` | 宽、高、圆角同时插值，弹簧曲线 `cubic-bezier(.32, 1.28, .48, 1)`；内容逐段淡入 |
| `components/island/SessionList.tsx` | 改成胶囊卡片里的行样式 |
| `views/IslandWindow.tsx` | 接入 `botExpression`；把拖动、刷新、额度、会话等状态传给 bot；收缩态宽度随内容（`useSyncWindowSize` 已支持） |
| `frontend/src/index.css` | 新增 `pn-capsule-*` 变量（浅深两套），值从 `Jolu.pen` 取 |

要点：

- 额度环交互（悬停、Tab、单击固定、Esc）只在展开态启用，用 `button` / `role="button"` 与 `aria-label`。
- 改 `innerHTML` 时焦点元素会失焦并触发 `focusout`，原型里出现过重入渲染报错；React 实现不手写 `innerHTML`，可以规避，但图例按钮要保持稳定的 `key`。
- 窗口尺寸：展开态比现在的 400 宽窄（380），`resize_island` 无需改动。确认 `IslandMotion` 的预留高度逻辑在新卡片高度下仍正确。
- 先保留现有 `DockedIsland` 停靠条；迷你胶囊是否取代它按 §0 的确认结果决定，放在阶段 3。

验收：用 Pencil 截取 A3-1/2/3/4，与 `pnpm tauri dev` 的实际窗口对照（浅深各一次）。

### 阶段 3 · 停靠、Token 轮播与气泡

| 改动 | 说明 |
|---|---|
| 贴边迷你胶囊 | 若确认取代：`DockedIsland.tsx` 换成高 32 的迷你胶囊，只留 bot 与额度环；悬停 400ms 恢复完整胶囊（沿用现有 peek 逻辑） |
| Token 轮播 | `Bot` 接入 `tokenTicker`，数值从 `LiveCountsContext` 订阅（与 `AnimatedTokens` 相同的追数键） |
| 会话气泡 | 有可信运行状态且 ≥ 2 个会话时，胶囊右侧显示会话数气泡；`sessionStatusUnknown` 时不显示 |

窗口尺寸注意：气泡在胶囊外侧，窗口宽度要把气泡算进去，`useSyncWindowSize` 量的节点要包住气泡。

### 阶段 4 · 后端事件与模型入口适配

后端（`backend/src/`）：

| 改动 | 位置 | 说明 |
|---|---|---|
| 指针位置推送 | `lib.rs` 新增后台任务 | 有灵动岛窗口可见时，约 30Hz 调 `AppHandle::cursor_position()`（Tauri 2.12.5 已核实，返回桌面左上角起算的物理坐标），位置变化才发 `island-pointer`，只发给灵动岛窗口；全部隐藏时停止采样 |
| 路由流水事件 | `gateway/wire.rs` 的 `record_route` | 在 `try_lock` 之前发 `gateway-route`，载荷即 `RouteLogEntry`，这样数据库忙、这条没写进库时界面仍能收到；`ProxyDeps` 需要能拿到 `AppHandle` 或一个发事件的回调（参照现有 `on_fallback` 的做法） |
| 请求开始事件 | `gateway/wire.rs` 选路完成处 | 发 `gateway-route-start`：ts、agent、请求模型、来源 slug。需鼠鼠确认（设计说明 §11 第 4 项） |
| 接线变化事件 | `gateway_wiring_set` / 漂移检测处 | 发 `gateway-wiring-changed`，灵动岛据此刷新头像与漂移气泡 |

前端：

| 文件 | 改动 |
|---|---|
| `hooks/useGaze.ts` | 改为订阅 `island-pointer`：物理坐标减去窗口 `outerPosition()`，再除以 `scaleFactor()` 得到窗口内的 CSS 坐标；窗口内 `pointermove` 仍保留，作为更低延迟的来源 |
| `hooks/useGatewayActivity.ts`（新增） | 只在 `gateway_status.mode === "relay"` 时启用；订阅上面三个事件，维护「当前转发」「最近一次去向」「今日成功 / 被拒数」；首次加载用 `gatewayApi.routeLog()` 与 `gatewayApi.status()` |
| `lib/gatewayActivity.ts`（新增）+ 测试 | 纯逻辑：流水 → 今日统计、按 Agent 取最近一次、被拒原因文案（429 → 冷却、401 → 凭据已失效、402 → 额度已用尽、503 → 已暂停） |
| `components/island/GatewaySection.tsx`（新增） | 展开态「模型入口」段 |
| `components/island/UsageIsland.tsx` | 收缩态接入 Agent 头像、转发路线、成功 / 被拒提示 |
| `backend/capabilities/` | 若灵动岛窗口需要调用 `gateway_status` / `gateway_route_log`，确认权限已覆盖 `island-*` 窗口 |

被拒原因：以状态码为准，与 `gateway/route.rs` 的拒绝分支一一对应（暂停 503、凭据 401、额度 402、冷却 429）。实施时再核对一遍。

测试：

- 后端：`cargo test`；给 `record_route` 的事件回调加单测，确认被拒记录 `source` 为空、事件不含凭证字段。
- 前端：`gatewayActivity.test.mjs` 覆盖今日统计跨零点、未知 Agent、被拒原因映射。

### 阶段 5 · 设计稿与文档收尾

- 用 Pencil 把 Token 轮播、视线跟随的说明补进 A3-1/2 与 A3-5/6。
- 更新 `AGENTS.md` 模块表的「现有实现」一列。
- 被取代的旧组件（若停靠条被取代：`DockedIsland.tsx`、`lib/dockQuotaView.ts` 中只服务停靠条的部分）删除，测试同步删除或迁移。

## 2. 验证

每个阶段至少：

```bash
pnpm --dir frontend typecheck
node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs
```

阶段 4 另跑 `cargo test --manifest-path backend/Cargo.toml`。

界面验收：

- `pnpm tauri dev` 实机检查：收缩、展开、停靠、拖动、右键菜单、分身，浅深色，系统「减少动画」开和关。
- 视线跟随：鼠标在灵动岛窗口外绕一圈，眼睛应持续跟随；多显示器、不同缩放比例（100% / 150%）各试一次。
- 模型入口：开启转发，用一个已接线的 Agent 发一次成功请求、一次被拒请求（例如暂停来源），对照 A3-9。
- 性能：灵动岛空闲时 CPU 占用不应明显高于现状。指针采样只在窗口可见时进行，位置不变不发事件。

## 3. 风险

| 风险 | 应对 |
|---|---|
| 30Hz 指针采样增加 CPU | 位置不变不发事件；灵动岛隐藏或系统锁屏时停止；必要时降到 20Hz |
| 透明窗口的背景模糊（`backdrop-filter`）在 WebView2 透明窗口上不一定生效 | 实机验证；不生效时浅色改用不透明白底 + 细描边，深色本来就是纯黑，不受影响 |
| 写流水用 `try_lock`，数据库忙时丢记录 | 事件在写库前发，界面不依赖写库成功；统计数字以库为准，界面注明「最近 20 条」 |
| 收缩态宽度随内容变化，频繁改窗口尺寸 | 沿用 `windowSizeSync` 的合并与容差；转发提示期间固定宽度 |
| 与分身共存 | 每个岛窗口各自订阅事件；`island-pointer` 广播给所有灵动岛窗口，由各窗口自行换算坐标 |
