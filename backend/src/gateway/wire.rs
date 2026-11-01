//! 调用方协议封装：解析模型 → 路由选连接 → 协议匹配 → 适配器转发 → 按协议回封。
//! 跨协议组合返回明确的 400（不做静默翻译，见执行文档 §5.4）。

use std::sync::{Arc, Mutex};

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::gateway::route::{self, RouteDecision};
use crate::proxy::ProxyDeps;

/// 调用方入口协议，由请求路径判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallerProtocol {
    /// Anthropic Messages（`/v1/messages`）。
    Anthropic,
    /// OpenAI Chat Completions（`/v1/chat/completions`）。
    Chat,
    /// OpenAI Responses（`/v1/responses`）。
    Responses,
}

impl CallerProtocol {
    /// 连接 protocol 字段的对应值（直通匹配用）。
    fn wire_name(self) -> &'static str {
        match self {
            CallerProtocol::Anthropic => "anthropic",
            CallerProtocol::Chat | CallerProtocol::Responses => "openai",
        }
    }
}

/// 错误响应（JSON，保持依赖面不变的手工拼装）。
pub(crate) fn error_response(status: StatusCode, message: &str) -> Response {
    let body = format!(
        "{{\"error\":{{\"type\":\"jolu_gateway\",\"message\":\"{}\"}}}}",
        message.replace('"', "\\\"")
    );
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

/// 成功与失败计一次熔断计数：连续失败达阈值（默认 5）且开启自动逃生时切回直连。
/// 与观测模式的 `note_upstream_unreachable` 同义，但阈值独立（`gateway_circuit_threshold`）。
pub(crate) fn note_outcome(deps: &Arc<ProxyDeps>, failed: bool, reason: &str) {
    use std::sync::atomic::Ordering;
    if !failed {
        deps.consecutive_failures.store(0, Ordering::Relaxed);
        return;
    }
    if !deps.fallback_enabled {
        return;
    }
    let count = deps.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
    if count >= deps.circuit_threshold {
        deps.consecutive_failures.store(0, Ordering::Relaxed);
        (deps.on_circuit)(reason);
    }
}

/// 从请求体提取 model 字段（任何协议都在顶层）。
fn extract_model(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()?
        .get("model")?
        .as_str()
        .map(str::to_string)
}

/// 提取会话亲和键：Anthropic 的 `metadata.user_id` 是 Claude Code 的会话标识；
/// OpenAI 系请求没有标准会话字段，回退为（调用方令牌 + 模型 slug）。
pub(crate) fn session_key(protocol: CallerProtocol, headers: &HeaderMap, body: &[u8], model: &str) -> String {
    if protocol == CallerProtocol::Anthropic {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) {
            if let Some(user) = value.pointer("/metadata/user_id").and_then(|v| v.as_str()) {
                if !user.trim().is_empty() {
                    return format!("user:{user}");
                }
            }
        }
    }
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.strip_prefix("Bearer ").unwrap_or(s).trim().to_string())
        .or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()).map(str::to_string))
        .unwrap_or_default();
    let slug = crate::gateway::route::split_model_id(model).map(|(s, _)| s).unwrap_or("");
    format!("tok:{token}:{slug}")
}

/// 网关运行态（亲和 + 冷却）的借用引用，避免在 async 里长期持锁。
struct GatewayState<'a> {
    affinity: &'a Mutex<crate::gateway::route::Affinity>,
    cooldowns: &'a Mutex<crate::gateway::route::Cooldowns>,
}

/// 统一入口。`path_and_query` 原样拼到上游 base 后（路径参数与查询都保真）。
pub(crate) async fn handle(
    protocol: CallerProtocol,
    deps: Arc<ProxyDeps>,
    headers: HeaderMap,
    body: axum::body::Bytes,
    path_and_query: &str,
) -> Response {
    let Some(model) = extract_model(&body) else {
        return error_response(StatusCode::BAD_REQUEST, "请求体缺少 model 字段或不是有效 JSON");
    };

    // 模型池快照：拿不到数据库锁时沿用旧索引（与观测模式 KeyIndex 同策略）
    let pool = {
        let guard = match deps.pool_index() {
            Ok(g) => g,
            Err(_) => return error_response(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用，请稍后重试"),
        };
        route::pool_snapshot(&deps.db, guard)
    };
    let settings_default = deps.gateway_default_connection();

    // 亲和参与选路：会话绑定连接被 429 冷却时不切换，直接报错（保 prompt cache）
    let state = match (deps.affinity.as_ref(), deps.cooldowns.as_ref()) {
        (Some(a), Some(c)) => GatewayState { affinity: a, cooldowns: c },
        _ => return error_response(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用，请稍后重试"),
    };
    let session = session_key(protocol, &headers, &body, &model);
    let decision = {
        let mut affinity = match state.affinity.lock() {
            Ok(g) => g,
            Err(_) => return error_response(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用，请稍后重试"),
        };
        let cooldowns = match state.cooldowns.lock() {
            Ok(g) => g,
            Err(_) => return error_response(StatusCode::SERVICE_UNAVAILABLE, "网关状态暂不可用，请稍后重试"),
        };
        route::decide_with_affinity(&pool, &model, settings_default.as_deref(), Some(&session), &mut affinity, &cooldowns)
    };

    match decision {
        RouteDecision::Direct(entry, upstream_model) => {
            if entry.protocol != protocol.wire_name() {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    &format!(
                        "连接「{}」只提供 {} 协议端点，不能服务 {} 请求；请为该连接选择 {} 兼容地址",
                        entry.slug,
                        entry.protocol,
                        wire_label(protocol),
                        wire_label(protocol),
                    ),
                );
            }
            let response = crate::gateway::adapter::direct::forward(
                deps.clone(),
                entry,
                upstream_model,
                axum::http::Method::POST,
                path_and_query,
                &headers,
                body,
            )
            .await;
            note_outcome(&deps, !response.status().is_success(), &format!("连接「{}」返回 {}", entry.slug, response.status()));
            // 成功路径补写会话绑定（decide_with_affinity 内部已续期，这里覆盖冷却后恢复等场景）
            if response.status().is_success() {
                if let Ok(mut affinity) = state.affinity.lock() {
                    affinity.bind(&session, &entry.connection_id, now_ms());
                }
            }
            response
        }
        RouteDecision::Subscription(entry) => {
            if entry.platform == "codex" {
                if protocol == CallerProtocol::Anthropic {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        "Codex 订阅只服务 OpenAI 协议请求",
                    );
                }
                crate::gateway::adapter::codex::forward(deps.clone(), entry, protocol, body).await
            } else if entry.platform == "claude" {
                if protocol != CallerProtocol::Anthropic {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        "Claude 订阅当前只服务 Anthropic Messages 协议请求",
                    );
                }
                crate::gateway::adapter::claude::forward(deps.clone(), entry, &session, body).await
            } else {
                error_response(
                    StatusCode::NOT_IMPLEMENTED,
                    "该平台的订阅适配器尚未接入",
                )
            }
        }
        RouteDecision::Reject(reason) => error_response(StatusCode::BAD_REQUEST, &reason),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn wire_label(protocol: CallerProtocol) -> &'static str {
    match protocol {
        CallerProtocol::Anthropic => "Anthropic Messages",
        CallerProtocol::Chat => "OpenAI Chat",
        CallerProtocol::Responses => "OpenAI Responses",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn deps_with_circuit(threshold: u32, hits: Arc<AtomicU32>, enabled: bool) -> Arc<ProxyDeps> {
        let mut deps = Arc::try_unwrap(crate::proxy::test_support::bare_deps()).unwrap_or_else(|_| {
            // bare_deps 是 Arc，拿不到所有权时重建一份最小 deps 供熔断计数测试
            crate::proxy::test_support::bare_deps();
            unreachable!()
        });
        deps.circuit_threshold = threshold;
        deps.fallback_enabled = enabled;
        deps.on_circuit = Arc::new(move |_| {
            hits.fetch_add(1, Ordering::Relaxed);
        });
        Arc::new(deps)
    }

    #[test]
    fn circuit_counts_consecutive_failures_and_resets_on_success() {
        let hits = Arc::new(AtomicU32::new(0));
        let deps = deps_with_circuit(3, hits.clone(), true);
        note_outcome(&deps, true, "a");
        note_outcome(&deps, true, "b");
        assert_eq!(hits.load(Ordering::Relaxed), 0, "未达阈值不熔断");
        note_outcome(&deps, true, "c");
        assert_eq!(hits.load(Ordering::Relaxed), 1, "达阈值熔断一次");
        // 熔断后计数归零，不会连续触发
        note_outcome(&deps, true, "d");
        assert_eq!(hits.load(Ordering::Relaxed), 1);
        // 成功清零
        note_outcome(&deps, false, "");
        note_outcome(&deps, true, "e");
        assert_eq!(hits.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn circuit_is_off_when_auto_escape_disabled() {
        let hits = Arc::new(AtomicU32::new(0));
        let deps = deps_with_circuit(1, hits.clone(), false);
        for _ in 0..5 {
            note_outcome(&deps, true, "x");
        }
        assert_eq!(hits.load(Ordering::Relaxed), 0, "关闭自动逃生时绝不自动切断");
    }

    #[test]
    fn session_key_prefers_anthropic_metadata_user_id() {
        let body = br#"{"model":"kimi/kimi-k2","metadata":{"user_id":"user-abc"}}"#;
        let key = session_key(CallerProtocol::Anthropic, &HeaderMap::new(), body, "kimi/kimi-k2");
        assert_eq!(key, "user:user-abc");
        // OpenAI 系没有标准会话字段 → 退化为令牌 + slug
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer jolu-codex".parse().unwrap());
        let body = br#"{"model":"codex/gpt-5.5"}"#;
        let key = session_key(CallerProtocol::Chat, &headers, body, "codex/gpt-5.5");
        assert_eq!(key, "tok:jolu-codex:codex");
    }
}
