//! Qoder agent_chat_generation 的请求体与 SSE 回复。
//! 移植自 magpie-community/plugins `packages/qoder`（MIT）的 qoderBody / events / failure。
use super::text_calls::{Piece, Splitter};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::sync::{LazyLock, Mutex};

const QODER_SYS: &str = "You are a Qoder agent. Use the instructions below and the tools available to you to assist the user.";
const EFFORT_ORDER: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];
const EFFORT_RANK: [&str; 8] = ["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"];
/// max_tokens 不超过此值的请求（会话标题、分类）不思考，预算留给回答。
const SMALL_BUDGET: i64 = 1024;
const NAMELESS: &str = "模型的原生工具调用没有名称，未作为工具转发";

/// 模型列表里一个条目读出的能力。
pub(super) struct Info {
    pub key: String,
    pub source: String,
    context: i64,
    windows: Vec<i64>,
    default_window: i64,
    thinks: bool,
    always_thinks: bool,
    efforts: Vec<String>,
    default_effort: String,
    raw: Value,
}

fn positive(v: &Value) -> i64 {
    v.as_i64().filter(|n| *n > 0).unwrap_or(0)
}

/// context_config 的各档上下文（小到大）与默认档；没有时读 available_context_windows。
fn windows_of(raw: &Value) -> (Vec<i64>, i64) {
    let mut windows = Vec::new();
    let mut default = 0;
    if let Some(config) = raw.get("context_config").or_else(|| raw.get("contextConfig")).and_then(Value::as_object) {
        for entry in config.values() {
            let tokens = positive(&entry["token_count"]);
            if tokens == 0 {
                continue;
            }
            if !windows.contains(&tokens) {
                windows.push(tokens);
            }
            if entry["is_default"] == true || entry["is_default"] == 1 || entry["is_default"] == "1" {
                default = tokens;
            }
        }
    }
    if windows.is_empty() {
        for v in raw.get("available_context_windows").or_else(|| raw.get("availableContextWindows")).and_then(Value::as_array).into_iter().flatten() {
            let n = positive(v);
            if n > 0 && !windows.contains(&n) {
                windows.push(n);
            }
        }
        default = positive(raw.get("default_context_window").or_else(|| raw.get("defaultContextWindow")).unwrap_or(&Value::Null));
        if !windows.contains(&default) {
            default = 0;
        }
    }
    windows.sort_unstable();
    (windows, default)
}

pub(super) fn model_info(raw: &Value) -> Info {
    let (windows, default_window) = windows_of(raw);
    let mut m = Info {
        key: raw["key"].as_str().unwrap_or_default().into(),
        source: raw["source"].as_str().unwrap_or_default().into(),
        context: windows.last().copied().unwrap_or_else(|| positive(&raw["max_input_tokens"])),
        windows,
        default_window,
        thinks: raw["is_reasoning"] == true,
        always_thinks: false,
        efforts: raw["reasoning_efforts"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect(),
        default_effort: String::new(),
        raw: raw.clone(),
    };
    if let Some(tc) = raw.get("thinking_config").filter(|v| v.is_object()) {
        m.thinks = !tc["enabled"].is_null();
        m.always_thinks = m.thinks && tc["disabled"].is_null();
        m.efforts.clear();
        if m.thinks {
            for (name, e) in tc["enabled"]["efforts"].as_object().into_iter().flatten() {
                m.efforts.push(name.clone());
                if e["is_default"] == true {
                    m.default_effort = name.clone();
                }
            }
            let rank = |s: &str| EFFORT_ORDER.iter().position(|e| *e == s).unwrap_or(EFFORT_ORDER.len());
            m.efforts.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
        }
    }
    m
}

fn fit_effort(want: &str, levels: &[String]) -> String {
    let want = if want == "ultra" && !levels.iter().any(|l| l == want) { "max" } else { want };
    if levels.is_empty() || levels.iter().any(|l| l == want) {
        return want.into();
    }
    let Some(at) = EFFORT_RANK.iter().position(|e| *e == want) else { return want.into() };
    let mut best = want.to_string();
    let mut dist = EFFORT_RANK.len();
    for level in levels {
        let Some(i) = EFFORT_RANK.iter().position(|e| e == level) else { continue };
        if level == "none" {
            continue;
        }
        let d = i.abs_diff(at);
        if d < dist || (d == dist && i > at) {
            best = level.clone();
            dist = d;
        }
    }
    best
}

/// 是否思考、用哪一档（"" 为模型默认或无档位）。
fn effort_for(asked: &str, m: &Info, budget: i64) -> (bool, String) {
    let mut want = asked.to_ascii_lowercase();
    if !EFFORT_RANK.contains(&want.as_str()) {
        want.clear();
    }
    if want.is_empty() && budget > 0 && budget <= SMALL_BUDGET {
        want = "none".into();
    }
    if !m.thinks || (want == "none" && !m.always_thinks) {
        return (false, String::new());
    }
    if m.efforts.is_empty() {
        return (true, String::new());
    }
    if want.is_empty() {
        return (true, m.default_effort.clone());
    }
    if want == "none" {
        return (true, m.efforts[0].clone());
    }
    (true, fit_effort(&want, &m.efforts))
}

fn image_block(p: &Value) -> Option<Value> {
    let url = p["image_url"].as_str().or_else(|| p["image_url"]["url"].as_str())?;
    Some(json!({"type":"image_url","image_url":{"url":url}}))
}

fn blocks_of(content: &Value) -> Vec<Value> {
    if let Some(text) = content.as_str() {
        return if text.is_empty() { Vec::new() } else { vec![json!({"type":"text","text":text})] };
    }
    let mut out = Vec::new();
    for p in content.as_array().into_iter().flatten() {
        if p["type"] == "text" && !p["text"].is_null() {
            out.push(json!({"type":"text","text":p["text"]}));
        } else if p["type"] == "image_url" {
            out.extend(image_block(p));
        }
    }
    out
}

fn text_of(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.into();
    }
    blocks_of(content).iter().filter_map(|b| b["text"].as_str()).collect()
}

fn hex_digest(seed: &str) -> String {
    Sha256::digest(seed.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// 客户端没给 id 的调用：按位置与内容生成，同一会话每轮一致，不破坏提示缓存。
fn call_id(at: usize, k: usize, c: &Value) -> String {
    let args = &c["function"]["arguments"];
    let said = args.as_str().map(str::to_owned).unwrap_or_else(|| if args.is_null() { "{}".into() } else { args.to_string() });
    let name = c["function"]["name"].as_str().unwrap_or_default();
    format!("call_{}", &hex_digest(&format!("{at}\0{k}\0{name}\0{said}"))[..32])
}

fn reasoning_of(m: &Value) -> String {
    let mut text = ["reasoning_content", "reasoning"].iter().find_map(|k| m[*k].as_str()).unwrap_or_default().to_string();
    if text.is_empty() {
        if let Some(details) = m["reasoning_details"].as_array() {
            text = details.iter().map(|d| d["text"].as_str().or_else(|| d["summary"].as_str()).unwrap_or_default()).collect();
        }
    }
    if text.trim().is_empty() { String::new() } else { text }
}

struct Turn {
    open: Vec<String>,
    kept: Vec<String>,
}

/// 对话转成 Qoder 接收的轮次：文本与图片块、助手的 tool_calls、按 tool_call_id 对应的
/// 工具结果。工具结果只能是文本，其中的图片挪到随后的用户轮。丢了 id 的结果按顺序补齐。
fn qoder_messages(msgs: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut seen: Vec<Value> = Vec::new();
    let mut turn: Option<Turn> = None;
    fn show_seen(out: &mut Vec<Value>, seen: &mut Vec<Value>) {
        if !seen.is_empty() {
            out.push(json!({"role":"user","content":std::mem::take(seen)}));
        }
    }
    for (i, m) in msgs.iter().enumerate() {
        let role = m["role"].as_str().unwrap_or_default();
        if role == "system" || role == "developer" {
            continue;
        }
        if role == "user" {
            turn = None;
        }
        if role != "user" && role != "tool" {
            show_seen(&mut out, &mut seen);
        }
        if role == "tool" {
            let mut txt = text_of(&m["content"]);
            let ims: Vec<Value> = m["content"].as_array().into_iter().flatten().filter(|p| p["type"] == "image_url").filter_map(image_block).collect();
            let named = m["tool_call_id"].as_str().map(str::trim).unwrap_or_default().to_string();
            let pick = turn.as_ref().and_then(|t| {
                let at = if named.is_empty() { None } else { t.open.iter().position(|id| *id == named) };
                at.or_else(|| t.open.iter().position(|id| !t.kept.contains(id)))
            });
            let Some(pick) = pick else {
                let said = txt.trim().to_string();
                if !said.is_empty() || !ims.is_empty() {
                    if !seen.is_empty() {
                        seen.push(json!({"type":"text","text":"\n\n"}));
                    }
                    if !said.is_empty() {
                        seen.push(json!({"type":"text","text":said}));
                    }
                    seen.extend(ims);
                }
                continue;
            };
            let answered = turn.as_mut().map(|t| t.open.remove(pick)).unwrap_or_default();
            if !ims.is_empty() {
                let mut of = format!("tool call {answered}");
                if let Some(name) = names.get(&answered).cloned().or_else(|| m["name"].as_str().map(str::to_owned)) {
                    of = format!("{name} ({of})");
                }
                seen.push(json!({"type":"text","text":format!("[From the result of {of}:]")}));
                let count = ims.len();
                seen.extend(ims);
                let note = if count == 1 {
                    "[The tool returned an image; it follows in the next message.]".to_string()
                } else {
                    format!("[The tool returned {count} images; they follow in the next message.]")
                };
                txt = if txt.trim().is_empty() { txt + &note } else { txt + "\n\n" + &note };
            }
            out.push(json!({"role":"tool","tool_call_id":answered,"content":txt}));
            continue;
        }
        if role == "assistant" && m["tool_calls"].as_array().is_some_and(|c| !c.is_empty()) {
            let calls: Vec<Value> = m["tool_calls"].as_array().unwrap().iter().enumerate().map(|(k, c)| {
                let id = c["id"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(|| call_id(i, k, c));
                let name = c["function"]["name"].clone();
                if let Some(n) = name.as_str() {
                    names.insert(id.clone(), n.into());
                }
                let args = &c["function"]["arguments"];
                let args = args.as_str().map(str::to_owned).unwrap_or_else(|| if args.is_null() { "{}".into() } else { args.to_string() });
                json!({"id":id,"type":"function","function":{"name":name,"arguments":args}})
            }).collect();
            let mut turn_msg = json!({"role":"assistant","content":text_of(&m["content"]),"tool_calls":calls});
            let think = reasoning_of(m);
            if !think.is_empty() {
                turn_msg["reasoning_content"] = json!(think);
            }
            out.push(turn_msg);
            let open: Vec<String> = calls.iter().filter_map(|c| c["id"].as_str().map(str::to_owned)).collect();
            let kept = msgs[i + 1..].iter().take_while(|x| x["role"] == "tool")
                .filter_map(|x| x["tool_call_id"].as_str().map(str::trim).map(str::to_owned))
                .filter(|id| open.contains(id)).collect();
            turn = Some(Turn { open, kept });
            continue;
        }
        let mut blocks = blocks_of(&m["content"]);
        if role == "assistant" {
            blocks.retain(|b| b["type"] == "text");
        }
        if blocks.is_empty() {
            continue;
        }
        if role == "user" && !seen.is_empty() {
            let mut merged = std::mem::take(&mut seen);
            merged.extend(blocks);
            blocks = merged;
        }
        let mut turn_msg = json!({"role":role,"content":blocks});
        if role == "assistant" {
            let think = reasoning_of(m);
            if !think.is_empty() {
                turn_msg["reasoning_content"] = json!(think);
            }
        }
        out.push(turn_msg);
        turn = None;
    }
    show_seen(&mut out, &mut seen);
    out
}

/// 按 Qoder 客户端的提示缓存标记：用户轮的块另存为 contents，最后一轮最后一个非思考/工具块加 ephemeral。
fn cache_marked(msgs: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = msgs.into_iter().map(|mut m| {
        if m["role"] == "user" && m["content"].is_array() {
            m["contents"] = m["content"].clone();
        }
        m
    }).collect();
    let Some(last) = out.last_mut() else { return out };
    if last["role"] != "user" {
        return out;
    }
    let Some(blocks) = last["content"].as_array_mut() else { return out };
    let uncached = ["thinking", "redacted_thinking", "tool_use", "tool_result"];
    let Some(at) = blocks.iter().rposition(|b| b["type"].as_str().is_some_and(|t| !uncached.contains(&t))) else { return out };
    blocks[at]["cache_control"] = json!({"type":"ephemeral"});
    let copy = Value::Array(blocks.clone());
    last["contents"] = copy;
    out
}

/// 请求要的上下文档：放得下就用默认档，否则取能放下的最小档，再不行用最大档。
fn window_for(chat: &Value, m: &Info, reply: i64) -> i64 {
    let max_input = positive(&m.raw["max_input_tokens"]);
    let base = if m.default_window > 0 { m.default_window } else if max_input > 0 { max_input } else { m.context };
    if m.windows.is_empty() {
        return m.context;
    }
    let size = json!({"m":chat["messages"],"t":chat["tools"]}).to_string().len() as i64;
    let need = (size + 2) / 3 + reply.max(0);
    if base > 0 && need <= base {
        return base;
    }
    m.windows.iter().copied().find(|w| *w >= need).unwrap_or(*m.windows.last().unwrap())
}

/// 同一会话的每轮用同一个 session_id：prompt_cache_key、user，否则用开头（系统提示与第一条用户消息）的摘要。
pub(super) fn session_of(chat: &Value) -> String {
    let seed = if let Some(key) = chat["prompt_cache_key"].as_str().filter(|s| !s.trim().is_empty()) {
        format!("key\0{key}")
    } else if let Some(user) = chat["user"].as_str().filter(|s| !s.trim().is_empty()) {
        format!("user\0{user}")
    } else {
        let msgs = chat["messages"].as_array().cloned().unwrap_or_default();
        let sys: Vec<String> = msgs.iter().filter(|x| x["role"] == "system" || x["role"] == "developer").map(|x| text_of(&x["content"])).collect();
        let first = msgs.iter().find(|x| x["role"] == "user").map(|x| x["content"].clone()).unwrap_or(json!(""));
        format!("chat\0{}", json!([sys, first]))
    };
    let h = hex_digest(&seed);
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// 一个任务：回答同一条用户提示的所有模型调用（含工具循环）共用 request_set_id 与业务记录。
#[derive(Clone)]
pub(super) struct Task {
    pub key: String,
    id: String,
    begin: i64,
    answered: bool,
    name: String,
    used: i64,
}

static TASKS: LazyLock<Mutex<HashMap<String, Task>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
const TASK_MAX: usize = 512;
const TASK_IDLE_MS: i64 = 2 * 60 * 60 * 1000;

pub(super) fn task_of(sid: &str, chat: &Value) -> Task {
    let now = chrono::Utc::now().timestamp_millis();
    let msgs = chat["messages"].as_array().cloned().unwrap_or_default();
    let at = msgs.iter().rposition(|x| x["role"] == "user");
    let content = at.map(|i| msgs[i]["content"].clone()).unwrap_or(Value::Null);
    let key = hex_digest(&json!([sid, at.map(|i| i as i64).unwrap_or(-1), content]).to_string());
    let Ok(mut tasks) = TASKS.lock() else {
        return Task { key, id: super::qoder_cosy::random_uuid(), begin: now, answered: false, name: String::new(), used: now };
    };
    tasks.retain(|_, t| now - t.used <= TASK_IDLE_MS);
    let task = tasks.entry(key.clone()).or_insert_with(|| {
        let prompt = at.map(|i| text_of(&msgs[i]["content"])).unwrap_or_default();
        Task { key: key.clone(), id: super::qoder_cosy::random_uuid(), begin: now, answered: false, name: prompt.chars().take(10).collect(), used: now }
    });
    task.used = now;
    let out = task.clone();
    if tasks.len() > TASK_MAX {
        if let Some(oldest) = tasks.iter().min_by_key(|(_, t)| t.used).map(|(k, _)| k.clone()) {
            tasks.remove(&oldest);
        }
    }
    out
}

/// 调用得到回答后，任务进入 processing 阶段。
pub(super) fn task_answered(key: &str) {
    if let Ok(mut tasks) = TASKS.lock() {
        if let Some(t) = tasks.get_mut(key) {
            t.answered = true;
        }
    }
}

/// 无 type 的 oneOf（各分支都是对象）补上 "type":"object"：Qoder 国内版 kmodel 不接受前者。
fn typed_unions(schema: &Value) -> Value {
    fn target<'a>(root: &'a Value, r: &str) -> Option<&'a Value> {
        let mut at = root;
        for k in r.strip_prefix("#/")?.split('/') {
            at = at.get(k.replace("~1", "/").replace("~0", "~"))?;
        }
        at.is_object().then_some(at)
    }
    fn object(root: &Value, b: &Value, seen: &mut Vec<String>) -> bool {
        if !b.is_object() {
            return false;
        }
        if let Some(t) = b.get("type") {
            return t == "object";
        }
        let Some(r) = b["$ref"].as_str() else { return false };
        if seen.iter().any(|s| s == r) || seen.len() > 16 {
            return false;
        }
        seen.push(r.into());
        target(root, r).is_some_and(|t| object(root, t, seen))
    }
    fn walk(root: &Value, s: &mut Value, depth: usize) {
        if !s.is_object() || depth > 64 {
            return;
        }
        let typed = s.get("type").is_none()
            && s["oneOf"].as_array().is_some_and(|branches| !branches.is_empty() && branches.iter().all(|b| object(root, b, &mut Vec::new())));
        if typed {
            s["type"] = json!("object");
        }
        let map = s.as_object_mut().unwrap();
        for (k, v) in map.iter_mut() {
            match k.as_str() {
                "items" | "additionalItems" | "unevaluatedItems" | "contains" | "not" | "if" | "then" | "else" | "propertyNames" | "additionalProperties" | "unevaluatedProperties" => walk(root, v, depth + 1),
                "anyOf" | "oneOf" | "allOf" | "prefixItems" => v.as_array_mut().into_iter().flatten().for_each(|x| walk(root, x, depth + 1)),
                "properties" | "patternProperties" | "$defs" | "definitions" | "dependentSchemas" => v.as_object_mut().into_iter().flat_map(|o| o.values_mut()).for_each(|x| walk(root, x, depth + 1)),
                _ => {}
            }
        }
    }
    if !schema.is_object() {
        return schema.clone();
    }
    let root = schema.clone();
    let mut out = schema.clone();
    walk(&root, &mut out, 0);
    out
}

/// 请求携带的 model_config：按 Qoder 客户端从列表条目挑字段，不原样回传整个条目。
fn model_config(m: &Info, thinking: bool) -> Result<Value, String> {
    let raw = &m.raw;
    let key = raw["key"].as_str().map(str::trim).unwrap_or_default();
    if key.is_empty() || key == "auto" || key == "default" || key != m.key {
        return Err(format!("Qoder 没有模型「{}」的模型键", m.key));
    }
    let max = raw["max_input_tokens"].as_i64().filter(|n| *n > 0).unwrap_or(200_000);
    let mut config = json!({
        "key": key,
        "display_name": raw["display_name"].as_str().filter(|s| !s.is_empty()).unwrap_or(key),
        "format": raw["format"].as_str().unwrap_or("openai"),
        "is_vl": raw["is_vl"] == true,
        "is_reasoning": raw["is_reasoning"] == true && thinking,
        "api_key": "",
        "url": "",
        "source": raw["source"].as_str().filter(|s| !s.is_empty()).unwrap_or("system"),
        "max_input_tokens": max,
    });
    match raw.get("outer_provider").filter(|v| !v.is_null() && *v != "" && *v != false) {
        Some(provider) => config["outer_provider"] = provider.clone(),
        None => config["model"] = json!(""),
    }
    Ok(config)
}

/// OpenAI Chat 请求转成 agent_chat_generation 的明文请求体。
pub(super) fn qoder_body(chat: &Value, m: &Info, task: &Task) -> Result<Value, String> {
    let msgs = chat["messages"].as_array().cloned().unwrap_or_default();
    let system: Vec<String> = msgs.iter().filter(|x| x["role"] == "system" || x["role"] == "developer").map(|x| text_of(&x["content"])).filter(|t| !t.is_empty()).collect();
    let tools: Vec<&Value> = if chat["tool_choice"] == "none" {
        Vec::new()
    } else {
        chat["tools"].as_array().into_iter().flatten().filter(|t| t["type"] == "function" && t["function"]["name"].as_str().is_some_and(|n| !n.is_empty())).collect()
    };
    let mut sys_text = if system.is_empty() { QODER_SYS.to_string() } else { format!("{QODER_SYS}\n\n{}", system.join("\n\n")) };
    if chat["tool_choice"] == "required" && !tools.is_empty() {
        sys_text.push_str("\nYou must call an available function in this response.");
    }
    let sys = json!({"type":"text","text":sys_text});
    let max_tokens = [&chat["max_completion_tokens"], &chat["max_tokens"]].iter().find_map(|v| v.as_i64().filter(|n| *n > 0)).unwrap_or(0);
    let (thinking, effort) = effort_for(chat["reasoning_effort"].as_str().unwrap_or_default(), m, max_tokens);
    let mut params = json!({"enable_thinking":thinking,"max_tokens":if max_tokens > 0 { max_tokens } else { 32_000 }});
    if !effort.is_empty() {
        params["reasoning_effort"] = json!(effort);
    }
    let window = window_for(chat, m, params["max_tokens"].as_i64().unwrap_or(0));
    if window > 0 {
        params["context_length"] = json!(window);
    }
    let request = super::qoder_cosy::random_uuid();
    let mut messages = vec![json!({"role":"system","content":[sys]})];
    messages.extend(cache_marked(qoder_messages(&msgs)));
    let mut body = json!({
        "request_id": request,
        "request_set_id": task.id,
        "chat_record_id": request,
        "session_id": session_of(chat),
        "parameters": params,
        "business": {"product":"app","version":super::qoder_cosy::COSY_VERSION,"type":"agent","id":task.id,"name":task.name,"begin_at":task.begin,"stage":if task.answered { "processing" } else { "start" },"model_key":m.key},
        "agent_id": "agent_common",
        "task_id": "common",
        "session_type": "app",
        "model_config": model_config(m, thinking)?,
        "system": [sys],
        "messages": messages,
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.iter().map(|t| {
            let params = t["function"]["parameters"].clone();
            json!({"type":"function","function":{
                "name": t["function"]["name"],
                "description": t["function"]["description"].as_str().unwrap_or_default(),
                "parameters": if params.is_object() { typed_unions(&params) } else { json!({"type":"object","properties":{}}) },
            }})
        }).collect());
    }
    Ok(body)
}

/// Qoder 失败的状态与说明。只有登录失效（401/403 且不是排队码）会触发换票重试。
#[derive(Debug, Clone)]
pub(super) struct Failure {
    pub status: u16,
    pub message: String,
    pub retry_after: Option<u64>,
    pub lapsed: bool,
    pub nameless: bool,
}

impl Failure {
    fn new(status: u16, message: impl Into<String>) -> Self {
        Self { status, message: message.into(), retry_after: None, lapsed: false, nameless: false }
    }
}

const QUEUED: &str = "10605";
const QUOTA_CODES: [&str; 10] = ["110", "112", "113", "114", "115", "116", "117", "118", "119", "122"];
const REQUEST_CODES: [&str; 8] = ["406", "416", "430", "80411", "80412", "100400", "100401", "100403"];

pub(super) fn failure(status: u16, text: &str) -> Failure {
    let status = if (400..=599).contains(&status) { status } else { 502 };
    let mut msg = text.trim().to_string();
    let mut code = String::new();
    let mut queue: Option<Value> = None;
    if let Ok(mut j) = serde_json::from_str::<Value>(&msg) {
        let code_of = |v: &Value| match &v["code"] {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let outer = code_of(&j);
        if ["", "401", "403"].contains(&outer.as_str()) {
            if let Some(inner) = j["message"].as_str().and_then(|m| serde_json::from_str::<Value>(m.trim()).ok()) {
                if inner.is_object() && code_of(&inner) == QUEUED {
                    j = inner;
                }
            }
        }
        code = code_of(&j);
        if let Some(m) = j["message"].as_str().filter(|m| !m.is_empty()) {
            msg = m.into();
        }
        let details = j["details"].as_str().and_then(|d| serde_json::from_str::<Value>(d.trim()).ok()).unwrap_or_else(|| j["details"].clone());
        if let Some(m) = details["error"]["message"].as_str() {
            msg = format!("{msg}: {m}");
        }
        queue = j.get("queue").filter(|q| q.is_object()).cloned().or_else(|| {
            let q: Value = serde_json::from_str(j["message"].as_str()?.trim()).ok()?;
            ["isQueued", "serviceAvailable", "retryAfterSeconds"].iter().any(|k| q.get(*k).is_some()).then_some(q)
        });
    }
    if msg.is_empty() {
        msg = format!("HTTP {status}");
    }
    if code == QUEUED || queue.as_ref().is_some_and(|q| q["isQueued"] == true || q["serviceAvailable"] == false) {
        let q = queue.unwrap_or(Value::Null);
        let wait = q["retryAfterSeconds"].as_f64().filter(|n| *n > 0.0).map(|n| n.ceil() as u64).unwrap_or(30);
        let model = q["modelKey"].as_str().map(|m| format!("（{m}）")).unwrap_or_default();
        let mut f = Failure::new(503, format!("Qoder 模型正在排队{model}，请 {wait} 秒后重试"));
        f.retry_after = Some(wait);
        return f;
    }
    if QUOTA_CODES.contains(&code.as_str()) {
        return Failure::new(429, format!("Qoder 额度已用尽：{msg}"));
    }
    if REQUEST_CODES.contains(&code.as_str()) {
        return Failure::new(400, format!("Qoder 拒绝了此请求（{code}）：{msg}"));
    }
    if msg.to_ascii_lowercase().contains("error in upstream response") {
        return Failure::new(502, msg);
    }
    if status == 401 || status == 403 {
        let mut f = Failure::new(401, format!("Qoder 登录已失效，请重新登录（Qoder 返回 {status}：{}）", msg.chars().take(300).collect::<String>()));
        f.lapsed = true;
        return f;
    }
    if status == 429 || msg.to_ascii_lowercase().contains("quota") {
        return Failure::new(429, format!("Qoder 额度已用尽：{msg}"));
    }
    Failure::new(status, msg)
}

/// 回复里的一个片段。
#[derive(Debug)]
pub(super) enum Event {
    Text(String),
    Reasoning(String),
    Tool { index: usize, id: String, name: String, args: String },
    Args { index: usize, text: String },
    Stop { reason: &'static str, usage: Option<Value> },
    Error(Failure),
}

#[derive(Default)]
struct Native {
    id: String,
    args: String,
    index: Option<usize>,
}

/// Qoder SSE：每个 data 行是一个信封，body 为 OpenAI chunk，其文本里可能带工具调用。
pub(super) struct Events {
    splitter: Splitter,
    next_index: usize,
    native: BTreeMap<i64, Native>,
    usage: Option<Value>,
    finish: String,
    said: bool,
    pub done: bool,
}

fn new_call_id() -> String {
    format!("call_{}", super::qoder_cosy::random_uuid().replace('-', ""))
}

impl Events {
    pub(super) fn new(tools: &[Value]) -> Self {
        Self { splitter: Splitter::new(tools), next_index: 0, native: BTreeMap::new(), usage: None, finish: String::new(), said: false, done: false }
    }

    fn piece(&mut self, p: Piece) -> Event {
        match p {
            Piece::Text(t) => Event::Text(t),
            Piece::Error(e) => Event::Error(Failure::new(502, e)),
            Piece::Call { name, args } => {
                let index = self.next_index;
                self.next_index += 1;
                Event::Tool { index, id: new_call_id(), name, args }
            }
        }
    }

    /// 把切分出的片段转成事件；遇到错误就结束本轮。
    fn pieces(&mut self, pieces: Vec<Piece>, out: &mut Vec<Event>) -> bool {
        for p in pieces {
            let e = self.piece(p);
            let failed = matches!(e, Event::Error(_));
            if !failed {
                self.said = true;
            }
            out.push(e);
            if failed {
                self.done = true;
                return false;
            }
        }
        true
    }

    pub(super) fn line(&mut self, line: &str) -> Vec<Event> {
        let mut out = Vec::new();
        if self.done {
            return out;
        }
        let Some(payload) = line.strip_prefix("data:").map(str::trim) else { return out };
        let Ok(env) = serde_json::from_str::<Value>(payload) else { return out };
        let inner = env["body"].as_str().unwrap_or_default();
        if let Some(code) = env["statusCodeValue"].as_i64().filter(|c| *c != 200) {
            self.done = true;
            out.push(Event::Error(failure(code as u16, if inner.is_empty() { payload } else { inner })));
            return out;
        }
        if inner.trim() == "[DONE]" {
            return self.finish();
        }
        let Ok(chunk) = serde_json::from_str::<Value>(inner.trim()) else { return out };
        if !chunk["choices"].is_array() && (!chunk["error"].is_null() || (!chunk["code"].is_null() && !chunk["message"].is_null())) {
            self.done = true;
            out.push(Event::Error(failure(502, inner)));
            return out;
        }
        let delta = &chunk["choices"][0]["delta"];
        if let Some(r) = delta["reasoning_content"].as_str().filter(|r| !r.is_empty()) {
            self.said = true;
            out.push(Event::Reasoning(r.into()));
        }
        if let Some(text) = delta["content"].as_str() {
            let pieces = self.splitter.feed(text);
            if !self.pieces(pieces, &mut out) {
                return out;
            }
        }
        for tc in delta["tool_calls"].as_array().into_iter().flatten() {
            let i = tc["index"].as_i64().unwrap_or(0);
            let name = tc["function"]["name"].as_str().map(str::trim).unwrap_or_default().to_string();
            let args = tc["function"]["arguments"].as_str().unwrap_or_default().to_string();
            let id = tc["id"].as_str().unwrap_or_default();
            if id.is_empty() && name.is_empty() && args.is_empty() {
                continue;
            }
            let call = self.native.entry(i).or_default();
            if !id.is_empty() {
                call.id = id.into();
            }
            match call.index {
                // id 或参数可能先于名称到达；没有名称的工具绝不发出
                None => {
                    call.args.push_str(&args);
                    if name.is_empty() {
                        continue;
                    }
                    let pending = self.splitter.flush();
                    if !self.pieces(pending, &mut out) {
                        return out;
                    }
                    self.splitter.saw_tool = true;
                    self.said = true;
                    let index = self.next_index;
                    self.next_index += 1;
                    let call = self.native.get_mut(&i).unwrap();
                    call.index = Some(index);
                    let id = if call.id.is_empty() { new_call_id() } else { call.id.clone() };
                    out.push(Event::Tool { index, id, name, args: call.args.clone() });
                }
                Some(index) if !args.is_empty() => out.push(Event::Args { index, text: args }),
                Some(_) => {}
            }
        }
        if let Some(f) = chunk["choices"][0]["finish_reason"].as_str().filter(|f| !f.is_empty()) {
            self.finish = f.into();
        }
        // 用量在 finish 之后单独一个无 choices 的 chunk
        if let Some(u) = chunk.get("usage").filter(|u| u.is_object()) {
            let prompt = u["prompt_tokens"].as_i64().unwrap_or(0);
            let completion = u["completion_tokens"].as_i64().unwrap_or(0);
            let mut usage = json!({"prompt_tokens":prompt,"completion_tokens":completion,"total_tokens":prompt + completion});
            if let Some(cached) = u["prompt_tokens_details"]["cached_tokens"].as_i64().filter(|n| *n > 0) {
                usage["prompt_tokens_details"] = json!({"cached_tokens":cached});
            }
            if let Some(reasoning) = u["completion_tokens_details"]["reasoning_tokens"].as_i64().filter(|n| *n > 0) {
                usage["completion_tokens_details"] = json!({"reasoning_tokens":reasoning});
            }
            self.usage = Some(usage);
        }
        out
    }

    pub(super) fn finish(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        if self.done {
            return out;
        }
        let pending = self.splitter.flush();
        if !self.pieces(pending, &mut out) {
            return out;
        }
        self.done = true;
        let nameless = self.native.values().any(|c| c.index.is_none());
        if nameless && !self.said {
            let mut f = Failure::new(502, NAMELESS);
            f.nameless = true;
            out.push(Event::Error(f));
            return out;
        }
        let reason = if self.splitter.saw_tool { "tool_calls" } else if self.finish == "length" { "length" } else { "stop" };
        out.push(Event::Stop { reason, usage: self.usage.take() });
        out
    }
}

/// 记录用量时把 OpenAI Chat 用量换成网关账本的字段名。
pub(super) fn metered(usage: &Value) -> Value {
    let mut out = Map::new();
    out.insert("input_tokens".into(), usage["prompt_tokens"].clone());
    out.insert("output_tokens".into(), usage["completion_tokens"].clone());
    out.insert("total_tokens".into(), usage["total_tokens"].clone());
    if let Some(cached) = usage["prompt_tokens_details"]["cached_tokens"].as_i64() {
        out.insert("cache_read_input_tokens".into(), json!(cached));
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing_entry() -> Value {
        json!({"key":"gmodel","display_name":"GLM-5.3","source":"system","enable":true,"is_vl":false,"is_reasoning":true,
            "max_input_tokens":180000,"format":"openai","price_factor":1,"api_key":"must-not-leak",
            "context_config":{"200k":{"token_count":200000,"is_default":true},"1m":{"token_count":1000000}},
            "thinking_config":{"enabled":{"efforts":{"high":{"is_default":true},"low":{},"max":{}}},"disabled":{}}})
    }

    #[test]
    fn listing_entry_reads_efforts_windows_and_config() {
        let m = model_info(&listing_entry());
        assert_eq!(m.efforts, ["low", "high", "max"]);
        assert_eq!(m.default_effort, "high");
        assert_eq!((m.default_window, m.context), (200_000, 1_000_000));
        assert_eq!(effort_for("", &m, 0), (true, "high".into()));
        assert_eq!(effort_for("none", &m, 0), (false, String::new()));
        assert_eq!(effort_for("", &m, 64), (false, String::new()), "小预算请求不思考");
        assert_eq!(effort_for("medium", &m, 0), (true, "high".into()));
        let config = model_config(&m, true).unwrap();
        assert_eq!(config["key"], "gmodel");
        assert_eq!(config["api_key"], "", "不回传条目里的 Key");
        assert_eq!(config["model"], "");
        assert!(model_config(&model_info(&json!({"key":"auto"})), false).is_err());
    }

    #[test]
    fn chat_turns_become_qoder_messages() {
        let msgs = vec![
            json!({"role":"system","content":"sys"}),
            json!({"role":"user","content":"read it"}),
            json!({"role":"assistant","content":"","reasoning_content":"think","tool_calls":[{"function":{"name":"read","arguments":{"path":"a"}}}]}),
            json!({"role":"tool","content":[{"type":"text","text":"done"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AA"}}]}),
            json!({"role":"user","content":"next"}),
        ];
        let out = qoder_messages(&msgs);
        assert_eq!(out.len(), 4);
        let id = out[1]["tool_calls"][0]["id"].as_str().unwrap();
        assert!(id.starts_with("call_"));
        assert_eq!(out[1]["tool_calls"][0]["function"]["arguments"], r#"{"path":"a"}"#);
        assert_eq!(out[1]["reasoning_content"], "think");
        assert_eq!(out[2]["tool_call_id"], id, "丢了 id 的结果按顺序对应调用");
        assert!(out[2]["content"].as_str().unwrap().contains("follows in the next message"));
        assert_eq!(out[3]["content"][1]["type"], "image_url", "工具图片挪到下一条用户消息");
        assert_eq!(out[3]["content"].as_array().unwrap().last().unwrap()["text"], "next");
        let marked = cache_marked(out);
        assert_eq!(marked[3]["content"].as_array().unwrap().last().unwrap()["cache_control"]["type"], "ephemeral");
        assert!(marked[0]["contents"].is_array());
    }

    #[test]
    fn body_carries_task_session_model_and_typed_tools() {
        let m = model_info(&listing_entry());
        let chat = json!({"model":"s/gmodel","messages":[{"role":"system","content":"be brief"},{"role":"user","content":"hi"}],
            "tool_choice":"required","tools":[{"type":"function","function":{"name":"pick","parameters":{"type":"object","properties":{"v":{"oneOf":[{"type":"object"},{"type":"object"}]}}}}}]});
        let task = task_of(&session_of(&chat), &chat);
        let body = qoder_body(&chat, &m, &task).unwrap();
        assert_eq!(body["request_set_id"], task.id);
        assert_eq!(body["business"]["model_key"], "gmodel");
        assert_eq!(body["business"]["stage"], "start");
        assert_eq!(body["parameters"]["reasoning_effort"], "high");
        assert_eq!(body["parameters"]["context_length"], 200_000);
        assert!(body["system"][0]["text"].as_str().unwrap().ends_with("be brief\nYou must call an available function in this response."));
        assert_eq!(body["tools"][0]["function"]["parameters"]["properties"]["v"]["type"], "object");
        assert_eq!(body["messages"][0]["role"], "system");
        task_answered(&task.key);
        let again = qoder_body(&chat, &m, &task_of(&session_of(&chat), &chat)).unwrap();
        assert_eq!(again["request_set_id"], task.id, "同一提示的后续调用沿用任务");
        assert_eq!(again["business"]["stage"], "processing");
    }

    #[test]
    fn failures_map_to_gateway_statuses() {
        let queued = failure(403, r#"{"code":"10605","message":"{\"isQueued\":true,\"retryAfterSeconds\":12,\"modelKey\":\"gmodel\"}"}"#);
        assert_eq!((queued.status, queued.retry_after, queued.lapsed), (503, Some(12), false));
        assert_eq!(failure(400, r#"{"code":"113","message":"used up"}"#).status, 429);
        assert_eq!(failure(400, r#"{"code":"406","message":"sensitive"}"#).status, 400);
        assert_eq!(failure(400, r#"{"message":"Error in upstream response"}"#).status, 502);
        let lapsed = failure(401, "");
        assert!(lapsed.lapsed && lapsed.status == 401);
        assert_eq!(failure(200, "x").status, 502);
    }

    fn data(body: &Value) -> String {
        format!("data: {}", json!({"statusCodeValue":200,"body":body.to_string()}))
    }

    #[test]
    fn sse_events_lift_text_reasoning_tools_and_usage() {
        let tools = [json!({"type":"function","function":{"name":"read"}})];
        let mut e = Events::new(&tools);
        let mut all = Vec::new();
        for chunk in [
            json!({"choices":[{"delta":{"reasoning_content":"r"}}]}),
            json!({"choices":[{"delta":{"content":"ok <tool_call>{\"name\":\"read\",\"arguments\":{}}</tool_call>"}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"native-1","function":{"arguments":"{\"a\""}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"read","arguments":":1}"}}]}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
            json!({"usage":{"prompt_tokens":5,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":3}}}),
        ] {
            all.extend(e.line(&data(&chunk)));
        }
        all.extend(e.line(&format!("data: {}", json!({"statusCodeValue":200,"body":"[DONE]"}))));
        assert!(matches!(&all[0], Event::Reasoning(r) if r == "r"));
        assert!(matches!(&all[1], Event::Text(t) if t == "ok "));
        assert!(matches!(&all[2], Event::Tool { index: 0, name, .. } if name == "read"));
        assert!(matches!(&all[3], Event::Tool { index: 1, id, args, .. } if id == "native-1" && args == "{\"a\":1}"));
        match all.last().unwrap() {
            Event::Stop { reason, usage } => {
                assert_eq!(*reason, "tool_calls");
                assert_eq!(usage.as_ref().unwrap()["total_tokens"], 7);
                assert_eq!(metered(usage.as_ref().unwrap())["cache_read_input_tokens"], 3);
            }
            other => panic!("{other:?}"),
        }
        let mut e = Events::new(&[]);
        let out = e.line(&format!("data: {}", json!({"statusCodeValue":401,"body":"{\"message\":\"expired\"}"})));
        assert!(matches!(&out[0], Event::Error(f) if f.lapsed));
        let mut e = Events::new(&[]);
        e.line(&data(&json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"x","function":{"arguments":"{}"}}]}}]})));
        assert!(matches!(&e.finish()[0], Event::Error(f) if f.nameless));
    }
}
