//! Trae 对话请求和 SSE 的协议边界；仅处理合成测试可确认的形状。
use super::trae_models::ModelRoute;
use serde_json::{json, Value};

pub(super) fn nonce() -> Result<String, String> {
    use rsa::rand_core::RngCore;
    let mut bytes = [0u8; 16];
    rsa::rand_core::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| "无法生成 Trae 请求标识")?;
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

pub(super) fn request(
    body: &[u8],
    slug: &str,
    route: &ModelRoute,
    session: &str,
) -> Result<Value, String> {
    let value: Value = serde_json::from_slice(body).map_err(|_| "Trae 请求不是有效 JSON")?;
    let object = value.as_object().ok_or("Trae 请求格式无效")?;
    const FIELDS: &[&str] = &[
        "model",
        "messages",
        "stream",
        "stream_options",
        "max_tokens",
        "max_completion_tokens",
        "temperature",
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "reasoning_effort",
    ];
    if object.keys().any(|k| !FIELDS.contains(&k.as_str())) {
        return Err("Trae 请求含不支持字段，内部授权与会话参数不可透传".into());
    }
    let prefix = format!("{slug}/");
    let model = value["model"]
        .as_str()
        .and_then(|m| m.strip_prefix(&prefix))
        .filter(|m| !m.trim().is_empty() && m.len() <= 512 && !m.chars().any(char::is_control))
        .ok_or("Trae 模型来源无效")?;
    if value.get("stream").is_some_and(|v| !v.is_boolean())
        || value
            .get("parallel_tool_calls")
            .is_some_and(|v| !v.is_boolean())
    {
        return Err("Trae 流式或并行工具参数无效".into());
    }
    if let Some(options) = value.get("stream_options") {
        let options = options.as_object().ok_or("Trae stream_options 无效")?;
        if options
            .iter()
            .any(|(k, v)| k != "include_usage" || !v.is_boolean())
        {
            return Err("Trae 不支持此流式选项".into());
        }
    }
    let tools = value
        .get("tools")
        .map(|v| v.as_array().ok_or("Trae tools 必须是数组"))
        .transpose()?
        .cloned()
        .unwrap_or_default();
    let mut native = Vec::new();
    for tool in &tools {
        let f = &tool["function"];
        let name = f["name"]
            .as_str()
            .filter(|n| !n.is_empty() && n.len() <= 256 && !n.chars().any(char::is_control))
            .ok_or("Trae 工具名称无效")?;
        if tool["type"] != "function"
            || tool.get("function").and_then(Value::as_object).is_none()
            || f.get("strict").is_some()
        {
            return Err("Trae 不支持此工具类型或 strict 参数".into());
        }
        let parameters = f.get("parameters").cloned().unwrap_or_else(|| json!({}));
        if !parameters.is_object() {
            return Err("Trae 工具 schema 必须是对象".into());
        }
        if f.get("description").is_some_and(|v| !v.is_string()) {
            return Err("Trae 工具描述无效".into());
        }
        native.push(json!({"type":"function","function":{"name":name,"description":f["description"].as_str().unwrap_or(""),"parameters":parameters.to_string()}}));
    }
    let mut out = json!({"function":route.function,"config_name":model,"model":model,"stream":true,"request_id":nonce()?,"session_id":session});
    if let Some(name) = &route.model_name {
        out["model_name"] = json!(name);
    }
    if let Some(effort) = value.get("reasoning_effort") {
        let effort = match effort.as_str() {
            Some("none") => {
                return Err("Trae 当前账号目录未声明支持关闭思考，不能兑现显式禁用请求".into())
            }
            Some("low" | "light") => "light",
            Some("xhigh" | "extra_high") => "extra_high",
            Some("medium") => "medium",
            Some("high") => "high",
            Some("max") => "max",
            _ => return Err("Trae 思考档位无效".into()),
        };
        if !route.reasoning.iter().any(|r| r == effort) {
            return Err("Trae 所选模型未声明支持此思考档位".into());
        }
        out["reasoning_effort"] = json!(effort);
    }
    if let Some(tokens) = value
        .get("max_completion_tokens")
        .or_else(|| value.get("max_tokens"))
    {
        let tokens = tokens
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or("Trae 输出预算必须是正整数")?;
        out["max_tokens"] = json!(route.max_tokens.map_or(tokens, |max| tokens.min(max)));
    }
    if let Some(t) = value.get("temperature") {
        let t = t
            .as_f64()
            .filter(|t| t.is_finite())
            .ok_or("Trae temperature 无效")?;
        out["temperature"] = json!(t);
    }
    if !native.is_empty() {
        out["tools"] = json!(native);
    }
    if let Some(choice) = value.get("tool_choice") {
        let name = if let Some(s) = choice.as_str() {
            s
        } else {
            choice
                .pointer("/function/name")
                .and_then(Value::as_str)
                .filter(|_| choice["type"] == "function")
                .ok_or("Trae tool_choice 无效")?
        };
        if !["auto", "none", "required"].contains(&name)
            && !tools
                .iter()
                .any(|t| t.pointer("/function/name").and_then(Value::as_str) == Some(name))
        {
            return Err("Trae 指定工具未声明".into());
        }
        if name != "none" && tools.is_empty() {
            return Err("Trae tool_choice 缺少工具定义".into());
        }
        out["tool_choice"] = json!(name);
    }
    if let Some(p) = value.get("parallel_tool_calls") {
        out["parallel_tool_calls"] = p.clone();
    }
    let input = value["messages"]
        .as_array()
        .filter(|m| !m.is_empty())
        .ok_or("Trae 请求缺少 messages")?;
    let mut messages: Vec<Value> = Vec::new();
    let mut pending = std::collections::BTreeSet::<String>::new();
    let mut seen = std::collections::BTreeSet::<String>::new();
    for m in input {
        let role = match m["role"].as_str() {
            Some("developer" | "system") => "system",
            Some("user") => "user",
            Some("assistant") => "assistant",
            Some("tool") => "tool",
            _ => return Err("Trae 消息角色无效".into()),
        };
        if role != "tool" && !pending.is_empty() {
            return Err("Trae 工具调用尚缺少对应结果".into());
        }
        if m.as_object().is_none_or(|o| {
            o.keys().any(|k| {
                ![
                    "role",
                    "content",
                    "tool_calls",
                    "tool_call_id",
                    "reasoning_content",
                ]
                .contains(&k.as_str())
            })
        }) {
            return Err("Trae 消息含不支持字段".into());
        }
        let mut blocks = Vec::new();
        match m.get("content") {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) => {
                blocks.push(json!({"type":"text","text":s}));
            }
            Some(Value::Array(parts)) => {
                for part in parts {
                    match part["type"].as_str() {
                        Some("text") if part["text"].is_string() => {
                            blocks.push(json!({"type":"text","text":part["text"]}))
                        }
                        Some("image_url") if route.images == Some(true) && role == "user" => {
                            let url = part
                                .pointer("/image_url/url")
                                .and_then(Value::as_str)
                                .filter(|s| {
                                    s.starts_with("https://") || s.starts_with("data:image/")
                                })
                                .ok_or("Trae 图片地址无效")?;
                            blocks.push(json!({"type":"image_url","image_url":{"url":url}}));
                        }
                        _ => return Err("Trae 此模型或消息位置不支持该内容类型，未丢弃内容".into()),
                    }
                }
            }
            _ => return Err("Trae 消息内容无效".into()),
        }
        if let Some(reasoning) = m.get("reasoning_content").filter(|v| !v.is_null()) {
            if role != "assistant" {
                return Err("Trae 思考历史只能来自 assistant".into());
            }
            let reasoning = reasoning.as_str().ok_or("Trae 思考历史格式无效")?;
            if !reasoning.is_empty() {
                blocks.insert(
                    0,
                    json!({"type":"text","text":format!("<think>\n{reasoning}\n</think>")}),
                );
            }
        }
        let mut message =
            json!({"role":role,"content":if blocks.is_empty() {Value::Null} else {json!(blocks)}});
        if let Some(calls) = m.get("tool_calls") {
            if role != "assistant" {
                return Err("Trae 工具调用只能在 assistant 消息中".into());
            }
            let calls = calls
                .as_array()
                .filter(|c| !c.is_empty())
                .ok_or("Trae 工具调用列表无效")?;
            let mut converted = Vec::new();
            for call in calls {
                let id = call["id"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 256)
                    .ok_or("Trae 工具调用 ID 无效")?;
                let name = call
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .ok_or("Trae 历史工具名称无效")?;
                let args = call
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                    .ok_or("Trae 工具参数必须是 JSON 字符串")?;
                let parsed: Value =
                    serde_json::from_str(args).map_err(|_| "Trae 历史工具参数不是有效 JSON")?;
                if !parsed.is_object() || call["type"] != "function" || !seen.insert(id.into()) {
                    return Err("Trae 历史工具参数或重复 ID 无效".into());
                }
                pending.insert(id.into());
                converted.push(json!({"id":id,"type":"function","function_call":{"name":name,"arguments":args}}));
            }
            message["tool_calls"] = json!(converted);
        }
        if role == "tool" {
            let id = m["tool_call_id"].as_str().ok_or("Trae 工具结果缺少 ID")?;
            if !pending.remove(id) {
                return Err("Trae 工具结果没有匹配的调用".into());
            }
            message["tool_call_id"] = json!(id);
            if message["content"].is_null()
                || message["content"] == json!([{"type":"text","text":""}])
            {
                message["content"] = json!([{"type":"text","text":"(no output)"}]);
            }
        }
        // 合并相邻用户/助手文本，不在并行工具结果之间插入额外消息。
        if matches!(role, "user" | "assistant")
            && message.get("tool_calls").is_none()
            && messages
                .last()
                .is_some_and(|last| last["role"] == role && last.get("tool_calls").is_none())
        {
            let last = messages.last_mut().unwrap();
            let mut combined = last["content"].as_array().cloned().unwrap_or_default();
            combined.extend(message["content"].as_array().into_iter().flatten().cloned());
            last["content"] = json!(combined);
        } else {
            messages.push(message);
        }
    }
    if !pending.is_empty() {
        return Err("Trae 历史工具调用缺少结果".into());
    }
    out["messages"] = json!(messages);
    Ok(out)
}

pub(super) fn conversation(
    id: &str,
    secret: &str,
    body: &str,
    session: Option<&str>,
) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    // 进程随机盐不落盘；连接和授权参与派生，同名会话也不会跨账号复用。
    static SALT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let salt = match SALT.get() {
        Some(v) => v.clone(),
        None => {
            let value = nonce()?;
            SALT.get_or_init(|| value).clone()
        }
    };
    let value: Value = serde_json::from_str(body).map_err(|_| "Trae 请求不是有效 JSON")?;
    let first = value["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| m["role"] == "user")
        .and_then(|m| m.get("content"));
    let key = if let Some(session) = session.filter(|s| !s.is_empty() && !s.starts_with("tok:")) {
        json!(["session", session])
    } else if let Some(first) = first.filter(|v| !v.is_null()) {
        json!(["first", first])
    } else {
        return nonce();
    };
    let encoded = json!([salt, id, secret, key]).to_string();
    let mut bytes = Sha256::digest(encoded.as_bytes()).to_vec();
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let hex: String = bytes[..16].iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

#[derive(Debug, PartialEq)]
pub(super) enum Event {
    Text(String),
    Reasoning(String),
    Tool(Value),
    Done { reason: String, usage: Value },
}

#[derive(Default)]
pub(super) struct Events {
    buffer: Vec<u8>,
    event: String,
    data: Vec<String>,
    data_size: usize,
    calls: Vec<NativeCall>,
    usage: serde_json::Map<String, Value>,
    reason: Option<String>,
    ended: bool,
    total: usize,
    splitter: Option<super::text_calls::Splitter>,
    text_calls: usize,
    pending_text_calls: Vec<Value>,
    thought: String,
    last_thought: String,
}
#[derive(Default)]
struct NativeCall {
    id: String,
    index: Option<u64>,
    name: String,
    args: String,
}
const MAX_EVENT: usize = 2 * 1024 * 1024;
const MAX_STREAM: usize = 16 * 1024 * 1024;
impl Events {
    pub fn new(tools: &[Value]) -> Self {
        Self {
            splitter: (!tools.is_empty()).then(|| super::text_calls::Splitter::new(tools)),
            ..Self::default()
        }
    }
    fn pieces(&mut self, pieces: Vec<super::text_calls::Piece>) -> Result<Vec<Event>, String> {
        let mut out = Vec::new();
        for piece in pieces {
            match piece {
                super::text_calls::Piece::Text(s) => out.push(Event::Text(s)),
                super::text_calls::Piece::Call { name, args } => {
                    self.text_calls += 1;
                    if self.text_calls > 128 {
                        return Err("Trae 工具调用数量超过限制".into());
                    }
                    self.pending_text_calls.push(json!({"id":format!("call_{}",nonce()?),"type":"function","function":{"name":name,"arguments":args}}));
                }
                super::text_calls::Piece::Error(_) => {
                    return Err("Trae 文本工具调用不完整或无法解析".into())
                }
            }
        }
        Ok(out)
    }
    pub fn done(&self) -> bool {
        self.ended
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Event>, String> {
        if self.ended {
            return Ok(Vec::new());
        }
        self.total = self
            .total
            .checked_add(bytes.len())
            .filter(|n| *n <= MAX_STREAM)
            .ok_or("Trae 回复超过大小限制")?;
        self.buffer.extend_from_slice(bytes);
        let mut out = Vec::new();
        let mut used = 0;
        while let Some(at) = self.buffer[used..].iter().position(|b| *b == b'\n') {
            let end = used + at;
            if end - used > MAX_EVENT {
                return Err("Trae SSE 行超过大小限制".into());
            }
            let line = std::str::from_utf8(&self.buffer[used..end])
                .map_err(|_| "Trae SSE 不是有效 UTF-8")?
                .trim_end_matches('\r')
                .to_owned();
            used = end + 1;
            if line.is_empty() {
                out.extend(self.flush()?);
            } else if let Some(value) = line.strip_prefix("event:") {
                // 兼容上游省略事件之间空行；已有 data 先结束。
                if !self.data.is_empty() {
                    out.extend(self.flush()?);
                }
                self.event = value.trim().into();
            } else if let Some(value) = line.strip_prefix("data:") {
                let value = value.strip_prefix(' ').unwrap_or(value);
                self.data_size += value.len();
                if self.data_size > MAX_EVENT {
                    return Err("Trae SSE 事件超过大小限制".into());
                }
                self.data.push(value.into());
            }
            if self.ended {
                break;
            }
        }
        self.buffer.drain(..used);
        if self.buffer.len() > MAX_EVENT {
            return Err("Trae SSE 行超过大小限制".into());
        }
        Ok(out)
    }
    pub fn finish(&mut self) -> Result<Vec<Event>, String> {
        if self.ended {
            return Ok(Vec::new());
        }
        let mut out = self.push(b"\n")?;
        out.extend(self.flush()?);
        if !self.ended {
            return Err("Trae 回复在完成事件前中断，未作为完整回答".into());
        }
        Ok(out)
    }
    fn usage(&mut self, value: &Value) {
        for (target, aliases) in [
            (
                "prompt_tokens",
                &[
                    "prompt_tokens",
                    "input_tokens",
                    "promptTokens",
                    "inputTokens",
                ][..],
            ),
            (
                "completion_tokens",
                &[
                    "completion_tokens",
                    "output_tokens",
                    "completionTokens",
                    "outputTokens",
                ][..],
            ),
            ("total_tokens", &["total_tokens", "totalTokens"][..]),
            (
                "cache_read_input_tokens",
                &[
                    "cache_read_input_tokens",
                    "cacheReadInputTokens",
                    "cached_tokens",
                    "prompt_cache_hit_tokens",
                ][..],
            ),
            (
                "cache_creation_input_tokens",
                &["cache_creation_input_tokens", "cacheCreationInputTokens"][..],
            ),
        ] {
            if let Some(n) = aliases
                .iter()
                .find_map(|k| value.get(*k).and_then(Value::as_u64))
            {
                self.usage.insert(target.into(), json!(n));
            }
        }
        if !self.usage.contains_key("cache_read_input_tokens") {
            if let Some(n) = value
                .pointer("/prompt_tokens_details/cached_tokens")
                .or_else(|| value.pointer("/input_tokens_details/cached_tokens"))
                .and_then(Value::as_u64)
            {
                // 保留 Chat 缓存是 prompt 子集的口径，不能扁平化成独立输入计数。
                self.usage
                    .insert("prompt_tokens_details".into(), json!({"cached_tokens":n}));
            }
        }
        if let Some(n) = value
            .get("reasoning_tokens")
            .or_else(|| value.pointer("/completion_tokens_details/reasoning_tokens"))
            .and_then(Value::as_u64)
        {
            self.usage.insert(
                "completion_tokens_details".into(),
                json!({"reasoning_tokens":n}),
            );
        }
    }
    fn call(&mut self, call: &Value, delta: bool) -> Result<(), String> {
        let id = call["id"].as_str().unwrap_or("");
        let index = call["index"].as_u64();
        let f = call
            .get("function")
            .or_else(|| call.get("function_call"))
            .unwrap_or(call);
        let name = f["name"].as_str().unwrap_or("");
        if name.len() > 256
            || name.chars().any(char::is_control)
            || id.len() > 256
            || id.chars().any(char::is_control)
        {
            return Err("Trae 工具调用标识无效".into());
        }
        let args = match f.get("arguments") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(v) if v.is_object() => v.to_string(),
            _ => return Err("Trae 工具参数格式无效".into()),
        };
        let at = self.calls.iter().position(|old| {
            (!id.is_empty() && old.id == id) || (index.is_some() && old.index == index)
        });
        if let Some(at) = at {
            let old = &mut self.calls[at];
            if (!id.is_empty() && !old.id.is_empty() && old.id != id)
                || (index.is_some() && old.index.is_some() && old.index != index)
            {
                return Err("Trae 工具调用标识冲突".into());
            }
            if old.id.is_empty() {
                old.id = id.into();
            }
            if old.index.is_none() {
                old.index = index;
            }
            if !name.is_empty() {
                if name.starts_with(&old.name) {
                    old.name = name.into();
                } else if !old.name.starts_with(name) {
                    return Err("Trae 工具名称增量冲突".into());
                }
            }
            if delta {
                old.args.push_str(&args);
            } else if !args.is_empty() && !old.args.starts_with(&args) {
                if args.starts_with(&old.args) {
                    old.args = args;
                } else if serde_json::from_str::<Value>(&old.args).is_ok() {
                    return Err("Trae 工具完整参数被不一致地改写".into());
                } else {
                    old.args.push_str(&args);
                }
            }
            if old.args.len() > MAX_EVENT {
                return Err("Trae 工具参数超过大小限制".into());
            }
        } else {
            if id.is_empty() && index.is_none() {
                return Err("Trae 工具增量缺少可匹配的 ID 或 index".into());
            }
            if self.calls.len() >= 128 {
                return Err("Trae 工具调用数量超过限制".into());
            }
            self.calls.push(NativeCall {
                id: id.into(),
                index,
                name: name.into(),
                args,
            });
        }
        Ok(())
    }
    fn close(&mut self) -> Result<Vec<Event>, String> {
        let pieces = self
            .splitter
            .as_mut()
            .map(|s| s.flush())
            .unwrap_or_default();
        let mut out = self.pieces(pieces)?;
        if self.text_calls > 0 && !self.calls.is_empty() {
            return Err("Trae 同时返回文本与原生工具，无法安全去重".into());
        }
        for c in &self.calls {
            if c.name.is_empty() {
                return Err("Trae 工具调用缺少完整名称".into());
            }
            let args = if c.args.is_empty() { "{}" } else { &c.args };
            if !serde_json::from_str::<Value>(args).is_ok_and(|v| v.is_object()) {
                return Err("Trae 工具参数不完整或不是 JSON 对象".into());
            }
            let id = if c.id.is_empty() {
                format!("call_{}", nonce()?)
            } else {
                c.id.clone()
            };
            out.push(Event::Tool(
                json!({"id":id,"type":"function","function":{"name":c.name,"arguments":args}}),
            ));
        }
        let has_tools = !self.calls.is_empty() || self.text_calls > 0;
        let reason = match self.reason.as_deref() {
            Some("length" | "content_filter") if has_tools => {
                return Err("Trae 工具回复被截断或过滤，未输出不完整调用".into())
            }
            _ if has_tools => "tool_calls".into(),
            Some("tool_calls") => return Err("Trae 声明工具终止但没有完整工具调用".into()),
            Some(reason) => reason.into(),
            None => "stop".into(),
        };
        // 只有明确完成且没有混合协议、截断或过滤时才释放文本工具调用。
        out.extend(self.pending_text_calls.drain(..).map(Event::Tool));
        out.push(Event::Done {
            reason,
            usage: Value::Object(self.usage.clone()),
        });
        self.ended = true;
        Ok(out)
    }
    fn flush(&mut self) -> Result<Vec<Event>, String> {
        if self.data.is_empty() {
            self.event.clear();
            return Ok(Vec::new());
        }
        let raw = std::mem::take(&mut self.data).join("\n");
        self.data_size = 0;
        let event = std::mem::take(&mut self.event);
        if raw == "[DONE]" {
            return self.close();
        }
        let d: Value = serde_json::from_str(&raw).map_err(|_| "Trae SSE data 不是有效 JSON")?;
        let event = if event.is_empty() {
            d.get("event")
                .or_else(|| d.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("")
        } else {
            &event
        };
        let name: String = event
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        if name == "error"
            || d.get("error").is_some_and(|v| !v.is_null() && *v != false)
            || d.get("code")
                .is_some_and(|v| *v != 0 && *v != "0" && !v.is_null())
        {
            return Err("Trae 上游返回对话业务错误，原始错误已隐藏".into());
        }
        if let Some(reason) = d
            .get("finish_reason")
            .or_else(|| d.get("stop_reason"))
            .or_else(|| d.get("finishReason"))
        {
            self.reason = Some(
                match reason.as_str() {
                    Some("length" | "max_tokens" | "max_output_tokens" | "max_length") => "length",
                    Some("sensitive" | "content_filter") => "content_filter",
                    Some("stop" | "end_turn") => "stop",
                    Some("tool_calls") => "tool_calls",
                    _ => return Err("Trae 返回未知终止原因".into()),
                }
                .into(),
            );
        }
        if let Some(usage) = d.get("usage") {
            self.usage(usage);
        }
        if name == "tokenusage" {
            self.usage(d.get("usage").unwrap_or(&d));
            return Ok(Vec::new());
        }
        if ["done", "responsedone", "streamdone"].contains(&name.as_str()) {
            return self.close();
        }
        if ![
            "",
            "output",
            "message",
            "delta",
            "content",
            "text",
            "answer",
            "chunk",
            "outputdelta",
            "messagedelta",
        ]
        .contains(&name.as_str())
        {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        if let Some(r) = [
            d.get("reasoning_content"),
            d.get("reasoning"),
            d.pointer("/delta/reasoning_content"),
            d.pointer("/delta/reasoning"),
        ]
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|r| !r.is_empty())
        {
            let repeated = (r == self.last_thought && r.len() >= 8)
                || (self.thought.len() >= 16 && r.len() >= 16 && self.thought.ends_with(r));
            self.last_thought = r.into();
            let fresh = if self.thought.len() >= 16 && r.starts_with(&self.thought) {
                &r[self.thought.len()..]
            } else {
                r
            };
            if !repeated && !fresh.is_empty() {
                self.thought.push_str(fresh);
                out.push(Event::Reasoning(fresh.into()));
            }
        }
        if let Some(text) = [
            d.get("response"),
            d.get("content"),
            d.pointer("/delta/content"),
            d.get("delta"),
        ]
        .into_iter()
        .flatten()
        .find_map(Value::as_str)
        {
            if !text.is_empty()
                && !text.starts_with("Building prompt:")
                && !text.starts_with("Completed building prompt")
            {
                if let Some(splitter) = self.splitter.as_mut() {
                    let pieces = splitter.feed(text);
                    out.extend(self.pieces(pieces)?);
                } else {
                    out.push(Event::Text(text.into()));
                }
            }
        }
        if let Some(calls) = d
            .get("tool_calls")
            .or_else(|| d.pointer("/delta/tool_calls"))
        {
            for call in calls.as_array().ok_or("Trae 工具调用列表无效")? {
                self.call(call, d.get("tool_calls").is_none())?;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn nested_cache_usage_keeps_its_prompt_subset_semantics() {
        let mut events = Events::default();
        let got = events.push(concat!(
            "event: TokenUsage\ndata: {\"prompt_tokens\":100,\"completion_tokens\":1,\"prompt_tokens_details\":{\"cached_tokens\":80}}\n\n",
            "event: done\ndata: {}\n\n"
        ).as_bytes()).unwrap();
        let Some(Event::Done { usage, .. }) = got.last() else {
            panic!("缺少完成事件")
        };
        assert_eq!(usage["prompt_tokens_details"]["cached_tokens"], 80);
        assert!(usage.get("cache_read_input_tokens").is_none());
    }

    #[test]
    fn text_tools_are_not_released_before_mixed_or_truncated_reply_is_checked() {
        let tools = vec![
            json!({"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}),
        ];
        for ending in [
            "event: output\ndata: {\"tool_calls\":[{\"id\":\"t\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{}\"}}]}\n\nevent: done\ndata: {}\n\n",
            "event: done\ndata: {\"finish_reason\":\"length\"}\n\n",
        ] {
            let mut events = Events::new(&tools);
            let first = events.push(b"event: output\ndata: {\"response\":\"<tool_call>{\\\"name\\\":\\\"lookup\\\",\\\"arguments\\\":{}}</tool_call>\"}\n\n").unwrap();
            assert!(!first.iter().any(|e| matches!(e, Event::Tool(_))), "完成验证前不能泄出可执行调用");
            assert!(events.push(ending.as_bytes()).is_err());
        }
    }

    #[test]
    fn thinking_history_is_retained_and_explicit_disable_is_not_silently_ignored() {
        let messages = json!({"model":"source/model","max_tokens":100,"messages":[
            {"role":"user","content":"question"},
            {"role":"assistant","content":[{"type":"thinking","thinking":"prior thought","signature":""},{"type":"text","text":"answer"}]},
            {"role":"user","content":"next"}
        ]});
        let chat =
            crate::gateway::translate::anthropic_request(messages.to_string().as_bytes()).unwrap();
        let output = request(&chat, "source", &route(), "s").unwrap();
        assert!(output["messages"][1]["content"]
            .to_string()
            .contains("prior thought"));
        assert!(output["messages"][1]["content"]
            .to_string()
            .contains("answer"));
        let mut disabled = messages;
        disabled["thinking"] = json!({"type":"disabled"});
        let chat =
            crate::gateway::translate::anthropic_request(disabled.to_string().as_bytes()).unwrap();
        let error = request(&chat, "source", &route(), "s").unwrap_err();
        assert!(
            error.contains("关闭思考"),
            "不能把未声明的禁用能力当成支持或无效枚举"
        );
    }
    #[test]
    fn explicit_argument_deltas_preserve_repeated_prefix_characters() {
        let mut events = Events::default();
        let mut got = Vec::new();
        for args in ["{\"s\":\"", "{", "ok\"}"] {
            let v = json!({"delta":{"tool_calls":[{"index":0,"function":{"arguments":args}}]}});
            // 名称只在首片给出。
            let mut v = v;
            if got.is_empty() && args == "{\"s\":\"" {
                v["delta"]["tool_calls"][0]["function"]["name"] = json!("lookup");
            }
            got.extend(
                events
                    .push(format!("event: output\ndata: {v}\n\n").as_bytes())
                    .unwrap(),
            );
        }
        got.extend(events.push(b"event: done\ndata: {}\n\n").unwrap());
        assert!(
            matches!(got.first(),Some(Event::Tool(v)) if v["function"]["arguments"]=="{\"s\":\"{ok\"}")
        );
    }
    #[test]
    fn complete_tools_override_generic_stop_and_reasoning_snapshots_are_not_duplicated() {
        let mut events = Events::default();
        let got=events.push(concat!(
            "event: output\ndata: {\"reasoning\":\"a sufficiently long thought\"}\n\n",
            "event: output\ndata: {\"reasoning\":\"a sufficiently long thought extended\"}\n\n",
            "event: output\ndata: {\"tool_calls\":[{\"id\":\"t\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{}\"}}]}\n\n",
            "event: done\ndata: {\"finish_reason\":\"stop\"}\n\n"
        ).as_bytes()).unwrap();
        let reasoning: String = got
            .iter()
            .filter_map(|e| {
                if let Event::Reasoning(s) = e {
                    Some(s.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(reasoning, "a sufficiently long thought extended");
        assert!(matches!(got.last(),Some(Event::Done{reason,..}) if reason=="tool_calls"));
    }
    #[test]
    fn conversation_ids_are_stable_for_same_account_but_not_shared_across_accounts() {
        let first = json!({"messages":[{"role":"user","content":"question"}]}).to_string();
        let later=json!({"messages":[{"role":"user","content":"question"},{"role":"assistant","content":"answer"},{"role":"user","content":"next"}]}).to_string();
        let a = conversation("a", "sealed-a", &first, None).unwrap();
        assert_eq!(a, conversation("a", "sealed-a", &later, None).unwrap());
        assert_ne!(a, conversation("b", "sealed-b", &first, None).unwrap());
        assert_ne!(a, conversation("a", "new-sealed", &first, None).unwrap());
        assert_ne!(
            conversation("a", "sealed-a", &first, Some("session-1")).unwrap(),
            conversation("a", "sealed-a", &first, Some("session-2")).unwrap()
        );
    }
    #[test]
    fn text_written_tools_are_held_until_complete_and_never_executed_as_text() {
        let tools = vec![
            json!({"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}),
        ];
        let mut events = Events::new(&tools);
        let a=events.push(b"event: output\ndata: {\"response\":\"<tool_call>{\\\"name\\\":\\\"lookup\\\",\\\"arguments\\\":\"}\n\n").unwrap();
        assert!(a.is_empty());
        let b=events.push(b"event: output\ndata: {\"response\":\"{}}</tool_call>\"}\n\nevent: done\ndata: {}\n\n").unwrap();
        assert!(matches!(b.first(),Some(Event::Tool(v)) if v["function"]["name"]=="lookup"));
        assert!(matches!(b.last(),Some(Event::Done{reason,..}) if reason=="tool_calls"));
    }
    #[test]
    fn sse_preserves_utf8_multiline_usage_zero_and_merges_tool_snapshots_once() {
        let mut events = Events::default();
        let raw = concat!(
            "event: output\r\ndata: {\"reasoning_content\":\"思考\",\n",
            "data: \"response\":\"你好\"}\r\n\r\n",
            "event: output\ndata: {\"tool_calls\":[{\"index\":0,\"id\":\"t\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{\\\"x\\\":\"}}]}\n\n",
            "event: output\ndata: {\"tool_calls\":[{\"index\":0,\"id\":\"t\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{\\\"x\\\":1}\"}}]}\n\n",
            "event: TokenUsage\ndata: {\"prompt_tokens\":0,\"completion_tokens\":2,\"cache_read_input_tokens\":0}\n\n",
            "event: response_done\ndata: {}\n\n"
        );
        let mut got = Vec::new();
        for byte in raw.as_bytes() {
            got.extend(events.push(&[*byte]).unwrap());
        }
        got.extend(events.finish().unwrap());
        assert_eq!(got[0], Event::Reasoning("思考".into()));
        assert_eq!(got[1], Event::Text("你好".into()));
        let calls: Vec<_> = got
            .iter()
            .filter_map(|e| {
                if let Event::Tool(v) = e {
                    Some(v)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["arguments"], "{\"x\":1}");
        assert_eq!(
            got.last(),
            Some(&Event::Done {
                reason: "tool_calls".into(),
                usage: json!({"prompt_tokens":0,"completion_tokens":2,"cache_read_input_tokens":0})
            })
        );
    }
    #[test]
    fn incomplete_stream_and_invalid_tools_never_become_success() {
        let mut events = Events::default();
        assert_eq!(
            events
                .push(b"event: output\ndata: {\"response\":\"partial\"}\n\n")
                .unwrap(),
            vec![Event::Text("partial".into())]
        );
        assert!(events.finish().is_err());
        let mut events = Events::default();
        assert!(events.push(b"event: output\ndata: {\"tool_calls\":[{\"id\":\"t\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{\"}}]}\n\nevent: done\ndata: {}\n\n").is_err());
        let mut events = Events::default();
        let error = events
            .push(b"event: error\ndata: {\"message\":\"private-token\"}\n\n")
            .unwrap_err();
        assert!(!error.contains("private-token"));
    }

    use super::*;
    fn route() -> ModelRoute {
        ModelRoute {
            function: "solo_agent".into(),
            model_name: Some("model__dev".into()),
            max_tokens: Some(8192),
            images: Some(false),
            reasoning: vec!["light".into(), "extra_high".into()],
        }
    }
    #[test]
    fn request_routes_tools_history_and_budget_without_accepting_internal_fields() {
        let body = json!({"model":"source/model","messages":[
            {"role":"developer","content":"rules"},
            {"role":"user","content":"question"},
            {"role":"assistant","content":null,"tool_calls":[{"id":"t1","type":"function","function":{"name":"lookup","arguments":"{\"x\":1}"}}]},
            {"role":"tool","tool_call_id":"t1","content":"result"}
        ],"tools":[{"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}],"tool_choice":{"type":"function","function":{"name":"lookup"}},"max_completion_tokens":9000,"reasoning_effort":"xhigh","stream":false});
        let output = request(
            body.to_string().as_bytes(),
            "source",
            &route(),
            "isolated-session",
        )
        .unwrap();
        assert_eq!(output["function"], "solo_agent");
        assert_eq!(output["config_name"], "model");
        assert_eq!(output["model_name"], "model__dev");
        assert_eq!(output["max_tokens"], 8192);
        assert_eq!(output["reasoning_effort"], "extra_high");
        assert_eq!(output["stream"], true);
        assert_eq!(output["session_id"], "isolated-session");
        assert_eq!(output["messages"][0]["role"], "system");
        assert_eq!(
            output["messages"][2]["tool_calls"][0]["function_call"]["arguments"],
            "{\"x\":1}"
        );
        assert_eq!(output["messages"][3]["tool_call_id"], "t1");
        assert_eq!(
            output["tools"][0]["function"]["parameters"],
            "{\"type\":\"object\"}"
        );
        assert_eq!(output["tool_choice"], "lookup");
        for extra in [
            json!({"session_id":"injected"}),
            json!({"function":"injected"}),
            json!({"disable_antidirt":true}),
            json!({"n":2}),
        ] {
            let mut invalid = body.clone();
            invalid
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(request(invalid.to_string().as_bytes(), "source", &route(), "s").is_err());
        }
    }
    #[test]
    fn unsupported_images_effort_and_tool_history_fail_instead_of_losing_information() {
        for body in [
            json!({"model":"source/model","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.invalid/image.png"}}]}]}),
            json!({"model":"source/model","messages":[{"role":"user","content":"x"}],"reasoning_effort":"high"}),
            json!({"model":"source/model","messages":[{"role":"tool","tool_call_id":"missing","content":"x"}]}),
            json!({"model":"other/model","messages":[{"role":"user","content":"x"}]}),
        ] {
            assert!(request(body.to_string().as_bytes(), "source", &route(), "s").is_err());
        }
    }
}
