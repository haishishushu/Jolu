//! Claude 订阅适配器的 stdio MCP 桥薄壳（核心逻辑在 `jolu_lib::gateway::bridge`）。
//!
//! 由网关经 `--mcp-config` 挂载：claude 进程的 stdin/stdout 即本进程的 stdin/stdout。
//! 环境变量：
//! - `JOLU_BRIDGE_TOOLS`：调用方请求里的 tools（Anthropic 格式 JSON）
//! - `JOLU_BRIDGE_CALLBACK`：网关回调端点（仅回环）
//! - `JOLU_BRIDGE_SESSION`：会话键（工具调用 id 的前缀）
//!
//! 一行一个 JSON-RPC 消息；超时与错误语义见桥核心模块。

use std::io::{BufRead, Write};

use jolu_lib::gateway::bridge::{handle_line, BridgeConfig, BridgeTool};

fn main() {
    let tools = std::env::var("JOLU_BRIDGE_TOOLS")
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .map(|value| BridgeTool::from_anthropic(&value))
        .unwrap_or_default();
    let config = BridgeConfig {
        tools,
        callback_url: std::env::var("JOLU_BRIDGE_CALLBACK").unwrap_or_default(),
        session: std::env::var("JOLU_BRIDGE_SESSION").unwrap_or_default(),
    };
    let client = reqwest::Client::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("桥需要一个小运行时等待回调");

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Some(response) = runtime.block_on(handle_line(&config, &client, &line)) else { continue };
        let mut stdout = std::io::stdout();
        if writeln!(stdout, "{response}").and_then(|_| stdout.flush()).is_err() {
            break;
        }
    }
}
