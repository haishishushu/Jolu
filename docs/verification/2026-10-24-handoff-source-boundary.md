# 会话交接的来源边界修复

## 定位结论

真实界面验收中，旧短会话的交接草稿提到了今天新建的工作区文件。进一步展开草稿后确认，v1 已说明该文件不是本会话产生的，来源与用途未验证、不要改动。因此本次不能定性为已发生事实编造，也没有证据证明 CLI 越界读取了工作区。

直接数据链路是 `handoff_start → extract_facts → git_snapshot(cwd) → render_facts → synth_prompt`。Git 快照是既有设计允许的当前工作区补充；问题在于原事实区总说明将全部内容称为“从会话记录中原样抽取”，合成提示词又同时要求与事实区一致、仅写会话出现的路径。来源声明和写作规则存在冲突，路径提醒容易被误解成凭空生成。

执行器已使用空临时目录、禁用工具/MCP/用户 hooks。通过 Context7 的 `/websites/code_claude` 查询官方自动加载机制作为排查背景；本次直接来源是程序提供的 Git 事实，故没有修改 CLI 参数、登录或供应商配置。参考：[CLI reference](https://code.claude.com/docs/en/cli-reference)、[settings reference](https://code.claude.com/docs/en/settings-reference)。

## 改动总结

- `handoff_facts.rs`：会话事实与程序补充分开说明；Git 子段明确是生成时读取的当前工作区，不代表本会话改动，也不是用户要求。Git 数据继续保留。
- `session_handoff.rs`：合成和续改规则明确叙述仅限源会话，Git 快照保留在程序事实区，不重复归入叙述、不伪造 `#n` 来源、不据此增加下一步。保留原 `unknown_paths` 提醒。
- 续改路径校验不再把 Git 补充及原始会话文件位置当作会话路径白名单，新增 Git-only 路径仍提示；旧提醒不会被隐藏。
- 提示词升为 `handoff-v2`。`PreviousHandoff` 带上数据库已有的提示词版本，只有同版本才能直接复用叙述。旧记录仍可查看，不改数据库结构，不删除旧正文。原纪要复用逻辑已有版本过滤；续改及其估算也只采用当前版本纪要。

## 验证

- 合成来源边界测试先在旧实现失败，证实缺少 Git 来源限制。初次测试夹具误用未实现的 `Message::default()`，修正夹具后才取得预期红测试，未将编译错误记为产品缺陷。
- `cargo test --manifest-path backend/Cargo.toml handoff --lib`：80 项通过，覆盖事实渲染、生成、纪要复用、已有版本保留、遮蔽与预算裁剪。
- 随后补充 `refine_keeps_git_only_warning_and_excludes_old_version_digest`，单项通过：旧版本纪要不进入续改；新出现的 Git-only 路径仍报警。
- 加强 `ensure_schema_adds_snapshot_column_to_old_table` 后单项通过：v1 正文和版本仍能读取，但不能直接当作 v2 复用。
- `git diff --check` 通过。
- 主代理执行 `cargo test --manifest-path backend/Cargo.toml --features handoff-eval --quiet`：487 项通过，0 失败，12 忽略；独立只读复审未发现阻塞缺陷。

以上开发测试全部使用合成消息、假执行器与内存数据库，子代理没有调用真实模型或修改真实配置与数据库。

主代理重新构建后通过真实界面复验同一短会话：旧 v1 不再直接复用；取消生成保留旧记录；重新生成的第 2 版使用 `handoff-v2`，当日 Git 文件仍在事实补充且未进入叙述，路径警告为 0。真实导出文件采用 UTF-8 并包含来源声明。详细过程与调用核算见 `2026-10-24-real-acceptance-preflight.md`；本次短会话结果不能代替正式长会话质量评测。
