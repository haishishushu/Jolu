//! 用量统计页（Jolu.pen 模块 C，画板 30–39）的跨供应商聚合查询。
//!
//! 与 db.rs 的单平台查询口径一致：
//! - 普通输入已扣除单独计量的缓存读取（Codex 的 input 含缓存，Claude 的不含）；
//! - 任一请求缺某个分项，区间内该分项就是未知（None），不补零；
//! - 费用只汇总能精确计价的请求，同时返回已计价条数，界面据此说明覆盖率；
//! - 统计聚合跳过退化行（[`EXCLUDE_DEGENERATE_SQL`]），调用明细照常列出。
//!
//! 一次查询把区间内的请求读进内存再分组：本机记录量级（一年数万条）下比多次 SQL 扫描更简单，
//! 也让概览、分析两个页签共用同一份口径。

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone, Timelike};
use rusqlite::{types::Value, Connection};
use serde::{Deserialize, Serialize};

use crate::db::EXCLUDE_DEGENERATE_SQL;

/// 账号筛选里的「未归属账号」：没有经本地代理归账到任何连接的本机记录
pub const UNATTRIBUTED: &str = "__unattributed__";

/// 两次请求间隔不超过 5 分钟视为连续活跃，用来估算今日活跃时长
const ACTIVE_GAP_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StatsFilter {
    /// [start, end) 的 UTC 毫秒
    pub start: i64,
    pub end: i64,
    #[serde(default)]
    pub platform: Option<String>,
    /// 空表示全部账号；可包含 [`UNATTRIBUTED`]
    #[serde(default)]
    pub connections: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// "success" | "error"；仅调用明细使用
    #[serde(default)]
    pub status: Option<String>,
    /// 调用 ID / 会话 ID 片段；仅调用明细使用
    #[serde(default)]
    pub search: Option<String>,
}

impl StatsFilter {
    pub fn validate(&self) -> Result<(), String> {
        if self.start >= self.end {
            return Err("日期范围无效".into());
        }
        if let Some(p) = &self.platform {
            if !crate::platforms::known(p) {
                return Err("未知平台".into());
            }
        }
        if self.status.as_deref().is_some_and(|s| !matches!(s, "success" | "error")) {
            return Err("未知状态筛选".into());
        }
        Ok(())
    }

    fn with_range(&self, start: i64, end: i64) -> Self {
        Self { start, end, ..self.clone() }
    }
}

/// 拼 WHERE 子句。`calls = false` 时只用区间、供应商、账号与模型——概览和分析不受状态 / 搜索影响。
fn where_clause(f: &StatsFilter, calls: bool) -> (String, Vec<Value>) {
    let mut sql = String::from("r.ts >= ? AND r.ts < ?");
    let mut args = vec![Value::Integer(f.start), Value::Integer(f.end)];
    if let Some(p) = &f.platform {
        sql.push_str(" AND r.platform = ?");
        args.push(Value::Text(p.clone()));
    }
    if !f.connections.is_empty() {
        let ids: Vec<&String> = f.connections.iter().filter(|c| c.as_str() != UNATTRIBUTED).collect();
        let mut parts = Vec::new();
        if !ids.is_empty() {
            parts.push(format!("r.connection_id IN ({})", vec!["?"; ids.len()].join(",")));
            args.extend(ids.into_iter().map(|id| Value::Text(id.clone())));
        }
        if f.connections.iter().any(|c| c == UNATTRIBUTED) {
            parts.push("r.connection_id IS NULL".into());
        }
        sql.push_str(&format!(" AND ({})", parts.join(" OR ")));
    }
    if let Some(m) = &f.model {
        sql.push_str(" AND r.model = ?");
        args.push(Value::Text(m.clone()));
    }
    if calls {
        match f.status.as_deref() {
            Some("success") => sql.push_str(&format!(" AND NOT ({FAILED_SQL})")),
            Some("error") => sql.push_str(&format!(" AND ({FAILED_SQL})")),
            _ => {}
        }
        if let Some(q) = f.search.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
            sql.push_str(" AND (instr(r.dedup_key, ?) > 0 OR instr(COALESCE(r.session_id, ''), ?) > 0)");
            args.push(Value::Text(q.to_string()));
            args.push(Value::Text(q.to_string()));
        }
    } else {
        sql.push_str(EXCLUDE_DEGENERATE_SQL);
    }
    (sql, args)
}

/// 失败请求：上游返回非 2xx，或原生来源明确标记失败。没有状态码的本机记录按成功处理（能写入用量即已返回）。
const FAILED_SQL: &str = "(r.status_code IS NOT NULL AND (r.status_code < 200 OR r.status_code >= 300)) OR COALESCE(r.native_outcome, '') = 'failed'";

/// 一条请求的统计视图；`None` 表示来源没提供
#[derive(Debug, Clone)]
struct Req {
    id: i64,
    call_id: String,
    platform: String,
    model: Option<String>,
    ts: i64,
    session_id: Option<String>,
    fresh_input: Option<i64>,
    output: Option<i64>,
    cache_read: Option<i64>,
    cache_write: Option<i64>,
    total: Option<i64>,
    hit_denominator: Option<i64>,
    cost: Option<f64>,
    effort: Option<String>,
    duration_ms: Option<i64>,
    first_token_ms: Option<i64>,
    status_code: Option<i64>,
    failed: bool,
}

fn load(conn: &Connection, f: &StatsFilter, calls: bool) -> rusqlite::Result<Vec<Req>> {
    let (clause, args) = where_clause(f, calls);
    let sql = format!(
        "SELECT r.id, r.dedup_key, r.platform, r.model, r.ts, r.session_id,
                r.input_tokens, r.output_tokens, r.cache_read_tokens, r.cache_write_tokens, r.total_tokens,
                r.input_known, r.output_known, r.cache_read_known, r.cache_write_known, r.total_known,
                r.input_semantics, r.effort, r.duration_ms, r.first_token_ms, r.status_code, r.native_outcome,
                m.reasoning, m.tool
         FROM requests r LEFT JOIN request_metrics m ON m.request_id = r.id
         WHERE {clause}
         ORDER BY r.ts DESC, r.id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(args), |r| {
        let known = |i: usize| -> rusqlite::Result<bool> { Ok(r.get::<_, i64>(i)? != 0) };
        let value = |i: usize, k: usize| -> rusqlite::Result<Option<i64>> {
            Ok(if known(k)? { Some(r.get(i)?) } else { None })
        };
        let platform: String = r.get(2)?;
        let model: Option<String> = r.get(3)?;
        let input = value(6, 11)?;
        let output = value(7, 12)?;
        let cache_read = value(8, 13)?;
        let cache_write = value(9, 14)?;
        let semantics: String = r.get(16)?;
        let fresh_input = match semantics.as_str() {
            "includes_cache" => input.zip(cache_read).map(|(i, c)| (i - c).max(0)),
            "excludes_cache" => input,
            _ => None,
        };
        let hit_denominator = match semantics.as_str() {
            "includes_cache" => input.zip(cache_read).map(|(i, _)| i),
            "excludes_cache" => input.zip(cache_read).zip(cache_write).map(|((i, r), w)| i + r + w),
            _ => None,
        };
        let reasoning: Option<i64> = r.get(22)?;
        let tool: Option<i64> = r.get(23)?;
        let cost = input.zip(output).zip(cache_write).zip(cache_read).and_then(|(((input, output), cache_write), cache_read)| {
            crate::pricing::estimate_usage(Some(&platform), model.as_deref(), crate::pricing::Usage {
                input, output, cache_write, cache_read,
                reasoning: reasoning.unwrap_or(0), tool: tool.unwrap_or(0),
                long_prompt: input > crate::pricing::GEMINI_LONG_PROMPT_TOKENS,
            })
        });
        let status_code: Option<i64> = r.get(20)?;
        let outcome: Option<String> = r.get(21)?;
        Ok(Req {
            id: r.get(0)?,
            call_id: r.get(1)?,
            failed: status_code.is_some_and(|c| !(200..300).contains(&c)) || outcome.as_deref() == Some("failed"),
            platform,
            model,
            ts: r.get(4)?,
            session_id: r.get(5)?,
            fresh_input,
            output,
            cache_read,
            cache_write,
            total: value(10, 15)?,
            hit_denominator,
            cost,
            effort: r.get(17)?,
            duration_ms: r.get(18)?,
            first_token_ms: r.get(19)?,
            status_code,
        })
    })?;
    rows.collect()
}

/// 已知值求和：任一请求缺该项则整体未知；没有请求时为 `empty`
fn sum_known<'a>(reqs: impl IntoIterator<Item = &'a Req>, pick: impl Fn(&Req) -> Option<i64>, empty: Option<i64>) -> Option<i64> {
    let mut any = false;
    let mut total = 0i64;
    for r in reqs {
        any = true;
        total += pick(r)?;
    }
    if any { Some(total) } else { empty }
}

fn hit_rate<'a>(reqs: impl IntoIterator<Item = &'a Req> + Clone) -> Option<f64> {
    let read = sum_known(reqs.clone(), |r| r.cache_read, None)?;
    let denominator = sum_known(reqs, |r| r.hit_denominator, None)?;
    (denominator > 0 && read <= denominator).then(|| read as f64 / denominator as f64)
}

#[derive(Debug, Serialize, PartialEq)]
pub struct TokenSplit {
    pub fresh_input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    pub total: Option<i64>,
}

fn split<'a>(reqs: impl IntoIterator<Item = &'a Req> + Clone, empty: Option<i64>) -> TokenSplit {
    TokenSplit {
        fresh_input: sum_known(reqs.clone(), |r| r.fresh_input, empty),
        output: sum_known(reqs.clone(), |r| r.output, empty),
        cache_read: sum_known(reqs.clone(), |r| r.cache_read, empty),
        cache_write: sum_known(reqs.clone(), |r| r.cache_write, empty),
        total: sum_known(reqs, |r| r.total, empty),
    }
}

#[derive(Debug, Serialize)]
pub struct CostSummary {
    /// 已计价请求的估算合计（USD）
    pub amount: f64,
    pub priced_requests: i64,
    pub requests: i64,
}

fn cost<'a>(reqs: impl IntoIterator<Item = &'a Req>) -> CostSummary {
    let mut out = CostSummary { amount: 0.0, priced_requests: 0, requests: 0 };
    for r in reqs {
        out.requests += 1;
        if let Some(c) = r.cost {
            out.amount += c;
            out.priced_requests += 1;
        }
    }
    out
}

#[derive(Debug, Serialize)]
pub struct TrendBucket {
    pub start: i64,
    pub end: i64,
    pub label: String,
    pub fresh_input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    pub total: Option<i64>,
    pub requests: i64,
    /// 已计价部分的估算费用
    pub cost: f64,
}

#[derive(Debug, Serialize)]
pub struct Trend {
    /// "2h" | "day" | "week"，界面据此写「每 2 小时 / 每天 / 每周」
    pub bucket: &'static str,
    pub points: Vec<TrendBucket>,
}

#[derive(Debug, Serialize)]
pub struct ModelRow {
    pub platform: String,
    pub model: Option<String>,
    pub requests: i64,
    pub tokens: Option<i64>,
    pub cache_hit_rate: Option<f64>,
    /// 只统计带耗时的请求（经本地代理转发）；没有则为 None
    pub avg_duration_ms: Option<f64>,
    /// 组内全部请求可计价时才有值，否则显示「—」
    pub cost: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct ProviderRow {
    pub platform: String,
    pub requests: i64,
    pub tokens: Option<i64>,
    pub cost: CostSummary,
}

#[derive(Debug, Serialize)]
pub struct SessionRow {
    pub platform: String,
    pub session_id: String,
    pub title: Option<String>,
    /// 范围内已知 Token 之和
    pub tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct Overview {
    pub tokens: TokenSplit,
    pub requests: i64,
    pub failed: i64,
    pub cache_hit_rate: Option<f64>,
    pub cost: CostSummary,
    /// 紧邻的上一段同长度区间的总 Token；没有记录或总量未知时为 None
    pub previous_total: Option<i64>,
    pub trend: Trend,
    pub models: Vec<ModelRow>,
    pub providers: Vec<ProviderRow>,
    /// 能归属到某个连接的请求数
    pub attributed: i64,
    pub sessions: Vec<SessionRow>,
}

fn local_ms(date: NaiveDate) -> i64 {
    let naive = date.and_hms_opt(0, 0, 0).expect("零点总是有效");
    Local.from_local_datetime(&naive).earliest().map(|d| d.timestamp_millis()).unwrap_or_else(|| naive.and_utc().timestamp_millis())
}

fn local_date(ms: i64) -> NaiveDate {
    Local.timestamp_millis_opt(ms).single().map(|d| d.date_naive()).unwrap_or_default()
}

/// 趋势分桶：两天内按 2 小时，62 天内按天，更长按周；边界跟随本地时区
fn buckets(start: i64, end: i64) -> (&'static str, Vec<(i64, i64, String)>) {
    let span = end - start;
    let mut out = Vec::new();
    if span <= 2 * 86_400_000 {
        let first = Local.timestamp_millis_opt(start).single();
        // 对齐到偶数整点，保证刻度落在 00:00 / 02:00 …
        let mut at = first
            .and_then(|d| d.with_minute(0)?.with_second(0)?.with_nanosecond(0))
            .map(|d| d - Duration::hours(i64::from(d.hour() % 2)))
            .map(|d| d.timestamp_millis())
            .unwrap_or(start);
        while at < end {
            let next = at + 2 * 3_600_000;
            let label = Local.timestamp_millis_opt(at).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default();
            out.push((at.max(start), next.min(end), label));
            at = next;
        }
        return ("2h", out);
    }
    let step = if span <= 62 * 86_400_000 { 1 } else { 7 };
    let mut day = local_date(start);
    loop {
        let at = local_ms(day);
        if at >= end {
            break;
        }
        let next_day = day + Duration::days(step);
        out.push((at.max(start), local_ms(next_day).min(end), day.format("%m/%d").to_string()));
        day = next_day;
    }
    (if step == 1 { "day" } else { "week" }, out)
}

fn trend(reqs: &[Req], start: i64, end: i64) -> Trend {
    let (bucket, ranges) = buckets(start, end);
    let mut groups: Vec<Vec<&Req>> = vec![Vec::new(); ranges.len()];
    for r in reqs {
        if let Some(i) = ranges.iter().position(|(s, e, _)| r.ts >= *s && r.ts < *e) {
            groups[i].push(r);
        }
    }
    let points = ranges
        .into_iter()
        .zip(groups)
        .map(|((start, end, label), group)| {
            let tokens = split(group.iter().copied(), Some(0));
            TrendBucket {
                start,
                end,
                label,
                fresh_input: tokens.fresh_input,
                output: tokens.output,
                cache_read: tokens.cache_read,
                cache_write: tokens.cache_write,
                total: tokens.total,
                requests: group.len() as i64,
                cost: cost(group.iter().copied()).amount,
            }
        })
        .collect();
    Trend { bucket, points }
}

fn models(reqs: &[Req]) -> Vec<ModelRow> {
    let mut groups: BTreeMap<(String, Option<String>), Vec<&Req>> = BTreeMap::new();
    for r in reqs {
        groups.entry((r.platform.clone(), r.model.clone())).or_default().push(r);
    }
    let mut rows: Vec<ModelRow> = groups
        .into_iter()
        .map(|((platform, model), group)| {
            let durations: Vec<i64> = group.iter().filter_map(|r| r.duration_ms).collect();
            let priced = cost(group.iter().copied());
            ModelRow {
                platform,
                model,
                requests: group.len() as i64,
                tokens: sum_known(group.iter().copied(), |r| r.total, Some(0)),
                cache_hit_rate: hit_rate(group.iter().copied()),
                avg_duration_ms: (!durations.is_empty()).then(|| durations.iter().sum::<i64>() as f64 / durations.len() as f64),
                cost: (priced.priced_requests == priced.requests).then_some(priced.amount),
            }
        })
        .collect();
    rows.sort_by(|a, b| b.tokens.unwrap_or(0).cmp(&a.tokens.unwrap_or(0)).then(b.requests.cmp(&a.requests)));
    rows
}

fn providers(reqs: &[Req]) -> Vec<ProviderRow> {
    let mut groups: HashMap<&str, Vec<&Req>> = HashMap::new();
    for r in reqs {
        groups.entry(r.platform.as_str()).or_default().push(r);
    }
    let mut rows: Vec<ProviderRow> = groups
        .into_iter()
        .map(|(platform, group)| ProviderRow {
            platform: platform.to_string(),
            requests: group.len() as i64,
            tokens: sum_known(group.iter().copied(), |r| r.total, Some(0)),
            cost: cost(group.iter().copied()),
        })
        .collect();
    rows.sort_by(|a, b| b.tokens.unwrap_or(0).cmp(&a.tokens.unwrap_or(0)).then(a.platform.cmp(&b.platform)));
    rows
}

fn top_sessions(conn: &Connection, reqs: &[Req], limit: usize) -> rusqlite::Result<Vec<SessionRow>> {
    let mut groups: HashMap<(&str, &str), i64> = HashMap::new();
    for r in reqs {
        if let Some(id) = r.session_id.as_deref() {
            *groups.entry((r.platform.as_str(), id)).or_default() += r.total.unwrap_or(0);
        }
    }
    let mut ranked: Vec<_> = groups.into_iter().filter(|(_, t)| *t > 0).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.truncate(limit);
    let mut title = conn.prepare("SELECT title FROM sessions WHERE platform = ?1 AND session_id = ?2")?;
    ranked
        .into_iter()
        .map(|((platform, session_id), tokens)| {
            let found: Option<Option<String>> = title
                .query_row(rusqlite::params![platform, session_id], |r| r.get(0))
                .map(Some)
                .or_else(|e| if e == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(e) })?;
            Ok(SessionRow {
                platform: platform.to_string(),
                session_id: session_id.to_string(),
                title: found.flatten().filter(|t| !t.trim().is_empty()),
                tokens,
            })
        })
        .collect()
}

/// 原生来源（ZCode / Qoder / WorkBuddy 等）覆盖不完整，没有记录不等于真实的 0
fn empty_value(f: &StatsFilter) -> Option<i64> {
    if f.platform.as_deref().is_some_and(crate::platforms::native) { None } else { Some(0) }
}

pub fn overview(conn: &Connection, f: &StatsFilter) -> rusqlite::Result<Overview> {
    let reqs = load(conn, f, false)?;
    let empty = empty_value(f);
    let span = f.end - f.start;
    let previous = load(conn, &f.with_range(f.start - span, f.start), false)?;
    Ok(Overview {
        tokens: split(reqs.iter(), empty),
        requests: reqs.len() as i64,
        failed: reqs.iter().filter(|r| r.failed).count() as i64,
        cache_hit_rate: hit_rate(reqs.iter()),
        cost: cost(reqs.iter()),
        previous_total: if previous.is_empty() { None } else { sum_known(previous.iter(), |r| r.total, None) },
        trend: trend(&reqs, f.start, f.end),
        models: models(&reqs),
        providers: providers(&reqs),
        attributed: count_attributed(conn, f)?,
        sessions: top_sessions(conn, &reqs, 5)?,
    })
}

fn count_attributed(conn: &Connection, f: &StatsFilter) -> rusqlite::Result<i64> {
    let (clause, args) = where_clause(f, false);
    conn.query_row(
        &format!("SELECT COUNT(*) FROM requests r WHERE {clause} AND r.connection_id IS NOT NULL"),
        rusqlite::params_from_iter(args),
        |r| r.get(0),
    )
}

#[derive(Debug, Serialize)]
pub struct ActivityDay {
    /// 本地日期 YYYY-MM-DD
    pub date: String,
    pub tokens: i64,
    pub requests: i64,
}

#[derive(Debug, Serialize)]
pub struct Activity {
    /// 日历起点（含）：一年前那一周的周一
    pub first_date: String,
    pub last_date: String,
    /// 只列出有记录的日期
    pub days: Vec<ActivityDay>,
    /// 今日活跃时长估算（分钟）：相邻请求间隔不超过 5 分钟的时段之和
    pub today_active_minutes: i64,
    pub week_active_days: i64,
    pub month_active_days: i64,
}

/// 全年活跃日历。只按供应商和账号筛选，日期固定为结束日往前一年。
pub fn activity(conn: &Connection, platform: Option<String>, connections: Vec<String>, now: i64) -> rusqlite::Result<Activity> {
    let today = local_date(now);
    let year_ago = today - Duration::days(364);
    let first = year_ago - Duration::days(i64::from(year_ago.weekday().num_days_from_monday()));
    let f = StatsFilter { start: local_ms(first), end: now.max(local_ms(first) + 1), platform, connections, ..Default::default() };
    let reqs = load(conn, &f, false)?;
    let mut days: BTreeMap<NaiveDate, (i64, i64)> = BTreeMap::new();
    let mut today_ts = Vec::new();
    for r in &reqs {
        let date = local_date(r.ts);
        let entry = days.entry(date).or_default();
        entry.0 += r.total.unwrap_or(0);
        entry.1 += 1;
        if date == today {
            today_ts.push(r.ts);
        }
    }
    today_ts.sort_unstable();
    let active_ms: i64 = today_ts.windows(2).map(|w| w[1] - w[0]).filter(|gap| *gap <= ACTIVE_GAP_MS).sum();
    let week_start = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let month_start = today.with_day(1).unwrap_or(today);
    let active: HashSet<NaiveDate> = days.keys().copied().collect();
    Ok(Activity {
        first_date: first.format("%Y-%m-%d").to_string(),
        last_date: today.format("%Y-%m-%d").to_string(),
        week_active_days: active.iter().filter(|d| **d >= week_start && **d <= today).count() as i64,
        month_active_days: active.iter().filter(|d| **d >= month_start && **d <= today).count() as i64,
        days: days
            .into_iter()
            .map(|(date, (tokens, requests))| ActivityDay { date: date.format("%Y-%m-%d").to_string(), tokens, requests })
            .collect(),
        today_active_minutes: active_ms / 60_000,
    })
}

#[derive(Debug, Serialize)]
pub struct CallRow {
    pub id: i64,
    pub call_id: String,
    pub ts: i64,
    pub platform: String,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub fresh_input: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    pub output: Option<i64>,
    pub total: Option<i64>,
    pub cost: Option<f64>,
    pub effort: Option<String>,
    pub duration_ms: Option<i64>,
    pub first_token_ms: Option<i64>,
    pub status_code: Option<i64>,
    pub failed: bool,
}

impl From<Req> for CallRow {
    fn from(r: Req) -> Self {
        Self {
            id: r.id,
            call_id: r.call_id,
            ts: r.ts,
            platform: r.platform,
            model: r.model,
            session_id: r.session_id,
            fresh_input: r.fresh_input,
            cache_read: r.cache_read,
            cache_write: r.cache_write,
            output: r.output,
            total: r.total,
            cost: r.cost,
            effort: r.effort,
            duration_ms: r.duration_ms,
            first_token_ms: r.first_token_ms,
            status_code: r.status_code,
            failed: r.failed,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CallPage {
    pub rows: Vec<CallRow>,
    pub total_count: i64,
    pub unpriced_count: i64,
    pub page: u32,
    pub page_size: u32,
    pub page_count: u32,
    /// 当前区间、供应商与账号下出现过的模型，供「全部模型」下拉使用
    pub models: Vec<String>,
}

pub const CALL_PAGE_SIZE: u32 = 10;

pub fn calls(conn: &Connection, f: &StatsFilter, page: u32) -> rusqlite::Result<CallPage> {
    let reqs = load(conn, f, true)?;
    let total_count = reqs.len() as i64;
    let page_count = (reqs.len() as u32).div_ceil(CALL_PAGE_SIZE).max(1);
    let page = page.clamp(1, page_count);
    let unpriced_count = reqs.iter().filter(|r| r.cost.is_none()).count() as i64;
    let rows = reqs
        .into_iter()
        .skip(((page - 1) * CALL_PAGE_SIZE) as usize)
        .take(CALL_PAGE_SIZE as usize)
        .map(CallRow::from)
        .collect();
    let (clause, args) = where_clause(&StatsFilter { model: None, status: None, search: None, ..f.clone() }, true);
    let mut stmt = conn.prepare(&format!("SELECT DISTINCT r.model FROM requests r WHERE {clause} AND r.model IS NOT NULL ORDER BY r.model"))?;
    let models = stmt.query_map(rusqlite::params_from_iter(args), |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(CallPage { rows, total_count, unpriced_count, page, page_size: CALL_PAGE_SIZE, page_count, models })
}

fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// 按当前筛选导出全部调用。不含 API Key 和对话正文；未知值留空，费用列标明估算与币种。
pub fn calls_csv(conn: &Connection, f: &StatsFilter) -> rusqlite::Result<(String, i64)> {
    let reqs = load(conn, f, true)?;
    let mut out = String::from("\u{feff}时间,供应商,模型,普通输入,缓存读取,缓存写入,输出,总 Token,估算费用（USD）,首字（毫秒）,总耗时（毫秒）,思考强度,状态,会话 ID,调用 ID,数据来源\r\n");
    let opt = |v: Option<i64>| v.map(|v| v.to_string()).unwrap_or_default();
    for r in &reqs {
        let time = Local.timestamp_millis_opt(r.ts).single().map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default();
        let status = match (r.failed, r.status_code) {
            (false, _) => "成功".to_string(),
            (true, Some(code)) => format!("失败 {code}"),
            (true, None) => "失败".to_string(),
        };
        let cells = [
            time,
            crate::platforms::get(&r.platform).map_or(r.platform.clone(), |p| p.name.clone()),
            r.model.clone().unwrap_or_default(),
            opt(r.fresh_input),
            opt(r.cache_read),
            opt(r.cache_write),
            opt(r.output),
            opt(r.total),
            r.cost.map(|c| format!("{c:.6}")).unwrap_or_default(),
            opt(r.first_token_ms),
            opt(r.duration_ms),
            r.effort.clone().unwrap_or_default(),
            status,
            r.session_id.clone().unwrap_or_default(),
            r.call_id.clone(),
            "本机日志".to_string(),
        ];
        out.push_str(&cells.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Ok((out, reqs.len() as i64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{insert_records, RequestRecord};

    fn rec(platform: &str, key: &str, ts: i64, model: &str, tokens: [Option<i64>; 5], session: Option<&str>) -> RequestRecord {
        RequestRecord {
            platform: platform.into(),
            source: format!("test-{platform}"),
            dedup_key: key.into(),
            session_id: session.map(String::from),
            ts,
            model: Some(model.into()),
            input_tokens: tokens[0],
            output_tokens: tokens[1],
            cache_read_tokens: tokens[2],
            cache_write_tokens: tokens[3],
            total_tokens: tokens[4],
            effort: None,
        }
    }

    fn seeded(records: &[RequestRecord]) -> (Connection, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("jolu-usage-stats-{}-{}.db", std::process::id(), records.len()));
        let _ = std::fs::remove_file(&path);
        let mut conn = crate::db::open(&path).unwrap();
        insert_records(&mut conn, records).unwrap();
        (conn, path)
    }

    fn filter(start: i64, end: i64) -> StatsFilter {
        StatsFilter { start, end, ..Default::default() }
    }

    #[test]
    fn overview_normalizes_input_and_keeps_unknown_fields_unknown() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let (conn, path) = seeded(&[
            rec("codex", "c1", base + 1_000, "gpt-5", [Some(1_000), Some(200), Some(800), Some(0), Some(1_200)], Some("s1")),
            rec("claude", "a1", base + 2_000, "claude-sonnet-4-5", [Some(100), Some(50), Some(300), Some(0), Some(450)], Some("s2")),
            rec("claude", "a0", base - 86_400_000 + 1_000, "claude-sonnet-4-5", [Some(10), Some(5), Some(0), Some(0), Some(15)], None),
        ]);
        let o = overview(&conn, &filter(base, base + 86_400_000)).unwrap();
        assert_eq!(o.requests, 2);
        assert_eq!(o.tokens.fresh_input, Some(200 + 100), "Codex 扣掉缓存读取，Claude 原样");
        assert_eq!(o.tokens.cache_read, Some(1_100));
        assert_eq!(o.tokens.total, Some(1_650));
        assert_eq!(o.previous_total, Some(15));
        assert_eq!(o.cost.requests, 2);
        assert_eq!(o.providers.len(), 2);
        assert_eq!(o.sessions[0].session_id, "s1");
        assert_eq!(o.trend.bucket, "2h");
        assert_eq!(o.trend.points.len(), 12);
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_cache_field_makes_that_split_unknown_and_platform_filter_applies() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let (conn, path) = seeded(&[
            rec("claude", "a1", base + 1_000, "claude-sonnet-4-5", [Some(100), Some(50), None, Some(0), Some(150)], None),
            rec("codex", "c1", base + 2_000, "gpt-5", [Some(10), Some(5), Some(0), Some(0), Some(15)], None),
        ]);
        let all = overview(&conn, &filter(base, base + 3_600_000)).unwrap();
        assert_eq!(all.tokens.cache_read, None);
        assert_eq!(all.cache_hit_rate, None);
        assert_eq!(all.trend.bucket, "2h");
        let codex = overview(&conn, &StatsFilter { platform: Some("codex".into()), ..filter(base, base + 3_600_000) }).unwrap();
        assert_eq!(codex.tokens.cache_read, Some(0));
        assert_eq!(codex.requests, 1);
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn calls_paginate_newest_first_and_search_by_session() {
        let base = local_ms(NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        let records: Vec<_> = (0..12)
            .map(|i| rec("claude", &format!("k{i}"), base + i * 1_000, "claude-sonnet-4-5", [Some(1), Some(1), Some(0), Some(0), Some(2)], Some(if i == 3 { "needle" } else { "other" })))
            .collect();
        let (conn, path) = seeded(&records);
        let page = calls(&conn, &filter(base, base + 60_000), 1).unwrap();
        assert_eq!(page.total_count, 12);
        assert_eq!(page.page_count, 2);
        assert_eq!(page.rows[0].call_id, "k11");
        assert_eq!(page.models, vec!["claude-sonnet-4-5".to_string()]);
        let found = calls(&conn, &StatsFilter { search: Some("needle".into()), ..filter(base, base + 60_000) }, 1).unwrap();
        assert_eq!(found.total_count, 1);
        assert_eq!(found.rows[0].call_id, "k3");
        let (csv, count) = calls_csv(&conn, &filter(base, base + 60_000)).unwrap();
        assert_eq!(count, 12);
        assert!(csv.starts_with('\u{feff}'));
        assert_eq!(csv.lines().count(), 13);
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn day_buckets_follow_local_midnight() {
        let start = local_ms(NaiveDate::from_ymd_opt(2026, 10, 15).unwrap());
        let end = local_ms(NaiveDate::from_ymd_opt(2026, 10, 22).unwrap());
        let (kind, ranges) = buckets(start, end);
        assert_eq!(kind, "day");
        assert_eq!(ranges.len(), 7);
        assert_eq!(ranges[0].2, "10/15");
        let (kind, ranges) = buckets(start, start + 12 * 3_600_000 + 1);
        assert_eq!(kind, "2h");
        assert_eq!(ranges.len(), 7);
        assert_eq!(ranges[6].2, "12:00");
    }

    #[test]
    fn csv_cells_are_quoted_when_needed() {
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
        assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_cell("plain"), "plain");
    }
}
