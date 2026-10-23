# 账号信息与切换交互验收

## 改动总结

- 账号卡片保留一行四列。Claude/Codex 固定展示 5 小时和周额度，缺失窗口显示未提供，不伪造剩余额度或重置时间；服务返回的额外窗口不再被截掉。
- 接入后端订阅元数据，历史记录标明来源和最近记录日期，不能据此显示实时订阅有效或已到期。
- Codex 显示重置卡总次数与当前可用次数，查询按钮只刷新该账号额度，不消费重置卡；其他来源未支持时明确说明。
- 账号切换入口无障碍名称/悬浮提示统一为“设置为使用”。删除并切换时先验证替代账号切换成功，失败保留原连接。
- 修复凭证读取与写入目录不一致：统一 CODEX_HOME / CLAUDE_CONFIG_DIR。Codex 自定义供应商 env_key 的读取与添加入口保持一致，缺少指定变量时拒绝使用其他保存的 Key。

## 验证证据

- `node --test frontend/src/lib/accountDisplay.test.mjs`：9 项通过，含缺失窗口、额外窗口、真实零、本机快照不冒充实时订阅。
- `node --test frontend/src/lib/accountActions.test.mjs`：2 项通过，替代账号失败不删除原连接，成功按顺序切换并删除。
- `cargo test --manifest-path backend/Cargo.toml creds:: --lib`：3 项通过。新增子进程回归完全使用临时目录和合成密钥，先复现路径读取失败、env_key 读取失败与错误回退，再验证修复。
- `node scripts/acceptance/account-metadata.cjs`：真实 Chromium 渲染组件，注入合成额度。1280px 下四列同一行、无卡片溢出；两类窗口、日期、重置卡零、查询与设置入口、邮箱隐藏断言通过；没有未捕获 JavaScript 异常。
- 截图：`output/playwright/account-metadata-light.png` / `account-metadata-dark.png`。只含合成账号。Pencil 只读截图核对画板 07 与 23，没有修改设计稿。
- 合成页面中 Vite 热更新 WebSocket 被浏览器本地网络权限拦截，另有 favicon 404；不影响组件验收，但不能把这次浏览器检查称作真实桌面或更新网络验收。

## 数据来源与边界

- [Claude 配置目录](https://code.claude.com/docs/en/env-vars) 与 [自定义目录中的 .claude.json](https://code.claude.com/docs/en/mcp-quickstart)：经 Context7 查询核对。
- [Codex 配置目录](https://github.com/openai/codex/blob/main/codex-rs/utils/home-dir/src/lib.rs)、[供应商 env_key](https://github.com/openai/codex/blob/main/codex-rs/model-provider-info/src/lib.rs)：经 Context7 查询核对。
- 实时订阅接口能力、完整授权恢复测试及正式更新条件分别记录在本日后端、更新审查报告；尚未将界面合成数据作为真实账号功能通过的证据。
