//! Claude 订阅适配器的 stdio MCP 工具桥（执行文档 §6.3）。
//!
//! 形态：核心逻辑在本模块（可单测），`bin/gateway_mcp_bridge.rs` 只是薄壳。
//! 行为：向 claude 进程声明调用方的 tools；claude 调工具时把
//! `(session, tool_call_id, name, arguments)` POST 到网关的回环回调端点并
//! 挂起等待，agent 的下一请求携带 tool_result → 网关 [`ToolCallHub::complete`]
//! 释放回调 → 桥把结果回给 claude，同一场会话继续。超时（默认 120s）回工具错误。
//!
//! 该桥只在 Claude 订阅适配器内出现，其他适配器一律不经 MCP。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// 回调等待上限；超时回 MCP 工具错误而不是让 claude 永远挂起。
pub const TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// 挂起中的工具调用表：桥回调挂起 → agent 的 tool_result 释放。
/// 一次调用一个 oneshot；跨线程只传 String（工具结果 JSON 文本），不含凭证。
pub(crate) struct ToolCallHub {
    pending: Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>,
}

impl ToolCallHub {
    pub(crate) fn new() -> Self {
        Self { pending: Mutex::new(HashMap::new()) }
    }

    /// 登记一次挂起调用，返回等待结果的 Receiver。
    pub(crate) fn register(&self, call_id: &str) -> tokio::sync::oneshot::Receiver<String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(call_id.to_string(), tx);
        }
        rx
    }

    /// agent 的 tool_result 到达时释放挂起的回调。返回 false = 没有这个调用（迟到或伪造）。
    /// 供第 11 步的 agent 侧 tool_result 回流命令使用。
    #[allow(dead_code)]
    pub(crate) fn complete(&self, call_id: &str, result: String) -> bool {
        match self.pending.lock() {
            Ok(mut pending) => match pending.remove(call_id) {
                Some(sender) => sender.send(result).is_ok(),
                None => false,
            },
            Err(_) => false,
        }
    }

    /// 当前挂起数（诊断用）。
    #[allow(dead_code)]
    pub(crate) fn pending_count(&self) -> usize {
        self.pending.lock().map(|p| p.len()).unwrap_or(0)
    }
}

/// MCP 工具声明（桥向 claude 声明调用方的工具）。
#[derive(Debug, Clone)]
pub struct BridgeTool {
    pub name: String,
    pub description: Option<String>,
    /// JSON Schema 原文（`{"type":"object",...}`）。
    pub input_schema: serde_json::Value,
}

impl BridgeTool {
    fn to_mcp(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "description": self.description.clone().unwrap_or_default(),
            "inputSchema": self.input_schema,
        })
    }

    /// 从 Anthropic 请求体的 tools 字段解析（调用方协议以 Anthropic 为主）。
    pub fn from_anthropic(value: &serde_json::Value) -> Vec<BridgeTool> {
        value
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| {
                        Some(BridgeTool {
                            name: tool.get("name")?.as_str()?.to_string(),
                            description: tool.get("description").and_then(|d| d.as_str()).map(str::to_string),
                            input_schema: tool.pointer("/input_schema").cloned()
                                .or_else(|| tool.get("input_schema").cloned())
                                .unwrap_or(serde_json::json!({"type":"object"})),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// 桥的运行配置：工具表 + 回调地址 + 会话键。
pub struct BridgeConfig {
    pub tools: Vec<BridgeTool>,
    pub callback_url: String,
    pub session: String,
}

/// 处理一行 stdin（JSON-RPC 请求），返回要写回 stdout 的响应行（notifications 不回）。
/// 工具调用会阻塞到回调返回或超时——这正是「挂起该次 MCP 调用」的实现。
pub async fn handle_line(config: &BridgeConfig, client: &reqwest::Client, line: &str) -> Option<String> {
    let request: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if request.get("method").is_none() {
        return None; // 响应或不认识的消息：不回
    }
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(|m| m.as_str()).unwrap_or_default();
    let result = match method {
        "initialize" => serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "jolu-bridge", "version": "0.1.0"},
        }),
        "tools/list" => serde_json::json!({"tools": config.tools.iter().map(BridgeTool::to_mcp).collect::<Vec<_>>()}),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or_default();
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or_default().to_string();
            let call_id = format!("{}:{}", config.session, params.get("call_id").and_then(|c| c.as_str()).unwrap_or(&name));
            let callback = tokio::time::timeout(
                TOOL_CALL_TIMEOUT,
                client
                    .post(&config.callback_url)
                    .json(&serde_json::json!({
                        "session": config.session,
                        "tool_call_id": call_id,
                        "name": name,
                        "arguments": params.get("arguments").cloned().unwrap_or(serde_json::json!({})),
                    }))
                    .send(),
            )
            .await;
            match callback {
                Ok(Ok(response)) if response.status().is_success() => {
                    match response.text().await {
                        Ok(text) => serde_json::from_str::<serde_json::Value>(&text)
                            .unwrap_or_else(|_| serde_json::json!({"content": [{"type":"text","text": text}]})),
                        Err(error) => tool_error(&format!("读取工具结果失败：{error}")),
                    }
                }
                Ok(Ok(response)) => tool_error(&format!("网关回调返回 {}", response.status())),
                Ok(Err(error)) => tool_error(&format!("网关回调不可达：{error}")),
                Err(_) => tool_error("等待工具结果超时（120 秒）"),
            }
        }
        _ => return None,
    };
    let id = id?;
    Some(serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string())
}

fn tool_error(message: &str) -> serde_json::Value {
    serde_json::json!({"content": [{"type": "text", "text": message}], "isError": true})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_register_and_complete_pair_up() {
        let hub = ToolCallHub::new();
        let rx = hub.register("call-1");
        assert!(hub.complete("call-1", "{\"ok\":true}".into()));
        assert_eq!(rx.blocking_recv().unwrap(), "{\"ok\":true}");
        // 完成后的重复 complete 不成立
        assert!(!hub.complete("call-1", "late".into()));
        assert_eq!(hub.pending_count(), 0);
    }

    #[tokio::test]
    async fn bridge_initialize_and_tools_list() {
        let config = BridgeConfig {
            tools: vec![BridgeTool {
                name: "read_file".into(),
                description: Some("读文件".into()),
                input_schema: serde_json::json!({"type":"object","properties":{"path":{"type":"string"}}}),
            }],
            callback_url: "http://127.0.0.1:1/unused".into(),
            session: "s1".into(),
        };
        let client = reqwest::Client::new();
        let init = handle_line(&config, &client, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
            .await
            .unwrap();
        assert!(init.contains("jolu-bridge"), "{init}");
        let list = handle_line(&config, &client, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
            .await
            .unwrap();
        assert!(list.contains("read_file") && list.contains("inputSchema"), "{list}");
        // 通知不回包
        assert!(handle_line(&config, &client, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).await.is_none());
    }

    /// 工具调用 → 回调挂起 → mock 网关返回结果 → MCP 响应携带内容。
    #[tokio::test]
    async fn bridge_tool_call_round_trips_through_callback() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = socket.read(&mut chunk).await.unwrap();
                if n == 0 { break; }
                raw.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&raw);
                let Some(split) = text.find("\r\n\r\n") else { continue };
                let length = text[..split].lines()
                    .find_map(|l| l.split_once(':'))
                    .filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if raw.len() >= split + 4 + length { break; }
            }
            let request = String::from_utf8_lossy(&raw).to_string();
            assert!(request.contains("\"name\":\"read_file\""), "{request}");
            assert!(request.contains("\"session\":\"s1\""), "{request}");
            let body = "{\"content\":[{\"type\":\"text\",\"text\":\"file-bytes\"}]}";
            let head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
            socket.flush().await.unwrap();
        });
        let config = BridgeConfig {
            tools: vec![BridgeTool { name: "read_file".into(), description: None, input_schema: serde_json::json!({"type":"object"}) }],
            callback_url: format!("http://{addr}/__jolu/bridge/callback"),
            session: "s1".into(),
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = handle_line(
            &config,
            &client,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read_file","arguments":{"path":"x"},"call_id":"c9"}}"#,
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert!(response.contains("file-bytes"), "{response}");
        assert!(response.contains("\"id\":3"), "{response}");
    }

    /// 回调不可达 → 返回 MCP 工具错误而不是让 claude 永远挂起。
    #[tokio::test]
    async fn bridge_tool_call_reports_unreachable_callback_as_tool_error() {
        let config = BridgeConfig {
            tools: vec![],
            callback_url: "http://127.0.0.1:1/dead".into(),
            session: "s1".into(),
        };
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(2)).build().unwrap();
        let response = handle_line(
            &config,
            &client,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"t","arguments":{}}}"#,
        )
        .await
        .unwrap();
        assert!(response.contains("isError") && response.contains("不可达"), "{response}");
    }

    #[test]
    fn bridge_tools_parse_from_anthropic_request() {
        let body = serde_json::json!({"tools": [
            {"name": "bash", "description": "跑命令", "input_schema": {"type": "object"}},
            {"no_name": true}
        ]});
        let tools = BridgeTool::from_anthropic(&body["tools"]);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "bash");
    }
}
