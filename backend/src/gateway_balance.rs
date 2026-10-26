//! OpenAI 兼容网关（new-api / one-api）的令牌余额，作为 sub2api `/v1/usage` 缺失时的兜底。
//!
//! | 网关 | 端点 | 单位 |
//! |---|---|---|
//! | new-api | `GET {root}/api/usage/token/` → `data.{total_available,total_used,unlimited_quota}` | 原始额度，÷ `/api/status` 的 `quota_per_unit` = 美元 |
//! | one-api | `GET {root}/v1/dashboard/billing/subscription` 的 `hard_limit_usd` 与 `.../usage` 的 `total_usage` | 美元；`total_usage` 是美分 |
//!
//! new-api 的 dashboard 端点会按站点显示方式换成人民币或 Token 数，单位不可靠，所以 new-api
//! 使用令牌额度与可选 `user_quota` 的较小值；不限额度且没有账号额度时返回不支持。
//! 只发 GET，凭证只发往用户填写的同一地址。

use crate::quota::{self, BalanceState, HttpOutcome};
use serde_json::Value;

/// one-api 对不限额度令牌返回的 hard_limit_usd
const UNLIMITED_USD: f64 = 100_000_000.0;

/// 去掉用户填写地址末尾的 `/v1…`，得到网关根地址
fn root(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    match base.rfind("/v1") {
        Some(index) if base[index + 3..].is_empty() || base[index + 3..].starts_with('/') => base[..index].to_string(),
        _ => base.to_string(),
    }
}

enum Fetched {
    Json(Value),
    /// 端点不存在或不是该类网关（404、非 JSON）：交给下一个来源
    Missing,
    Done(BalanceState),
}

fn get(url: &str, key: Option<&str>) -> Fetched {
    let Ok(client) = quota::client() else {
        return Fetched::Done(BalanceState::Failed { reason: "无法创建网关查询客户端".into() });
    };
    let mut request = client.get(url).header("Accept", "application/json");
    if let Some(key) = key { request = request.bearer_auth(key); }
    let Ok(response) = request.send() else {
        return Fetched::Done(BalanceState::Failed { reason: "网关余额查询失败，请检查网络后刷新".into() });
    };
    if response.status().as_u16() == 404 { return Fetched::Missing; }
    match quota::read(response) {
        HttpOutcome::Body(body) => serde_json::from_str(&body).map_or(Fetched::Missing, Fetched::Json),
        HttpOutcome::Unauthorized(reason) => Fetched::Done(BalanceState::Unauthorized { reason }),
        HttpOutcome::Forbidden(reason) => Fetched::Done(BalanceState::Forbidden { reason }),
        HttpOutcome::RateLimited(reason) => Fetched::Done(BalanceState::RateLimited { reason }),
        HttpOutcome::Failed(_) => Fetched::Done(BalanceState::Failed { reason: "网关余额接口查询失败".into() }),
    }
}

fn number(value: Option<&Value>) -> Option<f64> {
    value.and_then(|v| v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())).filter(|n: &f64| n.is_finite())
}

fn round_cents(value: f64) -> f64 {
    if value.abs() > f64::MAX / 100.0 { value } else { (value * 100.0).round() / 100.0 }
}

/// new-api `/api/usage/token/`：`code` 为 true 才有数据；不限额度没有余额
fn parse_new_api(body: &Value, status: Option<&Value>) -> Option<BalanceState> {
    if body.get("code").and_then(Value::as_bool) != Some(true) {
        // 业务失败不等同于鉴权失效；不把上游自由文本带回界面，避免泄露令牌。
        return (body.get("code").and_then(Value::as_bool) == Some(false)).then(|| BalanceState::Failed {
            reason: "网关余额查询失败".into(),
        });
    }
    let invalid = || Some(BalanceState::Failed { reason: "网关额度或换算比例缺失、无效，无法确认余额".into() });
    let Some(data) = body.get("data").filter(|value| value.is_object()) else { return invalid() };
    let unlimited = match data.get("unlimited_quota") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        _ => return invalid(),
    };
    let mut available = if unlimited {
        f64::INFINITY
    } else {
        let Some(available) = number(data.get("total_available")) else { return invalid() };
        available
    };
    if let Some(user_quota) = data.get("user_quota").filter(|value| !value.is_null()) {
        let Some(user_available) = number(user_quota.get("total_available")) else { return invalid() };
        available = available.min(user_available);
    }
    if !available.is_finite() {
        return Some(BalanceState::Unsupported { reason: "该令牌在网关上不限额度，没有可显示的余额".into() });
    }
    let Some(quota_per_usd) = number(status.and_then(|status| status.pointer("/data/quota_per_unit"))).filter(|n| *n > 0.0) else {
        return invalid();
    };
    if data.get("total_used").is_some_and(|value| !value.is_null() && number(Some(value)).is_none_or(|used| used < 0.0)) {
        return Some(BalanceState::Failed { reason: "网关已用额度无效".into() });
    }
    let balance = round_cents(available / quota_per_usd);
    let used = number(data.get("total_used")).map(|used| round_cents(used / quota_per_usd));
    if !balance.is_finite() || used.is_some_and(|used| !used.is_finite()) { return invalid(); }
    Some(BalanceState::Ok {
        balance,
        currency: "USD".into(),
        used,
    })
}

/// one-api dashboard：余额 = hard_limit_usd − total_usage / 100
fn parse_dashboard(subscription: &Value, usage: &Value) -> Option<BalanceState> {
    let limit = number(subscription.get("hard_limit_usd"))?;
    if limit < 0.0 {
        return Some(BalanceState::Failed { reason: "网关额度上限无效".into() });
    }
    if limit >= UNLIMITED_USD {
        return Some(BalanceState::Unsupported { reason: "该令牌在网关上不限额度，没有可显示的余额".into() });
    }
    let used = number(usage.get("total_usage"))? / 100.0;
    if used < 0.0 {
        return Some(BalanceState::Failed { reason: "网关已用额度无效".into() });
    }
    Some(BalanceState::Ok { balance: round_cents(limit - used), currency: "USD".into(), used: Some(round_cents(used)) })
}

/// 依次尝试 new-api 与 one-api；都不像时返回 None，调用方保留原来的「不支持」原因
pub(crate) fn gateway_billing_balance(base: &str, key: &str) -> Option<BalanceState> {
    let root = root(base);
    match get(&format!("{root}/api/usage/token/"), Some(key)) {
        Fetched::Done(state) => return Some(state),
        Fetched::Json(body) => {
            let status = if body.get("code").and_then(Value::as_bool) == Some(true) {
                // 公开站点配置无需认证，绝不附带用户的 API Key。
                match get(&format!("{root}/api/status"), None) {
                    Fetched::Json(status) => Some(status),
                    Fetched::Done(state) => return Some(state),
                    Fetched::Missing => None,
                }
            } else { None };
            if let Some(state) = parse_new_api(&body, status.as_ref()) { return Some(state) }
        },
        Fetched::Missing => {}
    }
    let subscription = match get(&format!("{root}/v1/dashboard/billing/subscription"), Some(key)) {
        Fetched::Json(body) => body,
        Fetched::Done(state) => return Some(state),
        Fetched::Missing => return None,
    };
    match get(&format!("{root}/v1/dashboard/billing/usage"), Some(key)) {
        Fetched::Json(usage) => parse_dashboard(&subscription, &usage),
        Fetched::Done(state) => Some(state),
        Fetched::Missing => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    #[ignore = "需要鼠鼠授权：读取本机 Claude API 配置，仅向已配置服务发 GET，只输出状态、币种与字段是否存在"]
    fn local_claude_gateway_balance_read_only_smoke() {
        fn kind(state: &BalanceState) -> &'static str {
            match state {
                BalanceState::Ok { .. } => "ok",
                BalanceState::Unsupported { .. } => "unsupported",
                BalanceState::Unauthorized { .. } => "unauthorized",
                BalanceState::Forbidden { .. } => "forbidden",
                BalanceState::RateLimited { .. } => "rate_limited",
                BalanceState::Failed { .. } => "failed",
            }
        }
        let key = crate::creds::read_secret("claude", "api").expect("未找到本机 Claude API 凭证");
        let base = crate::creds::local_base_url("claude", "api").expect("本机 Claude 未配置网关地址");
        // 与 connection_balance 使用同样的官方套餐排除和网关回退条件。
        if crate::coding_plan::detect_provider(&base).is_some() {
            println!("Claude gateway product: state=unsupported currency=unknown balance_present=false used_present=false");
            panic!("配置属于编程套餐，产品不提供钱包余额查询");
        }
        let primary = quota::sub2api_balance(&base, &key);
        println!("Claude gateway sub2api: state={}", kind(&primary));
        let state = match primary {
            BalanceState::Unsupported { reason } if reason == quota::S2_ENDPOINT_MISSING => {
                gateway_billing_balance(&base, &key).unwrap_or(BalanceState::Unsupported { reason })
            }
            state => state,
        };
        let state_kind = kind(&state);
        let (currency, balance_present, used_present) = match &state {
            BalanceState::Ok { balance, currency, used } => {
                let unit = if currency.len() == 3 && currency.bytes().all(|byte| byte.is_ascii_alphabetic()) { currency.as_str() } else { "unknown" };
                (unit, balance.is_finite(), used.is_some_and(f64::is_finite))
            }
            _ => ("unknown", false, false),
        };
        println!("Claude gateway product: state={state_kind} currency={currency} balance_present={balance_present} used_present={used_present}");
        assert!(balance_present, "产品余额链路未返回有效余额，state={state_kind}");
    }

    #[test]
    #[ignore = "需要鼠鼠授权：读取本机 Claude API 配置，向已识别套餐发只读 GET，只输出状态、窗口数与重置字段是否存在"]
    fn local_claude_coding_plan_read_only_smoke() {
        let key = crate::creds::read_secret("claude", "api").expect("未找到本机 Claude API 凭证");
        let base = crate::creds::local_base_url("claude", "api").expect("本机 Claude 未配置套餐地址");
        let provider = match crate::coding_plan::detect_provider(&base).expect("配置未识别为编程套餐，测试不适用") {
            crate::coding_plan::PlanProvider::ZhipuCn => "zhipu_cn",
            crate::coding_plan::PlanProvider::ZhipuEn => "zhipu_en",
            crate::coding_plan::PlanProvider::Kimi => "kimi",
            crate::coding_plan::PlanProvider::MiniMaxCn => "minimax_cn",
            crate::coding_plan::PlanProvider::MiniMaxEn => "minimax_en",
            crate::coding_plan::PlanProvider::ZenMux => "zenmux",
            crate::coding_plan::PlanProvider::OpencodeGo => "opencode_go",
            crate::coding_plan::PlanProvider::Volcengine => "volcengine",
        };
        println!("Claude coding plan: provider_class={provider}");
        let state = crate::coding_plan::coding_plan_quota(&base, &key);
        if let quota::QuotaState::Failed { reason } = &state {
            // 只匹配代码生成的前缀/固定字段说明，绝不打印上游自由文本或解析错误正文。
            let category = if reason.starts_with("智谱套餐接口错误:") || reason.starts_with("MiniMax 套餐接口错误") || reason.starts_with("ZenMux 套餐接口错误:") {
                "business_error"
            } else if reason.contains("响应格式无法解析:") {
                "json_parse_error"
            } else if reason.contains("缺少 data 字段") {
                "missing_data_field"
            } else if reason.contains("未返回可识别") || reason.contains("用量响应形态不认识") {
                "unrecognized_quota_fields"
            } else if reason.ends_with("查询超时（15 秒）") {
                "network_timeout"
            } else if reason.ends_with("查询失败，请检查网络后刷新") {
                "network_error"
            } else { "unclassified_failure" };
            println!("Claude coding plan: failure_class={category}");
            for code in 100..=599 {
                for prefix in ["智谱套餐查询失败", "Kimi 用量查询失败", "MiniMax 用量查询失败", "ZenMux 用量查询失败", "OpenCode Go 用量查询失败"] {
                    if reason == &format!("{prefix}: 服务端错误（HTTP {code}）") || reason == &format!("{prefix}: 查询失败（HTTP {code}）") {
                        println!("Claude coding plan: http_status={code}");
                    }
                }
            }
        }
        let (kind, windows, resets_present) = match &state {
            quota::QuotaState::Ok { windows, .. } => ("ok", windows.len(), windows.iter().any(|window| window.resets_at.is_some())),
            quota::QuotaState::Unsupported { .. } => ("unsupported", 0, false),
            quota::QuotaState::Unauthorized { .. } => ("unauthorized", 0, false),
            quota::QuotaState::Forbidden { .. } => ("forbidden", 0, false),
            quota::QuotaState::RateLimited { .. } => ("rate_limited", 0, false),
            quota::QuotaState::Failed { .. } => ("failed", 0, false),
        };
        println!("Claude coding plan: state={kind} windows={windows} resets_present={resets_present}");
        assert!(kind == "ok" && windows > 0, "套餐查询未返回额度窗口，state={kind}");
    }

    fn parse_new_api(body: &Value) -> Option<BalanceState> {
        super::parse_new_api(body, Some(&json!({"data":{"quota_per_unit":500000}})))
    }

    #[test]
    fn root_strips_only_a_trailing_v1_segment() {
        assert_eq!(root("https://gw.example.test/v1"), "https://gw.example.test");
        assert_eq!(root("https://gw.example.test/v1/"), "https://gw.example.test");
        assert_eq!(root("https://gw.example.test/proxy/v1/chat"), "https://gw.example.test/proxy");
        assert_eq!(root("https://gw.example.test"), "https://gw.example.test");
        assert_eq!(root("https://gw.example.test/v10"), "https://gw.example.test/v10");
    }

    #[test]
    fn new_api_converts_quota_and_keeps_real_zero() {
        let ok = parse_new_api(&json!({"code":true,"data":{"total_available":0,"total_used":1_250_000,"unlimited_quota":false}})).unwrap();
        match ok {
            BalanceState::Ok { balance, used, currency } => {
                assert_eq!(balance, 0.0);
                assert_eq!(used, Some(2.5));
                assert_eq!(currency, "USD");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(parse_new_api(&json!({"code":true,"data":{"unlimited_quota":true}})), Some(BalanceState::Unsupported { .. })));
        assert!(matches!(parse_new_api(&json!({"code":false,"message":"无效的令牌"})), Some(BalanceState::Failed { .. })));
        // 不是 new-api 的 JSON：交给下一个来源
        assert!(parse_new_api(&json!({"object":"list"})).is_none());
        assert!(parse_new_api(&json!({"code":404,"message":"not found"})).is_none());
        assert!(matches!(parse_new_api(&json!({"code":true,"data":{}})), Some(BalanceState::Failed { .. })));
    }

    #[test]
    fn dashboard_uses_cents_and_detects_unlimited() {
        match parse_dashboard(&json!({"hard_limit_usd":20.0}), &json!({"total_usage":525.0})).unwrap() {
            BalanceState::Ok { balance, used, .. } => assert_eq!((balance, used), (14.75, Some(5.25))),
            other => panic!("{other:?}"),
        }
        assert!(matches!(parse_dashboard(&json!({"hard_limit_usd":100000000.0}), &json!({"total_usage":0})), Some(BalanceState::Unsupported { .. })));
        assert!(parse_dashboard(&json!({}), &json!({"total_usage":1})).is_none());
    }

    #[test]
    fn gateway_does_not_turn_negative_or_invalid_values_into_zero() {
        match parse_new_api(&json!({"code":true,"data":{"total_available":-500000,"total_used":0}})).unwrap() {
            BalanceState::Ok { balance, .. } => assert_eq!(balance, -1.0),
            other => panic!("{other:?}"),
        }
        assert!(matches!(parse_new_api(&json!({"code":true,"data":{"total_available":0,"total_used":-1}})), Some(BalanceState::Failed { .. })));
        assert!(matches!(parse_dashboard(&json!({"hard_limit_usd":10}), &json!({"total_usage":-1})), Some(BalanceState::Failed { .. })));
        match parse_dashboard(&json!({"hard_limit_usd":10}), &json!({"total_usage":1200})).unwrap() {
            BalanceState::Ok { balance, .. } => assert_eq!(balance, -2.0),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn new_api_business_error_does_not_expose_upstream_message_or_assume_auth_failure() {
        let state = parse_new_api(&json!({"code":false,"message":"database error containing secret-fixture"})).unwrap();
        assert!(matches!(state, BalanceState::Failed { .. }));
        assert!(!serde_json::to_string(&state).unwrap().contains("secret-fixture"));
    }

    #[test]
    fn new_api_uses_site_conversion_and_account_cap() {
        let body = json!({"code":true,"data":{"total_available":1000000,"total_used":250000,"user_quota":{"total_available":200000}}});
        let status = json!({"data":{"quota_per_unit":100000}});
        match super::parse_new_api(&body, Some(&status)).unwrap() {
            BalanceState::Ok { balance, used, .. } => assert_eq!((balance, used), (2.0, Some(2.5))),
            other => panic!("{other:?}"),
        }
        let unlimited = json!({"code":true,"data":{"unlimited_quota":true,"user_quota":{"total_available":200000}}});
        assert!(matches!(super::parse_new_api(&unlimited, Some(&status)), Some(BalanceState::Ok { balance: 2.0, .. })));
    }

    #[test]
    fn new_api_requires_valid_conversion_and_complete_balance() {
        let body = json!({"code":true,"data":{"total_available":500000}});
        for status in [None, Some(json!({"data":{}})), Some(json!({"data":{"quota_per_unit":0}})), Some(json!({"data":{"quota_per_unit":-1}})), Some(json!({"data":{"quota_per_unit":"NaN"}}))] {
            assert!(matches!(super::parse_new_api(&body, status.as_ref()), Some(BalanceState::Failed { .. })));
        }
        for data in [json!({}), json!({"total_available":"bad"}), json!({"total_available":100,"user_quota":{}})] {
            assert!(matches!(parse_new_api(&json!({"code":true,"data":data})), Some(BalanceState::Failed { .. })));
        }
    }

    #[test]
    fn server_failure_is_not_a_missing_gateway_and_public_status_has_no_credentials() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
                        let mut bytes = [0; 4096];
                        let length = stream.read(&mut bytes).unwrap();
                        stream.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                        return String::from_utf8_lossy(&bytes[..length]).to_ascii_lowercase();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(10)),
                    Err(error) => panic!("local test server: {error}"),
                }
            }
        });
        let result = get(&format!("http://{address}/api/status"), None);
        let request = server.join().unwrap();
        assert!(request.starts_with("get /api/status "));
        assert!(!request.contains("authorization:"));
        assert!(matches!(result, Fetched::Done(BalanceState::Failed { .. })));
    }
}
