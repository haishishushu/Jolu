//! 前后端共用的平台目录。来源是否存在由本机检测决定，不能由图标推断。
use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Deserialize)]
pub struct Platform {
    pub id: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub limitation: Option<String>,
}

static CATALOG: LazyLock<Vec<Platform>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../frontend/src/lib/platformCatalog.json"))
        .expect("随程序打包的平台目录必须有效")
});

pub fn get(id: &str) -> Option<&'static Platform> {
    CATALOG.iter().find(|p| p.id == id)
}
pub fn known(id: &str) -> bool {
    get(id).is_some()
}
/// 使用本机来源的平台。自定义供应商（`custom`）只有用户填写的 API Key 连接，不属于本机来源。
pub fn native(id: &str) -> bool {
    known(id) && !matches!(id, "claude" | "codex" | "custom")
}
pub fn uses_native_quota(platform: &str, kind: &str) -> bool {
    native(platform) && !(matches!(platform,"zcode"|"opencode") && kind=="api")
}
pub fn supports(id: &str, capability: &str) -> bool {
    get(id).is_some_and(|p| p.capabilities.iter().any(|c| c == capability))
}
pub fn limitation(id: &str) -> String {
    get(id)
        .and_then(|p| p.limitation.clone())
        .unwrap_or_else(|| "该平台未提供此项数据".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn api_plan_quota_is_not_blocked_by_native_platform() {
        for p in ["zcode","opencode"] {assert!(!uses_native_quota(p,"api"));assert!(uses_native_quota(p,"auth"));}
        assert!(uses_native_quota("gemini","api"));
    }
    #[test]
    fn all_ten_platforms_have_unique_ids() {
        assert_eq!(CATALOG.len(), 10);
        for id in [
            "claude",
            "codex",
            "gemini",
            "grok",
            "opencode",
            "zcode",
            "trae",
            "qoder",
            "workbuddy",
            "custom",
        ] {
            assert_eq!(CATALOG.iter().filter(|p| p.id == id).count(), 1);
        }
        assert!(!known("tare"));
        assert!(!supports("trae", "local_sessions"));
        // OpenCode 已列入目录但尚未接入，不声明任何能力
        assert!(supports("opencode","local_sessions"));
        // 自定义供应商只有用户填写的 API Key 连接，不是本机来源平台
        assert!(known("custom") && !native("custom"));
        assert!(supports("custom", "api_connection") && !supports("custom", "local_sessions"));
    }
}
