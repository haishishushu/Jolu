//! 账号汇聚网关（模块 J）：把 `connections` 表里已登记的订阅与 API Key
//! 汇成统一模型源，各 CLI 工具把 base_url 指向本机网关后按 `slug/model` 取用。
//!
//! 设计依据：`docs/plans/2026-10-29-magpie-style-gateway-design.md`。
//!
//! 纪律：
//! - **与观测模式严格分流**：观测模式（`proxy.rs`）保持零转换透传与「不注入凭证」；
//!   网关模式在 [`crate::gateway`] 内改写凭证与模型名，两条数据路径不共用 `forward()`。
//! - **凭证只进不出**：上游凭证仅存在后端内存与 `connections` 表，DTO / 日志 / 导出
//!   一律只有掩码（沿用 `connections.rs` 的纪律）。
//! - **同协议直通优先**：`connections.protocol` 已声明每个连接的协议；跨协议组合
//!   返回明确的 400 引导换端点，不做静默翻译（协议转换是后续扩展位）。

pub mod adapter;
pub mod bridge;
pub mod catalog;
pub mod escape;
pub mod groups;
pub mod identity;
pub mod route_log;
pub mod route;
pub mod server;
pub mod wire;
pub mod wiring;
pub mod wiring_store;

/// 本地代理的工作模式。默认 `Observe`，行为与历史版本完全一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 观测透传：`proxy.rs::forward`，不改写凭证与模型。
    Observe,
    /// 账号汇聚网关：按模型 slug 选连接，改写上游地址 / 凭证 / 模型名。
    Relay,
}

/// 从设置解析模式。未知值按 `Observe` 处理并给出原因，绝不因配置写错而拒服务。
pub fn mode_from(settings: &crate::settings::Settings) -> (Mode, Option<String>) {
    match settings.proxy_mode.trim() {
        "" | "observe" => (Mode::Observe, None),
        "relay" => (Mode::Relay, None),
        other => (
            Mode::Observe,
            Some(format!("未知的代理模式「{other}」，按观测模式处理")),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_parses_known_values_and_falls_back_to_observe() {
        let mut s = crate::settings::Settings::default();
        assert_eq!(mode_from(&s), (Mode::Observe, None));
        s.proxy_mode = "relay".into();
        assert_eq!(mode_from(&s), (Mode::Relay, None));
        s.proxy_mode = "  ".into();
        assert_eq!(mode_from(&s), (Mode::Observe, None));
        s.proxy_mode = " Relay ".into();
        let (mode, warn) = mode_from(&s);
        assert_eq!(mode, Mode::Observe);
        assert!(warn.as_deref().unwrap_or("").contains("未知"), "{warn:?}");
    }
}
