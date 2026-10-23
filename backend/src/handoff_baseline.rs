//! 会话交接 · 原工具的压缩摘要（D-03、计划 3.3 第 9 条）：只作备用。
//!
//! 默认读原文；只有压缩前的原文已经读不到时，才把原工具摘要当作早期部分的纪要来源，
//! 并注明「来自原工具压缩摘要，未经 Jolu 复核」。
//! - Claude Code：取最后一份 `isCompactSummary: true` 的 user 记录；
//! - Codex：只认本地压缩（`compacted.payload.message` 非空）；远程压缩的加密记录（只有 `encrypted_content`）跳过。

use serde_json::Value as Json;

use crate::session_threads::parse_detail;

#[derive(Debug, Clone, PartialEq)]
pub struct NativeBaseline {
    /// `claude-native` / `codex-native`
    pub source: &'static str,
    pub summary: String,
    /// 摘要之后第一条消息的序号（= 摘要之前可解析的消息条数）
    pub boundary_idx: usize,
}

impl NativeBaseline {
    /// 摘要之前一条原文都没有：压缩前的原文已经读不到，才需要用原工具摘要
    pub fn raw_missing(&self) -> bool {
        self.boundary_idx == 0
    }
}

fn text_of(content: &Json) -> String {
    match content {
        Json::String(s) => s.trim().to_string(),
        Json::Array(blocks) => blocks.iter().filter_map(|b| b.get("text").and_then(Json::as_str)).collect::<Vec<_>>().join("\n").trim().to_string(),
        _ => String::new(),
    }
}

fn lines_of(jsonl: &str) -> Vec<String> {
    jsonl.lines().map(String::from).collect()
}

pub fn claude_baseline(jsonl: &str) -> Option<NativeBaseline> {
    let lines = lines_of(jsonl);
    let (at, summary) = lines.iter().enumerate().rev().find_map(|(i, line)| {
        let v: Json = serde_json::from_str(line).ok()?;
        if v.get("isCompactSummary").and_then(Json::as_bool) != Some(true) {
            return None;
        }
        let text = text_of(v.get("message")?.get("content")?);
        (!text.is_empty()).then_some((i, text))
    })?;
    Some(NativeBaseline { source: "claude-native", summary, boundary_idx: parse_detail("claude", &lines[..at]).len() })
}

pub fn codex_baseline(jsonl: &str) -> Option<NativeBaseline> {
    let lines = lines_of(jsonl);
    let (at, summary) = lines.iter().enumerate().rev().find_map(|(i, line)| {
        let v: Json = serde_json::from_str(line).ok()?;
        if v.get("type").and_then(Json::as_str) != Some("compacted") {
            return None;
        }
        let text = v.get("payload")?.get("message")?.as_str()?.trim().to_string();
        (!text.is_empty()).then_some((i, text))
    })?;
    Some(NativeBaseline { source: "codex-native", summary, boundary_idx: parse_detail("codex", &lines[..at]).len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: &str = include_str!("../tests/fixtures/handoff/claude-compacted.jsonl");
    const CLAUDE_ONLY_SUMMARY: &str = include_str!("../tests/fixtures/handoff/claude-summary-only.jsonl");
    const CODEX_ENCRYPTED: &str = include_str!("../tests/fixtures/handoff/codex-encrypted.jsonl");
    const CODEX_LOCAL: &str = include_str!("../tests/fixtures/handoff/codex-local-compact.jsonl");

    #[test]
    fn claude_baseline_reads_last_summary() {
        let b = claude_baseline(CLAUDE).expect("有压缩摘要");
        assert_eq!(b.source, "claude-native");
        assert_eq!(b.summary, "占位：原工具压缩摘要二");
        // 摘要之前：#0 用户、#1 助手（两次工具调用合并）、#2 用户（重复的 u1 已去重）、#3 助手
        assert_eq!(b.boundary_idx, 4);
        assert!(!b.raw_missing());
        let only = claude_baseline(CLAUDE_ONLY_SUMMARY).expect("有压缩摘要");
        assert_eq!(only.summary, "占位：原文缺失时的原工具摘要");
        assert!(only.raw_missing());
        assert_eq!(claude_baseline("{\"type\":\"user\",\"message\":{\"content\":\"占位\"}}"), None);
    }

    #[test]
    fn codex_baseline_skips_encrypted() {
        assert_eq!(codex_baseline(CODEX_ENCRYPTED), None);
        let b = codex_baseline(CODEX_LOCAL).expect("有本地压缩摘要");
        assert_eq!(b.source, "codex-native");
        assert_eq!(b.summary, "占位：本地压缩摘要二");
        assert_eq!(b.boundary_idx, 2);
    }
}
