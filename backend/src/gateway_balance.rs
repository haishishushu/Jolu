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

/// 一次网关 GET 拿到的 HTTP 结果：状态码，以及按 `quota::read` 解读出来的响应。
/// 404 要单独当成「不是这类网关」，所以状态码不能丢。
struct GatewayReply {
    status: u16,
    outcome: HttpOutcome,
}

/// 没拿到 HTTP 响应的两类原因，分别对应抽取前那两条不同的文案。
enum GatewayError {
    /// 建不出 HTTP 客户端
    Client,
    /// 连接被拒、读超时、TLS 握手失败等：请求没有走到 HTTP 响应
    Transport,
}

/// 唯一碰 socket 的边界。生产实现是 `ReqwestGateway`；测试注入假实现，
/// 不再为一条断言起本地 TCP 服务器。
trait GatewayHttp {
    /// 发一次 GET，固定带 `Accept: application/json`；`key` 为 `None` 时不发凭据。
    fn get(&self, url: &str, key: Option<&str>) -> Result<GatewayReply, GatewayError>;
}

/// 生产实现：与抽取前同一串 reqwest 调用，失败按「建不出客户端」与「请求失败」两类返回。
struct ReqwestGateway;

impl GatewayHttp for ReqwestGateway {
    fn get(&self, url: &str, key: Option<&str>) -> Result<GatewayReply, GatewayError> {
        let client = quota::client().map_err(|_| GatewayError::Client)?;
        let mut request = client.get(url).header("Accept", "application/json");
        if let Some(key) = key { request = request.bearer_auth(key); }
        let response = request.send().map_err(|_| GatewayError::Transport)?;
        let status = response.status().as_u16();
        Ok(GatewayReply { status, outcome: quota::read(response) })
    }
}

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

fn get(gateway: &impl GatewayHttp, url: &str, key: Option<&str>) -> Fetched {
    let reply = match gateway.get(url, key) {
        Ok(reply) => reply,
        Err(GatewayError::Client) => return Fetched::Done(BalanceState::Failed { reason: "无法创建网关查询客户端".into() }),
        Err(GatewayError::Transport) => return Fetched::Done(BalanceState::Failed { reason: "网关余额查询失败，请检查网络后刷新".into() }),
    };
    if reply.status == 404 { return Fetched::Missing; }
    match reply.outcome {
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
    gateway_billing_balance_with(&ReqwestGateway, base, key)
}

/// 与 `gateway_billing_balance` 完全同一条流程，只是网络层由调用方注入：
/// 生产传 `ReqwestGateway`，测试传假网关。
fn gateway_billing_balance_with(gateway: &impl GatewayHttp, base: &str, key: &str) -> Option<BalanceState> {
    let root = root(base);
    match get(gateway, &format!("{root}/api/usage/token/"), Some(key)) {
        Fetched::Done(state) => return Some(state),
        Fetched::Json(body) => {
            let status = if body.get("code").and_then(Value::as_bool) == Some(true) {
                // 公开站点配置无需认证，绝不附带用户的 API Key。
                match get(gateway, &format!("{root}/api/status"), None) {
                    Fetched::Json(status) => Some(status),
                    Fetched::Done(state) => return Some(state),
                    Fetched::Missing => None,
                }
            } else { None };
            if let Some(state) = parse_new_api(&body, status.as_ref()) { return Some(state) }
        },
        Fetched::Missing => {}
    }
    let subscription = match get(gateway, &format!("{root}/v1/dashboard/billing/subscription"), Some(key)) {
        Fetched::Json(body) => body,
        Fetched::Done(state) => return Some(state),
        Fetched::Missing => return None,
    };
    match get(gateway, &format!("{root}/v1/dashboard/billing/usage"), Some(key)) {
        Fetched::Json(usage) => parse_dashboard(&subscription, &usage),
        Fetched::Done(state) => Some(state),
        Fetched::Missing => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io;

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

    /// 假网关返回给注入点的一条结果。
    enum FakeReply {
        /// 拿到了 HTTP 响应：状态码 + `quota::read` 会给出的解读结果
        Reply(u16, HttpOutcome),
        /// 没拿到响应（连接被拒、读超时、TLS 失败）
        Transport(io::ErrorKind),
    }

    impl FakeReply {
        /// 200 + JSON 正文
        fn json(body: &str) -> Self {
            FakeReply::Reply(200, HttpOutcome::Body(body.to_string()))
        }

        /// 服务端错误：`quota::read` 对 5xx 给出的就是 `Failed`，上游正文不会进入这个模块
        fn server_error(detail: &str) -> Self {
            FakeReply::Reply(500, HttpOutcome::Failed(detail.to_string()))
        }
    }

    /// 一次注入调用：URL、是否携带凭据、以及假网关这次模拟的传输失败种类。
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct FakeCall {
        url: String,
        key: bool,
        transport: Option<io::ErrorKind>,
    }

    /// 注入用的网关边界：`reply` 决定每次返回什么，`calls` 记下真实发生过的请求。
    struct FakeGateway<F> {
        reply: F,
        calls: std::sync::Mutex<Vec<FakeCall>>,
    }

    impl<F: Fn(&str) -> FakeReply> FakeGateway<F> {
        fn new(reply: F) -> Self {
            Self { reply, calls: std::sync::Mutex::new(Vec::new()) }
        }

        fn calls(&self) -> Vec<FakeCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl<F: Fn(&str) -> FakeReply> GatewayHttp for FakeGateway<F> {
        fn get(&self, url: &str, key: Option<&str>) -> Result<GatewayReply, GatewayError> {
            let reply = (self.reply)(url);
            let transport = match &reply {
                FakeReply::Transport(kind) => Some(*kind),
                FakeReply::Reply(..) => None,
            };
            self.calls.lock().unwrap().push(FakeCall { url: url.to_string(), key: key.is_some(), transport });
            match reply {
                FakeReply::Reply(status, outcome) => Ok(GatewayReply { status, outcome }),
                FakeReply::Transport(_) => Err(GatewayError::Transport),
            }
        }
    }

    /// 断言是「服务器失败」：文案固定，且不回显给定原文。
    fn assert_server_failure(state: &BalanceState, expected_reason: &str, leaked: &[&str]) {
        assert!(matches!(state, BalanceState::Failed { .. }), "应判成服务器失败，实际 {state:?}");
        let json = serde_json::to_string(state).expect("序列化状态");
        assert!(json.contains(expected_reason), "文案不对: {json}");
        for text in leaked {
            assert!(!json.contains(text), "输出回显了上游原文 `{text}`: {json}");
        }
    }

    #[test]
    fn refused_connection_is_a_server_failure_not_a_missing_gateway() {
        // reqwest 把 WSAECONNREFUSED 报成 send 失败；这里注入同一类传输失败。
        let gateway = FakeGateway::new(|_: &str| FakeReply::Transport(io::ErrorKind::ConnectionRefused));
        let state = gateway_billing_balance_with(&gateway, "https://gw.example.test/v1", "sk-live-fixture")
            .expect("连接被拒必须报成服务器失败，不能退化成「缺少网关」");
        assert_server_failure(&state, "网关余额查询失败，请检查网络后刷新", &["sk-live-fixture"]);
        assert_eq!(gateway.calls(), vec![FakeCall {
            url: "https://gw.example.test/api/usage/token/".into(),
            key: true,
            transport: Some(io::ErrorKind::ConnectionRefused),
        }]);
    }

    #[test]
    fn read_timeout_is_a_server_failure_not_a_missing_gateway() {
        // 网关在超时时间内没回包，reqwest 同样只能报 send 失败；不能被当成「不是这类网关」。
        let gateway = FakeGateway::new(|_: &str| FakeReply::Transport(io::ErrorKind::TimedOut));
        let state = gateway_billing_balance_with(&gateway, "https://gw.example.test/v1", "sk-live-fixture")
            .expect("读超时必须报成服务器失败，不能退化成「缺少网关」");
        assert_server_failure(&state, "网关余额查询失败，请检查网络后刷新", &["sk-live-fixture"]);
        assert_eq!(gateway.calls(), vec![FakeCall {
            url: "https://gw.example.test/api/usage/token/".into(),
            key: true,
            transport: Some(io::ErrorKind::TimedOut),
        }]);
    }

    #[test]
    fn http_500_with_an_error_body_is_a_server_failure_and_hides_the_upstream_text() {
        let gateway = FakeGateway::new(|_: &str| FakeReply::server_error("服务端错误（HTTP 500）: upstream secret-fixture"));
        let state = gateway_billing_balance_with(&gateway, "https://gw.example.test/v1", "sk-live-fixture")
            .expect("500 必须报成服务器失败，不能退化成「缺少网关」");
        assert_server_failure(&state, "网关余额接口查询失败", &["secret-fixture", "HTTP 500", "sk-live-fixture"]);
        assert_eq!(gateway.calls().len(), 1, "首个端点就失败，不该再查 one-api");
    }

    #[test]
    fn healthy_gateway_parses_and_the_public_status_request_carries_no_credentials() {
        let gateway = FakeGateway::new(|url: &str| match url {
            url if url.ends_with("/api/usage/token/") => FakeReply::json(
                r#"{"code":true,"data":{"total_available":1000000,"total_used":250000,"unlimited_quota":false}}"#,
            ),
            url if url.ends_with("/api/status") => FakeReply::json(r#"{"data":{"quota_per_unit":100000}}"#),
            other => panic!("未预期的网关地址: {other}"),
        });
        let state = gateway_billing_balance_with(&gateway, "https://gw.example.test/v1", "sk-live-fixture")
            .expect("正常的 new-api 响应应解析出余额");
        let json = serde_json::to_string(&state).expect("序列化状态");
        assert!(!json.contains("sk-live-fixture"), "输出回显了 API Key: {json}");
        match &state {
            BalanceState::Ok { balance, used, currency } => {
                assert_eq!((*balance, *used, currency.as_str()), (10.0, Some(2.5), "USD"));
            }
            other => panic!("{other:?}"),
        }
        let calls = gateway.calls();
        let urls: Vec<&str> = calls.iter().map(|call| call.url.as_str()).collect();
        assert_eq!(urls, ["https://gw.example.test/api/usage/token/", "https://gw.example.test/api/status"]);
        assert!(calls[0].key, "令牌端点需要带凭据");
        assert!(!calls[1].key, "公开站点配置绝不能带用户的 API Key");
    }

    /// 真实 socket 的端到端用例，只在手动运行时执行：注入测试覆盖不到 reqwest 那条链路。
    /// Windows 上 `accept` 出来的 socket 会继承 listener 的非阻塞模式，必须显式切回阻塞，
    /// 否则 `read` 立刻返回 `WouldBlock(10035)`；这里把超时放宽到 10 秒，且只在这个用例里放宽。
    #[test]
    #[ignore = "真实 socket 端到端：手动运行 cargo test gateway_balance -- --ignored"]
    fn real_socket_gateway_still_works_end_to_end() {
        use std::io::Write;
        use std::time::{Duration, Instant};

        const SOCKET_TIMEOUT: Duration = Duration::from_secs(10);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定本地端口");
        let address = listener.local_addr().expect("读取本地端口");
        listener.set_nonblocking(true).expect("监听改为非阻塞");
        let deadline = Instant::now() + Duration::from_secs(30);
        let server = std::thread::spawn(move || {
            let mut requests: Vec<String> = Vec::new();
            while requests.len() < 2 && Instant::now() < deadline {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("本地测试服务器 accept 失败: {error}"),
                };
                stream.set_nonblocking(false).expect("连接切回阻塞模式");
                stream.set_read_timeout(Some(SOCKET_TIMEOUT)).expect("设置读超时");
                stream.set_write_timeout(Some(SOCKET_TIMEOUT)).expect("设置写超时");
                let request = read_request_headers(&mut stream);
                let body = if request.starts_with("get /api/usage/token/ ") {
                    r#"{"code":true,"data":{"total_available":1000000,"total_used":250000,"unlimited_quota":false}}"#
                } else if request.starts_with("get /api/status ") {
                    r#"{"data":{"quota_per_unit":100000}}"#
                } else {
                    ""
                };
                let status = if body.is_empty() { "404 Not Found" } else { "200 OK" };
                write!(
                    &mut stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .expect("写响应");
                stream.flush().expect("刷新响应");
                requests.push(request);
            }
            requests
        });
        let state = gateway_billing_balance(&format!("http://{address}/v1"), "sk-real-socket-fixture")
            .expect("真实 socket 路径把本地 200 判成了缺少网关");
        let requests = server.join().expect("本地测试服务器线程");
        assert_eq!(requests.len(), 2, "应依次请求令牌端点与公开状态端点");
        assert!(requests[0].starts_with("get /api/usage/token/ "), "{}", requests[0]);
        assert!(requests[0].contains("authorization:"), "令牌请求没带凭据: {}", requests[0]);
        assert!(requests[1].starts_with("get /api/status "), "{}", requests[1]);
        assert!(!requests[1].contains("authorization:"), "公开状态请求带了凭据: {}", requests[1]);
        match &state {
            BalanceState::Ok { balance, used, .. } => assert_eq!((*balance, *used), (10.0, Some(2.5))),
            other => panic!("真实 socket 路径未解析出余额: {other:?}"),
        }
    }

    /// 读到请求头结束（`\r\n\r\n`）为止，返回小写文本，便于按路径与头名匹配。
    fn read_request_headers(stream: &mut std::net::TcpStream) -> String {
        use std::io::Read;
        let mut bytes = Vec::new();
        let mut chunk = [0; 2048];
        while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(length) => bytes.extend_from_slice(&chunk[..length]),
                Err(error) => panic!("本地测试服务器读取请求失败: {error}"),
            }
        }
        String::from_utf8_lossy(&bytes).to_ascii_lowercase()
    }
}
