//! 自定义及兼容服务的只读余额。已知官方端点按精确主机识别，其余仅探测配置的网关。
use crate::quota::{self, BalanceState, HttpOutcome};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Provider { DeepSeek, SiliconFlow, OpenRouter }

fn failed(reason: &str) -> BalanceState { BalanceState::Failed { reason: reason.into() } }

fn route(base: &str) -> Result<Option<(Provider, String)>, BalanceState> {
    let url = url::Url::parse(base.trim()).map_err(|_| failed("余额查询地址无效"))?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none()
        || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some()
    { return Err(failed("余额查询地址须为 HTTP(S) 服务地址，不能包含账号、查询参数或片段")); }
    // 不把自定义代理、非标准端口或相似域名当作官方服务。
    if url.scheme() != "https" || url.port_or_known_default() != Some(443) { return Ok(None); }
    let path = url.path().trim_end_matches('/');
    let found = match (url.host_str().unwrap_or_default(), path) {
        ("api.deepseek.com", "" | "/v1") => (Provider::DeepSeek, "/user/balance"),
        ("api.siliconflow.cn", "" | "/v1") => (Provider::SiliconFlow, "/v1/user/info"),
        ("openrouter.ai", "" | "/api/v1") => (Provider::OpenRouter, "/api/v1/credits"),
        _ => return Ok(None),
    };
    Ok(Some((found.0, format!("{}{}", url.origin().ascii_serialization(), found.1))))
}

fn number(value: Option<&Value>) -> Option<f64> {
    value.and_then(|v| v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())).filter(|n| n.is_finite())
}

fn parse(provider: Provider, body: &Value) -> BalanceState {
    let bad = || failed("服务返回的余额字段缺失或无效，不能视为零");
    let (balance, currency, used) = match provider {
        Provider::DeepSeek => {
            let Some(items) = body.get("balance_infos").and_then(Value::as_array) else { return bad(); };
            if items.len() > 1 { return BalanceState::Unsupported { reason: "该账号返回多个币种余额，不能合并为单一金额，请在服务后台分别查看".into() }; }
            let Some(item) = items.first() else { return bad(); };
            let Some(balance) = number(item.get("total_balance")) else { return bad(); };
            let Some(currency @ ("CNY" | "USD")) = item.get("currency").and_then(Value::as_str) else { return bad(); };
            (balance, currency, None)
        }
        Provider::SiliconFlow => {
            if body.get("status").and_then(Value::as_bool) == Some(false)
                || (body.get("status").and_then(Value::as_bool) != Some(true) && body.get("code").and_then(Value::as_i64) != Some(20000))
            { return failed("硅基流动余额接口返回业务错误"); }
            let Some(balance) = number(body.pointer("/data/totalBalance")) else { return bad(); };
            (balance, "CNY", None)
        }
        Provider::OpenRouter => {
            let Some(total) = number(body.pointer("/data/total_credits")).filter(|n| *n >= 0.0) else { return bad(); };
            let Some(used) = number(body.pointer("/data/total_usage")).filter(|n| *n >= 0.0) else { return bad(); };
            let balance = total - used;
            if !balance.is_finite() { return bad(); }
            (balance, "USD", Some(used))
        }
    };
    BalanceState::Ok { balance, currency: currency.into(), used }
}
fn unsupported() -> BalanceState { BalanceState::Unsupported { reason: "服务未提供可用的余额查询接口".into() } }

pub(crate) fn query(base: &str, key: &str) -> BalanceState {
    if key.trim().is_empty() { return BalanceState::Unauthorized { reason: "余额查询尚未配置凭证".into() }; }
    match route(base) {
        Err(state) => state,
        Ok(Some((provider, endpoint))) => official(provider, &endpoint, key),
        Ok(None) => match quota::sub2api_balance(base, key) {
            BalanceState::Unsupported { reason } if reason == quota::S2_ENDPOINT_MISSING => {
                crate::gateway_balance::gateway_billing_balance(base, key).unwrap_or_else(unsupported)
            }
            state => state,
        },
    }
}

fn official(provider: Provider, endpoint: &str, key: &str) -> BalanceState {
    let Ok(client) = quota::client() else { return failed("无法创建余额查询客户端"); };
    let Ok(response) = client.get(endpoint).bearer_auth(key).header("Accept", "application/json").send() else {
        return failed("余额查询失败，请检查网络后刷新");
    };
    match quota::read(response) {
        HttpOutcome::Body(body) => match serde_json::from_str(&body) {
            Ok(body) => parse(provider, &body),
            Err(_) => failed("余额接口响应格式无法识别"),
        },
        HttpOutcome::Unauthorized(reason) => BalanceState::Unauthorized { reason },
        HttpOutcome::Forbidden(_) if provider == Provider::OpenRouter => BalanceState::Forbidden { reason: "OpenRouter 账户余额接口要求管理密钥，普通模型 Key 没有此权限".into() },
        HttpOutcome::Forbidden(reason) => BalanceState::Forbidden { reason },
        HttpOutcome::RateLimited(reason) => BalanceState::RateLimited { reason },
        HttpOutcome::Failed(reason) => BalanceState::Failed { reason },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn amount(state: BalanceState, expected: f64, currency: &str, used: Option<f64>) {
        match state {
            BalanceState::Ok { balance, currency: actual, used: actual_used } => {
                assert_eq!(balance, expected); assert_eq!(actual, currency); assert_eq!(actual_used, used);
            }
            _ => panic!("预期真实金额"),
        }
    }

    #[test]
    fn official_routes_use_exact_https_origin_and_supported_base_paths() {
        for (base, provider, endpoint) in [
            ("https://api.deepseek.com/v1/", Provider::DeepSeek, "https://api.deepseek.com/user/balance"),
            ("https://api.siliconflow.cn/v1", Provider::SiliconFlow, "https://api.siliconflow.cn/v1/user/info"),
            ("https://openrouter.ai/api/v1", Provider::OpenRouter, "https://openrouter.ai/api/v1/credits"),
        ] {
            assert_eq!(route(base).unwrap(), Some((provider, endpoint.into())));
        }
        for base in ["https://api.deepseek.com.evil.test/v1", "https://gateway.test/api.deepseek.com", "http://api.deepseek.com/v1", "https://api.deepseek.com:8443/v1", "https://api.deepseek.com/proxy/v1"] {
            assert!(route(base).unwrap().is_none());
        }
        for base in ["file:///secret", "https://name:pass@host.test/v1", "https://host.test/v1?key=secret", "https://host.test/v1#fragment", "not-a-url"] {
            assert!(route(base).is_err());
        }
    }

    #[test]
    fn deepseek_balance_keeps_currency_zero_and_debt_without_fabricating_spend() {
        amount(parse(Provider::DeepSeek, &json!({"is_available":false,"balance_infos":[{"currency":"CNY","total_balance":"0"}]})), 0.0, "CNY", None);
        amount(parse(Provider::DeepSeek, &json!({"balance_infos":[{"currency":"USD","total_balance":"-0.25"}]})), -0.25, "USD", None);
        for body in [json!({"balance_infos":[]}), json!({"balance_infos":[{"currency":"CNY"}]}), json!({"balance_infos":[{"currency":"CNY","total_balance":"NaN"}]})] {
            assert!(matches!(parse(Provider::DeepSeek, &body), BalanceState::Failed { .. }));
        }
        assert!(matches!(parse(Provider::DeepSeek, &json!({"balance_infos":[{"currency":"CNY","total_balance":"1"},{"currency":"USD","total_balance":"2"}]})), BalanceState::Unsupported { .. }));
    }

    #[test]
    fn siliconflow_uses_total_not_granted_or_recharged_balance() {
        amount(parse(Provider::SiliconFlow, &json!({"status":true,"code":20000,"data":{"balance":"0.88","chargeBalance":"88","totalBalance":"88.88"}})), 88.88, "CNY", None);
        assert!(matches!(parse(Provider::SiliconFlow, &json!({"status":true,"data":{"balance":"10"}})), BalanceState::Failed { .. }));
        let state = parse(Provider::SiliconFlow, &json!({"status":false,"message":"echo sk-test-sensitive","data":{"totalBalance":"100"}}));
        assert!(matches!(state, BalanceState::Failed { .. }));
        assert!(!serde_json::to_string(&state).unwrap().contains("sk-test-sensitive"));
    }

    #[test]
    fn openrouter_wallet_is_credits_minus_usage_and_missing_is_not_zero() {
        amount(parse(Provider::OpenRouter, &json!({"data":{"total_credits":100.5,"total_usage":25.75}})), 74.75, "USD", Some(25.75));
        amount(parse(Provider::OpenRouter, &json!({"data":{"total_credits":0,"total_usage":1}})), -1.0, "USD", Some(1.0));
        for body in [json!({"data":{"total_credits":10}}), json!({"data":{"total_credits":10,"total_usage":-1}})] {
            assert!(matches!(parse(Provider::OpenRouter, &body), BalanceState::Failed { .. }));
        }
    }

    fn server(responses: Vec<(&'static str, u16, &'static str)>) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            for (path, status, body) in responses {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(5)),
                        Err(error) => panic!("本机合成接口未收到请求：{error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
                let mut request = Vec::new();
                let mut bytes = [0; 1024];
                while !request.windows(4).any(|chunk| chunk == b"\r\n\r\n") {
                    let n = stream.read(&mut bytes).unwrap();
                    assert!(n > 0 && request.len() < 8192);
                    request.extend_from_slice(&bytes[..n]);
                }
                let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
                assert!(request.starts_with(&format!("get {path} http/1.1")));
                assert_eq!(request.contains("authorization: bearer synthetic-balance-token"), !path.ends_with("/api/status"));
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        (base, handle)
    }

    #[test]
    fn custom_balance_queries_real_same_origin_gateway_paths_and_preserves_failures() {
        let (base, server) = server(vec![("/proxy/v1/usage", 200, r#"{"balance":0,"unit":"CNY","isValid":true}"#)]);
        amount(query(&format!("{base}/proxy/v1"), "synthetic-balance-token"), 0.0, "CNY", None);
        server.join().unwrap();
        let (base, server) = self::server(vec![
            ("/proxy/v1/usage", 404, "{}"),
            ("/proxy/api/usage/token/", 200, r#"{"code":true,"data":{"total_available":200,"total_used":100}}"#),
            ("/proxy/api/status", 200, r#"{"data":{"quota_per_unit":100}}"#),
        ]);
        amount(query(&format!("{base}/proxy/v1"), "synthetic-balance-token"), 2.0, "USD", Some(1.0));
        server.join().unwrap();
        let (base, server) = self::server(vec![
            ("/v1/usage", 404, "{}"), ("/api/usage/token/", 404, "{}"),
            ("/v1/dashboard/billing/subscription", 200, r#"{"hard_limit_usd":10}"#),
            ("/v1/dashboard/billing/usage", 200, r#"{"total_usage":250}"#),
        ]);
        amount(query(&base, "synthetic-balance-token"), 7.5, "USD", Some(2.5));
        server.join().unwrap();
        let (base, server) = self::server(vec![("/v1/usage", 500, r#"{"message":"echo synthetic-balance-token"}"#)]);
        let state = query(&base, "synthetic-balance-token");
        assert!(matches!(state, BalanceState::Failed { .. }));
        assert!(!serde_json::to_string(&state).unwrap().contains("synthetic-balance-token"));
        server.join().unwrap();
    }
}
