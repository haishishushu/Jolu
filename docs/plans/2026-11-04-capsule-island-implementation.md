# 胶囊灵动岛 · 执行文档（2026-11-04）

**交给：鼠鼠**
**状态：阶段 1–4 已实现（2026-11-04），待真机验收；阶段 5 部分完成**
**设计说明：`docs/specs/2026-11-04-capsule-island-design.md`**
**设计稿：`Jolu.pen` 模块 A3（A3-1 至 A3-12）；原型：`output/island-prototype/`**

---

## 0. 开工前（已定）

1. 鼠鼠决定：A3 **不取代** A2，两种样式并存，由设置 `island_style`（`classic` 默认 / `capsule`）在「灵动岛」页切换；经典样式的代码与行为保持现状。
2. 迷你胶囊只在胶囊样式下取代停靠条；经典样式仍是 120×14 停靠条。
3. `AGENTS.md` §1 已更新：例外说明改为两种样式并存，模块表新增 A3 一行。
4. 原型只作行为参考，不直接搬代码：原型是原生 DOM + 字符串拼接，正式实现按项目的 React + Tailwind 写。

## 0.1 实现对照（2026-11-04）

| 内容 | 文件 |
|---|---|
| 样式设置 | 后端 `settings.rs` 的 `island_style` 与 `set_island_style`；前端 `useSettings.setIslandStyle`、`views/IslandView.tsx` 的「灵动岛样式」 |
| bot 与纯逻辑 | `components/island/Bot.tsx`、`lib/botExpression.ts`、`lib/botGaze.ts`、`lib/tokenTicker.ts` |
| 一次性表情与睡着 | `hooks/useBotShots.ts`（完成 / 惊讶 / 打招呼 / 额度重置；30 分钟无写入睡着） |
| 胶囊与卡片 | `components/island/CapsuleIsland.tsx`、`capsule.css`（`pn-capsule-*` 变量也在这里） |
| 贴边迷你胶囊 | `components/island/CapsuleMini.tsx`；悬停 400ms 探出完整胶囊 |
| 视线跟随 | `hooks/useGaze.ts`（窗口内 pointermove + 后端 `island-pointer`）；后端 `island_pointer.rs` |
| 模型入口 | 后端 `gateway/activity.rs`、`gateway/wire.rs`；前端 `lib/gatewayActivity.ts`、`hooks/useGatewayActivity.ts`、`components/island/CapsuleGateway.tsx` |
| 打开主窗口页面 | 后端 `open_main_section`（「打开模型入口」「修复」） |

真机交互修正（2026-11-04，真机验收后）：

| 问题 | 原因 | 处理 |
|---|---|---|
| 双击无法展开 | 收缩层自己是 `role="button"`，双击处理器把所有 `[role=button]` 都排除掉了 | 收缩态直接展开；展开态只排除真正的交互控件 |
| Esc 不生效 | 只挂在外壳的 `keydown` 上，展开后焦点不一定在外壳 | 改为窗口级 `keydown`；连接下拉打开时先交给下拉 |
| 点卡片外不收回 | 原型靠 document 事件，桌面端窗口外收不到 | 窗口内空白与 document 上「卡片外」都收回；点别的窗口走窗口失焦；右键菜单打开时排除失焦误判 |
| 展开时胶囊整块往右跳 | 原生窗口左上角固定，内容在窗口里居中 | 胶囊自由态改为按水平中心缩放窗口（`resize_free_centered`），与原型一致 |
| 窗口比胶囊宽一截 | 容器上的 `min-w-[400px]` | 去掉，窗口完全跟内容 |
| 收回箭头不符合原型 | 原型没有该按钮 | 移除，收回靠双击、Esc、点卡片外 |
| API 态漏「Token」字样 | 文案遗漏 | 补上；本机 Token 不再在底部重复一行 |
| 成功绿点残留 30 秒 | 只按 30 秒刷新 | 成功 10 秒后再刷新一次 |

真机验证（隔离目录 + CDP，`island_style=capsule`）：双击展开/收回、Esc、点卡片外、停靠迷你胶囊、悬停 400ms 探出、探出后双击展开、双击迷你胶囊解除停靠、窗口外鼠标视线、连接下拉展开、右键菜单不误收起、设置页切换样式、经典样式回归，均通过。

「点别的窗口收回」走原生 `WindowEvent::Focused(false)` → 全局 `island-blur`（按岛 id 过滤）。CDP 的合成点击不会让窗口取得系统焦点，这一条无法在无头环境模拟，需真机点一下别的窗口确认；窗口内边距上的「点卡片外」已用真机点击验证。

与本文原计划的差异：
- `IslandWindow.tsx` 按样式分流，经典样式继续用 `UsageIsland` / `IslandMotion` / `DockedIsland`，没有改它们的布局。
- 胶囊自己做宽、高、圆角插值（不经 `IslandMotion`）；卡片超过窗口可用高度时整卡内部滚动。
- 指针推送只在「胶囊样式 + 灵动岛可见 + 非免打扰」时采样，拖动窗口期间暂停；Windows 直接用 `GetCursorPos`，不经事件循环。
- 今日经网关次数来自新增的 `gateway_today_counts`（按本地零点统计流水表），不再用最近 20 条流水估算；快照读取期间到达的实时结束事件按计数时刻补加。
- 两个事件都用普通 `emit`：Tauri 对前端默认目标的监听不做窗口过滤，订阅范围由前端控制（只有灵动岛窗口订阅）。
- 订阅分支协议不匹配时直接返回 400，不发开始事件（与 Direct 分支一致）。
- 切换连接或平台时只更新额度基线，不触发「额度重置」表情。
- 胶囊外壳是 `role="group"`；展开 / 收回由收缩层（`role="button"`）和卡片头部的收回按钮负责，隐藏的那一层用 `inert`，Tab 不会落到看不见的控件上。
- 自动化「等额度」表情（`wait`）尚未接数据源，表情库里保留，暂不触发。

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
| 请求开始事件 | `gateway/wire.rs` 的 `handle`，`decide_with_affinity` 返回 `Direct` / `Subscription` 之后、调用适配器之前 | 发 `gateway-route-start`：`request_id`、ts、agent、请求模型、来源 slug。`Reject` 分支不发（拒绝是即时的，直接发结束事件）。`gateway-route` 带上同一个 `request_id` 用于配对；id 用进程内自增（如 `AtomicU64`）生成，只在内存和事件里用，不写库。上游出错、超时、客户端断开的分支也必须发结束事件。鼠鼠已确认新增（2026-11-04），完整规则见设计说明 §9.1 |
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
- 后端：请求开始事件的单测，覆盖成功请求先发开始、后发结束且 `request_id` 相同；被拒只发结束；上游出错也发结束；并发两个请求的 id 不同。
- 前端：`gatewayActivity.test.mjs` 覆盖今日统计跨零点、未知 Agent、被拒原因映射；开始与结束按 id 配对、只有结束时直接显示结果、只有开始时 120 秒收回、并发时显示最新一条。
- 对照原型：`output/island-prototype/compare.html` 的有 / 无对比和 `index.html?checkStart=1` 的 18 项检查。

### 阶段 5 · 设计稿与文档收尾

- ~~用 Pencil 补 Token 轮播与视线跟随~~：已补为 A3-11 / A3-12。
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


## 4. 2026-11-05 返工：贴边动效与卡片生硬

鼠鼠反馈「胶囊停靠状态动效和卡片很生硬」，要求动效和样式全部按 `output/island-prototype` 的 HTML 走，胶囊与经典样式各自独立。

### 4.1 根因

- 旧实现先等原生窗口 `resize_island` 完成（`enough()` + 200/320ms 兜底）再改外壳尺寸，动效永远慢半拍，看起来是「卡住 → 突然跳」。
- 停靠时探出会把 `zoom` 在 `island_shrink_scale` 与 `island_scale` 之间切换，缩放与形变互相打断。
- 内容框按停靠边 `items-*` 对齐，窗口变大时外壳被窗口左上角带着跑，左侧/底部停靠会整块平移。
- 悬停探出挂在整窗上（`p-3.5` 外边距也算），响应区和视觉不一致。

### 4.2 现在的做法

- 外壳尺寸 = 目标尺寸，窗口尺寸 = 实测「预留盒子」（`reserve`）：变大立刻跟上，变小等外壳缩完（`SHRINK_HOLD_MS = 460`，覆盖 420ms 弹簧）再收，窗口永远先到位、不裁切。
- 外壳按停靠边绝对钉住：`top-0 left-1/2 -translate-x-1/2`（上/自由）、`bottom-0 …`、`left-0 top-1/2 -translate-y-1/2`、`right-0 …`，窗口变大时内容不位移。
- `zoom` 在胶囊里恒为 `island_scale`；迷你胶囊按原型固定 32px（bot 26），不再受「缩小后大小」影响（该设置只服务经典停靠条，设置页文案已注明）。
- 悬停 400ms 探出、移开立即缩回，挂在 `.cap-shell` 上而不是整窗；手势（双击展开/收回、Esc、点卡片外、失焦收回）保持不变。
- 气泡（运行中会话 / 接线漂移）改成三栏网格 `1fr auto 1fr`：原生窗口按中心对齐，岛永远在正中，气泡出现时不再把岛推开；左右停靠用单行 flex（贴边不居中）。
- 样式对齐原型：阴影静置 `0 8px 24px`、悬停 `0 10px 30px`、展开 `0 16px 40px`；迷你胶囊增补 `cap-pulse` 呼吸点。

### 4.3 验证

- `pnpm --dir frontend typecheck`、`node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs`（294 项）通过。
- 浏览器预览用 CDP 采样形变（`?window=island`，临时把默认样式改成胶囊再还原）：

| 过渡 | 采样结果 | 结论 |
|---|---|---|
| 迷你 → 探出 | 400ms 后开始，50→96→246→329→352 宽、32→40 高，13 个不同宽度，中心恒为 500 | 弹簧过渡连续，无位移 |
| 探出 → 迷你 | 立即开始，352→262→129→62→43→50（轻微回弹），24 个采样值 | 与原型一致的回弹 |
| 迷你 → 卡片 | 50→273→319→360→379→380 宽、32→456 高，最值 466.8 后回落，中心恒为 500 | 卡片不再是硬切 |
| 卡片 → 迷你 | 380→200→91→48→42→50，中心恒为 500 | 收回同样连续 |

- 贴边锚定：上下停靠时外壳贴住窗口的上/下边，左右停靠时贴住左/右边，卡片打开期间保持同一边（`openDockEdge` 记忆），收回后按指针位置回落迷你或完整胶囊。

### 4.4 真机反馈后的调整（同日）

- **贴边不投影**：`data-docked="true"` + `data-open="false"` 时 `box-shadow: none`。窗口四周边距只有 14px，24px 模糊的阴影会被窗口边缘切出一条硬边（看起来是胶囊底下多了一条阴影）；卡片仍保留 `0 16px 40px`。
- **顶部 / 底部贴边不进入迷你态**：原型只有 `.dock.docked-left` / `.docked-right`，迷你胶囊是左右贴边的形态。「缩小后大小」式的停靠条形态在上下贴边没有对应设计，现在上/下贴边保持完整胶囊（仍按边锚定、仍可在卡片内操作），悬停也不再触发 400ms 探出。
- **展开后的卡片可以拖动**：去掉原型「open 时 pointerdown 直接返回」的移植（原型是网页，卡片不需要挪动；桌面端要把窗口拖走）。拖动仍避开按钮、输入框和链接，拖动结束后若吸附到边缘按原逻辑收回。

### 4.5 阴影彻底去掉（胶囊本身）

上一版只让「贴边」不投影，自由态的胶囊仍然带 `0 8px 24px`，而窗口内边距只有 14px，24px 的模糊照样被窗口边缘切断，看起来还是「胶囊底下多一条阴影」。现在：

- `.cap-shell` 静态与悬停都不投影（`box-shadow: none`），贴边动态的例外规则也一并删掉：任何状态下胶囊本体都是干净的一层描边。
- 只有展开的卡片投影，取值改成 `0 4px 16px`：偏移 4 + 模糊 16 = 20px，正好等于窗口内边距，永远不会被切边。
- 胶囊窗口的内边距 `p-3.5` → `p-5`（14 → 20px），把卡片阴影的 20px 完整包进窗口；迷你胶囊、贴边完整胶囊的锚定逻辑不变。
- `.cap-bubble`（运行中会话、积分气泡）阴影同步降到 `0 3px 12px`，同样落在窗口内。

## 5. 2026-11-05 追加：API Key 胶囊的「正在写入」与多会话气泡（对照原型）

按 `output/island-prototype/app.js` 的 `collapsedHTML()` / `bubblesHTML()` / `expression()` 逐条核对收缩胶囊，
补齐胶囊侧的数据层字段并修正增量段的显示条件。`views/classic/` 与经典专用组件没有改动，行为不变。

### 5.1 数据层

| 字段 | 真实来源 | 拿不到时 |
|---|---|---|
| `data.working` | `useLiveUsage` 的会话列表里存在 `state === "running"`；后端按请求开始 / 结束事件与会话存活状态判定。采集源出错（`collection.status.failed_sources` 含本平台）时为 `false` | `false`（不显示增量段、不出气泡） |
| `data.liveDelta` | `live-usage` 推送的 `running_tokens`（`db::sum_running_tokens`：只把 running 会话的本轮 Token 求和）；`useLiveUsage` 新增 `runningDelta` 暴露它，只在推送到达时更新，不参与逐帧追数 | `null`（胶囊只显示破折号，不写 0） |

- `IslandData` 新增两个可选字段（`working?` / `liveDelta?`），注释标明「胶囊样式用，经典样式不读」。
- 网关活动（`useGatewayActivity` / `gateway_today_counts`）只有「请求在途」和成功 / 被拒 / 失败次数，没有 Token 增量，
  所以没有拿它当这两个字段的来源；增量必须是本机会话记录里的真实数字。
- 浏览器预览（`mock/data.ts`）没有这两个字段，胶囊按会话状态自行推断 `working`，增量回落到 `deltaText` 示例文案。

### 5.2 收缩胶囊的增量段

- 出现条件（2026-11-05 晚改为常驻，见 §5.6；原型是 `const live = S.working || (S.shot && S.sessions.length)`）：
  `!unavailable && !relay && (carried != null ? carried !== 0 : Boolean(deltaText))`。
  Auth 分支同样有这个段（原型 auth 分支也是 `quotaHTML() + wait + live`），不是只有 API 分支。
- 内容：`<span class="cap-hair" />` + `.cap-live`（绿色 + 脉冲点，`.cap-live` 自带 `::before` 圆点），
  数字用 `compactTokens` 格式化（`+12.8K`）。原型只有数字、没有 `Token` 后缀，这一处去掉了后缀。
- 增量未知：只显示 `—`（与额度芯片、今日 Token 的未知写法一致），不写 `+0`；`deltaText` 有值时（浏览器预览）用它。
- 宽度：段在同一个 `.cap-main` 里，`mainW` 由 `ResizeObserver` 实测，`capsuleW = mainW + 59` 自动包含新段；
  `.cap-cl` 是 `white-space: nowrap`，外壳 `overflow: hidden` 只会在测量前那一帧有可能切边，下一帧跟平；
  窗口仍按预留盒子「变大立刻跟、变小等形变结束」，数字位数变化只让胶囊宽 1–2px，不裁切不换行。

### 5.3 多会话气泡

- 条件 `writing && running >= 2`（原型 `S.working && S.sessions.length >= 2`），只在完整胶囊出现，卡片与迷你态不画。
- 数字用 running 会话数，失败 / 最近活跃不计入；状态不可判定时整段不渲染（不假装有会话在跑）。
- 位置沿用 A3-1：`.cap-bubble` 在胶囊右侧 8px、高 40、圆角 20，转圈用 `.cap-spinner`（`inset: 4px`），
  三栏网格保证气泡出现时胶囊不位移。

### 5.4 表情优先级

`lib/botExpression.ts` 与原型 `expression()` 的顺序逐条一致：出错 › 网关被拒 › 额度用尽 › 拖动 › 任一窗口 ≥ 80%
› 等额度 › 刷新中 › 转发中 › 转发成功 › 工作中 › 一次性表情 › 睡着 › 打招呼 › 贴边 › 悬停 › 空闲，不需要改。

### 5.5 验证

- `pnpm --dir frontend typecheck` → exit 0。
- `node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs` → 294 项全过。

### 5.6 2026-11-05 追加：工作中眼睛变绿、增量段常驻

真机看过之后提的两条修改。只动 `components/island/` 与两个 hook，经典样式（`UsageIsland.tsx` / `views/classic/`）的行为不变。

**眼睛变绿（只在工作中）**

- `Bot.tsx` 根节点 `<svg class="cbot">` 新增 `data-expr={expr}`，值就是 `lib/botExpression.ts` 算出的表情；`work` 只在 `writing === true` 且没有更高优先级状态（出错 / 网关被拒 / 额度用尽 / 拖动 / 任一窗口 ≥ 80% / 等额度 / 刷新中 / 转发中）时出现。
- 颜色由 `capsule.css` 一条规则给出：`.cbot[data-expr="work"] .cbot-eyes-inner > * { fill: var(--pn-capsule-green) }`。
  命中的是最内层眼睛形状——`.cbot-eyes-inner` 的直接子节点，即两只眼睛各自渲染的 `<rect>`（`work` 的 `pill` 形状）：
  CSS 的 `fill` 声明覆盖 `Bot.tsx` 里 `EYES[expr].c` 写出的 SVG `fill` 属性。
- 不新增颜色变量：浅色用 `--pn-capsule-green: #28a745`（bot 底色 `--pn-capsule-bot: #1d1d1f`，对比约 5.4:1），
  深色自动翻成 `#30d158`（底色 `#2c2c2e`，约 6.9:1）。只动眼睛，bot 底色与额度环不动。
- `EYES.work.g`（眼睛外那圈 drop-shadow 光晕）一起换成 `color-mix(in srgb, var(--pn-capsule-green) 55%, transparent)`，
  免得浅色主题下深绿瞳孔外面套一圈亮薄荷光晕。
- `work` 之外的表情（一次性表情 `happy / surprise / greet / reset`、`refresh` 的青、`worry` 的橙、`error` 的红叉、`sleep` 的灰线）
  不命中这条规则，表情库预览保持原色；`.cbot-blink` / `.cbot-scan` 的 keyframes 作用在 `.cbot-eyes-inner` 的 `transform` 上，
  与 `fill` 无关，眨眼与扫视照旧。

**增量段常驻**

- 值：`data.carriedDelta` 取 `live-usage` 推送的 `today_tokens`（`db::today_fresh_tokens`：今天已落库请求的新鲜 Token 合计）。
  它与 `running_tokens` 不同——会话结束后仍是同一个数，跨天由后端重置；只有它为 null 或 0（今天还没落库任何请求、
  而第一个请求正在写入）时才退回本轮 `running_tokens`。前端不自己累加，也不做日期判断。
- 条件：`const carried = data.carriedDelta ?? data.deltaTokens ?? null`，
  `liveVisible = !data.unavailable && !relay && (carried != null ? carried !== 0 : Boolean(data.deltaText))`。
  不再依赖 `writing || deltaPhase !== "idle"`，所以会话结束时这一段留在原地；被清零成 0 / null 后自然消失
  （没有数值时退回 `deltaText`，只在浏览器预览的示例数据里出现）。
- 追数：`useLiveUsage` 给今日累计加了一个追数键 `today`（`counter.setTarget("today", u.today_tokens, ...)`，
  数值不变的心跳推送不重跑补间），经 `useIslandWindowModel` 的 `data.carriedCountKey` 传下去；
  `LiveTokens` 只在 `writing` 为真时订阅 `countKey`，结束后传 `undefined`，数字停在 `tokens` 的最后读数，不再逐帧。
  卡片里分区头部的 `AnimatedTokens` 同样只在 `writing` 时订阅 `data.deltaCountKey`，结束后显示常驻值，
  而不是追数被清空后的 `+0`。
- 视觉不变：仍是一个 `.cap-hair` + 绿色 `.cap-live`（`+xx.xK`），宽度继续由 `mainW` 实测驱动（`capsuleW = mainW + 59`），
  `.cap-cl` 是 `white-space: nowrap`，位数变多只让胶囊宽几像素，不换行不裁切。
- 验证：`pnpm --dir frontend typecheck` → exit 0；`node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs` → 294 项全过。

### 5.7 2026-11-05 追加：增量段改成 A + B 组合（本轮终值锁存 + 今日累计兜底）

§5.6 的常驻值只有一个来源——今日累计写入。会话结束时它停在今日总量（`+2.3M`）上，那不是「本轮写了多少」；
应用重启（内存态丢干净）或当天第一笔请求还没落库时，也只靠它兜底。这一版把两件事拆开，优先级写死。

**显示优先级：锁存值 ?? 今日累计**

| 情况 | 显示 | 来源 |
|---|---|---|
| 有本轮锁存值（> 0） | 本轮写入的终值 | A · `running_tokens` 的最新非零读数（只含 running 会话） |
| 没有锁存（应用重启、当天第一笔还没落库） | 今日已落库合计 | B · `today_tokens` |
| 两者都未知 / 为 0，或采集源出错 | 整段不渲染（胶囊收窄） | `null` |

表达式（`lib/carriedDelta.ts` + `hooks/useIslandWindowModel.ts`）：

```ts
const carried = carriedDeltaView({ latched: latchedDelta, todayTokens: live.todayTokens, sourceFailed })
// value  = latched > 0 ? latched : todayTokens > 0 ? todayTokens : null
data.carriedDelta   = carried.value                       // 数字
data.carriedSource  = carried.source                      // "session" | "today" | null，决定文案
data.carriedCountKey = carriedDeltaCountKey(carried.source, working) // 只有进行中才有追数键
```

**锁存在哪**

- 只存一处：`useIslandWindowModel` 的 `latchedRef`（`useRef`，判定源）+ `latchedDelta`（`useState`，渲染镜像）。
  渲染体按 ref 递推——同一帧里 React 可能执行两次渲染体（严格模式 / 并发重放），按 ref 递推才能每一步都拿到
  上一次的锁存值；`useLayoutEffect` 把 ref 的值同步进 state，提交后、绘制前落定，不会出现一帧旧读数。
  `useLiveUsage` 不新增锁存，前端任何地方都不累加 Token。
- 判定规则，纯函数 `latchSessionDelta`：
  1. `working` 为真且 `running_tokens` > 0 → 记这个最新值（本轮累计写入）；
  2. 本轮刚起跑（0 / 未知）→ 保持上一次锁存，避免被 0 抹掉后闪一下；
  3. 没有可记的本轮值，且 `today_tokens` 归 0 / null → 清空；
  4. 其余情况（会话已结束、今日累计还有值）→ 停在最后一个锁存值上，不跳变、不重新起跳。
- 新一轮：`working` 重新为真、`running_tokens` 从 0 / 较小值往上走时被新值覆盖，等于从新的一轮重新计
  （不做「只涨不跌」的取最大）。

**清空时机**

| 时机 | 具体触发点 |
|---|---|
| 跨天 | 后端把 `today_tokens` 重置为 0（区间内无记录）或 null（有 total 未知的历史行）→ 规则 3 清空。前端不判日期，昨天的终值不会一直挂着；跨过午夜而本轮仍在写时按规则 1 继续记本轮（那不是昨天的终值） |
| 手动刷新 | 右键「立即刷新」、连接更新、平台切换都走现有刷新入口，`refreshSerial` 递增 → `resetMark` 发现变化就把锁存置空；仍在写的会话在下一次推送重新锁存，没有本轮值就回落到今日累计 |
| 重启 | 内存态锁存随进程消失（`latchedRef` / `latchedDelta` 初始为 `null`），显示自动落到 B 的今日累计 |

清空后既没有本轮值、今日累计也是 0 / null → `carriedDelta` 为 `null` → `liveVisible` 为假 → 整段不渲染，
胶囊按 `mainW` 实测宽度收窄。

**两种来源的文案**

数字外观不变（`.cap-hair` + 绿色 `.cap-live` 的 `+xx.xK`），来源差异写在 `title` 与 `aria-label`，并用
`data-carried-source` 标出来源：

| 来源 | `title` / `aria-label` 原文 |
|---|---|
| `session`（锁存） | `本轮写入 +30.2K（本轮累计 30,200 Token；会话结束后停在这个读数，不再增长）` |
| `today`（兜底） | `今日累计写入 +2.30M（今日已落库 2,300,000 Token，含今天所有会话）` |
| 无数值 | `本轮写入暂无数据` / `今日累计写入暂无数据`（浏览器预览走 `deltaText` 示例分支，不报错） |

**结束后无动画**

- 追数键只在 `working` 为真时给：锁存分支订阅本轮合计 `total`，兜底分支订阅 `today`；结束后 `carriedCountKey`
  为 `undefined`，`LiveTokens` 的 `countKey` 也传空 → `useLiveCount` 取不到帧值，直接显示 `tokens` 的静态读数，
  不跑补间、不跳回 `+0`。
- `working` 期间的实时追数保持原样（本轮 `total` 的 900ms 补间、心跳推送不重跑补间都不变）。
- 展开卡片头部的 `AnimatedTokens` 沿用 §5.6 的判定（`writing ? (liveDelta ?? deltaTokens) : carried`），
  结束后显示锁存的本轮终值，`countKey` 传空——本来就没有「结束后跳回 +0」的问题，无需另外改。
- 视觉不变：宽度仍由 `mainW` 实测驱动（`capsuleW = mainW + 59`），不换行、不裁切，不新增颜色变量
  （仍是 `--pn-capsule-green`）。

**验证**

- `pnpm --dir frontend typecheck` → exit 0。
- `node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs` → 306 项全过（基线 294 + 新增
  12 项 `lib/carriedDelta.test.mjs`：锁存递推、新一轮覆盖、0 / 未知不抹值、结束后停住、跨天清空、
  跨午夜进行中不清、手动刷新回落今日、显示优先级、0 / null 不渲染、采集源出错、追数键）。

### 5.8 2026-11-05 追加：ZCode 回归——眼睛不变绿、增量段整段消失

真机回归（ZCode 连接，会话正在写入 Token）：胶囊左侧 bot 眼睛不变绿，尾部原本的绿色 `+xx.xK`
整段消失，只剩 `Zcode —`。§5.7 把增量段改成 A + B 时同时收紧了两处条件，这一节记录实测值、根因、
新的显示优先级，以及 `working` 的新信号源。

**实测（CDP `Runtime.evaluate` + 后端返回值 + 本机库，2026-11-05 18:36:12）**

| 观察点 | 实际值 |
|---|---|
| `live_usage({platform:"zcode", since:null})` 的 `running_tokens` | `null` |
| 同上 `today_tokens` | `null` |
| 同上 `active_sessions` | 1 条：`state: "done"`、`running_tokens: null`、`delta_tokens: 748852`、`last_seen_ms` = 18:36:12 |
| 同上 `active_window_seconds` / `cursor` | `90` / `18922` |
| `collection_status` | `ok: true`、`failed_sources: []`、`consecutive_failures: 0` → `sourceFailed === false` |
| `gateway_status` | `mode: "observe"`、`enabled: false` → `useGatewayActivity().relay === false`，`activity.inFlight` 恒为空 |
| `usage.db` 里 zcode 今日请求 | 89 条；88 条 `total_known = 1`，**1 条 `total_known = 0`**（00:47:59 的失败请求，`raw_usage_json` 为空、`provider_total_tokens` 为空）→ 今日合计 20,253,295 Token 但 `today_fresh_tokens` 的 `CASE WHEN MIN(total_known)=1 … END` 落到 NULL |
| ZCode 来源库 `model_usage.status` 分布 | `completed 7904` / `error 110` / `cancelled 29`，**没有任何 running / pending 行** |
| island 窗口 DOM | `.cap-shell[data-cap-expr="peek"]`、`.cbot[data-expr="peek"]`、**无 `.cap-live`**、`.cap-main` 文本 = `Zcode` + `—` |

**根因（两条条件收紧 + ZCode 的结构性事实）**

1. `working` 只剩 `live.sessions.some(s => s.state === "running")`（并对 `sourceFailed` 直接置 false）。
   ZCode 不是生命周期来源（`db::live_usage` 里 `lifecycle` 只含 codex / claude），
   `running_tokens` 恒为 `null`；它的 `model_usage` 只在请求**结束时**落行，
   `zcode_session_state` 因此永远算不出 `running`，`sessions.activity_state` 只会是 done / failed。
   → 眼睛从来没有变绿的信号。
2. `liveVisible` 依赖 `carried`（`carriedDelta ?? deltaTokens ?? null`）非空非 0。`carried` 的两条腿对 ZCode 都不成立：
   A（锁存）要 `working && running_tokens > 0`，`running_tokens` 恒 null；
   B（今日累计）要 `today_tokens > 0`，当天只要有一条请求的 total 未知就整天返回 null（上表第 7 行）。
   → 增量段整段被吞掉。
3. `sourceFailed` 只是把这两个症状坐实：它把 `islandSessions` 的状态改成 `unknown`、把 `working` 压成 false，
   而它本来不该否认「明明有请求在途 / 刚刚还在写」。

**新的显示优先级（`lib/carriedDelta.ts` 的 `carriedDeltaSegment`）**

| 情况 | 显示 | 来源 |
|---|---|---|
| 有本轮锁存（> 0） | 本轮写入终值 `+xx.xK` | A · `running_tokens` 锁存 |
| 否则今日累计（> 0） | 今日已落库合计 | B · `today_tokens` |
| 否则仍在写入（`writing`） | **照常渲染这一段，数值显示 `+—`**（未知，`data-carried-unknown="true"`） | 无数值 |
| 三者都没有 | 整段不渲染（胶囊收窄） | `null` |

浏览器预览的 `deltaText` 示例文案继续优先于 `+—` 分支，`data-carried-source` 文案机制（§5.7 的
`session` / `today` 两套 `title`）原样保留，只为未知分支加了 `data-carried-unknown` 与
`本轮正在写入，读数未知（该来源不返回本轮 Token，也没有今日累计可兜底）` 的 `title` / `aria-label`。

**`working` 的新信号源（`lib/islandWriting.ts` 的 `islandWriting`，三个并列，任一为真）**

1. `live.sessions` 有 `state === "running"`（codex / claude 的生命周期来源，原有信号）；
2. **网关请求在途**：`gateway.relay && gateway.activity.inFlight.length > 0`
   ——`useGatewayActivity` 的 `inFlight` 就是「请求在处理中」的真实事件；只有 relay 模式才有
   `gateway-route-start` / `gateway-route` 事件，直连连接（上表 `mode: observe`）恒为 0，不会误报；
3. **最近活跃窗口内仍有写入**：来源只给「最近活跃」时（`activityKind === "recent"`，含 zcode），
   最新会话的 `last_seen_ms` 落在 `active_window_seconds`（90 秒）内就是运行证据
   ——没有生命周期可读时，「刚刚写进去了 Token」是唯一可信的运行痕迹。
   未来时间戳留 60 秒容差，超出不算。

`sourceFailed` 不再参与 `working` 判定：它只让**展示**的会话状态变「未知」
（`islandSessions` 仍然逐条改 state），不再把在途请求压成 false。

**关键表达式**

```ts
// useIslandWindowModel.ts
const gateway = useGatewayActivity(capsule)          // 前移：working 要读它的 inFlight
const writing = islandWriting({
  sessions: live.sessions,
  recentWindow: live.activityKind === "recent",
  activeWindowSeconds: live.activeWindowSeconds,
  inFlight: gateway.relay ? gateway.activity.inFlight.length : 0,
  now: quotaNow,
})
const working = writing.working                       // running 会话 ∥ 网关在途 ∥ 窗口内仍有写入

// CapsuleIsland.tsx
const segment = carriedDeltaSegment({
  value: data.carriedDelta ?? data.deltaTokens ?? null,
  source: data.carriedDelta != null ? data.carriedSource ?? null : null,
  text: data.deltaText ?? null,
  writing,                                            // data.working ?? 本地推断
  unavailable: Boolean(data.unavailable),
  relay: Boolean(relay),
})
```

A + B 的既有语义不变：结束后停在锁存读数、不跳变；跨天 / 手动刷新清空锁存；前端不累加 Token。

**验证**

- `pnpm --dir frontend typecheck` → exit 0。
- `node --test frontend/src/lib/*.test.mjs frontend/src/hooks/*.test.mjs` → **324 项全过**
  （基线 306 + 新增 18 项：`lib/islandWriting.test.mjs` 10 项——running 信号、ZCode 的最近活跃信号、
  窗口边界、生命周期来源不把 done 当 running、网关在途单独驱动、三者皆无、未来时间戳容差、
  `active_window_seconds` 为 0 的兜底、`lastWriteAt`；`lib/carriedDelta.test.mjs` 新增 8 项——
  锁存显示、今日累计显示、未知但仍在写入显示 `+—`、静止态不凭空出现 `+—`、`value === 0` 与未知同处理、
  预览文案优先、连接不可用不渲染、网关占位不渲染）。
- 用实测快照回放新逻辑（同一份 18:36:12 的 `live_usage`）：
  静止态（+6 分钟）→ `writing=false`、段不渲染；写入中（最后落库后 12 秒）→ `writing=true`、
  段渲染为 `+—`；采集源出错 + 网关在途 → `writing=true`、`+—`；生命周期来源 running → `working=true`。
- island 窗口实测（18:36:12，改动前的运行中实例）：`.cap-shell[data-cap-expr="peek"]`、无 `.cap-live`、
  `.cap-main` = `Zcode —`——静止态没有凭空出现 `+—`。改动后同一窗口的 DOM 复查需在应用重新启动后进行
  （该实例在 18:41 前后已退出，本文档不代替那次复查）。



