//! 价目数据表（**由脚本生成，不要手改**）。
//!
//! 生成方式：`node scripts/pricing/sync-price.mjs`（从 sub2api 维护的 LiteLLM 镜像同步）。
//! 每条记录带 `source` 指向官方定价页，便于人工复核与追溯。
//!
//! 核验日期：2026-11-03。
//! 官方来源：
//! - Anthropic：https://platform.claude.com/docs/en/about-claude/pricing
//! - OpenAI：https://developers.openai.com/api/docs/pricing
//! - Google：https://ai.google.dev/gemini-api/docs/pricing
//! - 智谱：https://docs.z.ai/guides/overview/pricing
//! - DeepSeek：https://api-docs.deepseek.com/quick_start/pricing
//!
//! 价格均为标准、非 Batch、每百万文本 Token 的美元价。

use super::{LongContext, OffPeak, OffPeakWindow, Price};

/// GPT-6 系长上下文规则：>272K 输入时整条请求 input/cache ×2、output ×1.5。
const LONG_272K_X2: LongContext = LongContext {
    threshold: 272_000,
    input_multiplier: 2.0,
    output_multiplier: 1.5,
    cached_multiplier: 2.0,
};

/// GLM-5 系长上下文规则：>32K 输入时输入 ×1.5、输出 ×1.22、缓存 ×1.5。
const LONG_32K_GLM: LongContext = LongContext {
    threshold: 32_000,
    input_multiplier: 1.5,
    output_multiplier: 1.222,
    cached_multiplier: 1.5,
};

/// DeepSeek 谷时：工作日 00:00-01:00、04:00-06:00、10:00-24:00 UTC，以及周末全天。
const DEEPSEEK_OFF_PEAK: OffPeak = OffPeak {
    multiplier: 0.5,
    windows: &[
        OffPeakWindow { start_hour: 0, end_hour: 1, weekdays: &[1, 2, 3, 4, 5], full_day: false },
        OffPeakWindow { start_hour: 4, end_hour: 6, weekdays: &[1, 2, 3, 4, 5], full_day: false },
        OffPeakWindow { start_hour: 10, end_hour: 0, weekdays: &[1, 2, 3, 4, 5], full_day: false },
        // 周末全天谷时
        OffPeakWindow { start_hour: 0, end_hour: 0, weekdays: &[6, 7], full_day: true },
    ],
};



pub(super) const PRICES: &[Price] = &[
    // ── Claude 5 系（2026-11 核实）。缓存写入取 5 分钟档 1.25×。
    Price { platform: "claude", model: "claude-opus-5-5", input: 4.0, cached_input: 0.2, output: 20.0, cache_write: Some(5.0), source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-opus-5", input: 5.0, cached_input: 0.5, output: 25.0, cache_write: Some(6.25), source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-sonnet-5-5", input: 2.0, cached_input: 0.1, output: 10.0, cache_write: Some(2.5), source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-sonnet-5", input: 2.0, cached_input: 0.2, output: 10.0, cache_write: Some(2.5), source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-haiku-5-5", input: 0.1, cached_input: 0.01, output: 0.5, cache_write: Some(0.125), source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-fable-5", input: 10.0, cached_input: 1.0, output: 50.0, cache_write: Some(12.5), source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    // ── GPT-6 系：>272K 输入时整条请求 input/cache ×2、output ×1.5（官方原文 for the full request）
    Price { platform: "codex", model: "gpt-6.1-sol", input: 2.0, cached_input: 0.1, output: 10.0, cache_write: Some(2.5), source: "https://developers.openai.com/api/docs/pricing", long_context: Some(LONG_272K_X2), off_peak: None },
    Price { platform: "codex", model: "gpt-6-sol", input: 2.0, cached_input: 0.1, output: 10.0, cache_write: Some(2.5), source: "https://developers.openai.com/api/docs/pricing", long_context: Some(LONG_272K_X2), off_peak: None },
    Price { platform: "codex", model: "gpt-6-luna", input: 0.1, cached_input: 0.01, output: 0.5, cache_write: Some(0.125), source: "https://developers.openai.com/api/docs/pricing", long_context: Some(LONG_272K_X2), off_peak: None },
    // Anthropic Claude API，标准 Global。缓存写入分 5m/1h，本地日志没有 TTL。
    Price { platform: "claude", model: "claude-opus-4-8", input: 5.0, cached_input: 0.5, output: 25.0, cache_write: None, source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-opus-4-7", input: 5.0, cached_input: 0.5, output: 25.0, cache_write: None, source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-opus-4-6", input: 5.0, cached_input: 0.5, output: 25.0, cache_write: None, source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-opus-4-5", input: 5.0, cached_input: 0.5, output: 25.0, cache_write: None, source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-sonnet-4-6", input: 3.0, cached_input: 0.3, output: 15.0, cache_write: None, source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-sonnet-4-5", input: 3.0, cached_input: 0.3, output: 15.0, cache_write: None, source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-sonnet-4", input: 3.0, cached_input: 0.3, output: 15.0, cache_write: None, source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-haiku-4-5", input: 1.0, cached_input: 0.1, output: 5.0, cache_write: None, source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-opus-4-1", input: 15.0, cached_input: 1.5, output: 75.0, cache_write: None, source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-opus-4", input: 15.0, cached_input: 1.5, output: 75.0, cache_write: None, source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-3-7-sonnet", input: 3.0, cached_input: 0.3, output: 15.0, cache_write: None, source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-3-5-sonnet", input: 3.0, cached_input: 0.3, output: 15.0, cache_write: None, source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    Price { platform: "claude", model: "claude-3-5-haiku", input: 0.8, cached_input: 0.08, output: 4.0, cache_write: None, source: "https://platform.claude.com/docs/en/about-claude/pricing", long_context: None, off_peak: None },
    // OpenAI API；cached_input 直接取官方列，不再用统一倍数推算。
    Price { platform: "codex", model: "gpt-6-astra", input: 10.0, cached_input: 1.0, output: 50.0, cache_write: Some(12.5), source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-5.6-sol", input: 4.0, cached_input: 0.4, output: 20.0, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-5.6", input: 4.0, cached_input: 0.4, output: 20.0, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-5.6-terra", input: 2.0, cached_input: 0.2, output: 12.0, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-5.6-luna", input: 0.2, cached_input: 0.02, output: 1.2, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-5.3-codex", input: 1.75, cached_input: 0.175, output: 14.0, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-5.2-codex", input: 1.75, cached_input: 0.175, output: 14.0, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-5.2", input: 1.75, cached_input: 0.175, output: 14.0, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-5.1-codex-max", input: 1.25, cached_input: 0.125, output: 10.0, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-5.1-codex", input: 1.25, cached_input: 0.125, output: 10.0, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-5.1", input: 1.25, cached_input: 0.125, output: 10.0, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-5-codex", input: 1.25, cached_input: 0.125, output: 10.0, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-5", input: 1.25, cached_input: 0.125, output: 10.0, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "codex-mini-latest", input: 1.5, cached_input: 0.375, output: 6.0, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-4.1", input: 2.0, cached_input: 0.5, output: 8.0, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-4.1-mini", input: 0.4, cached_input: 0.1, output: 1.6, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-4.1-nano", input: 0.1, cached_input: 0.025, output: 0.4, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-4o", input: 2.5, cached_input: 1.25, output: 10.0, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "codex", model: "gpt-4o-mini", input: 0.15, cached_input: 0.075, output: 0.6, cache_write: None, source: "https://developers.openai.com/api/docs/pricing", long_context: None, off_peak: None },
    // Google Gemini API，标准价、文本模态；输出价含思考 Token，工具调用提示按输入价。
    // 缓存按小时存储计费，本地记录没有时长；Gemini 记录也没有缓存写入字段，cache_write 保持 None。
    // 核验日期：2026-10-01，https://ai.google.dev/gemini-api/docs/pricing（3.6–3.8 flash 价目 2027-01-01 起上调）。
    Price { platform: "gemini", model: "gemini-2.5-pro", input: 1.25, cached_input: 0.125, output: 10.0, cache_write: None, source: "https://ai.google.dev/gemini-api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "gemini", model: "gemini-2.5-flash", input: 0.3, cached_input: 0.03, output: 2.5, cache_write: None, source: "https://ai.google.dev/gemini-api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "gemini", model: "gemini-2.5-flash-lite", input: 0.1, cached_input: 0.01, output: 0.4, cache_write: None, source: "https://ai.google.dev/gemini-api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "gemini", model: "gemini-3-flash-preview", input: 0.5, cached_input: 0.05, output: 3.0, cache_write: None, source: "https://ai.google.dev/gemini-api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "gemini", model: "gemini-3.1-pro-preview", input: 2.0, cached_input: 0.2, output: 12.0, cache_write: None, source: "https://ai.google.dev/gemini-api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "gemini", model: "gemini-3.1-flash-lite", input: 0.25, cached_input: 0.025, output: 1.5, cache_write: None, source: "https://ai.google.dev/gemini-api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "gemini", model: "gemini-3.5-flash", input: 1.5, cached_input: 0.15, output: 9.0, cache_write: None, source: "https://ai.google.dev/gemini-api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "gemini", model: "gemini-3.5-flash-lite", input: 0.3, cached_input: 0.03, output: 2.5, cache_write: None, source: "https://ai.google.dev/gemini-api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "gemini", model: "gemini-3.6-flash", input: 0.75, cached_input: 0.075, output: 3.75, cache_write: None, source: "https://ai.google.dev/gemini-api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "gemini", model: "gemini-3.7-flash", input: 0.75, cached_input: 0.075, output: 3.75, cache_write: None, source: "https://ai.google.dev/gemini-api/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "gemini", model: "gemini-3.8-flash", input: 0.75, cached_input: 0.075, output: 3.75, cache_write: None, source: "https://ai.google.dev/gemini-api/docs/pricing", long_context: None, off_peak: None },
    // ── 智谱 GLM（官方页 USD 折算）；>32K 输入长上下文加价
    Price { platform: "zcode", model: "glm-5.3", input: 1.4, cached_input: 0.26, output: 4.4, cache_write: None, source: "https://docs.z.ai/guides/overview/pricing", long_context: Some(LONG_32K_GLM), off_peak: None },
    Price { platform: "zcode", model: "glm-5.3-flash", input: 0.15, cached_input: 0.026, output: 0.5, cache_write: None, source: "https://docs.z.ai/guides/overview/pricing", long_context: Some(LONG_32K_GLM), off_peak: None },
    Price { platform: "zcode", model: "glm-5.2", input: 1.18, cached_input: 0.29, output: 4.13, cache_write: Some(1.18), source: "https://docs.z.ai/guides/overview/pricing", long_context: Some(LONG_32K_GLM), off_peak: None },
    Price { platform: "zcode", model: "glm-5.1", input: 1.4, cached_input: 0.28, output: 4.4, cache_write: Some(1.0), source: "https://docs.z.ai/guides/overview/pricing", long_context: Some(LONG_32K_GLM), off_peak: None },
    // ── DeepSeek：谷时半价（工作日特定时段 + 周末全天）
    Price { platform: "zcode", model: "deepseek-v4-flash", input: 0.3, cached_input: 0.006, output: 1.2, cache_write: Some(0.0), source: "https://api-docs.deepseek.com/quick_start/pricing", long_context: None, off_peak: Some(DEEPSEEK_OFF_PEAK) },
    Price { platform: "claude", model: "deepseek-v4.1-flash-1017", input: 0.3, cached_input: 0.006, output: 1.2, cache_write: Some(0.0), source: "https://api-docs.deepseek.com/quick_start/pricing", long_context: None, off_peak: Some(DEEPSEEK_OFF_PEAK) },
    // ── Kimi
    Price { platform: "zcode", model: "kimi-k3", input: 2.95, cached_input: 0.29, output: 14.74, cache_write: Some(2.95), source: "https://platform.kimi.com/docs/pricing", long_context: None, off_peak: None },
    Price { platform: "zcode", model: "kimi-k2.5", input: 0.6, cached_input: 0.1, output: 2.25, cache_write: Some(0.6), source: "https://platform.kimi.com/docs/pricing", long_context: None, off_peak: None },
];
