# 交接提示词评测

仅在鼠鼠批准样本清单和 CLI 调用预算后执行。程序默认拒绝发起 CLI；不在发布包中提供。

```powershell
cargo run --manifest-path backend/Cargo.toml --features handoff-eval --bin handoff_eval -- select <会话目录> claude
# 人工检查 output/handoff-eval/selection.json，只勾选批准的会话，划分 dev / test
cargo run --manifest-path backend/Cargo.toml --features handoff-eval --bin handoff_eval -- questions --split dev --allow-cli --max-calls 20
cargo run --manifest-path backend/Cargo.toml --features handoff-eval --bin handoff_eval -- run --split dev --allow-cli --max-calls 80
cargo run --manifest-path backend/Cargo.toml --features handoff-eval --bin handoff_eval -- report --split dev
```

每次运行最多 80 次调用，可用 --max-calls 降低。`output/handoff-eval/budget.json` 在启动每次调用前持久化计数，跨运行累计上限为80，失败也占预算；不可并行运行多个评测进程，不得清空账本后继续消耗预算。中断后的已完成阶段缓存在按源文件内容和提示词版本计算的目录中。

默认筛选至少50个去重后的真实用户轮次（不把工具结果、注入消息算作轮次）。没有符合条件的样本时，可用 `select <目录> claude --min-turns 1` 做短会话探索性验证，并在报告中明确不能替代长会话正式评测。筛选会显示轮次与估计Token，超过150,000 Token的样本不进入清单。

题目包含事实、文件轨迹、下一步、决策原因、未知问题；程序题由原始记录提取。基线与新方案各回答一次，再随机交换顺序盲评。报告按会话 bootstrap 重采样 2000 次给出区间。同一服务商自评存在偏差，结果不能当作总体效果保证。开发集调参后再运行测试集，不得反复调参窥看测试集。

selection、源文本衍生缓存与报告仅存 output。不要提交或外发这些文件。2026-10-23鼠鼠已授权当前登录Claude Code与累计约80次调用；执行情况见当天的真实验证记录。
