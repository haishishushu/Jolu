//! 会话交接 · 段落纪要（计划 3.3 第 3 条）：每份纪要只从精简后的原文生成一次，绑定消息范围与校验值；
//! 校验通过就一直复用；某条消息被改写只重做覆盖它的那一份；原文被截断时丢弃超出范围的纪要；
//! 纪要总量超过上限时把最早的若干份归并成一份「早期纪要」（level 2），原文不动。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::handoff_input::CondensedMsg;

/// 段落纪要
pub const LEVEL_SEGMENT: i64 = 1;
/// 早期纪要（归并而成）
pub const LEVEL_ROLLUP: i64 = 2;
/// 一份早期纪要的输出估计（约 2,500 字），决定一次要归并多少份
const ROLLUP_OUTPUT_TOKENS: usize = 1_000;
const RELEVANT_LIMIT: usize = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DigestRow {
    /// 落库后的 id；生成流程里尚未落库的新纪要用负数临时 id
    pub id: i64,
    pub platform: String,
    pub session_id: String,
    pub level: i64,
    /// 闭区间消息序号
    pub from_idx: usize,
    pub to_idx: usize,
    pub range_hash: String,
    pub digest_md: String,
    pub tokens: usize,
    /// `raw` / `claude-native` / `codex-native`
    pub source: String,
    pub rolled_into: Option<i64>,
    pub prompt_version: String,
    pub created_ms: i64,
}

/// 闭区间 `[from, to]` 的校验值：按原消息指纹计算，旧输出是否被遮蔽不影响；越界时为空串
pub fn range_hash(msgs: &[CondensedMsg], from: usize, to: usize) -> String {
    if from > to || to >= msgs.len() {
        return String::new();
    }
    let mut h = Sha256::new();
    for m in &msgs[from..=to] {
        h.update(m.idx.to_le_bytes());
        h.update(m.sig.as_bytes());
    }
    h.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// 可复用的纪要，以及 `[0, upto)` 内需要新建或重做的闭区间范围。
///
/// - 只认当前提示词版本、来自原文（`raw`）、范围仍在会话内且校验值一致的纪要；
/// - 被归并的段落纪要只在它所属的早期纪要失效时才作为候选；
/// - 从 #0 起贪心拼接，同一起点优先覆盖更长的那份；拼不上的空档就是需要处理的范围。
///   起点在 `upto` 之后的纪要不用（那部分直接使用最近原文）
pub fn reusable(stored: &[DigestRow], msgs: &[CondensedMsg], upto: usize) -> (Vec<DigestRow>, Vec<(usize, usize)>) {
    let version = crate::session_handoff::PROMPT_VERSION;
    let valid: Vec<&DigestRow> = stored
        .iter()
        .filter(|r| r.prompt_version == version && r.source == "raw" && r.from_idx <= r.to_idx && r.to_idx < msgs.len())
        .filter(|r| r.range_hash == range_hash(msgs, r.from_idx, r.to_idx))
        .collect();
    let valid_ids: HashSet<i64> = valid.iter().map(|r| r.id).collect();
    let candidates: Vec<&DigestRow> = valid.into_iter().filter(|r| r.rolled_into.is_none_or(|p| !valid_ids.contains(&p))).filter(|r| r.from_idx < upto).collect();

    let mut reused = Vec::new();
    let mut need = Vec::new();
    let mut cursor = 0;
    while cursor < upto {
        let pick = candidates.iter().filter(|r| r.from_idx == cursor).max_by_key(|r| (r.to_idx, r.level));
        if let Some(row) = pick {
            reused.push((*row).clone());
            cursor = row.to_idx + 1;
            continue;
        }
        let next = candidates.iter().map(|r| r.from_idx).filter(|&f| f > cursor).min();
        match next {
            Some(n) if n < upto => {
                need.push((cursor, n - 1));
                cursor = n;
            }
            _ => {
                need.push((cursor, upto - 1));
                break;
            }
        }
    }
    (reused, need)
}

/// 纪要总量超过 `limit` 时，返回要归并的最早若干份（至少 2 份）的 id；不需要或无法归并时为 None
pub fn needs_rollup(active: &[DigestRow], limit: usize) -> Option<Vec<i64>> {
    let total: usize = active.iter().map(|r| r.tokens).sum();
    if total <= limit || active.len() < 2 {
        return None;
    }
    let mut sorted: Vec<&DigestRow> = active.iter().collect();
    sorted.sort_by_key(|r| r.from_idx);
    let excess = total - limit + ROLLUP_OUTPUT_TOKENS;
    let mut removed = 0;
    let mut ids = Vec::new();
    for r in sorted {
        ids.push(r.id);
        removed += r.tokens;
        if ids.len() >= 2 && removed >= excess {
            break;
        }
    }
    Some(ids)
}

fn normalize(text: &str) -> String {
    text.replace('\\', "/").to_lowercase()
}

/// 不构成关键词的虚字；中文按相邻两字切词时，含这些字的组合跳过
const STOP_CHARS: &str = "的了是在和与把被再也就都要请帮我你他她它这那些个吗呢吧啊让给得地着很更";
const STOP_WORDS: [&str; 12] = ["the", "and", "for", "with", "this", "that", "to", "of", "in", "on", "is", "it"];

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c) || ('\u{3400}'..='\u{4dbf}').contains(&c)
}

/// 意见里的路径（带 `/`、`\` 或文件扩展名）与关键词（ASCII 词至少 2 个字符；中文按相邻两字）
fn terms(instruction: &str) -> (Vec<String>, Vec<String>) {
    let mut paths = Vec::new();
    let mut words = Vec::new();
    let separators = |c: char| c.is_whitespace() || "，。、；：？！「」『』（）《》“”‘’…—,;:?!()[]{}<>\"'`*|=".contains(c);
    for chunk in instruction.split(separators).filter(|s| !s.is_empty()) {
        let ascii: String = chunk.chars().filter(|c| !is_cjk(*c)).collect();
        let pathish = ascii.contains('/') || ascii.contains('\\') || ascii.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()));
        if pathish && ascii.chars().count() >= 3 {
            let p = normalize(ascii.trim_matches('.'));
            if !paths.contains(&p) {
                paths.push(p);
            }
            continue;
        }
        // 中英混排：拆成连续的中文段和 ASCII 段
        let mut runs: Vec<(bool, String)> = Vec::new();
        for c in chunk.chars() {
            let cjk = is_cjk(c);
            match runs.last_mut() {
                Some((kind, s)) if *kind == cjk => s.push(c),
                _ => runs.push((cjk, c.to_string())),
            }
        }
        for (cjk, run) in runs {
            if cjk {
                let chars: Vec<char> = run.chars().collect();
                for pair in chars.windows(2) {
                    if pair.iter().any(|c| STOP_CHARS.contains(*c)) {
                        continue;
                    }
                    let w: String = pair.iter().collect();
                    if !words.contains(&w) {
                        words.push(w);
                    }
                }
            } else {
                let w = run.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_').to_lowercase();
                if w.chars().count() >= 2 && !STOP_WORDS.contains(&w.as_str()) && !w.chars().all(|c| c.is_ascii_digit()) && !words.contains(&w) {
                    words.push(w);
                }
            }
        }
    }
    (paths, words)
}

/// 与修改意见相关的纪要：意见里出现的路径与关键词出现在哪份纪要正文里就带哪份；
/// 路径命中优先，其次按命中的关键词数，同分取较新的；最多 3 份，都不命中时为空
pub fn relevant_digests(instruction: &str, digests: &[DigestRow]) -> Vec<i64> {
    let (paths, words) = terms(instruction);
    if paths.is_empty() && words.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(usize, usize, i64)> = digests
        .iter()
        .filter_map(|d| {
            // 每份纪要都有相同的小节标题，只匹配正文行
            let body = normalize(&d.digest_md.lines().filter(|l| !l.trim_start().starts_with('#')).collect::<Vec<_>>().join("\n"));
            let path_hits = paths.iter().filter(|p| body.contains(p.as_str())).count();
            let word_hits = words.iter().filter(|w| body.contains(w.as_str())).count();
            let score = path_hits * 100 + word_hits;
            (score > 0).then_some((score, d.to_idx, d.id))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    scored.into_iter().take(RELEVANT_LIMIT).map(|(_, _, id)| id).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::handoff_input::condense;
    use crate::session_handoff::PROMPT_VERSION;
    use crate::session_threads::Message;

    pub(crate) fn row(id: i64, from: usize, to: usize, msgs: &[CondensedMsg]) -> DigestRow {
        DigestRow {
            id,
            platform: "claude".into(),
            session_id: "s".into(),
            level: LEVEL_SEGMENT,
            from_idx: from,
            to_idx: to,
            range_hash: range_hash(msgs, from, to),
            digest_md: format!("纪要 {from}-{to}"),
            tokens: 100,
            source: "raw".into(),
            rolled_into: None,
            prompt_version: PROMPT_VERSION.into(),
            created_ms: 0,
        }
    }

    fn msgs(n: usize) -> Vec<Message> {
        (0..n).map(|i| Message { role: "user".into(), ts: None, text: format!("消息 {i}"), tools: vec![] }).collect()
    }

    #[test]
    fn range_hash_tracks_content_and_bounds() {
        let a = condense(&msgs(5));
        let mut changed = msgs(5);
        changed[2].text = "改写".into();
        let b = condense(&changed);
        assert_eq!(range_hash(&a, 0, 1), range_hash(&b, 0, 1));
        assert_ne!(range_hash(&a, 0, 2), range_hash(&b, 0, 2));
        assert_eq!(range_hash(&a, 3, 5), "");
    }

    #[test]
    fn changed_message_redoes_one_digest() {
        let original = condense(&msgs(40));
        let stored = vec![row(1, 0, 9, &original), row(2, 10, 19, &original), row(3, 20, 29, &original)];
        let mut edited = msgs(40);
        edited[15].text = "被改写的消息".into();
        let now = condense(&edited);
        let (reused, need) = reusable(&stored, &now, 30);
        assert_eq!(reused.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1, 3]);
        assert_eq!(need, vec![(10, 19)]);
    }

    #[test]
    fn truncation_drops_out_of_range_digests() {
        let original = condense(&msgs(40));
        let stored = vec![row(1, 0, 15, &original), row(2, 16, 29, &original)];
        let now = condense(&msgs(20));
        let (reused, need) = reusable(&stored, &now, 18);
        assert_eq!(reused.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1]);
        assert_eq!(need, vec![(16, 17)]);
    }

    #[test]
    fn reusable_prefers_valid_rollup_and_falls_back_to_segments() {
        let c = condense(&msgs(30));
        let mut a = row(1, 0, 9, &c);
        let mut b = row(2, 10, 19, &c);
        let mut early = row(3, 0, 19, &c);
        early.level = LEVEL_ROLLUP;
        a.rolled_into = Some(3);
        b.rolled_into = Some(3);
        let (reused, need) = reusable(&[a.clone(), b.clone(), early.clone()], &c, 25);
        assert_eq!(reused.iter().map(|r| r.id).collect::<Vec<_>>(), vec![3]);
        assert_eq!(need, vec![(20, 24)]);
        // 早期纪要失效（版本不同）时，退回它归并的段落纪要
        early.prompt_version = "handoff-v0".into();
        let (reused, _) = reusable(&[a, b, early], &c, 25);
        assert_eq!(reused.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1, 2]);
        // 起点在最近窗口里的纪要不用
        let late = row(4, 22, 24, &c);
        let (reused, need) = reusable(&[late], &c, 20);
        assert!(reused.is_empty());
        assert_eq!(need, vec![(0, 19)]);
    }

    #[test]
    fn needs_rollup_picks_earliest() {
        let c = condense(&msgs(40));
        let mut rows: Vec<DigestRow> = (0..4).map(|i| row(i + 1, i as usize * 10, i as usize * 10 + 9, &c)).collect();
        for r in &mut rows {
            r.tokens = 25_000;
        }
        // 100k 超出 60k：最早的两份（50k）够抵消 41k
        assert_eq!(needs_rollup(&rows, 60_000), Some(vec![1, 2]));
        assert_eq!(needs_rollup(&rows[..2], 60_000), None);
        assert_eq!(needs_rollup(&rows[..1], 1_000), None);
    }

    #[test]
    fn relevant_digests_by_path() {
        let c = condense(&msgs(40));
        let mut a = row(1, 0, 9, &c);
        a.digest_md = "### 完成的改动\n- 修改 `src/a.ts` 的列表渲染（#3）".into();
        let mut b = row(2, 10, 19, &c);
        b.digest_md = "### 完成的改动\n- 调整 `src/b.ts` 的接口（#12）".into();
        assert_eq!(relevant_digests("请补上 src/a.ts 改动的原因", &[a.clone(), b.clone()]), vec![1]);
        assert_eq!(relevant_digests(r"src\b.ts 那段", &[a.clone(), b.clone()]), vec![2]);
        assert_eq!(relevant_digests("接口", &[a.clone(), b.clone()]), vec![2]);
        assert!(relevant_digests("天气", &[a, b]).is_empty());
    }

    #[test]
    fn relevant_digests_at_most_three() {
        let c = condense(&msgs(60));
        let rows: Vec<DigestRow> = (0..5)
            .map(|i| {
                let mut r = row(i + 1, i as usize * 10, i as usize * 10 + 9, &c);
                r.digest_md = "测试失败：`pnpm test` 报错".into();
                r
            })
            .collect();
        let picked = relevant_digests("补上测试失败的原因", &rows);
        assert_eq!(picked, vec![5, 4, 3]);
    }
}
