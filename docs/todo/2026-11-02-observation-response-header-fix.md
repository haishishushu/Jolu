# 修复执行文档 · 观测模式响应头被覆盖（2026-11-02）

**交给：鼠鼠**
**状态：2026-11-04 已修复，待提交**
**位置：`backend/src/proxy.rs:545`**
**严重度：低（静默失效，不影响主链路）**

## 1. 问题

观测模式给转发响应打的标记头 `x-jolu-proxy: 1` **从来没有真正出现在响应里**。

```rust
// proxy.rs:544-549 现状
// http::response::Builder 没有批量 headers 方法，先建 body 再整体替换头表
let mut builder = Response::builder().status(status).header("x-jolu-proxy", "1");
match builder.headers_mut() {
    Some(slot) => *slot = headers,   // ← 这里把整张头表换成上游的
    None => return error_response(StatusCode::BAD_GATEWAY, "本地代理：构造响应失败"),
}
```

`*slot = headers` 是**整体赋值**，不是合并。上游响应头一写进去，前一行的 `x-jolu-proxy` 就被一并覆盖掉了。

## 2. 为什么算缺陷（不是「潜在风险」）

已核实的三条事实：

| 项 | 结果 |
| --- | --- |
| `x-jolu-proxy` 在代码里出现次数 | **1 处**（`proxy.rs:545`，只写不读） |
| 有代码依赖这个头 | **没有** |
| 有测试断言过这个头 | **没有**（`grep` 计数 0） |

所以它是**确定性失效**：每一次观测模式转发，这个头都缺席。不是偶发、不是边界条件。

之所以一直没被发现：主链路（计时、归账）走的是响应流内部逻辑，不读这个头；没有测试守住它，也就没有失败信号。

## 3. 影响面

- **不影响**：计时、首字延迟、`requests` 表回填、按连接归账、CLI 侧正常收发
- **影响**：外部无法通过响应头判断「这个响应经过 Jolu 转发」。用于人工排查或第三方识别时拿不到标记
- 网关模式（`gateway/adapter/direct.rs`）的 `x-jolu-gateway` **已经正确**（第 3 步开发时修好并加了测试断言），本缺陷只存在于观测模式

## 4. 修复方案

把打标记的动作挪到**整表替换之后**：

```rust
// 建议改法
let mut builder = Response::builder().status(status);
match builder.headers_mut() {
    Some(slot) => *slot = headers,
    None => return error_response(StatusCode::BAD_GATEWAY, "本地代理：构造响应失败"),
}
// 归账 / 标记头必须在头表整体替换之后加，否则会被上面的赋值抹掉
builder = builder.header("x-jolu-proxy", "1");
builder
    .body(timed_body(response, core))
    .unwrap_or_else(|error| error_response(StatusCode::BAD_GATEWAY, &format!("本地代理：构造响应失败：{error}")))
```

同时更新上面那行注释（现在写的是「先建 body 再整体替换头表」，没提这个顺序陷阱）。

## 5. 验证要求

修复后必须补一个**会失败的测试**（当前没有任何测试覆盖这个头，这是缺陷能长期潜伏的根本原因）：

- 在 `proxy.rs` 的测试模块里，参照现有 `spawn_proxy` + `spawn_chunked_upstream` 的手法起一个 mock 上游
- 断言观测模式转发后的响应头包含 `x-jolu-proxy: 1`
- **先确认这个断言在修复前失败**，再应用修复让它通过——不做这一步就无法证明测试真的守住了问题

另外核对：

- `cargo test --manifest-path backend/Cargo.toml` 全绿（基线：726 通过 / 0 失败）
- 观测模式的手工验收（`scripts/acceptance/README.md`）里，转发的响应头多出 `x-jolu-proxy` 属预期

## 6. 同类问题排查（顺便）

同类「先设头再整表替换」的写法在同仓库还有一处已修好：

- `gateway/adapter/direct.rs` 的 `x-jolu-gateway`（第 3 步已修，并有端到端测试断言）

`grep -rn "headers_mut()" backend/src` 可复查是否还有第三处。当前只查到这两处。

## 7. 不做的事

- 不改 `gateway/` 下的任何逻辑（那边已经正确）
- 不改响应头语义（`x-jolu-proxy` 的值保持 `1`，不引入新格式）
- 不为这个头增加前端展示——它只用于排查与识别

## 2026-11-04 执行补记

- 已先用真实本地 HTTP 流式转发测试复现：响应头为 None，预期为 1。
- 改为先接收上游头表，再插入本地标记；使用 insert 保证上游同名头不会形成重复标记。
- 测试同时保留 content-type、逐块透传和计时归账断言。
- 本轮不构建安装包、不提交、不发布；后端全量结果见费用计划的同日补记。
