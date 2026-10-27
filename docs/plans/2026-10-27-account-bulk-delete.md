# 账号统一操作区与批量删除计划

鼠鼠要求详情、已签到状态/签到、删除放在同一操作区，并且账号管理、一键签到均支持批量删除。沿用当前真实签到状态（本轮/今日），不把单次已领取改写为整月完成。

## 方案

- 一键签到每个账号底部按「详情｜签到状态或签到操作｜删除」排列，移除账号行的独立更多菜单；原单账号删除确认、详情面板和领取防重复继续复用。
- 两处共用「批量管理」入口；进入后显示逐账号复选框、全选当前列表、已选数量、删除所选及退出。筛选变化/账号移除时清理不再可见的已选项，不删除隐藏的未选账号。
- 一键签到批量模式列出各已保存连接，包括暂停连接与原先被签到身份去重隐藏的连接；勾选的是明确连接 ID，不能删除未选的其他同身份连接。普通领取继续按真实身份去重。
- 删除前共用确认弹窗列出所选连接、平台与数量。仅移除 Jolu 连接及凭据，保留签到/用量历史和原工具登录；当前使用连接明确标注，不自动切换 CLI 或挑选替代账号。
- 顺序调用现有 `remove_connection`，单项失败保留并继续处理其他项；部分失败显示原因且仅重试失败项。同步 ref 防重复，执行期间禁用关闭、退出和重复删除；自动化引用等后端保护继续生效。
- 不修改供应商协议和现有删除接口；不执行真实用户账号删除。所有验收使用合成 IPC/数据。未授权 Git 写操作，保留现有工作区全部修改。

## 共享接口（锁定）

- `lib/accountActions.ts`：`retainAccountSelection(selected, availableIds): string[]`；`removeAccounts(ids, remove): Promise<BatchAccountDeleteResult>`；结果 `{deletedIds:string[], failed:{id:string,message:string}[]}`。去重、只处理传入 ID、不自动重试。
- `hooks/useAccountSelection.ts`：`useAccountSelection(items: readonly {id:string}[])` 返回 `{active, selectedIds, selected:ReadonlySet<string>, enter(),exit(),toggle(id),selectAll(),clear(),discard(ids)}`。
- `components/providers/AccountBatchControls.tsx`：导出 `BatchAccountTarget {id,label,provider,current?}`；`AccountBatchToolbar({selection,total,disabled?,onDelete})`；`BatchAccountDeleteDialog({targets,hidden?,onClose,onConfirm(ids)})`。确认组件合并已成功项，重试仅传失败 ID，全成功时关闭；失败理由在隐私态脱敏。
- 父页面负责执行期间阻止本页其他操作，删除后按成功 ID 刷新并清除选择；账号管理显式一次 reload，签到页面沿用 operating/pendingRefresh 协调事件。

## 任务

- [x] 共享选择/删除流程：先验证过滤失效、ID 去重、失败继续、只重试失败、空选择不调用，再实现 hook 和确认 UI。
- [x] 账号管理：顶部批量入口、当前筛选列表复选、当前使用说明、部分失败、刷新与隐私；独立合成验收。
- [x] 一键签到：底部三项操作并列、批量模式真实连接选择（含暂停/重复）、异步与领取互斥、原详情和单删回归。
- [x] Pencil 同步两处批量状态、确认及统一操作区；必要前端逻辑、类型/构建和合成 UI 浅深色/窄屏回归，独立审查。

验收：前端 249 项逻辑测试、类型检查与构建，签到、批量账号管理、原账号卡片及连接弹窗合成验收均通过；独立审查无阻塞，补齐复选框可区分标签。设计更新至 160，结果及真实账号验证边界见 `docs/verification/2026-10-27-account-bulk-delete.md`。
