# 签到卡摘要、详情与删除入口验收

## 改动总结

- 主卡固定展示今日奖励、本周/本月签到天数、总额度/可用额度；长说明、分项奖励、累计及全部资源包移入详情。非手动签到版本的周/月显示「不适用」。
- 摘要只选单一范围或明确的 Trae 积分汇总，多独立范围显示「多项额度」及项数；计算余额仍明确标记，零、未知、失败、过期分别保留。
- 详情按钮放底部左侧，与签到按钮并排，打开右侧 480px 面板，窄屏适配视口；完整内容使用同一个 quota hook 的结果，不新增请求路径。
- 删除入口从独立垃圾桶改到每个账号行右侧「⋯」账号操作，菜单内为红色删除项。复用既有确认、执行冻结、防连点、失败保留与历史保留。
- 详情开关由供应商卡按连接 ID 持有，同一连接刷新/身份重挂后保持详情并更新数据；账号移除或页面失活时关闭。隐私态标题和无障碍标签使用隐藏值。

## 验证

- 新增摘要选择、失败与过期回归先出现失败；实现后 `node --test frontend/src/lib/checkinStats.test.mjs`：18 通过、0 失败。
- `pnpm --dir frontend typecheck`：通过。
- `pnpm --dir frontend build`：通过。仅有既有 Skills mock 和 Tauri window 静态/动态混用的分包提示。
- `JOLU_COMPONENT_URL=http://localhost:5173 node scripts/acceptance/daily-checkin.cjs`：通过。真实 React、合成 IPC，覆盖统一摘要、多范围不混加、右侧详情、窄屏、Esc 关闭、隐私、打开详情不增加额度请求、刷新保持详情、账号菜单删除确认/失败/取消/防重复与历史保留，以及原六版本/去重/批量/切页流程。
- `node scripts/acceptance/account-connect-dialogs.cjs`：通过，既有账号连接与保存、右侧通知流程无回归。
- 独立只读审查无阻塞问题；核对身份刷新、菜单绑定、摘要单位及未知/计算值、同一查询复用。
- Pencil 149 / 150（统一摘要）、151 / 152（右侧详情）、153 / 154（账号菜单）与实际浅深色截图对照；删除确认仍沿用 147 / 148。当前开发 Jolu 进程保持响应，前端由 Vite 热更新。

截图均为合成数据：`output/acceptance/daily-checkin/statistics-a-light.png` / `statistics-a-dark.png`、`account-details-light.png` / `account-details-dark.png` / `account-details-narrow.png`、`account-menu-dark.png`。

本轮只改前端展示，未改后端供应商协议、持久化或账号数据，未执行真实删除、领奖、Git 暂存、提交、推送或分支操作，因此不重复执行后端全量测试。
