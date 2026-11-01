//! 连接适配器：按连接类型选择数据路径。
//!
//! - `direct`：`kind=api` 连接按 `connections.protocol` 同协议直通（第 3 步实现）
//! - `codex`：Codex 官方订阅（第 5 步实现）
//! - `claude`：Claude 官方订阅，真实二进制 + MCP 工具桥（第 7 步实现）

pub mod direct;
