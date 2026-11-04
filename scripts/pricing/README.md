# 价目维护

产品使用 `backend/src/pricing/table.rs` 的静态价目，不在运行时同步价格。
同步脚本只生成候选文件，不修改生产表，也不会触发安装包构建。

## 生成候选

在仓库根目录执行，明确列出已有模型的精确 ID：

```powershell
node scripts/pricing/sync-price.mjs --models claude-sonnet-4-5,gpt-5
```

默认读取 `Wei-Shaw/model-price-repo` 的第三方镜像。每次成功生成独立的
`output/pricing/<时间>/table.candidate.rs` 和 `review.json`，后者记录原表和上游 SHA-256、
所选模型的完整原始条目。缺模型、缺来源、缺必要价格、非法 JSON 或下载失败时返回非零，
不生成候选；已有产品价目不受影响。

Node 直连网络不可用时，可用系统网络先下载，再离线生成：

```powershell
New-Item -ItemType Directory -Force -Path output/pricing | Out-Null
Invoke-WebRequest -Uri 'https://raw.githubusercontent.com/Wei-Shaw/model-price-repo/main/model_prices_and_context_window.json' -OutFile output/pricing/upstream.json
node scripts/pricing/sync-price.mjs --models claude-sonnet-4-5 --input output/pricing/upstream.json
```

## 人工复核后应用

1. 从 `review.json` 的来源地址核对官方价格、币种、计费单位、生效日期与促销期限。第三方数据附带官方 URL 不等于已核实。
2. 脚本仅将每 token 基础价换算为每百万 token，精确匹配现有模型，不猜别名、不推算缓存折扣。缺缓存写价格保持 `None`，缺缓存读价格则失败。
3. **复杂规则仍保留本地值**：逐项比较上游阶梯阈值、峰谷、缓存时长等字段；若规则变化，应同步修改计价逻辑及测试。不能只复制基础价格。
4. 新增模型先明确来源平台和 token 语义；当前脚本拒绝自动分配平台或把私有别名映射到相似模型。
5. 核对原表哈希，防止复核期间代码变化；人工应用选中差异，更新核验日期及来源。不要把候选文件头的“待复核”标记当成完成证明。
6. 执行 `node --test scripts/pricing/*.test.mjs` 与 `cargo test --manifest-path backend/Cargo.toml --lib pricing::tests`；改动统计展示时另跑相关用量测试。

本轮仅补维护工具，未更新产品价格。候选目录已由仓库的 output 忽略规则排除。
