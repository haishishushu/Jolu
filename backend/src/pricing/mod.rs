//! 费用估算。
//!
//! 本地日志没有账单金额；这里只对已核验的精确模型 ID 估算。价格均为标准、
//! 非 Batch、每百万文本 Token 的美元价。未知型号、非日期后缀变体和无法判断
//! 缓存写入 TTL 的 Claude 请求返回 `None`，由界面显示“—”。
//!
//! 结构：计价逻辑在本文件（稳定），价目数据在 [`table`]（脚本生成，会膨胀）。
//! 拆分的判断标准是**对外 API 不变**——所有调用方无需改动。
//!
//! 核验日期：2026-11-03。官方来源见 [`table`] 的文件头。

mod table;

use table::PRICES;

#[derive(Clone, Copy)]
pub(super) struct Price {
    pub(super) platform: &'static str,
    pub(super) model: &'static str,
    pub(super) input: f64,
    pub(super) cached_input: f64,
    pub(super) output: f64,
    /// `None` 表示来源日志不足以确定该类别的价格。
    pub(super) cache_write: Option<f64>,
    /// 官方定价页 URL，供界面标注与人工复核。
    pub(super) source: &'static str,
    /// 长上下文阶梯：输入超过阈值时**整条请求**按加价倍率计（不是只算超出部分）。
    pub(super) long_context: Option<LongContext>,
    /// 峰谷定价：谷时按倍率折价。仅 DeepSeek 等少数厂商有。
    pub(super) off_peak: Option<OffPeak>,
}

/// 长上下文加价：超过 `threshold` 个输入 token 时，整条请求的单价乘以对应倍率。
///
/// 官方规则（GPT-6.1 Sol）：「>272K input tokens 时 input/cache ×2、output ×1.5，
/// **for the full request**」——注意是整条请求，不是分段计价。
#[derive(Clone, Copy)]
pub(super) struct LongContext {
    pub(super) threshold: i64,
    pub(super) input_multiplier: f64,
    pub(super) output_multiplier: f64,
    pub(super) cached_multiplier: f64,
}

/// 峰谷定价：在指定时段按倍率折价。时段用 UTC 小时区间 + 星期表示。
#[derive(Clone, Copy)]
pub(super) struct OffPeak {
    /// 谷时倍率（如 0.5 表示半价）。
    pub(super) multiplier: f64,
    /// 谷时窗口
    pub(super) windows: &'static [OffPeakWindow],
}

/// 一个谷时窗口。
#[derive(Clone, Copy)]
pub(super) struct OffPeakWindow {
    /// UTC 起始小时（含）
    pub(super) start_hour: u8,
    /// UTC 结束小时（不含）；结束小于起始表示跨夜。`full_day` 为 true 时忽略。
    pub(super) end_hour: u8,
    /// 生效星期（1 = 周一 …… 7 = 周日）
    pub(super) weekdays: &'static [u8],
    /// 全天生效。用显式标记表达，避免「start=0,end=0」被读成空区间。
    pub(super) full_day: bool,
}

/// Gemini 长提示档：单次输入超过阈值时整条请求按此价计（Pro 系列）。
#[derive(Clone, Copy)]
struct LongTier { model: &'static str, input: f64, cached_input: f64, output: f64 }
pub const GEMINI_LONG_PROMPT_TOKENS: i64 = 200_000;
const GEMINI_LONG: &[LongTier] = &[
    LongTier { model: "gemini-2.5-pro", input: 2.5, cached_input: 0.25, output: 15.0 },
    LongTier { model: "gemini-3.1-pro-preview", input: 4.0, cached_input: 0.4, output: 18.0 },
];

/// 日期快照后缀：`-2025-09-29`（11 字符）或 `-20250929`（9 字符）。
/// 两种格式在不同厂商的日志里都出现过，都必须能匹配到基础模型名。
fn is_date_snapshot_suffix(value: &str) -> bool {
    let bytes = value.as_bytes();
    match bytes.len() {
        11 => {
            bytes[0] == b'-'
                && bytes[1..].iter().enumerate().all(|(index, byte)| {
                    if matches!(index, 4 | 7) { *byte == b'-' } else { byte.is_ascii_digit() }
                })
        }
        9 => bytes[0] == b'-' && bytes[1..].iter().all(u8::is_ascii_digit),
        _ => false,
    }
}

/// 按平台 + 模型名查价目。模型名比对大小写不敏感，允许日期快照后缀（`-2025-09-29`）。
fn price_of(platform: Option<&str>, model: &str) -> Option<Price> {
    let model = model.to_ascii_lowercase();
    PRICES.iter().copied().find(|price| {
        platform.is_none_or(|value| value == price.platform)
            && (model == price.model
                || model.strip_prefix(price.model).is_some_and(is_date_snapshot_suffix))
    })
}

/// 一条或一组请求的用量。`reasoning` / `tool` 只对 Gemini 参与计价（思考按输出价、工具提示按输入价）；
/// `long_prompt` 表示单次输入超过 [`GEMINI_LONG_PROMPT_TOKENS`]，聚合调用方无法从总和判断时传 false。
#[derive(Clone, Copy, Default)]
pub struct Usage {
    pub input: i64,
    pub output: i64,
    pub cache_write: i64,
    pub cache_read: i64,
    pub reasoning: i64,
    pub tool: i64,
    pub long_prompt: bool,
}

pub fn estimate_usage(platform: Option<&str>, model: Option<&str>, usage: Usage) -> Option<f64> {
    estimate_usage_at(platform, model, usage, chrono::Utc::now())
}

/// 带时间戳的估算：峰谷定价需要知道请求发生的时刻。
/// 聚合调用方（按天/周汇总）应传各请求的实际时间；无法确定时传 `Utc::now()` 按峰价处理。
pub fn estimate_usage_at(
    platform: Option<&str>,
    model: Option<&str>,
    usage: Usage,
    at: chrono::DateTime<chrono::Utc>,
) -> Option<f64> {
    use chrono::{Datelike, Timelike};
    let price = price_of(platform, model?)?;
    let Usage { input, output, cache_write, cache_read, reasoning, tool, long_prompt } = usage;
    let write_price = if cache_write == 0 { 0.0 } else { price.cache_write? };

    // 峰谷折价：命中谷时窗口时整条请求按倍率折算
    let off_peak = price
        .off_peak
        .filter(|off| off.windows.iter().any(|window| in_off_peak(window, at.hour() as u8, at.weekday().number_from_monday() as u8)))
        .map_or(1.0, |off| off.multiplier);

    // 长上下文加价：超过阈值时整条请求按倍率（同样是整条，不是分段）
    let (input_mult, cached_mult, output_mult) = match price.long_context {
        Some(long) if input > long.threshold => (long.input_multiplier, long.cached_multiplier, long.output_multiplier),
        _ => (1.0, 1.0, 1.0),
    };

    if price.platform == "gemini" {
        // 官方口径：prompt 计数含缓存命中部分，命中部分按缓存价；思考 Token 按输出价；工具调用提示按输入价。
        let (input_price, cached_price, output_price) = match GEMINI_LONG.iter().find(|tier| tier.model == price.model) {
            Some(tier) if long_prompt => (tier.input, tier.cached_input, tier.output),
            _ => (price.input, price.cached_input, price.output),
        };
        let ordinary_input = input.saturating_sub(cache_read).max(0) + tool.max(0);
        return Some((ordinary_input as f64 * input_price * input_mult
            + cache_read as f64 * cached_price * cached_mult
            + (output + reasoning.max(0)) as f64 * output_price * output_mult) / 1_000_000.0 * off_peak);
    }
    let ordinary_input = if price.platform == "codex" {
        input.saturating_sub(cache_read).saturating_sub(cache_write)
    } else {
        input
    };
    Some((ordinary_input as f64 * price.input * input_mult
        + output as f64 * price.output * output_mult
        + cache_write as f64 * write_price * input_mult
        + cache_read as f64 * price.cached_input * cached_mult) / 1_000_000.0 * off_peak)
}

fn in_off_peak(window: &OffPeakWindow, hour: u8, weekday: u8) -> bool {
    if !window.weekdays.contains(&weekday) {
        return false;
    }
    if window.full_day {
        return true;
    }
    if window.start_hour <= window.end_hour {
        hour >= window.start_hour && hour < window.end_hour
    } else {
        // 跨夜窗口（如 22:00–06:00）
        hour >= window.start_hour || hour < window.end_hour
    }
}

/// 非 Gemini 的简化计价：不含思考/工具 Token 与长提示档；需要这些信息的调用方使用 [`estimate_usage`]。
pub fn estimate(
    platform: Option<&str>,
    model: Option<&str>,
    input: i64,
    output: i64,
    cache_write: i64,
    cache_read: i64,
) -> Option<f64> {
    estimate_usage(platform, model, Usage { input, output, cache_write, cache_read, ..Default::default() })
}

/// 只查询是否有价目（不含计算）。界面用它区分「模型未收录」与其他未计价原因。
pub fn has_price(platform: Option<&str>, model: &str) -> bool {
    price_of(platform, model).is_some()
}

/// 某次请求命中的单价快照，供界面展示费用拆解。
/// 返回的是**生效单价**（已含长上下文加价与谷时折价），与 `cost` 口径一致。
#[derive(Debug, Clone, Copy)]
pub struct AppliedPrice {
    pub input: f64,
    pub cached_input: f64,
    pub output: f64,
    pub cache_write: Option<f64>,
    pub source: &'static str,
    pub long_context: bool,
    pub off_peak: bool,
}

/// 按请求的时间与用量算出生效单价。`input` 用于判定长上下文阈值。
pub fn applied_price(
    platform: Option<&str>,
    model: &str,
    input: i64,
    at: chrono::DateTime<chrono::Utc>,
) -> Option<AppliedPrice> {
    use chrono::{Datelike, Timelike};
    let price = price_of(platform, model)?;
    let long = price.long_context.filter(|lc| input > lc.threshold);
    let (input_mult, cached_mult, output_mult) = match long {
        Some(lc) => (lc.input_multiplier, lc.cached_multiplier, lc.output_multiplier),
        None => (1.0, 1.0, 1.0),
    };
    let off_peak_hit = price.off_peak.is_some_and(|off| {
        off.windows.iter().any(|w| in_off_peak(w, at.hour() as u8, at.weekday().number_from_monday() as u8))
    });
    let off_mult = price.off_peak.filter(|_| off_peak_hit).map_or(1.0, |off| off.multiplier);
    Some(AppliedPrice {
        input: price.input * input_mult * off_mult,
        cached_input: price.cached_input * cached_mult * off_mult,
        output: price.output * output_mult * off_mult,
        cache_write: price.cache_write.map(|w| w * input_mult * off_mult),
        source: price.source,
        long_context: long.is_some(),
        off_peak: off_peak_hit,
    })
}


#[cfg(test)]
mod tests {
    use super::*;

    fn at_utc(y: i32, m: u32, d: u32, h: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn new_models_are_priced() {
        for (platform, model) in [
            ("claude", "claude-opus-5-5"),
            ("claude", "claude-sonnet-5-5"),
            ("claude", "claude-haiku-5-5"),
            ("codex", "gpt-6.1-sol"),
            ("codex", "gpt-6-sol"),
            ("zcode", "glm-5.3"),
            ("zcode", "deepseek-v4-flash"),
            ("zcode", "kimi-k3"),
        ] {
            assert!(has_price(Some(platform), model), "{platform}/{model} 应有价目");
            let cost = estimate_usage(
                Some(platform),
                Some(model),
                Usage { input: 1_000_000, output: 0, ..Default::default() },
            );
            assert!(cost.is_some_and(|c| c > 0.0), "{platform}/{model} 应能算出金额");
        }
    }

    #[test]
    fn date_snapshot_suffix_still_matches() {
        // claude-sonnet-4-5-20250929 → 命中 claude-sonnet-4-5
        assert!(has_price(Some("claude"), "claude-sonnet-4-5-20250929"));
        assert!(has_price(Some("claude"), "claude-opus-4-6-20260205"));
        // 非日期后缀不匹配
        assert!(!has_price(Some("claude"), "claude-sonnet-4-5-custom"));
    }

    #[test]
    fn long_context_multiplies_whole_request() {
        // GPT-6.1 Sol：<=272K 标准价；>272K 整条请求 input×2、output×1.5
        let short = estimate_usage_at(
            Some("codex"), Some("gpt-6.1-sol"),
            Usage { input: 272_000, output: 0, ..Default::default() },
            at_utc(2026, 11, 3, 12),
        ).unwrap();
        let long = estimate_usage_at(
            Some("codex"), Some("gpt-6.1-sol"),
            Usage { input: 272_001, output: 0, ..Default::default() },
            at_utc(2026, 11, 3, 12),
        ).unwrap();
        // 输入只多 1 token，但单价翻倍 → 金额接近 2 倍
        assert!(long > short * 1.99, "越界后整条请求应按 2 倍价计：{short} vs {long}");

        // 输出倍率 1.5
        let out_short = estimate_usage_at(
            Some("codex"), Some("gpt-6.1-sol"),
            Usage { input: 1_000, output: 1_000_000, ..Default::default() },
            at_utc(2026, 11, 3, 12),
        ).unwrap();
        let out_long = estimate_usage_at(
            Some("codex"), Some("gpt-6.1-sol"),
            Usage { input: 300_000, output: 1_000_000, ..Default::default() },
            at_utc(2026, 11, 3, 12),
        ).unwrap();
        assert!(out_long > out_short, "长上下文输出价应更高");
    }

    #[test]
    fn glm_long_context_threshold_is_32k() {
        let short = estimate_usage_at(
            Some("zcode"), Some("glm-5.3"),
            Usage { input: 32_000, output: 0, ..Default::default() },
            at_utc(2026, 11, 3, 12),
        ).unwrap();
        let long = estimate_usage_at(
            Some("zcode"), Some("glm-5.3"),
            Usage { input: 32_001, output: 0, ..Default::default() },
            at_utc(2026, 11, 3, 12),
        ).unwrap();
        assert!((long / short - 1.5).abs() < 0.01, "GLM 越界后应 ×1.5：{short} vs {long}");
    }

    #[test]
    fn deepseek_off_peak_is_half_price_on_weekend() {
        // 2026-11-07 是周六 → 全天谷时半价
        let weekend = estimate_usage_at(
            Some("zcode"), Some("deepseek-v4-flash"),
            Usage { input: 1_000_000, output: 0, ..Default::default() },
            at_utc(2026, 11, 7, 12),
        ).unwrap();
        // 2026-11-04 是周三 12:00 UTC → 峰时（10:00-24:00 是谷时！需选 08:00）
        let weekday_peak = estimate_usage_at(
            Some("zcode"), Some("deepseek-v4-flash"),
            Usage { input: 1_000_000, output: 0, ..Default::default() },
            at_utc(2026, 11, 4, 8),
        ).unwrap();
        assert!((weekday_peak / weekend - 2.0).abs() < 0.01, "谷时应为半价：峰 {weekday_peak} 谷 {weekend}");

        // 周三 12:00 UTC 落在 10:00-24:00 → 谷时
        let weekday_off = estimate_usage_at(
            Some("zcode"), Some("deepseek-v4-flash"),
            Usage { input: 1_000_000, output: 0, ..Default::default() },
            at_utc(2026, 11, 4, 12),
        ).unwrap();
        assert!((weekday_off - weekend).abs() < 1e-9, "工作日 10-24 点也应是谷时");
    }

    #[test]
    fn unknown_model_returns_none_not_zero() {
        // 未收录的模型必须返回 None，不能算成 0 元（否则会被误读为免费）
        assert_eq!(estimate_usage(Some("zcode"), Some("unknown-model-xyz"), Usage { input: 1000, output: 100, ..Default::default() }), None);
        assert!(!has_price(Some("zcode"), "unknown-model-xyz"));
        assert!(!has_price(Some("zcode"), "unknown-model-xyz"));
    }

    #[test]
    fn applied_price_carries_official_source() {
        let at = at_utc(2026, 11, 3, 12);
        assert!(applied_price(Some("claude"), "claude-opus-5-5", 1000, at).unwrap().source.contains("platform.claude.com"));
        assert!(applied_price(Some("codex"), "gpt-6.1-sol", 1000, at).unwrap().source.contains("developers.openai.com"));
        assert!(applied_price(Some("zcode"), "glm-5.3", 1000, at).unwrap().source.contains("docs.z.ai"));
        assert!(applied_price(Some("zcode"), "deepseek-v4-flash", 1000, at).unwrap().source.contains("deepseek.com"));
    }

    #[test]
    fn off_peak_window_handles_cross_midnight() {
        // 10:00-24:00 的窗口写成 start=10 end=0（跨夜）
        let window = OffPeakWindow { start_hour: 10, end_hour: 0, weekdays: &[1, 2, 3, 4, 5], full_day: false };
        assert!(in_off_peak(&window, 10, 3), "10 点应在窗口内");
        assert!(in_off_peak(&window, 23, 3), "23 点应在窗口内");
        assert!(!in_off_peak(&window, 8, 3), "8 点不在窗口内");
        assert!(!in_off_peak(&window, 12, 6), "周六不在工作日窗口内");
    }

    #[test]
    fn gemini_long_prompt_still_works_after_refactor() {
        // 回归：Gemini 长提示档沿用原有逻辑
        let short = estimate_usage(
            Some("gemini"), Some("gemini-2.5-pro"),
            Usage { input: 100_000, output: 0, ..Default::default() },
        ).unwrap();
        let long = estimate_usage(
            Some("gemini"), Some("gemini-2.5-pro"),
            Usage { input: 100_000, output: 0, long_prompt: true, ..Default::default() },
        ).unwrap();
        assert!(long > short, "Gemini 长提示应更贵：{short} vs {long}");
    }
}
