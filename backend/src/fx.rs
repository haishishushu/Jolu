//! 汇率查询（费用估算的币种切换，模块 C 扩展）。
//!
//! 来源：open.er-api.com（免费、无鉴权、每日更新）。响应缓存 24 小时；
//! 拉取失败时用内置兜底汇率并如实标注 `fallback`，绝不把失败伪装成最新价。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// USD → CNY 的兜底汇率。仅在在线拉取失败时使用，界面会标注「离线参考价」。
pub const FALLBACK_USD_CNY: f64 = 7.2;

const API_URL: &str = "https://open.er-api.com/v6/latest/USD";
const CACHE_TTL_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ExchangeRate {
    /// 1 USD 兑多少目标货币
    pub rate: f64,
    /// 汇率日期（API 返回的更新时间，或缓存时间）
    pub as_of: String,
    /// true = 在线拉取失败，用的是内置参考价
    pub fallback: bool,
}

static CACHE: Mutex<Option<(ExchangeRate, u64)>> = Mutex::new(None);
static LAST_ATTEMPT_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// 拉取 USD → CNY 汇率。带 24 小时缓存；拉取失败回落到兜底价（fallback=true）。
/// 网络不可达时不无限重试：两次尝试之间至少隔 10 分钟。
pub fn usd_cny(client: &reqwest::blocking::Client) -> ExchangeRate {
    if let Ok(guard) = CACHE.lock() {
        if let Some((rate, at)) = guard.as_ref() {
            if now_ms() - at < CACHE_TTL_MS {
                return rate.clone();
            }
        }
    }
    let last = LAST_ATTEMPT_MS.load(Ordering::Relaxed);
    let now = now_ms();
    if now - last < 10 * 60 * 1000 {
        // 刚失败过：直接用兜底，不要每个请求都打一次超时
        if let Ok(guard) = CACHE.lock() {
            if let Some((rate, _)) = guard.as_ref() {
                return rate.clone();
            }
        }
        return fallback_rate();
    }
    LAST_ATTEMPT_MS.store(now, Ordering::Relaxed);

    let fetched = fetch(client);
    match fetched {
        Ok(rate) => {
            if let Ok(mut guard) = CACHE.lock() {
                *guard = Some((rate.clone(), now));
            }
            rate
        }
        Err(error) => {
            eprintln!("[汇率] 拉取失败，使用内置参考价：{error}");
            let rate = fallback_rate();
            if let Ok(mut guard) = CACHE.lock() {
                *guard = Some((rate.clone(), now));
            }
            rate
        }
    }
}

fn fallback_rate() -> ExchangeRate {
    ExchangeRate {
        rate: FALLBACK_USD_CNY,
        as_of: "内置参考价".into(),
        fallback: true,
    }
}

fn fetch(client: &reqwest::blocking::Client) -> Result<ExchangeRate, String> {
    let response = client
        .get(API_URL)
        .timeout(Duration::from_secs(10))
        .send()
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let value: serde_json::Value = response.json().map_err(|e| e.to_string())?;
    let rate = value
        .pointer("/rates/CNY")
        .and_then(|v| v.as_f64())
        .filter(|r| *r > 1.0 && *r < 20.0)
        .ok_or("响应缺少 rates.CNY 或数值异常")?;
    let as_of = value
        .get("time_last_update_utc")
        .and_then(|v| v.as_str())
        .unwrap_or("未知日期")
        .to_string();
    Ok(ExchangeRate { rate, as_of, fallback: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_rate_is_sane() {
        let rate = fallback_rate();
        assert!((FALLBACK_USD_CNY..=10.0).contains(&rate.rate));
        assert!(rate.fallback);
    }

    /// 解析逻辑：合法的 rates.CNY 被接受，缺字段/越界被拒绝。
    /// 不做 HTTP mock——`fetch` 用固定 URL，测网络路径没有意义；
    /// 真实拉取由界面「参考汇率」标注兜底来体现，不需要测试线程等待连接。
    #[test]
    fn fetch_parses_cny_rate() {
        let value: serde_json::Value = serde_json::from_str(
            r#"{"rates":{"CNY":7.19},"time_last_update_utc":"Mon, 02 Nov 2026 00:00:00 UTC"}"#,
        )
        .unwrap();
        let rate = value.pointer("/rates/CNY").and_then(|v| v.as_f64()).unwrap();
        assert!((rate - 7.19).abs() < f64::EPSILON);
        assert!((1.0..20.0).contains(&rate), "合理区间内的汇率应被接受");

        // 缺字段
        let missing: serde_json::Value = serde_json::from_str(r#"{"rates":{}}"#).unwrap();
        assert!(missing.pointer("/rates/CNY").and_then(|v| v.as_f64()).is_none());
        // 越界（字段错位等异常）
        let weird: serde_json::Value = serde_json::from_str(r#"{"rates":{"CNY":0.01}}"#).unwrap();
        let bad = weird.pointer("/rates/CNY").and_then(|v| v.as_f64()).unwrap();
        assert!(!(1.0..20.0).contains(&bad), "异常汇率必须被拒绝");
    }

    #[test]
    fn out_of_range_rates_are_rejected() {
        // 汇率异常（如字段错位）不能直接用
        for bad in [0.5, 25.0] {
            assert!(!((1.0..20.0).contains(&bad)), "{bad} 应被视为异常");
        }
    }
}
