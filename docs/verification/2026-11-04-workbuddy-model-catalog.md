# WorkBuddy 账号模型目录修复

## 原因与改动

原 `workbuddy::models` 根据地区返回写死的 `default` / `default-model`，没有查询账号可用模型。它们是默认路由入口，不能代表具体模型清单。

现在由所选连接的加密授权请求官方 `GET /v3/config`；有企业身份时合并 `GET /console/enterprises/{enterpriseId}/config/models`。只返回官方配置中当前订阅原生 Chat 端点可用的模型 ID；模型 ID 原样用于实际 Chat 请求。目录采用官方 `agents.models`、`chat` 标签、`availableModels` 与 `disabled` 规则。不同域名、独立 Key、本地和自定义协议模型不由本订阅适配器接管。

过滤官方默认及自动路由 ID：`default`、`default-model`、`auto`、`auto-chat`、`fast-model`、`balanced-model`、`deep-model`。官方只返回路由策略、未公开具体模型时，清单保持为空；不会推测背后的模型名称。

`/v1/models` 与模型池共用核实后的账号目录。缓存存于现有 `models_json`，带授权密文 SHA-256、获取时间和模型 ID，5 分钟过期；不接受旧的普通数组作为已核实清单。查询完成后比较原密文再写入，重新登录期间的迟到结果不会污染新账号。无效来源不触发模型查询。失败不回退到默认别名；全部查询失败时返回明确的目录获取错误。

## 官方依据

- Context7：先解析 `/websites/codebuddy_cn`，再查询 CLI 模型目录与 SDK。官方 SDK 的 `getAvailableModels` / `supportedModels` 和模型配置说明分别见 https://www.codebuddy.cn/docs/cli/sdk-typescript 、https://www.codebuddy.cn/docs/cli/models 。这些公开文档没有完整 HTTP 契约，因此另查官方随包程序。
- 只读程序文件：`D:/WorkBuddy/resources/app.asar.unpacked/cli/dist/codebuddy-lite-wb.mjs`。SHA-256：`92899eb4b9d9403a95ceae17dc95e6816bb28dd9fe099790240b5b54d1dac330`。没有读取账号文件、Cookie 或原工具真实凭据。
- UTF-8 字符偏移约 3396990：`ModelsProductProvider` 企业目录接口，响应 `data` 为模型数组，企业配置优先级高于云端配置。
- 约 8615700 / 8621200：`CloudProductManager` / `CloudProductProvider` 请求 `/v3/config`，响应 `data` 为产品配置。Jolu 不上传工作区仓库信息。
- 约 3213500 / 8604574：`AgentManager.computeModelsFromRaw` 与 `AvailableModelsFilterProvider` 的模型可选规则。约 8602000–8604355：`AUTO_MODEL_IDS` 和自动分档列表。
- 约 7076217：账号目录认证拦截器从同一账号填入 Bearer、X-User-Id、X-Enterprise-Id、X-Tenant-Id、X-Domain；其中 tenant 与 enterprise 使用同一企业 ID。约 8643870 的 `X-Product: SaaS` 与约 30101 的 `X-Requested-With: XMLHttpRequest` 已对照。
- 约 3708900 / 3684300：模型选择和发送 JSON 都按 `models.id` 查找，没有另行猜测请求模型名。

国际地区沿用已绑定账号的官方域名；本轮只核对了上述国内安装包，未做国际真实服务验收，不把共享代码测试称为双地区真实验证。

## 安全与验证边界

请求仅使用所选账号及企业头，遵循 Jolu 网络设置，禁止重定向；目录查询总计限时 8 秒，响应最多 2 MiB。缓存只保存模型 ID 和授权摘要，不保存远端配置里的 Key。

已先复现菜单错误测试：无效来源仍被返回 `default`（`output/acceptance/workbuddy-model-menu-red.txt`）。合成测试覆盖官方过滤规则、账号与企业头隔离、模型池目录一致性、换密文和过期缓存失效、迟到结果拒绝、重定向拒绝及旧别名不回退。仅使用回环 HTTP 和合成凭据，真实账号模型名单与真实生成仍另行验收。

一次编译被 Windows 资源文件临时占用阻断；随后测试中误用未启用的 Axum JSON feature，已改用 JSON 字符串，未增加依赖。独立审查指出自动分档别名也必须过滤，已补齐对应测试。最终网关 **135 项通过、0 失败**，日志 `output/acceptance/workbuddy-model-catalog-tests.txt`；`git diff --check` 通过。没有构建安装包或执行 Git 写操作。

## 2026-11-04 补充：按 magpie 重写目录与对话签名

真实账号复现：`/v1/models` 报「WorkBuddy 模型目录获取失败（HTTP 400）」。无凭据对照确认，请求带 `X-User-Id` 而 User-Agent 不是 WorkBuddy 客户端时，服务端返回 `400 {"code":12403,"msg":"check ua, get coding copilot version error"}`。原实现的 UA 为 `Jolu/WorkBuddy-ModelCatalog`，并且把会话 `domain`（可能是 `www.codebuddy.cn`）当成请求主机。

按 magpie-community/plugins `packages/workbuddy`（MIT）改为：

- 主机固定为各版本官方 API：国内 `https://copilot.tencent.com`，国际 `https://www.workbuddy.ai`；会话 `domain` 只用作 `X-Domain`。
- 目录与对话都带 magpie `sign` 的整组头：`User-Agent: WorkBuddy/5.9.3 <应用名>/5.9.3 CLI/2.165.0`、`X-IDE-Type/Name/Version`、`X-Product`、`X-Agent-Intent/Purpose`、企业头。对话另加 `chatHeaders`（`X-Agent-Type`、`X-Agent-Runtime`、`X-Stainless-*`）。
- 目录只取 `/v3/config` 中 `cli` agent 声明的模型，保持原顺序，包含 `auto` 等官方声明项；不再单独请求企业目录（企业身份由请求头区分）。
- 目录读不到或为空时，沿用该授权上次读到的清单，再没有则使用 magpie 内置清单；授权本身失效时仍然报错。

验证：WorkBuddy 适配器 5 项、网关模块 158 项测试通过（仅回环 HTTP 与合成凭据）。真实账号目录与对话需重启 Jolu 后由鼠鼠在界面上复核。
