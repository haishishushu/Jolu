//! 编程套餐（Coding Plan）额度的只读查询（cc-switch 对齐，2026-09 源）
//!
//! | 供应商 | 端点 | 凭证 | 窗口 |
//! |---|---|---|---|
//! | Kimi For Coding | GET api.kimi.com/coding/v1/usages | Bearer | 5h + 周 |
//! | 智谱 GLM 中国版 | GET open.bigmodel.cn/api/monitor/usage/quota/limit | 裸 key | 5h(unit=3) + 周(unit=6) |
//! | 智谱 GLM 国际版 | GET api.z.ai/api/monitor/usage/quota/limit | 裸 key | 同上 |
//! | MiniMax 中/国际 | GET {api.minimaxi.com\|api.minimax.io}/v1/api/openplatform/coding_plan/remains | Bearer | 5h + 周(激活时) |
//! | ZenMux | GET {base_url}（base 即用量端点） | Bearer | 5h + 7d（带美元） |
//! | OpenCode Go | GET opencode.ai/zen/go/v1/usage | Bearer | rolling(5h) + 周 + 月 |
//! | ZCode 账号额度 | GET zcode.z.ai/api/v1/zcode-plan/billing/balance | Bearer（ZCode 客户端 JWT）+ `X-Device-Mid` | 来源 `data.balances[]` 的每个 bucket 一个窗口 |
//! | 火山方舟 | 需要控制面 AK/SK，Jolu 不保存此类凭证 | — | 不支持查询 |
//!
//! 纪律与 [`crate::quota`] 一致：只读、15 秒超时、禁止重定向、五态区分。
//! 鉴权头差异：智谱用 `Authorization: <裸key>`（无 Bearer）；其余 Bearer。

use crate::quota::{self, QuotaState, QuotaWindow};

/// 窗口命名（key, 名称），与前端额度窗口渲染约定一致
const W5H: (&str, &str) = ("5h", "5 小时额度");
pub(crate) const W7D: (&str, &str) = ("7d", "周额度");
pub(crate) const W30D: (&str, &str) = ("30d", "月额度");
pub(crate) const WCREDITS: (&str, &str) = ("credits", "Grok 积分额度");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PlanProvider {
    Kimi,
    ZhipuCn,
    ZhipuEn,
    MiniMaxCn,
    MiniMaxEn,
    ZenMux,
    OpencodeGo,
    Volcengine,
}

/// 按数据面 base_url 域名识别套餐供应商；不命中返回 None（维持 sub2api 路由）。
/// 智谱团队版 base_url 与个人版相同，靠 extras 显式区分，不在此识别。
pub(crate) fn detect_provider(base_url: &str) -> Option<PlanProvider> {
    let url = reqwest::Url::parse(base_url).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let host = url.host_str()?;
    let path = url.path();
    let under = |prefix: &str| path == prefix || path.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('/'));
    match host {
        "api.kimi.com" if under("/coding") => Some(PlanProvider::Kimi),
        "open.bigmodel.cn" | "bigmodel.cn" => Some(PlanProvider::ZhipuCn),
        "api.z.ai" => Some(PlanProvider::ZhipuEn),
        "api.minimaxi.com" => Some(PlanProvider::MiniMaxCn),
        "api.minimax.io" => Some(PlanProvider::MiniMaxEn),
        "zenmux.ai" | "api.zenmux.ai" => Some(PlanProvider::ZenMux),
        "opencode.ai" if under("/zen/go") => Some(PlanProvider::OpencodeGo),
        host if host.ends_with(".volces.com") && (under("/api/plan") || under("/api/coding")) => Some(PlanProvider::Volcengine),
        _ => None,
    }
}

/// 统一入口：detect_provider 命中后由 lib.rs / validate_connection 调用。
pub(crate) fn coding_plan_quota(base_url: &str, api_key: &str) -> QuotaState {
    match detect_provider(base_url) {
        Some(PlanProvider::ZhipuCn) | Some(PlanProvider::ZhipuEn) => query_zhipu(base_url, api_key),
        Some(PlanProvider::Kimi) => query_kimi(api_key),
        Some(PlanProvider::MiniMaxCn) => query_minimax(api_key, true),
        Some(PlanProvider::MiniMaxEn) => query_minimax(api_key, false),
        Some(PlanProvider::ZenMux) => query_zenmux(base_url, api_key),
        Some(PlanProvider::OpencodeGo) => query_opencode_go(api_key),
        // 火山控制面 OpenAPI 只认 AK/SK 签名，推理 Key 无法查询；Jolu 不保存这类高权限凭证
        Some(PlanProvider::Volcengine) => QuotaState::Unsupported {
            reason: "火山方舟套餐额度需要控制面 AccessKey，Jolu 不保存此类凭证，暂不支持查询".into(),
        },
        // detect 未命中时不应进入本函数；防御性返回而不是 panic
        None => QuotaState::Unsupported { reason: "该 base_url 未命中任何套餐供应商".into() },
    }
}

/// 按原工具配置已明确识别的套餐直接查询。查询端点全部是固定官方域名，与用户的
/// base_url 无关——中转/反代 base 不影响套餐查询。
pub(crate) fn query_plan_direct(provider: PlanProvider, api_key: &str) -> QuotaState {
    match provider {
        PlanProvider::ZhipuCn => query_zhipu("https://open.bigmodel.cn", api_key),
        PlanProvider::ZhipuEn => query_zhipu("https://api.z.ai", api_key),
        PlanProvider::Kimi => query_kimi(api_key),
        PlanProvider::MiniMaxCn => query_minimax(api_key, true),
        PlanProvider::MiniMaxEn => query_minimax(api_key, false),
        PlanProvider::OpencodeGo => query_opencode_go(api_key),
        // 需要地址或独立凭证的平台：调用方不会传入，防御性返回
        PlanProvider::ZenMux | PlanProvider::Volcengine => QuotaState::Unsupported {
            reason: "该套餐无法自动识别，请将 base_url 填写为套餐官方地址".into(),
        },
    }
}

/* ─────────────────────── 智谱 GLM ─────────────────────── */

/// 控制台同源监控端点（非公开文档 API）。中国/国际站同路径同 JSON 形态。
const ZHIPU_QUOTA_PATH: &str = "/api/monitor/usage/quota/limit";

fn zhipu_quota_base(base_url: &str) -> &'static str {
    if matches!(detect_provider(base_url), Some(PlanProvider::ZhipuCn)) { "https://open.bigmodel.cn" } else { "https://api.z.ai" }
}

fn millis_to_iso(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(ms / 1000, ((ms % 1000) * 1_000_000) as u32).map(|t| t.to_rfc3339())
}

/// JSON 值 → ISO 重置时间：字符串原样；数字区分秒（<1e12）与毫秒；≤0 视为无。
fn extract_reset_time(value: &serde_json::Value) -> Option<String> {
    if let Some(s) = value.as_str() { return Some(s.to_string()); }
    if let Some(n) = value.as_i64() {
        if n <= 0 { return None; }
        let ms = if n < 1_000_000_000_000 { n * 1000 } else { n };
        return millis_to_iso(ms);
    }
    None
}

/// 智谱条目解析中间态
struct ZhipuEntry {
    reset_ms: Option<i64>,
    percent: Option<f64>,
    amount: Option<String>,
    remaining: Option<String>,
}

/// 智谱条目按 `unit` 显式分类：3=5 小时滚动窗，6=周窗（number 有 5/7/1 多种实测，
/// 只锚定 unit）。缺失或不识别时走兜底：无 reset 优先归 5h，其余按 reset 升序补位。
pub(crate) fn parse_zhipu_windows(data: &serde_json::Value) -> (Vec<QuotaWindow>, Option<String>) {
    let classify = |item: &serde_json::Value| match item.get("unit").and_then(|v| v.as_i64()) {
        Some(3) => Some(0usize), // 0 = 5h 槽位
        Some(6) => Some(1usize), // 1 = 周槽位
        _ => None,
    };
    let mut slots: [Option<ZhipuEntry>; 2] = [None, None];
    let mut unclassified: Vec<ZhipuEntry> = Vec::new();

    if let Some(limits) = data.get("limits").and_then(|v| v.as_array()) {
        for item in limits {
            let kind = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if !(kind.eq_ignore_ascii_case("TOKENS_LIMIT") || kind.eq_ignore_ascii_case("CREDIT_LIMIT")) {
                continue;
            }
            // credits 用量（智谱新接口）：currentValue 已用 / usage 总量
            let amount = match (
                item.get("currentValue").and_then(|v| v.as_f64()),
                item.get("usage").and_then(|v| v.as_f64()),
            ) {
                (Some(used), Some(total)) if total > 0.0 && used.is_finite() && used >= 0.0 => {
                    Some(format!("{used:.0} / {total:.0} credits"))
                }
                _ => None,
            };
            let entry = ZhipuEntry {
                reset_ms: item.get("nextResetTime").and_then(|v| v.as_i64()),
                percent: item.get("percentage").and_then(|v| v.as_f64())
                    .filter(|v| v.is_finite() && (0.0..=100.0).contains(v)),
                amount,
                remaining: item.get("remaining").and_then(|v| v.as_f64())
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    .map(|v| format!("{v:.0} {}", if kind.eq_ignore_ascii_case("CREDIT_LIMIT") { "Credits" } else { "Tokens" })),
            };
            match classify(item) {
                Some(slot) if slots[slot].is_none() => slots[slot] = Some(entry),
                _ => unclassified.push(entry),
            }
        }
    }
    // 兜底：无 reset 的优先归 5h（5h 桶 0% 等状态可能没有 nextResetTime），其余按 reset 升序
    unclassified.sort_by_key(|e| (e.reset_ms.is_some(), e.reset_ms.unwrap_or(i64::MIN)));
    for entry in unclassified {
        let slot = if slots[0].is_none() { 0 } else if slots[1].is_none() { 1 } else { break };
        slots[slot] = Some(entry);
    }

    let mut windows = Vec::new();
    for ((key, name), slot) in [W5H, W7D].into_iter().zip(slots) {
        if let Some(e) = slot {
            windows.push(QuotaWindow {
                key: key.into(),
                window_name: name.into(),
                used_percent: e.percent,
                amount_text: e.amount,
                remaining_text: e.remaining,
                resets_at: e.reset_ms.and_then(millis_to_iso),
            });
        }
    }
    let level = data.get("level").and_then(|v| v.as_str()).map(str::to_string);
    (windows, level)
}

pub(crate) fn business_code(value: Option<&serde_json::Value>) -> Option<i64> {
    value.and_then(|value| value.as_i64().or_else(|| value.as_str()?.trim().parse().ok()))
}

/// 上游错误文本可能回显凭证，只向界面返回固定文案和安全的数字业务码。
pub(crate) fn business_failure(provider: &str, code: Option<&serde_json::Value>) -> QuotaState {
    let reason = match business_code(code) {
        Some(code) => format!("{provider} 套餐接口错误（业务码 {code}）"),
        None => format!("{provider} 套餐接口返回业务错误"),
    };
    QuotaState::Failed { reason }
}

/// query 的解析半区：仅已核实的无套餐响应归为不支持，其余业务错误保持 Failed。
fn zhipu_state_from_body(body: &serde_json::Value) -> QuotaState {
    if body.get("success").and_then(|v| v.as_bool()) == Some(false) {
        if business_code(body.get("code")) == Some(500)
            && body.get("msg").and_then(|value| value.as_str()) == Some("当前用户不存在coding plan")
        {
            return QuotaState::Unsupported { reason: "当前凭证未关联 Coding Plan，无法查询套餐额度".into() };
        }
        return business_failure("智谱", body.get("code"));
    }
    let Some(data) = body.get("data") else {
        return QuotaState::Failed { reason: "智谱套餐响应缺少 data 字段".into() };
    };
    let (windows, level) = parse_zhipu_windows(data);
    if windows.is_empty() {
        return QuotaState::Failed { reason: "智谱未返回可识别的套餐额度窗口".into() };
    }
    QuotaState::Ok { windows, plan: level, account: None }
}

/// 查询智谱套餐额度（个人版）。
fn query_zhipu(base_url: &str, api_key: &str) -> QuotaState {
    let c = match quota::client() { Ok(c) => c, Err(e) => return QuotaState::Failed { reason: e } };
    let url = format!("{}{ZHIPU_QUOTA_PATH}", zhipu_quota_base(base_url));
    let request = c.get(&url)
        .header("Authorization", api_key) // 智谱不加 Bearer 前缀
        .header("Content-Type", "application/json")
        .header("Accept-Language", "en-US,en");
    match request.send() {
        Ok(r) => match quota::read(r) {
            quota::HttpOutcome::Body(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(v) => zhipu_state_from_body(&v),
                Err(e) => QuotaState::Failed { reason: format!("智谱套餐响应格式无法解析: {e}") },
            },
            quota::HttpOutcome::Unauthorized(reason) => QuotaState::Unauthorized { reason },
            quota::HttpOutcome::Forbidden(reason) => QuotaState::Forbidden { reason },
            quota::HttpOutcome::RateLimited(reason) => QuotaState::RateLimited { reason },
            quota::HttpOutcome::Failed(reason) => QuotaState::Failed { reason: format!("智谱套餐查询失败: {reason}") },
        },
        Err(e) if e.is_timeout() => QuotaState::Failed { reason: "智谱套餐查询超时（15 秒）".into() },
        Err(_) => QuotaState::Failed { reason: "智谱套餐查询失败，请检查网络后刷新".into() },
    }
}

/* ─────────────────────── ZCode 账号额度 ─────────────────────── */

/// ZCode 账号额度（`GET https://zcode.z.ai/api/v1/zcode-plan/billing/balance`）。
///
/// **一个 bucket 就是一个额度窗口**：同一个 plan 下可能并存多个按模型分开的 bucket
/// （实测 `GLM-5.3` 约 300 万 token/日、`GLM-5.3-Flash` 约 500 万 token/日），
/// 所以去重键必须落在 `bucket_id` / `entitlement_id` 上，不能按 plan 或按窗口名合并。
#[derive(Debug, Default)]
pub(crate) struct ZcodePlanBalance {
    pub(crate) windows: Vec<QuotaWindow>,
    /// `plans[].name` 原样透传（如 `ZCode Start Plan`），不由用量推断档位。
    pub(crate) plan: Option<String>,
    /// `plans[].status` 原样（active / expired / …），界面口径由调用方映射。
    pub(crate) plan_status: Option<String>,
    /// `plans[].ends_at`（unix 秒）→ RFC3339。
    pub(crate) plan_ends_at: Option<String>,
    /// `data.server_time`（unix 秒）→ RFC3339，作为观测时间。
    pub(crate) observed_at: Option<String>,
}

/// `code 0` 之后的三种情形必须分开，任何两种都不能合并：
/// 有额度 / 成功但没有额度数据 / 形态不认识（算查询失败）。
#[derive(Debug)]
pub(crate) enum ZcodePlanBalanceOutcome {
    Parsed(ZcodePlanBalance),
    /// `data` 缺失、为 `null` 或为空对象：查询成功，但来源没有给额度数据。
    /// 这既不是查询失败，也不是真实用量的 0%。
    NoQuotaData,
    /// `data` / `plans` / `balances` 的类型不是约定形态：按查询失败上报。
    Malformed(String),
}

/// 数值字段既可能是数字，也可能是数字字符串；非有限值一律当未提供。
fn json_f64(value: Option<&serde_json::Value>) -> Option<f64> {
    let number = match value? {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }?;
    (number.is_finite() && number >= 0.0).then_some(number)
}

/// 秒 / 毫秒时间戳（数字或数字字符串）→ RFC3339；≤0 或不可解析视为未提供。
fn unix_time_iso(value: Option<&serde_json::Value>) -> Option<String> {
    let number = match value? {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }?;
    if !number.is_finite() || number <= 0.0 {
        return None;
    }
    let millis = if number < 1_000_000_000_000.0 { (number * 1000.0).round() } else { number.round() };
    millis_to_iso(millis as i64)
}

/// 取一个非空字符串字段（去首尾空白）；空串等同缺失。
fn text_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|value| value.as_str()).map(str::trim).filter(|text| !text.is_empty()).map(str::to_owned)
}

/// 数值文案：整数不带小数点，小数最多两位且不留尾随 0。
fn quantity(value: f64) -> String {
    if (value.fract()).abs() < 1e-9 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}").trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// `plans[].entitlements[]` 按 `entitlement_id` 建索引：balance 条目本身没有
/// `show_name` / `unit_type` 时回填，避免窗口标题退化成编号。
fn entitlement_index(plans: Option<&serde_json::Value>) -> std::collections::HashMap<&str, &serde_json::Value> {
    let mut index = std::collections::HashMap::new();
    for plan in plans.and_then(|value| value.as_array()).into_iter().flatten() {
        for entitlement in plan.get("entitlements").and_then(|value| value.as_array()).into_iter().flatten() {
            if let Some(id) = entitlement.get("entitlement_id").and_then(|value| value.as_str()) {
                index.entry(id).or_insert(entitlement);
            }
        }
    }
    index
}

/// 解析 ZCode 账号额度响应体（已通过业务码校验的 `code 0` 响应）。
pub(crate) fn parse_zcode_plan_balance(body: &serde_json::Value) -> ZcodePlanBalanceOutcome {
    let Some(data) = body.get("data").filter(|value| !value.is_null()) else {
        return ZcodePlanBalanceOutcome::NoQuotaData;
    };
    let Some(data) = data.as_object() else {
        return ZcodePlanBalanceOutcome::Malformed("ZCode 额度响应的 data 不是对象".into());
    };
    let mut balance = ZcodePlanBalance::default();
    if let Some(plans) = data.get("plans") {
        let Some(plans) = plans.as_array() else {
            return ZcodePlanBalanceOutcome::Malformed("ZCode 额度响应的 plans 不是数组".into());
        };
        // 多条 plan 时优先 active，其次第一条；不按名称或优先级猜档位。
        let chosen = plans
            .iter()
            .find(|plan| plan.get("status").and_then(|value| value.as_str()).is_some_and(|status| status.eq_ignore_ascii_case("active")))
            .or_else(|| plans.first());
        if let Some(plan) = chosen.filter(|plan| plan.is_object()) {
            balance.plan = text_field(plan, "name");
            balance.plan_status = text_field(plan, "status");
            balance.plan_ends_at = unix_time_iso(plan.get("ends_at"));
        }
    }
    balance.observed_at = unix_time_iso(data.get("server_time"));
    // 有 plans 没有 balances：套餐在、额度窗口不在，仍是「有额度数据」的一种，交给调用方表达。
    let Some(balances) = data.get("balances") else { return ZcodePlanBalanceOutcome::Parsed(balance) };
    let Some(balances) = balances.as_array() else {
        return ZcodePlanBalanceOutcome::Malformed("ZCode 额度响应的 balances 不是数组".into());
    };
    let entitlements = entitlement_index(data.get("plans"));
    let mut seen = std::collections::HashSet::new();
    for (index, bucket) in balances.iter().enumerate() {
        if !bucket.is_object() {
            continue;
        }
        // 去重键：bucket_id 优先，其次 entitlement_id；都缺失时用序号，避免把不同桶并成一行。
        let key = ["bucket_id", "entitlement_id"]
            .iter()
            .find_map(|name| text_field(bucket, *name))
            .unwrap_or_else(|| format!("bucket-{index}"));
        if !seen.insert(key.clone()) {
            continue;
        }
        let entitlement = text_field(bucket, "entitlement_id").as_deref().and_then(|id| entitlements.get(id)).copied();
        let window_name = text_field(bucket, "show_name")
            .or_else(|| entitlement.and_then(|value| text_field(value, "show_name")))
            .unwrap_or_else(|| format!("额度窗口 {}", index + 1));
        let unit = text_field(bucket, "unit_type")
            .or_else(|| entitlement.and_then(|value| text_field(value, "unit_type")));
        let suffix = unit.map(|unit| format!(" {unit}")).unwrap_or_default();
        let used = json_f64(bucket.get("used_units"));
        let total = json_f64(bucket.get("total_units"));
        // total_units 为 0 或缺总量时百分比是「未知」，不能 clamp 成 0%（那不是真实用量）。
        let used_percent = match (used, total) {
            (Some(used), Some(total)) if total > 0.0 => Some(used / total * 100.0),
            _ => None,
        };
        let amount_text = match (used, total) {
            (Some(used), Some(total)) if total > 0.0 => Some(format!("{} / {}{suffix}", quantity(used), quantity(total))),
            _ => None,
        };
        let remaining_text = match (json_f64(bucket.get("remaining_units")), json_f64(bucket.get("available_units"))) {
            // 剩余与可用不同时两个都给，界面才不用在两者之间二选一。
            (Some(remaining), Some(available)) if (remaining - available).abs() > f64::EPSILON => {
                Some(format!("剩余 {}{suffix}（可用 {}）", quantity(remaining), quantity(available)))
            }
            (Some(remaining), _) => Some(format!("剩余 {}{suffix}", quantity(remaining))),
            (None, Some(available)) => Some(format!("可用 {}{suffix}", quantity(available))),
            _ => None,
        };
        balance.windows.push(QuotaWindow {
            key,
            window_name,
            used_percent,
            amount_text,
            remaining_text,
            resets_at: unix_time_iso(bucket.get("period_end")),
        });
    }
    ZcodePlanBalanceOutcome::Parsed(balance)
}

/* ─────────────────────── Kimi For Coding ─────────────────────── */

/// `limits[].detail` 为 5h 桶（limit/remaining 绝对值），顶层 `usage` 为周桶；
/// resetTime 兼容 ISO 字符串与秒/毫秒时间戳。usage 缺失 → 无周桶。
fn parse_kimi_windows(body: &serde_json::Value) -> Vec<QuotaWindow> {
    // limit/remaining 是绝对值；limit=0 视为空窗按 0% 处理（cc-switch 同款语义）
    let tier = |limit: f64, remaining: f64| {
        let used = (limit - remaining).max(0.0);
        let percent = if limit > 0.0 { used / limit * 100.0 } else { 0.0 };
        Some(percent).filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
    };
    let mut windows = Vec::new();
    if let Some(limits) = body.get("limits").and_then(|v| v.as_array()) {
        for item in limits {
            let Some(detail) = item.get("detail") else { continue };
            windows.push(QuotaWindow {
                key: W5H.0.into(),
                window_name: W5H.1.into(),
                used_percent: tier(
                    detail.get("limit").and_then(|v| v.as_f64()).unwrap_or(1.0),
                    detail.get("remaining").and_then(|v| v.as_f64()).unwrap_or(0.0),
                ),
                amount_text: None,
                remaining_text: None,
                resets_at: detail.get("resetTime").and_then(extract_reset_time),
            });
        }
    }
    if let Some(usage) = body.get("usage") {
        windows.push(QuotaWindow {
            key: W7D.0.into(),
            window_name: W7D.1.into(),
            used_percent: tier(
                usage.get("limit").and_then(|v| v.as_f64()).unwrap_or(1.0),
                usage.get("remaining").and_then(|v| v.as_f64()).unwrap_or(0.0),
            ),
            amount_text: None,
            remaining_text: None,
            resets_at: usage.get("resetTime").and_then(extract_reset_time),
        });
    }
    windows
}

fn query_kimi(api_key: &str) -> QuotaState {
    let c = match quota::client() { Ok(c) => c, Err(e) => return QuotaState::Failed { reason: e } };
    match c.get("https://api.kimi.com/coding/v1/usages")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .send()
    {
        Ok(r) => match quota::read(r) {
            quota::HttpOutcome::Body(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(v) => {
                    let windows = parse_kimi_windows(&v);
                    if windows.is_empty() { QuotaState::Failed { reason: "Kimi 未返回可识别的额度窗口".into() } }
                    else { QuotaState::Ok { windows, plan: None, account: None } }
                }
                Err(e) => QuotaState::Failed { reason: format!("Kimi 用量响应格式无法解析: {e}") },
            },
            quota::HttpOutcome::Unauthorized(reason) => QuotaState::Unauthorized { reason },
            quota::HttpOutcome::Forbidden(reason) => QuotaState::Forbidden { reason },
            quota::HttpOutcome::RateLimited(reason) => QuotaState::RateLimited { reason },
            quota::HttpOutcome::Failed(reason) => QuotaState::Failed { reason: format!("Kimi 用量查询失败: {reason}") },
        },
        Err(e) if e.is_timeout() => QuotaState::Failed { reason: "Kimi 用量查询超时（15 秒）".into() },
        Err(_) => QuotaState::Failed { reason: "Kimi 用量查询失败，请检查网络后刷新".into() },
    }
}

/* ─────────────────────── MiniMax ─────────────────────── */

/// 编程套餐剩余百分比在 `model_remains` 的 `general` 条目；`current_*_remaining_percent`
/// 是「剩余」，反转为已用。周桶仅 `current_weekly_status==1` 时存在（3=无周限额）。
fn parse_minimax_windows(body: &serde_json::Value) -> Vec<QuotaWindow> {
    let mut windows = Vec::new();
    let Some(items) = body.get("model_remains").and_then(|v| v.as_array()) else { return windows };
    let Some(item) = items.iter().find(|i| i.get("model_name").and_then(|v| v.as_str()) == Some("general")) else { return windows };
    let used_of = |remain: f64| (100.0 - remain).clamp(0.0, 100.0);
    if let Some(remain) = item.get("current_interval_remaining_percent").and_then(|v| v.as_f64()) {
        windows.push(QuotaWindow {
            key: W5H.0.into(),
            window_name: W5H.1.into(),
            used_percent: Some(used_of(remain)),
            amount_text: None,
            remaining_text: None,
            resets_at: item.get("end_time").and_then(|v| v.as_i64()).and_then(millis_to_iso),
        });
    }
    if item.get("current_weekly_status").and_then(|v| v.as_i64()) == Some(1) {
        if let Some(remain) = item.get("current_weekly_remaining_percent").and_then(|v| v.as_f64()) {
            windows.push(QuotaWindow {
                key: W7D.0.into(),
                window_name: W7D.1.into(),
                used_percent: Some(used_of(remain)),
                amount_text: None,
                remaining_text: None,
                resets_at: item.get("weekly_end_time").and_then(|v| v.as_i64()).and_then(millis_to_iso),
            });
        }
    }
    windows
}

fn minimax_state_from_body(body: &serde_json::Value) -> QuotaState {
    if let Some(base_resp) = body.get("base_resp") {
        if business_code(base_resp.get("status_code")) != Some(0) {
            return business_failure("MiniMax", base_resp.get("status_code"));
        }
    }
    let windows = parse_minimax_windows(body);
    if windows.is_empty() { QuotaState::Failed { reason: "MiniMax 未返回可识别的额度窗口".into() } }
    else { QuotaState::Ok { windows, plan: None, account: None } }
}

fn query_minimax(api_key: &str, cn: bool) -> QuotaState {
    let host = if cn { "api.minimaxi.com" } else { "api.minimax.io" };
    let c = match quota::client() { Ok(c) => c, Err(e) => return QuotaState::Failed { reason: e } };
    match c.get(&format!("https://{host}/v1/api/openplatform/coding_plan/remains"))
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .send()
    {
        Ok(r) => match quota::read(r) {
            quota::HttpOutcome::Body(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(v) => minimax_state_from_body(&v),
                Err(e) => QuotaState::Failed { reason: format!("MiniMax 用量响应格式无法解析: {e}") },
            },
            quota::HttpOutcome::Unauthorized(reason) => QuotaState::Unauthorized { reason },
            quota::HttpOutcome::Forbidden(reason) => QuotaState::Forbidden { reason },
            quota::HttpOutcome::RateLimited(reason) => QuotaState::RateLimited { reason },
            quota::HttpOutcome::Failed(reason) => QuotaState::Failed { reason: format!("MiniMax 用量查询失败: {reason}") },
        },
        Err(e) if e.is_timeout() => QuotaState::Failed { reason: "MiniMax 用量查询超时（15 秒）".into() },
        Err(_) => QuotaState::Failed { reason: "MiniMax 用量查询失败，请检查网络后刷新".into() },
    }
}

/* ─────────────────────── ZenMux ─────────────────────── */

/// base_url 本身就是用量端点；usage_percentage 为 0–1 小数；5h 桶带美元已用/上限。
fn parse_zenmux_windows(data: &serde_json::Value) -> (Vec<QuotaWindow>, Option<String>) {
    let mut windows = Vec::new();
    let tier = |key: (&str, &str), node: Option<&serde_json::Value>| {
        let Some(q) = node else { return None };
        Some(QuotaWindow {
            key: key.0.into(),
            window_name: key.1.into(),
            used_percent: q.get("usage_percentage").and_then(|v| v.as_f64())
                .map(|p| p * 100.0)
                .filter(|v| v.is_finite() && (0.0..=100.0).contains(v)),
            amount_text: match (
                q.get("used_value_usd").and_then(|v| v.as_f64()),
                q.get("max_value_usd").and_then(|v| v.as_f64()),
            ) {
                (Some(used), Some(max)) if max > 0.0 && used.is_finite() => Some(format!("${used:.2} / ${max:.2}")),
                _ => None,
            },
            remaining_text: None,
            resets_at: q.get("resets_at").and_then(|v| v.as_str()).map(str::to_string),
        })
    };
    windows.extend(tier(W5H, data.get("quota_5_hour")));
    windows.extend(tier(W7D, data.get("quota_7_day")));
    let plan = data.get("plan").and_then(|p| p.get("tier")).and_then(|v| v.as_str()).map(|tier| {
        let status = data.get("account_status").and_then(|v| v.as_str()).unwrap_or("");
        if status.is_empty() { tier.to_string() } else { format!("{tier} ({status})") }
    });
    (windows, plan)
}

fn zenmux_state_from_body(body: &serde_json::Value) -> QuotaState {
    if body.get("success").and_then(|v| v.as_bool()) != Some(true) {
        return business_failure("ZenMux", body.get("code"));
    }
    match body.get("data") {
        Some(data) => {
            let (windows, plan) = parse_zenmux_windows(data);
            if windows.is_empty() { QuotaState::Failed { reason: "ZenMux 未返回可识别的额度窗口".into() } }
            else { QuotaState::Ok { windows, plan, account: None } }
        }
        None => QuotaState::Failed { reason: "ZenMux 响应缺少 data 字段".into() },
    }
}

fn query_zenmux(base_url: &str, api_key: &str) -> QuotaState {
    let c = match quota::client() { Ok(c) => c, Err(e) => return QuotaState::Failed { reason: e } };
    match c.get(base_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .send()
    {
        Ok(r) => match quota::read(r) {
            quota::HttpOutcome::Body(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(v) => zenmux_state_from_body(&v),
                Err(e) => QuotaState::Failed { reason: format!("ZenMux 用量响应格式无法解析: {e}") },
            },
            quota::HttpOutcome::Unauthorized(reason) => QuotaState::Unauthorized { reason },
            quota::HttpOutcome::Forbidden(reason) => QuotaState::Forbidden { reason },
            quota::HttpOutcome::RateLimited(reason) => QuotaState::RateLimited { reason },
            quota::HttpOutcome::Failed(reason) => QuotaState::Failed { reason: format!("ZenMux 用量查询失败: {reason}") },
        },
        Err(e) if e.is_timeout() => QuotaState::Failed { reason: "ZenMux 用量查询超时（15 秒）".into() },
        Err(_) => QuotaState::Failed { reason: "ZenMux 用量查询失败，请检查网络后刷新".into() },
    }
}

/* ─────────────────────── OpenCode Go ─────────────────────── */

/// 第一方但未文档化的路由（上线次日就改过一次形态），逐窗口防御解析：
/// 缺失或 percent 不可解析的窗口跳过；全空由调用方按「形态不认识」报错。
fn parse_opencode_windows(body: &serde_json::Value) -> Vec<QuotaWindow> {
    const WINDOWS: [(&str, &str, &str); 3] = [
        ("rolling", W5H.0, W5H.1),
        ("weekly", W7D.0, W7D.1),
        ("monthly", W30D.0, W30D.1),
    ];
    let Some(usage) = body.get("usage") else { return Vec::new() };
    let mut windows = Vec::new();
    for (node_key, key, name) in WINDOWS {
        let Some(w) = usage.get(node_key) else { continue };
        let Some(percent) = w.get("percent").and_then(|v| v.as_f64()) else { continue };
        windows.push(QuotaWindow {
            key: key.into(),
            window_name: name.into(),
            used_percent: Some(percent).filter(|v| v.is_finite() && (0.0..=100.0).contains(v)),
            amount_text: None,
            remaining_text: None,
            // percent=0 时 resetsAt 是占位值（滚动窗按最后记账时间整窗清零，窗口早已过期）
            resets_at: if percent > 0.0 { w.get("resetsAt").and_then(extract_reset_time) } else { None },
        });
    }
    windows
}

/// 用量端点只认 `Authorization: Bearer`（与推理侧 x-api-key 正好相反，不能互换）。
fn query_opencode_go(api_key: &str) -> QuotaState {
    let c = match quota::client() { Ok(c) => c, Err(e) => return QuotaState::Failed { reason: e } };
    let send = c.get("https://opencode.ai/zen/go/v1/usage")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .send();
    match send {
        Ok(r) => {
            // 403 EntitlementError：key 本身有效（Zen 与 Go 共用同一把 workspace key），
            // 但该工作区没有 Go 订阅——与 401 认证失败分开提示。
            if r.status().as_u16() == 403 {
                return QuotaState::Forbidden {
                    reason: "Key 有效但该工作区未订阅 OpenCode Go（HTTP 403）".into(),
                };
            }
            match quota::read(r) {
                quota::HttpOutcome::Body(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                    Ok(v) => {
                        let windows = parse_opencode_windows(&v);
                        if windows.is_empty() {
                            QuotaState::Failed { reason: "OpenCode Go 用量响应形态不认识（上游曾变更过结构）".into() }
                        } else {
                            QuotaState::Ok { windows, plan: None, account: None }
                        }
                    }
                    Err(e) => QuotaState::Failed { reason: format!("OpenCode Go 用量响应格式无法解析: {e}") },
                },
                quota::HttpOutcome::Unauthorized(reason) => QuotaState::Unauthorized { reason },
                quota::HttpOutcome::Forbidden(reason) => QuotaState::Forbidden { reason },
                quota::HttpOutcome::RateLimited(reason) => QuotaState::RateLimited { reason },
                quota::HttpOutcome::Failed(reason) => QuotaState::Failed { reason: format!("OpenCode Go 用量查询失败: {reason}") },
            }
        }
        Err(e) if e.is_timeout() => QuotaState::Failed { reason: "OpenCode Go 用量查询超时（15 秒）".into() },
        Err(_) => QuotaState::Failed { reason: "OpenCode Go 用量查询失败，请检查网络后刷新".into() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_no_plan_is_unsupported_but_other_business_failures_stay_failed() {
        let no_plan = zhipu_state_from_body(&serde_json::json!({"code":500,"success":false,"msg":"当前用户不存在coding plan"}));
        assert!(matches!(no_plan, QuotaState::Unsupported { .. }));
        for message in ["other failure", "当前用户不存在coding plan synthetic-secret"] {
            assert!(matches!(zhipu_state_from_body(&serde_json::json!({"code":500,"success":false,"msg":message})), QuotaState::Failed { .. }));
        }
    }

    #[test]
    fn business_error_messages_never_echo_credentials_and_only_keep_numeric_codes() {
        for code in [serde_json::json!(500), serde_json::json!("500"), serde_json::json!("synthetic-secret")] {
            let states = [
                zhipu_state_from_body(&serde_json::json!({"success":false,"code":code,"msg":"synthetic-secret"})),
                minimax_state_from_body(&serde_json::json!({"base_resp":{"status_code":code,"status_msg":"synthetic-secret"}})),
                zenmux_state_from_body(&serde_json::json!({"success":false,"code":code,"message":"synthetic-secret"})),
            ];
            for state in states {
                let QuotaState::Failed { reason } = state else { panic!("unknown business failure must remain failed") };
                assert!(!reason.contains("synthetic-secret"));
                if code != serde_json::json!("synthetic-secret") { assert!(reason.contains("500")); }
            }
        }
    }

    #[test]
    #[ignore = "需要鼠鼠授权：只读查询本机已配置的智谱套餐，只输出 HTTP 状态、数字业务码及字段是否存在"]
    fn local_zhipu_response_classification() {
        let key = crate::creds::read_secret("claude", "api").expect("缺少本机 API 凭证");
        let base = crate::creds::local_base_url("claude", "api").expect("缺少本机 API 地址");
        assert!(matches!(detect_provider(&base), Some(PlanProvider::ZhipuCn | PlanProvider::ZhipuEn)), "当前配置不是此测试支持的套餐");
        let client = quota::client().unwrap_or_else(|_| panic!("client_initialization_failed"));
        let response = client.get(format!("{}{ZHIPU_QUOTA_PATH}", zhipu_quota_base(&base)))
            .header("Authorization", &key)
            .header("Content-Type", "application/json")
            .header("Accept-Language", "en-US,en")
            .send().unwrap_or_else(|_| panic!("network_request_failed"));
        println!("Coding plan diagnostic: http_status={}", response.status().as_u16());
        let body: serde_json::Value = response.json().unwrap_or_else(|_| panic!("json_parse_failed"));
        let business_code = body.get("code").and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse::<i64>().ok()));
        let success = body.get("success").and_then(serde_json::Value::as_bool);
        println!("Coding plan diagnostic: business_code={business_code:?} success={success:?} data_present={}", body.get("data").is_some_and(|value| !value.is_null()));
        let message_class = match body.get("msg").and_then(serde_json::Value::as_str) {
            Some("当前用户不存在coding plan") => "no_coding_plan",
            Some("系统异常" | "系统错误" | "系统繁忙，请稍后重试") => "server_error",
            _ => "unclassified",
        };
        println!("Coding plan diagnostic: message_class={message_class}");
        assert_eq!(success, Some(true), "business_response_not_successful");
    }

    #[test]
    fn provider_detection_requires_an_exact_official_https_host_and_path() {
        for base in [
            "https://open.bigmodel.cn.evil.test/api/anthropic",
            "https://gateway.test/api.z.ai/v1",
            "https://gateway.test/?upstream=api.minimaxi.com",
            "https://api.z.ai@gateway.test/v1",
            "https://notzenmux.test/v1",
            "http://open.bigmodel.cn/api/anthropic",
            "https://api.kimi.com/coding-other/v1",
            "https://opencode.ai/zen/gopher/v1",
            "https://ark.cn-beijing.volces.com/api/planless/v3",
        ] {
            assert!(detect_provider(base).is_none(), "误识别地址：{base}");
        }
    }

    #[test]
    fn detect_provider_routes_by_base_url_domain() {
        assert!(matches!(detect_provider("https://api.kimi.com/coding/v1"), Some(PlanProvider::Kimi)));
        assert!(matches!(detect_provider("https://open.bigmodel.cn/api/anthropic"), Some(PlanProvider::ZhipuCn)));
        assert!(matches!(detect_provider("https://bigmodel.cn/api/anthropic"), Some(PlanProvider::ZhipuCn)));
        assert!(matches!(detect_provider("https://api.z.ai/api/anthropic"), Some(PlanProvider::ZhipuEn)));
        assert!(matches!(detect_provider("https://api.minimaxi.com/v1"), Some(PlanProvider::MiniMaxCn)));
        assert!(matches!(detect_provider("https://api.minimax.io/v1"), Some(PlanProvider::MiniMaxEn)));
        assert!(matches!(detect_provider("https://zenmux.ai/api/example"), Some(PlanProvider::ZenMux)));
        assert!(matches!(detect_provider("https://opencode.ai/zen/go"), Some(PlanProvider::OpencodeGo)));
        assert!(matches!(detect_provider("https://opencode.ai/zen/go/v1"), Some(PlanProvider::OpencodeGo)));
        assert!(matches!(detect_provider("https://ark.cn-beijing.volces.com/api/plan/v3"), Some(PlanProvider::Volcengine)));
        assert!(matches!(detect_provider("https://ark.cn-beijing.volces.com/api/coding/v3"), Some(PlanProvider::Volcengine)));
        // 大小写不敏感
        assert!(matches!(detect_provider("https://API.KIMI.COM/coding/v1"), Some(PlanProvider::Kimi)));
    }

    #[test]
    fn detect_provider_rejects_non_plan_gateways_and_official() {
        // 官方端点、sub2api 网关、Zen 按量版都不命中
        assert!(detect_provider("https://api.anthropic.com").is_none());
        assert!(detect_provider("https://gateway.example.com").is_none());
        assert!(detect_provider("https://opencode.ai/zen/v1").is_none());
        // DouBaoSeed 按量付费路径不命中
        assert!(detect_provider("https://ark.cn-beijing.volces.com/api/v3").is_none());
        assert!(detect_provider("").is_none());
    }

    #[test]
    fn zhipu_quota_region_ignores_domain_names_in_query_parameters() {
        assert_eq!(zhipu_quota_base("https://api.z.ai/api/anthropic?hint=bigmodel.cn"), "https://api.z.ai");
        assert_eq!(zhipu_quota_base("https://open.bigmodel.cn/api/anthropic?hint=api.z.ai"), "https://open.bigmodel.cn");
    }

    #[test]
    fn zhipu_real_shape_credit_limit_two_windows() {
        // 2026-09-17 实测：type=CREDIT_LIMIT，unit=3/6 定窗口，percentage 为已用百分比
        let body: serde_json::Value = serde_json::from_str(r#"{
            "code":200,"msg":"Operation successful","success":true,
            "data":{"level":"pro","limits":[
                {"type":"CREDIT_LIMIT","unit":3,"number":5,"usage":12000,"currentValue":5710,"remaining":6289,"percentage":47,"nextResetTime":1789648624765},
                {"type":"CREDIT_LIMIT","unit":6,"number":1,"usage":60000,"currentValue":5710,"remaining":54289,"percentage":9,"nextResetTime":1790235145997}
            ]}}"#).unwrap();
        let (windows, level) = parse_zhipu_windows(&body.get("data").unwrap());
        assert_eq!(level.as_deref(), Some("pro"));
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].key, "5h");
        assert_eq!(windows[0].used_percent, Some(47.0));
        assert_eq!(windows[0].amount_text.as_deref(), Some("5710 / 12000 credits"));
        assert_eq!(windows[0].remaining_text.as_deref(), Some("6289 Credits"));
        assert!(windows[0].resets_at.is_some());
        assert_eq!(windows[1].key, "7d");
        assert_eq!(windows[1].used_percent, Some(9.0));
        assert_eq!(windows[1].remaining_text.as_deref(), Some("54289 Credits"));
    }

    #[test]
    fn zhipu_unit_field_anchors_window_not_reset_order() {
        // issue #3036：周期末尾周桶比 5h 桶更早重置，按时间排序必然标反；unit 优先
        let data = serde_json::json!({"limits":[
            {"type":"TOKENS_LIMIT","unit":6,"number":7,"percentage":42.0,"nextResetTime":1_000_003_600_000i64},
            {"type":"TOKENS_LIMIT","unit":3,"number":5,"percentage":1.0,"nextResetTime":1_000_018_000_000i64}
        ]});
        let (windows, _) = parse_zhipu_windows(&data);
        assert_eq!(windows[0].key, "5h");
        assert_eq!(windows[0].used_percent, Some(1.0));
        assert_eq!(windows[1].key, "7d");
        assert_eq!(windows[1].used_percent, Some(42.0));
    }

    #[test]
    fn zhipu_missing_unit_falls_back_to_heuristics() {
        // unit 缺失：无 reset 的优先归 5h（5h 桶 0% 时可能没有 nextResetTime），其余按 reset 升序
        let data = serde_json::json!({"limits":[
            {"type":"TOKENS_LIMIT","percentage":25.0,"nextResetTime":2_000_000_000_000i64},
            {"type":"TOKENS_LIMIT","percentage":0.0}
        ]});
        let (windows, _) = parse_zhipu_windows(&data);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].key, "5h");
        assert_eq!(windows[0].used_percent, Some(0.0));
        assert!(windows[0].resets_at.is_none());
        assert_eq!(windows[1].key, "7d");
    }

    #[test]
    fn zhipu_old_plan_single_entry_and_type_case_insensitive() {
        // 老套餐只回 1 条；type 大小写不敏感；TIME_LIMIT 等其它类型跳过
        let data = serde_json::json!({"limits":[
            {"type":"tokens_limit","percentage":2.0,"nextResetTime":1_774_967_594_803i64},
            {"type":"TIME_LIMIT","percentage":7.0}
        ]});
        let (windows, _) = parse_zhipu_windows(&data);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].key, "5h");
        assert_eq!(windows[0].used_percent, Some(2.0));
    }

    #[test]
    fn zhipu_out_of_range_percentage_is_unknown_not_clamped() {
        // 岛内纪律：越界/非数 → None（不画条），区别于 cc-switch 的透传
        let data = serde_json::json!({"limits":[
            {"type":"TOKENS_LIMIT","unit":3,"percentage":150.0,"nextResetTime":1_000_000_000_000i64}
        ]});
        let (windows, _) = parse_zhipu_windows(&data);
        assert_eq!(windows[0].used_percent, None);
    }

    #[test]
    fn zhipu_business_error_maps_to_failed() {
        let state = zhipu_state_from_body(&serde_json::json!({"success":false,"msg":"invalid key"}));
        assert!(matches!(state, QuotaState::Failed { .. }));
    }

    #[test]
    fn kimi_limits_detail_and_weekly_usage() {
        let body = serde_json::json!({
            "limits": [{"detail": {"limit": 300, "remaining": 100, "resetTime": "2026-09-17T12:00:00Z"}}],
            "usage": {"limit": 1000, "remaining": 250, "resetTime": 1_800_000_000}
        });
        let windows = parse_kimi_windows(&body);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].key, "5h");
        assert_eq!(windows[0].used_percent, Some(200.0 / 300.0 * 100.0));
        assert!(windows[0].resets_at.as_deref().unwrap().starts_with("2026-09-17"));
        assert_eq!(windows[1].key, "7d");
        assert_eq!(windows[1].used_percent, Some(75.0));
        assert!(windows[1].resets_at.is_some()); // 秒级时间戳自动升为毫秒
    }

    #[test]
    fn kimi_zero_limit_counts_zero_and_missing_usage_skipped() {
        let windows = parse_kimi_windows(&serde_json::json!({
            "limits": [{"detail": {"limit": 0, "remaining": 0}}]
        }));
        assert_eq!(windows.len(), 1); // 顶层 usage 缺失 → 无周桶
        assert_eq!(windows[0].used_percent, Some(0.0)); // limit=0 时按 0 处理
        let empty = parse_kimi_windows(&serde_json::json!({}));
        assert!(empty.is_empty());
    }

    #[test]
    fn minimax_general_only_and_remaining_inverted() {
        // 只取 model_name=general（跳过 video）；剩余百分比反转为已用；周桶仅 status=1 展示
        let body = serde_json::json!({
            "model_remains": [
                {"model_name": "video", "current_interval_remaining_percent": 10.0},
                {"model_name": "general",
                 "current_interval_remaining_percent": 80.0, "end_time": 1_800_000_000_000i64,
                 "current_weekly_status": 1, "current_weekly_remaining_percent": 90.0,
                 "weekly_end_time": 1_805_000_000_000i64}
            ]
        });
        let windows = parse_minimax_windows(&body);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].key, "5h");
        assert_eq!(windows[0].used_percent, Some(20.0));
        assert!(windows[0].resets_at.is_some());
        assert_eq!(windows[1].key, "7d");
        assert_eq!(windows[1].used_percent, Some(10.0));
    }

    #[test]
    fn minimax_weekly_status_three_means_no_weekly_limit() {
        // status=3：该套餐无周限额（remaining 恒 100），不展示，避免常绿假象
        let body = serde_json::json!({"model_remains":[{"model_name":"general",
            "current_interval_remaining_percent":50.0,
            "current_weekly_status":3, "current_weekly_remaining_percent":100.0}]});
        let windows = parse_minimax_windows(&body);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].key, "5h");
    }

    #[test]
    fn zenmux_windows_with_usd_amounts_and_plan() {
        let data = serde_json::json!({
            "quota_5_hour": {"usage_percentage": 0.42, "resets_at": "2026-09-17T12:00:00Z", "used_value_usd": 4.2, "max_value_usd": 10.0},
            "quota_7_day": {"usage_percentage": 0.08, "resets_at": "2026-09-22T00:00:00Z"},
            "plan": {"tier": "pro"}, "account_status": "active"
        });
        let (windows, plan) = parse_zenmux_windows(&data);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].key, "5h");
        assert_eq!(windows[0].used_percent, Some(42.0)); // 0-1 小数 ×100
        assert_eq!(windows[0].amount_text.as_deref(), Some("$4.20 / $10.00"));
        assert_eq!(windows[1].amount_text, None);
        assert_eq!(plan.as_deref(), Some("pro (active)"));
    }

    #[test]
    fn zenmux_missing_windows_yield_empty() {
        let (windows, plan) = parse_zenmux_windows(&serde_json::json!({"plan":{"tier":"pro"}}));
        assert!(windows.is_empty());
        assert_eq!(plan.as_deref(), Some("pro")); // account_status 缺失时只显示档位，不渲染空括号
    }

    #[test]
    fn opencode_go_three_windows_and_rate_limited_pinned_at_100() {
        // percent=0 时上游 resetsAt 是「now+窗口时长」占位值，丢弃不展示倒计时；
        // status=rate-limited 时上游已把 percent 钉在 100，无需特判
        let body = serde_json::json!({"usage":{
            "rolling": {"status":"ok","percent":37,"resetsAt":"2026-09-17T12:00:00Z"},
            "weekly":  {"status":"ok","percent":10,"resetsAt":"2026-09-22T00:00:00Z"},
            "monthly": {"status":"rate-limited","percent":100,"resetsAt":"2026-10-01T00:00:00Z"}
        }});
        let windows = parse_opencode_windows(&body);
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].key, "5h");
        assert_eq!(windows[0].used_percent, Some(37.0));
        assert!(windows[0].resets_at.is_some());
        assert_eq!(windows[2].key, "30d");
        assert_eq!(windows[2].used_percent, Some(100.0));
    }

    #[test]
    fn opencode_go_zero_percent_drops_reset_placeholder() {
        let body = serde_json::json!({"usage":{
            "rolling": {"status":"ok","percent":0,"resetsAt":"2026-09-17T12:00:00Z"}
        }});
        let windows = parse_opencode_windows(&body);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].used_percent, Some(0.0));
        assert!(windows[0].resets_at.is_none()); // 占位重置时间不展示
    }

    #[test]
    fn opencode_go_legacy_flat_shape_is_unrecognized() {
        // 2026-08-11 上线当天即作废的旧扁平形态：整卡不识别
        let windows = parse_opencode_windows(&serde_json::json!({
            "rollingUsage": {"usagePercent": 37, "resetInSec": 3600}
        }));
        assert!(windows.is_empty());
    }

    #[test]
    #[ignore = "需要本机智谱 Key 与网络；仅显式执行"]
    fn local_zhipu_read_only_smoke() {
        let key = crate::creds::read_secret("claude", "api").expect("缺少本机 Key");
        let state = query_zhipu("https://open.bigmodel.cn/api/anthropic", &key);
        assert!(matches!(state, QuotaState::Ok { .. }), "{state:?}");
        if let QuotaState::Ok { windows, plan, .. } = state {
            for w in windows {
                println!("{} used={:?} amount={:?} has_reset={}", w.key, w.used_percent, w.amount_text, w.resets_at.is_some());
            }
            println!("plan={plan:?}");
        }
    }

    /* ------------------- ZCode 账号额度（离线夹具） ------------------- */

    fn iso(seconds: i64) -> String {
        chrono::DateTime::from_timestamp(seconds, 0).unwrap().to_rfc3339()
    }

    /// 实测形态（2026-11-05）：`GLM-5.3` 300 万 token/日已用 0、`GLM-5.3-Flash`
    /// 500 万 token/日已用 100 万，两条 bucket 属于同一个 plan。
    fn zcode_balance_body() -> serde_json::Value {
        serde_json::json!({
            "code": 0,
            "msg": "ok",
            "logid": "fixture-logid",
            "data": {
                "server_time": 1_762_339_200i64,
                "plans": [{
                    "user_plan_id": "up-1",
                    "plan_id": "zcode-v3-start-plan-0817",
                    "name": "ZCode Start Plan",
                    "description": "fixture",
                    "priority": 1,
                    "status": "active",
                    "starts_at": 1_762_000_000i64,
                    "ends_at": 1_762_372_799i64,
                    "entitlements": [
                        {"entitlement_id": "ent-1", "show_name": "GLM-5.3", "meter": "model_usage",
                         "unit_type": "token", "period": "daily", "grant_units": 3_000_000i64, "capabilities": []},
                        {"entitlement_id": "ent-2", "show_name": "GLM-5.3-Flash", "meter": "model_usage",
                         "unit_type": "token", "period": "daily", "grant_units": 5_000_000i64, "capabilities": []}
                    ]
                }],
                "balances": [
                    {"bucket_id": "bkt-1", "user_plan_id": "up-1", "plan_id": "zcode-v3-start-plan-0817",
                     "entitlement_id": "ent-1", "show_name": "GLM-5.3", "meter": "model_usage", "unit_type": "token",
                     "priority": 1, "plan_priority": 1, "entitlement_priority": 1,
                     "total_units": 3_000_000i64, "used_units": 0, "remaining_units": 3_000_000i64,
                     "available_units": 3_000_000i64,
                     "period_start": 1_762_300_000i64, "period_end": 1_762_339_199i64},
                    {"bucket_id": "bkt-2", "user_plan_id": "up-1", "plan_id": "zcode-v3-start-plan-0817",
                     "entitlement_id": "ent-2", "show_name": "GLM-5.3-Flash", "meter": "model_usage", "unit_type": "token",
                     "priority": 2, "plan_priority": 1, "entitlement_priority": 2,
                     "total_units": 5_000_000i64, "used_units": 1_000_000i64, "remaining_units": 4_000_000i64,
                     "available_units": 3_500_000i64,
                     "period_start": 1_762_300_000i64, "period_end": 1_762_339_199i64}
                ]
            }
        })
    }

    #[test]
    fn zcode_balance_maps_every_bucket_with_plan_and_reset() {
        let ZcodePlanBalanceOutcome::Parsed(balance) = parse_zcode_plan_balance(&zcode_balance_body()) else {
            panic!("实测形态必须解析成功");
        };
        assert_eq!(balance.plan.as_deref(), Some("ZCode Start Plan"));
        assert_eq!(balance.plan_status.as_deref(), Some("active"));
        assert_eq!(balance.plan_ends_at.as_deref(), Some(iso(1_762_372_799).as_str()));
        assert_eq!(balance.observed_at.as_deref(), Some(iso(1_762_339_200).as_str()));
        assert_eq!(balance.windows.len(), 2, "两个 bucket 不能被合并成一个窗口");
        // 真实 0：已用 0 不等于未知，进度条要能画在 0%
        assert_eq!(balance.windows[0].key, "bkt-1");
        assert_eq!(balance.windows[0].window_name, "GLM-5.3");
        assert_eq!(balance.windows[0].used_percent, Some(0.0));
        assert_eq!(balance.windows[0].amount_text.as_deref(), Some("0 / 3000000 token"));
        assert_eq!(balance.windows[0].remaining_text.as_deref(), Some("剩余 3000000 token"));
        assert_eq!(balance.windows[0].resets_at.as_deref(), Some(iso(1_762_339_199).as_str()));
        assert_eq!(balance.windows[1].key, "bkt-2");
        assert_eq!(balance.windows[1].used_percent, Some(20.0));
        // 剩余与可用不同时两个都写出来，界面不用二选一
        assert_eq!(balance.windows[1].remaining_text.as_deref(), Some("剩余 4000000 token（可用 3500000）"));
    }

    #[test]
    fn zcode_balance_zero_total_is_unknown_not_zero_percent() {
        let body = serde_json::json!({"code":0,"data":{"balances":[
            {"bucket_id":"bkt-zero","show_name":"GLM-5.3","unit_type":"token",
             "total_units":0,"used_units":0,"remaining_units":0,"available_units":0}
        ]}});
        let ZcodePlanBalanceOutcome::Parsed(balance) = parse_zcode_plan_balance(&body) else { panic!("应解析成功") };
        assert_eq!(balance.windows.len(), 1);
        // total_units=0 → 未知（不画条），既不是 0% 也不是满格
        assert_eq!(balance.windows[0].used_percent, None);
        assert_eq!(balance.windows[0].amount_text, None);
        assert_eq!(balance.windows[0].remaining_text.as_deref(), Some("剩余 0 token"));
        // 总量缺失同样是未知
        let body = serde_json::json!({"code":0,"data":{"balances":[{"bucket_id":"bkt-x","used_units":10}]}});
        let ZcodePlanBalanceOutcome::Parsed(balance) = parse_zcode_plan_balance(&body) else { panic!("应解析成功") };
        assert_eq!(balance.windows[0].used_percent, None);
        assert_eq!(balance.windows[0].window_name, "额度窗口 1", "无名称时用序号，不退化成空标题");
    }

    #[test]
    fn zcode_balance_keeps_sibling_buckets_and_dedupes_repeats() {
        let body = serde_json::json!({"code":0,"data":{"balances":[
            {"bucket_id":"bkt-a","entitlement_id":"ent-shared","show_name":"GLM-5.3","total_units":100,"used_units":10},
            {"bucket_id":"bkt-b","entitlement_id":"ent-shared","show_name":"GLM-5.3","total_units":200,"used_units":50},
            {"bucket_id":"bkt-a","entitlement_id":"ent-shared","show_name":"GLM-5.3","total_units":999,"used_units":999},
            {"entitlement_id":"ent-only","show_name":"GLM-5.3-Flash","total_units":10,"used_units":1}
        ]}});
        let ZcodePlanBalanceOutcome::Parsed(balance) = parse_zcode_plan_balance(&body) else { panic!("应解析成功") };
        let keys: Vec<&str> = balance.windows.iter().map(|window| window.key.as_str()).collect();
        // 同一 entitlement 的两个 bucket 各成一行；重复的 bucket_id 只留第一条；缺 bucket_id 时退回 entitlement_id
        assert_eq!(keys, vec!["bkt-a", "bkt-b", "ent-only"]);
        assert_eq!(balance.windows[0].used_percent, Some(10.0));
        assert_eq!(balance.windows[1].used_percent, Some(25.0));
    }

    #[test]
    fn zcode_balance_separates_no_data_from_unrecognised_shape() {
        // data 缺失 / null：成功但没有额度数据
        assert!(matches!(parse_zcode_plan_balance(&serde_json::json!({"code":0})), ZcodePlanBalanceOutcome::NoQuotaData));
        assert!(matches!(parse_zcode_plan_balance(&serde_json::json!({"code":0,"data":null})), ZcodePlanBalanceOutcome::NoQuotaData));
        // data 是空对象：也不是失败，只是没有窗口
        let ZcodePlanBalanceOutcome::Parsed(balance) = parse_zcode_plan_balance(&serde_json::json!({"code":0,"data":{}})) else {
            panic!("空 data 不应算查询失败");
        };
        assert!(balance.windows.is_empty());
        assert!(balance.plan.is_none() && balance.observed_at.is_none(), "没有数据就不编造套餐与观测时间");
        // 形态不认识 → 查询失败
        for body in [
            serde_json::json!({"code":0,"data":[]}),
            serde_json::json!({"code":0,"data":{"balances":"nope"}}),
            serde_json::json!({"code":0,"data":{"plans":"nope"}}),
        ] {
            assert!(matches!(parse_zcode_plan_balance(&body), ZcodePlanBalanceOutcome::Malformed(_)), "应判为形态不认识：{body}");
        }
        // 数组里的非对象条目跳过，不影响其他桶
        let body = serde_json::json!({"code":0,"data":{"balances":[null,"x",{"bucket_id":"bkt-ok"}]}});
        let ZcodePlanBalanceOutcome::Parsed(balance) = parse_zcode_plan_balance(&body) else { panic!("应解析成功") };
        assert_eq!(balance.windows.len(), 1);
    }

    #[test]
    fn zcode_balance_falls_back_to_entitlement_metadata_and_string_numbers() {
        let body = serde_json::json!({"code":0,"data":{
            "plans":[{"name":"ZCode Start Plan","status":"expired","entitlements":[
                {"entitlement_id":"ent-1","show_name":"GLM-5.3 日额度","unit_type":"token"}
            ]}],
            "balances":[{"bucket_id":"bkt-1","entitlement_id":"ent-1",
                "total_units":"1000000","used_units":"250000","remaining_units":"750000",
                "period_end":"1762339199"}]
        }});
        let ZcodePlanBalanceOutcome::Parsed(balance) = parse_zcode_plan_balance(&body) else { panic!("应解析成功") };
        // balance 条目没有 show_name / unit_type 时回填 entitlement 的定义
        assert_eq!(balance.windows[0].window_name, "GLM-5.3 日额度");
        assert_eq!(balance.windows[0].used_percent, Some(25.0));
        assert_eq!(balance.windows[0].amount_text.as_deref(), Some("250000 / 1000000 token"));
        assert_eq!(balance.windows[0].resets_at.as_deref(), Some(iso(1_762_339_199).as_str()));
        // 数字字符串与匹配 active 之外的 plan：只透传状态，不猜档位
        assert_eq!(balance.plan_status.as_deref(), Some("expired"));
        assert_eq!(balance.plan.as_deref(), Some("ZCode Start Plan"));
    }
}
