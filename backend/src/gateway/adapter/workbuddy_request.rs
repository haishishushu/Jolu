//! WorkBuddy Chat 请求兼容：移植 magpie 插件的 FLAGGED / unflag / systemOn。
//! 来源：magpie-community/plugins@9bba5fed，packages/workbuddy/index.mjs（MIT）。
//! 只改发往此来源的副本；不改本地会话、工具参数、图片或其他供应商请求。
use serde_json::{json, Value};

/// ASCII 不区分大小写匹配；to_ascii_lowercase 保持 UTF-8 字节位置不变。
fn replace(text: &str, from: &str, to: &str, skip_the: bool) -> String {
    let lower = text.to_ascii_lowercase();
    let needle = from.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut end = 0;
    for (at, _) in lower.match_indices(&needle) {
        if skip_the && lower[..at].ends_with("the ") {
            continue;
        }
        out.push_str(&text[end..at]);
        out.push_str(to);
        end = at + from.len();
    }
    out.push_str(&text[end..]);
    out
}

fn unflagged(text: &str) -> String {
    let mut text = text.to_string();
    // 计费标记只用于 Anthropic 客户端，不属于用户任务。
    while let Some(at) = text.to_ascii_lowercase().find("x-anthropic-billing-header:") {
        let end = text[at..].find('\n').map_or(text.len(), |n| at + n + 1);
        text.replace_range(at..end, "");
    }
    for (from, to) in [
        ("You are Claude Code, Anthropic's official CLI for Claude", "You are Claude Code, Anthropic's CLI for Claude"),
        ("You are Claude Code, Anthropic’s official CLI for Claude", "You are Claude Code, Anthropic's CLI for Claude"),
        ("Main branch (you will usually use this for PRs)", "Main branch (usually the base for PRs)"),
    ] {
        text = replace(&text, from, to, false);
    }
    text = replace(&text, "Codex CLI is an open source project led by OpenAI.", "The Codex CLI is an open source project led by OpenAI.", true);
    // 上游正则中的可选句点先匹配长形式，避免添加双句点。
    for from in ["You are an AI agent powered by DeepSeek Harness.", "You are an AI agent powered by DeepSeek Harness"] {
        text = replace(&text, from, "You are an AI agent, powered by DeepSeek Harness.", false);
    }
    let prefix = "Web GUI whose URL the runtime context names.";
    let lower = text.to_ascii_lowercase();
    let mut edits = Vec::new();
    for (at, _) in lower.match_indices(&prefix.to_ascii_lowercase()) {
        let rest = &text[at + prefix.len()..];
        let trimmed = rest.trim_start_matches(char::is_whitespace);
        if trimmed.len() < rest.len() && trimmed.to_ascii_lowercase().starts_with("when the user refers") {
            edits.push((at, at + prefix.len() + rest.len() - trimmed.len() + "when the user refers".len()));
        }
    }
    for (start, end) in edits.into_iter().rev() {
        text.replace_range(start..end, "Web GUI whose URL the runtime context names. The interface, when the user refers");
    }
    text
}

pub(super) fn normalize(body: &mut Value) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut).filter(|v| !v.is_empty()) else { return };
    messages.retain_mut(|message| {
        let system = message["role"] == "system";
        match message.get_mut("content") {
            Some(Value::String(content)) => {
                let next = unflagged(content);
                if next != *content {
                    *content = next;
                    if system && content.trim().is_empty() { return false; }
                }
            }
            Some(Value::Array(parts)) => {
                let mut changed = false;
                parts.retain_mut(|part| {
                    let Some(Value::String(text)) = part.get_mut("text") else { return true };
                    let next = unflagged(text);
                    if next == *text { return true; }
                    *text = next;
                    changed = true;
                    !text.trim().is_empty()
                });
                if changed && system && parts.is_empty() { return false; }
            }
            _ => {}
        }
        true
    });
    for message in messages.iter_mut() {
        if message["role"] == "developer" { message["role"] = json!("system"); }
        if message["role"] != "assistant" { continue; }
        let text = message["reasoning_content"].as_str().filter(|s| !s.is_empty()).map(str::to_owned)
            .unwrap_or_else(|| message["reasoning_details"].as_array().into_iter().flatten()
                .filter_map(|d| d["text"].as_str().or_else(|| d["summary"].as_str())).collect());
        if let Some(obj) = message.as_object_mut() {
            if !text.is_empty() && obj.get("reasoning").and_then(Value::as_str).is_none_or(str::is_empty) {
                obj.insert("reasoning".into(), json!(text));
            }
            obj.remove("reasoning_content");
            obj.remove("reasoning_details");
        }
    }
    if messages.first().is_none_or(|m| m["role"] != "system") {
        messages.insert(0, json!({"role":"system", "content":"You are a helpful assistant."}));
    }
    if let Some(obj) = body.as_object_mut() {
        if let Some(limit) = obj.remove("max_completion_tokens") {
            obj.entry("max_tokens").or_insert(limit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_lines_are_reworded_without_erasing_identity_or_surrounding_text() {
        for (from, to) in [
            ("YOU ARE CLAUDE CODE, ANTHROPIC’S OFFICIAL CLI FOR CLAUDE.", "You are Claude Code, Anthropic's CLI for Claude."),
            ("Codex CLI is an open source project led by OpenAI.", "The Codex CLI is an open source project led by OpenAI."),
            ("You are an AI agent powered by DeepSeek Harness.", "You are an AI agent, powered by DeepSeek Harness."),
            ("Web GUI whose URL the runtime context names.\n  When the user refers to it", "Web GUI whose URL the runtime context names. The interface, when the user refers to it"),
        ] {
            let want = format!("中文🙂 {to}\n保留后文");
            let actual = unflagged(&format!("中文🙂 {from}\n保留后文"));
            assert_eq!(actual, want);
            assert_eq!(unflagged(&actual), actual);
        }
        assert_eq!(unflagged("Claude Code / Codex / DeepSeek Harness"), "Claude Code / Codex / DeepSeek Harness");
    }

    #[test]
    fn content_blocks_preserve_images_metadata_and_tool_arguments() {
        let mut body = json!({"messages":[
            {"role":"system", "content":[{"type":"text","text":"x-anthropic-billing-header: test"}]},
            {"role":"user", "content":[{"type":"text","text":"Main branch (you will usually use this for PRs): main","extra":7},{"type":"image_url","image_url":{"url":"data:image/png;base64,abc"}}]},
            {"role":"assistant","content":"You are Claude Code, Anthropic's official CLI for Claude.","tool_calls":[{"id":"call_1","function":{"name":"read","arguments":"You are Claude Code, Anthropic's official CLI for Claude."}}]}
        ],"stream":true,"tools":[{"function":{"name":"read"}}]});
        let original = body.clone();
        normalize(&mut body);
        assert_eq!(body["messages"][0], json!({"role":"system","content":"You are a helpful assistant."}));
        assert_eq!(body["messages"][1]["content"][0]["text"], "Main branch (usually the base for PRs): main");
        assert_eq!(body["messages"][1]["content"][0]["extra"], 7);
        assert_eq!(body["messages"][1]["content"][1], original["messages"][1]["content"][1]);
        assert_eq!(body["messages"][2]["tool_calls"], original["messages"][2]["tool_calls"]);
        assert_eq!(body["tools"], original["tools"]);
        let once = body.clone();
        normalize(&mut body);
        assert_eq!(body, once);
    }

    #[test]
    fn system_and_reasoning_fields_follow_workbuddy_contract() {
        let mut body = json!({"max_completion_tokens":64,"max_tokens":32,"messages":[
            {"role":"developer","content":"保留约束"},
            {"role":"assistant","content":"ok","reasoning_content":"思考"},
            {"role":"assistant","content":"ok","reasoning":"原有","reasoning_details":[{"text":"另一个"}]}
        ]});
        normalize(&mut body);
        assert_eq!(body["max_tokens"], 32);
        assert!(body.get("max_completion_tokens").is_none());
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["reasoning"], "思考");
        assert_eq!(body["messages"][2]["reasoning"], "原有");
        assert!(body["messages"][1].get("reasoning_content").is_none());
    }
}
