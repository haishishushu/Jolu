//! OpenCode 本机登录只读检测；凭证值不进入 DTO。
use std::path::{Path,PathBuf};

pub fn data_dir(home:&Path)->PathBuf {
    if crate::validation::isolated() { return home.join(".local/share/opencode"); }
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p|p.is_absolute()).unwrap_or_else(||home.join(".local/share")).join("opencode")
}
pub fn auth_providers(text:&str)->Vec<(String,String)> {
    let Ok(serde_json::Value::Object(map))=serde_json::from_str(text) else{return vec![]};
    map.into_iter().filter_map(|(id,v)| {
        let kind=v.get("type")?.as_str()?;
        matches!(kind,"oauth"|"api").then(||(id,kind.into()))
    }).collect()
}
pub fn openai_logged_in()->bool {
    if crate::validation::isolated(){return false}
    crate::creds::home().and_then(|h|std::fs::read_to_string(data_dir(&h).join("auth.json")).ok()).is_some_and(|s|auth_providers(&s).iter().any(|(id,kind)|id=="openai"&&kind=="oauth"))
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn auth_providers_never_returns_secrets(){let v=auth_providers(r#"{"openai":{"type":"oauth","access":"SECRET","refresh":"REFRESH"},"x":{"type":"api","key":"KEY"}}"#);assert_eq!(v,vec![("openai".into(),"oauth".into()),("x".into(),"api".into())]);assert!(!format!("{v:?}").contains("SECRET"));}
    #[test] fn auth_providers_tolerates_bad_json(){assert!(auth_providers("invalid").is_empty());}
}
