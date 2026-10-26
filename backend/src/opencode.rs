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
/// OpenCode 里登录的 OpenAI 订阅（auth.json 的 `openai` 条目，`{type:"oauth",access,refresh,expires(毫秒),accountId}`），
/// 与 Codex 同一个 ChatGPT OAuth 客户端，额度走 Codex 的 wham/usage。只读，不刷新：
/// 刷新会轮换 refresh token，让 OpenCode 里的登录失效，过期时请用户在 OpenCode 中重新登录。
fn openai_oauth(text:&str, now_ms:i64)->Result<(String,Option<String>),crate::quota::QuotaState> {
    use crate::quota::QuotaState;
    let missing=||QuotaState::Unauthorized{reason:"未找到 OpenCode 的 OpenAI 订阅登录，请先在 OpenCode 中登录 OpenAI".into()};
    let value:serde_json::Value=serde_json::from_str(text).map_err(|_|missing())?;
    let entry=value.get("openai").filter(|v|v.get("type").and_then(|t|t.as_str())==Some("oauth")).ok_or_else(missing)?;
    let access=entry.get("access").and_then(|v|v.as_str()).map(str::trim).filter(|s|!s.is_empty()).ok_or_else(missing)?;
    if entry.get("expires").and_then(|v|v.as_i64()).is_some_and(|expires|expires<=now_ms) {
        return Err(QuotaState::Unauthorized{reason:"OpenCode 的 OpenAI 登录已过期，请在 OpenCode 中使用一次或重新登录".into()});
    }
    let account=entry.get("accountId").and_then(|v|v.as_str()).map(str::trim).filter(|s|!s.is_empty()).map(str::to_string);
    Ok((access.to_string(),account))
}
pub fn local_openai_quota()->crate::quota::QuotaState {
    use crate::quota::QuotaState;
    if crate::validation::isolated(){return QuotaState::Unauthorized{reason:"隔离环境不读取本机 OpenCode 登录".into()}}
    let Some(text)=crate::creds::home().and_then(|h|std::fs::read_to_string(data_dir(&h).join("auth.json")).ok()) else {
        return QuotaState::Unauthorized{reason:"未找到 OpenCode 的 OpenAI 订阅登录，请先在 OpenCode 中登录 OpenAI".into()};
    };
    match openai_oauth(&text,chrono::Utc::now().timestamp_millis()) {
        Ok((access,account))=>crate::quota::codex_oauth_quota(&access,account.as_deref()),
        Err(state)=>state,
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn auth_providers_never_returns_secrets(){let v=auth_providers(r#"{"openai":{"type":"oauth","access":"SECRET","refresh":"REFRESH"},"x":{"type":"api","key":"KEY"}}"#);assert_eq!(v,vec![("openai".into(),"oauth".into()),("x".into(),"api".into())]);assert!(!format!("{v:?}").contains("SECRET"));}
    #[test] fn auth_providers_tolerates_bad_json(){assert!(auth_providers("invalid").is_empty());}
    #[test] fn openai_oauth_reads_access_and_account_but_rejects_expired_or_api_entries(){
        let ok=openai_oauth(r#"{"openai":{"type":"oauth","access":"A","refresh":"R","expires":2000,"accountId":"acc"}}"#,1000).unwrap();
        assert_eq!(ok,("A".into(),Some("acc".into())));
        assert!(matches!(openai_oauth(r#"{"openai":{"type":"oauth","access":"A","expires":1000}}"#,1000),Err(crate::quota::QuotaState::Unauthorized{..})));
        assert!(openai_oauth(r#"{"openai":{"type":"api","key":"K"}}"#,0).is_err());
        assert!(openai_oauth("bad",0).is_err());
    }
}
