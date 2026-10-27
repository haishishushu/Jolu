//! 浏览器授权凭证仅在后端保管，验证成功后才允许用户保存连接。
use crate::{login_detect::LoginDetection, quota::QuotaState};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::{Arc, LazyLock, Mutex, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use tauri::{Emitter, Manager, WebviewWindow};

const PREFIX: &str = "jolu-browser:v1:";
const LIFETIME: Duration = Duration::from_secs(600);

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub(crate) enum Credential {
    Workbuddy { edition: String, session: crate::workbuddy_browser_auth::Session },
    Qoder { edition: String, session: crate::qoder_desktop::Session },
    Trae { edition: String, session: crate::trae_browser_auth::Session },
    Grok { session: crate::grok_browser_auth::Session },
    Zcode { session: crate::zcode_browser_auth::Session },
    Cli { session: crate::cli_browser_auth::Session },
}
impl Credential {
    fn platform(&self) -> &str { match self { Self::Workbuddy { .. } => "workbuddy", Self::Qoder { .. } => "qoder",Self::Trae{..}=>"trae",Self::Grok{..}=>"grok",Self::Zcode{..}=>"zcode",Self::Cli{session}=>&session.platform } }
    fn edition(&self)->Option<&str> {match self {Self::Workbuddy{edition,..}|Self::Qoder{edition,..}|Self::Trae{edition,..}=>Some(edition),_=>None}}
    fn identity(&self) -> Option<(&str, Option<&str>)> {
        match self {
            Self::Workbuddy { session, .. } => Some((session.user_id.as_str(), session.enterprise_id.as_deref())),
            Self::Qoder { session, .. } => session.user_id.as_deref().map(|id| (id, None)),
            Self::Trae { session, .. } => Some((session.user_id.as_str(), None)),
            Self::Grok { session } => Some((session.user_id.as_str(), None)),
            Self::Zcode { session } => Some((session.user_id.as_str(), None)),
            Self::Cli { session } => session.credentials.pointer("/tokens/account_id").and_then(serde_json::Value::as_str)
                .or(session.account.as_deref()).map(|id| (id, None)),
        }.filter(|(id, _)| !id.trim().is_empty())
    }
    fn official_account(&self) -> Option<String> {
        let account = match self {
            Self::Workbuddy { session, .. } => session.email.as_deref(),
            Self::Qoder { session, .. } => session.account.as_deref(),
            Self::Trae { session, .. } => session.account.as_deref(),
            Self::Grok { session } => session.account.as_deref(),
            Self::Zcode { session } => session.account.as_deref(),
            Self::Cli { session } => session.account.as_deref(),
        };
        account.and_then(safe_account)
    }
    /// 缺邮箱时仅返回官方身份的稳定指纹，原 UID、企业 ID 和令牌均留在后端。
    pub(crate) fn account_label(&self) -> Option<String> {
        self.official_account().or_else(|| {
            let (identity, enterprise) = self.identity()?;
            let source = serde_json::to_vec(&("jolu-account-v1", self.platform(), self.edition(), identity, enterprise)).ok()?;
            let digest = format!("{:x}", Sha256::digest(source));
            Some(format!("账号 · {}", &digest[..16]))
        })
    }
    /// 身份必须由本次官方请求核实；额度能力、权限和临时失败不代表登录失效。
    fn authorize(&self) -> Result<QuotaState, String> {
        let verified = match self {
            Self::Workbuddy { edition, session } => { crate::workbuddy_browser_auth::verify_session(edition, session)?; true },
            Self::Qoder { edition, session } => { crate::qoder_desktop::verify_session(edition, session)?; true },
            Self::Trae { edition, session } => { crate::trae_browser_auth::verify_session(edition, session)?; true },
            _ => false,
        };
        let state = self.query();
        match state {
            QuotaState::Ok { .. } => Ok(state),
            QuotaState::Unauthorized { reason } => Err(reason),
            _ if verified => Ok(state),
            QuotaState::Unsupported { reason } | QuotaState::Forbidden { reason }
                | QuotaState::RateLimited { reason } | QuotaState::Failed { reason } => Err(reason),
        }
    }
    pub(crate) fn query(&self) -> QuotaState {
        match self {
            Self::Workbuddy { edition, session } => crate::workbuddy_quota::authorized_quota(edition, session),
            Self::Qoder { edition, session } => {
                let mut state = crate::qoder_desktop::query(edition, &session.token);
                if let QuotaState::Ok { account: Some(account), .. } = &mut state { account.email = session.account.clone(); }
                state
            }
            Self::Trae{edition,session}=>crate::trae_quota::authorized_quota(edition,session),
            Self::Grok{session}=>crate::grok_quota::authorized_quota(session),
            Self::Zcode{session}=>crate::zcode_quota::authorized_quota(session),
            Self::Cli{session}=>crate::cli_browser_auth::query(session),
        }
    }
}

fn safe_account(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && value.len() <= 320 && !value.chars().any(char::is_control)).then(|| value.to_string())
}

pub(crate) fn is_browser(secret: Option<&str>) -> bool { secret.is_some_and(|s| s.starts_with(PREFIX)) }
/// 旧账号不迁移、不猜区域；只在后端解封自身凭证提取非敏感版本。
pub(crate) fn edition_for(id:&str,platform:&str,sealed:Option<&str>)->Option<String> {
    if crate::validation::isolated() {return None;}
    let sealed=sealed.filter(|value|is_browser(Some(value)))?;
    let credential=load(id,platform,sealed).ok()?;
    credential.edition().filter(|edition|supported(platform,edition)).map(str::to_string)
}
pub(crate) fn validate(id:&str,platform:&str,sealed:&str) -> Result<(),String> {
    if crate::validation::isolated() { return Err("隔离验收不读取浏览器真实授权".into()); }
    load(id,platform,sealed)?.authorize().map(|_| ())
}
pub(crate) fn load(id: &str, platform: &str, sealed: &str) -> Result<Credential, String> {
    let invalid = || "浏览器授权无法解锁，请使用原 Windows 用户重新授权".to_string();
    let data = sealed.strip_prefix(PREFIX).filter(|s| s.len() <= 256 * 1024).ok_or_else(invalid)?;
    let bytes = STANDARD.decode(data).map_err(|_| invalid())?;
    let plain = crate::auth_snapshot::protect(&bytes, id.as_bytes(), false).map_err(|_| invalid())?;
    let credential: Credential = serde_json::from_slice(&plain).map_err(|_| invalid())?;
    if credential.platform() != platform { return Err(invalid()); }
    Ok(credential)
}

fn save(conn: &Connection, id: &str, _name: &str, credential: &Credential, detection: &LoginDetection) -> Result<String, String> {
    let account = credential.official_account().or_else(|| detection.account.as_deref().and_then(safe_account))
        .or_else(|| credential.account_label()).ok_or("官方未返回可识别账号，请重新授权")?;
    let name = crate::connections::validate_name(&account.chars().take(80).collect::<String>())?;
    let mut existing = None;
    if let Some(identity) = credential.identity() {
        let mut stmt = conn.prepare("SELECT id,secret FROM connections WHERE platform=?1 AND kind='auth' ORDER BY created_ms,id").map_err(|_| "读取浏览器账号失败")?;
        let rows = stmt.query_map([credential.platform()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))
            .map_err(|_| "读取浏览器账号失败")?;
        for row in rows {
            let (stored_id, sealed) = row.map_err(|_| "读取浏览器账号失败")?;
            let Some(sealed) = sealed.filter(|value| is_browser(Some(value))) else { continue };
            let Ok(stored) = load(&stored_id, credential.platform(), &sealed) else { continue };
            if stored.edition() == credential.edition() && stored.identity() == Some(identity) { existing = Some(stored_id); break; }
        }
    }
    let id = existing.as_deref().unwrap_or(id);
    let plain = serde_json::to_vec(credential).map_err(|_| "无法保存浏览器授权")?;
    let protected = crate::auth_snapshot::protect(&plain, id.as_bytes(), true)?;
    let sealed = format!("{PREFIX}{}", STANDARD.encode(protected));
    if existing.is_some() {
        // 复用连接 ID，保留用户名称、创建时间及关联历史；新凭证使旧同步结果失效。
        conn.execute("UPDATE connections SET secret=?2,masked='已加密保存',status='connected',last_sync_ms=NULL,account_email=?3 WHERE id=?1",
            params![id,sealed,account]).map_err(|_| "更新浏览器账号失败")?;
    } else {
        conn.execute("INSERT INTO connections(id,platform,kind,name,label,secret,masked,status,created_ms,account_email) VALUES(?1,?2,'auth',?3,'浏览器授权',?4,'已加密保存','connected',?5,?6)",
            params![id,credential.platform(),name,sealed,chrono::Utc::now().timestamp_millis(),account]).map_err(|_| "保存浏览器账号失败")?;
    }
    Ok(id.into())
}

fn supported(platform: &str, edition: &str) -> bool {
    matches!((platform, edition), ("workbuddy", "cn" | "ai") | ("qoder"|"trae", "cn" | "global") | ("claude"|"codex"|"gemini"|"grok"|"opencode"|"zcode","default"))
}
/// 所有供应商的浏览器授权统一交给操作系统默认处理器，不指定浏览器或创建内嵌窗口。
fn open_default_browser(app:&tauri::AppHandle,url:String)->Result<(),String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url(url,None::<&str>).map_err(|_|"无法打开系统默认浏览器，请重新授权".into())
}

#[cfg(windows)]
fn random_hex(bytes: usize) -> Result<String, String> {
    use windows_sys::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
    let mut output = vec![0u8; bytes];
    let status = unsafe { BCryptGenRandom(std::ptr::null_mut(), output.as_mut_ptr(), output.len() as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if status < 0 { return Err("无法生成授权安全随机数".into()); }
    Ok(output.iter().map(|b| format!("{b:02x}")).collect())
}
#[cfg(not(windows))]
fn random_hex(_: usize) -> Result<String, String> { Err("浏览器授权凭证存储当前仅支持 Windows".into()) }

#[derive(Clone, Serialize)]
pub(crate) struct Status {
    state: &'static str,
    detection: Option<LoginDetection>,
    message: Option<String>,
}
struct Attempt {
    started: Instant,
    cancelled: Arc<AtomicBool>,
    status: Status,
    credential: Option<Credential>,
}
impl Attempt {
    fn saveable(&self) -> bool { self.status.state == "ready" && self.credential.is_some() && !self.cancelled.load(Ordering::Acquire) && self.started.elapsed() < LIFETIME }
}
type Shared = Arc<Mutex<Attempt>>;
static ATTEMPTS: LazyLock<Mutex<HashMap<String, Shared>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn require_main(window: &WebviewWindow) -> Result<(), String> {
    if window.label() != "main" { return Err("只能在主窗口管理账号授权".into()); }
    if crate::validation::isolated() { return Err("隔离验收环境禁止读取或创建真实授权".into()); }
    Ok(())
}
fn attempt(id: &str) -> Result<Shared, String> {
    ATTEMPTS.lock().map_err(|_| "授权状态不可用")?.get(id).cloned().ok_or_else(|| "授权已取消或失效，请重新发起".into())
}
fn fail(attempt: &Shared, message: &str) {
    if let Ok(mut state) = attempt.lock() {
        if state.status.state == "waiting" { state.status = Status { state:"failed", detection:None, message:Some(message.into()) }; }
    }
}
fn ready(attempt: &Shared, credential: Credential, state: QuotaState) {
    let (plan, account, message) = match state {
        QuotaState::Ok { plan, account, .. } => (plan, account.and_then(|a| a.email), None),
        QuotaState::Unauthorized { .. } => return,
        QuotaState::Unsupported { reason } | QuotaState::Forbidden { reason }
            | QuotaState::RateLimited { reason } | QuotaState::Failed { reason } => {
            let account = match &credential {
                Credential::Workbuddy { session, .. } => session.email.clone(),
                Credential::Qoder { session, .. } => session.account.clone(),
                Credential::Trae { session, .. } => session.account.clone(),
                _ => return,
            };
            (None, account, Some(format!("账号身份已验证；额度查询：{reason}")))
        },
    };
    let account = credential.official_account().or_else(|| account.as_deref().and_then(safe_account))
        .or_else(|| credential.account_label());
    if account.is_none() { fail(attempt, "官方未返回可识别账号，请重新授权"); return; }
    if let Ok(mut attempt) = attempt.lock() {
        if attempt.cancelled.load(Ordering::Acquire) || attempt.started.elapsed() >= LIFETIME || attempt.status.state != "waiting" { return; }
        let detection = LoginDetection { found:true, account, plan, sources:vec!["浏览器授权 · 已验证账号身份".into()], masked:Some("已验证，保存后加密存储".into()), message:message.clone() };
        attempt.credential = Some(credential);
        attempt.status = Status { state:"ready", detection:Some(detection), message };
    }
}

enum PendingLogin {
    Qoder{edition:String,nonce:String,verifier:String,login_url:String},
    Workbuddy{edition:String,pending:crate::workbuddy_browser_auth::PendingLogin},
    Trae{edition:String,pending:crate::trae_browser_auth::Pending},
    Grok(crate::grok_browser_auth::Pending),
    Zcode(crate::zcode_browser_auth::Pending),
    Cli(crate::cli_browser_auth::Pending),
}
impl PendingLogin {
    fn begin(platform:&str,edition:&str)->Result<Self,String> {
        match platform {
            "qoder"=>{
                let uuid=||->Result<String,String>{let mut value=random_hex(16)?;value.replace_range(12..13,"4");value.replace_range(16..17,"8");Ok(format!("{}-{}-{}-{}-{}",&value[..8],&value[8..12],&value[12..16],&value[16..20],&value[20..]))};
                let (nonce,verifier,machine)=(uuid()?,random_hex(32)?,uuid()?);
                let login_url=crate::qoder_desktop::login_url(edition,&nonce,&verifier,&machine)?;
                Ok(Self::Qoder{edition:edition.into(),nonce,verifier,login_url})
            }
            "workbuddy"=>Ok(Self::Workbuddy{edition:edition.into(),pending:crate::workbuddy_browser_auth::begin(edition)?}),
            "trae"=>Ok(Self::Trae{edition:edition.into(),pending:crate::trae_browser_auth::begin(edition)?}),
            "grok"=>crate::grok_browser_auth::begin().map(Self::Grok),
            "zcode"=>crate::zcode_browser_auth::begin().map(Self::Zcode),
            "claude"|"codex"|"gemini"|"opencode"=>crate::cli_browser_auth::begin(platform).map(Self::Cli),
            _=>Err("该供应商没有可用的浏览器登录协议".into()),
        }
    }
    fn login_url(&self)->Option<&str>{match self {
        Self::Qoder{login_url,..}=>Some(login_url),Self::Workbuddy{pending,..}=>Some(&pending.login_url),Self::Trae{pending,..}=>Some(&pending.login_url),
        Self::Grok(p)=>Some(&p.login_url),Self::Zcode(p)=>Some(&p.login_url),Self::Cli(_)=>None,
    }}
    fn user_code(&self)->Option<&str>{match self {Self::Grok(p)=>p.user_code.as_deref(),Self::Zcode(p)=>p.user_code.as_deref(),_=>None}}
    fn poll(&mut self)->Result<Option<Credential>,String>{match self {
        Self::Qoder{edition,nonce,verifier,..}=>Ok(crate::qoder_desktop::poll_login(edition,nonce,verifier)?.map(|session|Credential::Qoder{edition:edition.clone(),session})),
        Self::Workbuddy{edition,pending}=>Ok(crate::workbuddy_browser_auth::poll(pending)?.map(|session|Credential::Workbuddy{edition:edition.clone(),session})),
        Self::Trae{edition,pending}=>Ok(crate::trae_browser_auth::poll(pending)?.map(|session|Credential::Trae{edition:edition.clone(),session})),
        Self::Grok(p)=>Ok(crate::grok_browser_auth::poll(p)?.map(|session|Credential::Grok{session})),
        Self::Zcode(p)=>Ok(crate::zcode_browser_auth::poll(p)?.map(|session|Credential::Zcode{session})),
        Self::Cli(p)=>Ok(p.poll()?.map(|session|Credential::Cli{session})),
    }}
}
fn active(attempt:&Shared)->bool {
    attempt.lock().is_ok_and(|s|!s.cancelled.load(Ordering::Acquire)&&s.started.elapsed()<LIFETIME&&s.status.state=="waiting")
}

#[tauri::command]
pub(crate) async fn start_browser_authorization(app: tauri::AppHandle, window: WebviewWindow, platform: String, edition: String) -> Result<String, String> {
    require_main(&window)?;
    if !supported(&platform,&edition){return Err("供应商或区域无效".into());}
    let id=random_hex(16)?;
    let shared=Arc::new(Mutex::new(Attempt{started:Instant::now(),cancelled:Arc::new(AtomicBool::new(false)),status:Status{state:"waiting",detection:None,message:Some("请在系统默认浏览器完成官方登录，验证成功后即可保存".into())},credential:None}));
    {
        let mut attempts=ATTEMPTS.lock().map_err(|_|"授权状态不可用")?;
        for (_,old) in attempts.drain(){if let Ok(mut old)=old.lock(){old.cancelled.store(true,Ordering::Release);old.credential=None;}}
        attempts.insert(id.clone(),shared.clone());
    }
    let (expiry_id,expiry_shared)=(id.clone(),shared.clone());
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(LIFETIME).await;
        if let Ok(mut state)=expiry_shared.lock(){state.cancelled.store(true,Ordering::Release);state.credential=None;}
        if let Ok(mut attempts)=ATTEMPTS.lock(){attempts.remove(&expiry_id);}
    });
    // ID先返回；网络准备和外部CLI启动均在后台，使慢请求也能被及时取消。
    tauri::async_runtime::spawn(async move {
        if !active(&shared){return;}
        let prepared=tauri::async_runtime::spawn_blocking(move ||PendingLogin::begin(&platform,&edition)).await;
        let mut pending=match prepared {Ok(Ok(p))=>p,Ok(Err(reason))=>{fail(&shared,&reason);return;},Err(_)=>{fail(&shared,"授权准备异常，请重新发起");return;}};
        if !active(&shared){return;}
        if let Some(code)=pending.user_code(){if let Ok(mut state)=shared.lock(){state.status.message=Some(format!("请在系统默认浏览器输入设备码 {code} 并完成官方授权"));}}
        if let Some(url)=pending.login_url(){if let Err(message)=open_default_browser(&app,url.into()){fail(&shared,&message);return;}}
        loop {
            if !active(&shared){fail(&shared,"授权已取消或超时，请重新发起");break;}
            let result=tauri::async_runtime::spawn_blocking(move ||{let result=pending.poll();(pending,result)}).await;
            match result {
                Ok((next,Ok(Some(credential))))=>{
                    drop(next);
                    if !active(&shared){return;}
                    let query=credential.clone();
                    match tauri::async_runtime::spawn_blocking(move ||query.authorize()).await {
                        Ok(Ok(state))=>ready(&shared,credential,state),
                        Ok(Err(reason))=>fail(&shared,&reason),
                        Err(_)=>fail(&shared,"官方账号身份验证异常，请重新授权"),
                    }
                    break;
                }
                Ok((next,Ok(None)))=>pending=next,
                Ok((_,Err(reason)))=>{fail(&shared,&reason);break;},
                Err(_)=>{fail(&shared,"授权轮询异常，请重新发起");break;},
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    Ok(id)
}

#[tauri::command]
pub(crate) async fn browser_authorization_status(window: WebviewWindow, id: String) -> Result<Status, String> {
    require_main(&window)?;
    let shared = attempt(&id)?;
    let state = shared.lock().map_err(|_| "授权状态不可用")?;
    if state.started.elapsed() >= LIFETIME { return Err("授权已超时，请重新发起".into()); }
    Ok(state.status.clone())
}

#[tauri::command]
pub(crate) async fn cancel_browser_authorization(window: WebviewWindow, id: String) -> Result<(), String> {
    require_main(&window)?;
    if let Some(shared) = ATTEMPTS.lock().map_err(|_| "授权状态不可用")?.remove(&id) {
        if let Ok(mut attempt) = shared.lock() { attempt.cancelled.store(true, Ordering::Release); attempt.credential=None; }
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn save_browser_connection(app: tauri::AppHandle, window: WebviewWindow, id: String, name: Option<String>) -> Result<String, String> {
    require_main(&window)?;
    let shared = attempt(&id)?;
    let connection_id = format!("browser:{id}");
    let pool = app.state::<crate::Db>().0.clone();
    let output = crate::with_database(pool, move |conn| {
        let mut state = shared.lock().map_err(|_| "授权状态不可用")?;
        if !state.saveable() { return Err("授权尚未完成或已失效".into()); }
        let credential = state.credential.as_ref().ok_or("未获取有效授权")?;
        let detection = state.status.detection.as_ref().ok_or("未获取账号状态")?;
        let output = save(conn,&connection_id,name.as_deref().unwrap_or_default(),credential,detection)?;
        state.credential = None;
        state.status.state = "saved";
        Ok(output)
    }).await?;
    ATTEMPTS.lock().map_err(|_| "授权状态不可用")?.remove(&id);
    crate::invalidate_network_cache(&output);
    let _ = app.emit("connections-changed",());
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn synthetic_credential()->Credential {
        Credential::Workbuddy{edition:"cn".into(),session:crate::workbuddy_browser_auth::Session{
            token:"synthetic-private-token".into(),domain:"www.workbuddy.cn".into(),user_id:"synthetic-user".into(),enterprise_id:None,email:None,expires_at:None,
        }}
    }
    fn synthetic_attempt() -> Shared {
        Arc::new(Mutex::new(Attempt { started: Instant::now(), cancelled: Arc::new(AtomicBool::new(false)),
            status: Status { state: "waiting", detection: None, message: None }, credential: None }))
    }
    #[test]
    fn verified_account_names_do_not_accept_frontend_labels() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let detection = LoginDetection { found: true, account: Some("verified@example.test".into()), ..Default::default() };
        let id = save(&db, "browser-verified", "任意前端名称", &synthetic_credential(), &detection).unwrap();
        let row = crate::connections::list(&db).unwrap().into_iter().find(|row| row.id == id).unwrap();
        assert_eq!(row.name, "verified@example.test");
        assert_eq!(row.account_email.as_deref(), Some("verified@example.test"));
    }
    #[test]
    fn missing_email_uses_stable_verified_identity_without_revealing_it() {
        let credential = synthetic_credential();
        let attempt = synthetic_attempt();
        ready(&attempt, credential.clone(), QuotaState::Unsupported { reason: "没有额度接口".into() });
        let detection = attempt.lock().unwrap().status.detection.clone().unwrap();
        let label = detection.account.as_deref().expect("已核实身份应有安全账号标识");
        assert!(label.starts_with("账号 · "));
        assert_eq!(label.chars().count(), 21);
        let public = serde_json::to_string(&detection).unwrap();
        assert!(!public.contains("synthetic-user"));
        assert!(!public.contains("synthetic-private-token"));
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let id = save(&db, "browser-stable", "", &credential, &detection).unwrap();
        let row = crate::connections::list(&db).unwrap().into_iter().find(|row| row.id == id).unwrap();
        assert_eq!(row.name, label);
        assert_eq!(row.account_email.as_deref(), Some(label));
        let mut renewed = credential.clone();
        if let Credential::Workbuddy { session, .. } = &mut renewed { session.token = "new-private-token".into(); }
        let renewed_attempt = synthetic_attempt();
        ready(&renewed_attempt, renewed.clone(), QuotaState::Unsupported { reason: "没有额度接口".into() });
        let renewed_detection = renewed_attempt.lock().unwrap().status.detection.clone().unwrap();
        assert_eq!(renewed_detection.account, detection.account);
        assert_eq!(save(&db, "browser-renewed", "", &renewed, &renewed_detection).unwrap(), id);
    }
    #[test]
    fn verified_identity_remains_saveable_when_quota_is_unavailable() {
        for state in [QuotaState::Unsupported { reason: "套餐暂不提供额度".into() },
            QuotaState::Forbidden { reason: "没有额度权限".into() },
            QuotaState::RateLimited { reason: "稍后刷新额度".into() },
            QuotaState::Failed { reason: "额度网络暂时失败".into() }] {
            let attempt = synthetic_attempt();
            ready(&attempt, synthetic_credential(), state);
            let state = attempt.lock().unwrap();
            assert!(state.saveable());
            assert!(state.status.detection.as_ref().unwrap().message.is_some());
        }
        let attempt = synthetic_attempt();
        ready(&attempt, synthetic_credential(), QuotaState::Unauthorized { reason: "登录已失效".into() });
        assert!(!attempt.lock().unwrap().saveable());
    }
    #[test]
    fn reauthorization_updates_only_same_browser_identity_and_keeps_history() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let original = synthetic_credential();
        save(&db, "browser-a", "保留名称", &original, &LoginDetection::default()).unwrap();
        db.execute("UPDATE connections SET name='保留名称',status='paused',created_ms=123,last_sync_ms=456 WHERE id='browser-a'", []).unwrap();
        db.execute("INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms) VALUES('monitor:native:workbuddy:main','workbuddy','auth','原工具','native:workbuddy:main','本机来源',1)", []).unwrap();
        let mut renewed = original.clone();
        if let Credential::Workbuddy { session, .. } = &mut renewed { session.token = "synthetic-new-token".into(); }
        let id = save(&db, "browser-retry", "不覆盖名称", &renewed, &LoginDetection::default()).unwrap();
        assert_eq!(id, "browser-a");
        let row = crate::connections::creds_of(&db, &id).unwrap().unwrap();
        assert!(!row.paused);
        assert_eq!(row.name, "保留名称");
        let Credential::Workbuddy { session, .. } = load(&id, "workbuddy", row.secret.as_deref().unwrap()).unwrap() else { panic!("expected browser session") };
        assert_eq!(session.token, "synthetic-new-token");
        let stored: (i64, Option<i64>) = db.query_row("SELECT created_ms,last_sync_ms FROM connections WHERE id=?1", [&id], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        assert_eq!(stored, (123, None));
        let mut other = original.clone();
        if let Credential::Workbuddy { session, .. } = &mut other { session.user_id = "synthetic-other-user".into(); }
        assert_eq!(save(&db, "browser-b", "账号B", &other, &LoginDetection::default()).unwrap(), "browser-b");
        let mut other_edition = original.clone();
        if let Credential::Workbuddy { edition, session } = &mut other_edition { *edition = "ai".into(); session.domain = "www.workbuddy.ai".into(); }
        assert_eq!(save(&db, "browser-ai", "国际版", &other_edition, &LoginDetection::default()).unwrap(), "browser-ai");
        assert_eq!(crate::connections::list(&db).unwrap().len(), 4);
    }
    #[test]
    fn qoder_and_trae_reauthorization_bind_user_and_edition_without_merging_accounts() {
        for original in [
            Credential::Qoder { edition: "cn".into(), session: crate::qoder_desktop::Session {
                token: "synthetic-token".into(), refresh_token: None, expires_at: None,
                account: None, user_id: Some("synthetic-user".into()) } },
            Credential::Trae { edition: "cn".into(), session: crate::trae_browser_auth::Session {
                token: "synthetic-token".into(), host: "https://cloudide.bytedance.net".into(), expires_at: "2099-01-01T00:00:00Z".into(),
                account: None, user_id: "synthetic-user".into() } },
        ] {
            let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
            save(&db, "browser-a", "账号A", &original, &LoginDetection::default()).unwrap();
            assert_eq!(save(&db, "browser-retry", "账号A重授权", &original, &LoginDetection::default()).unwrap(), "browser-a");
            let mut other = original.clone();
            match &mut other {
                Credential::Qoder { session, .. } => session.user_id = Some("synthetic-other".into()),
                Credential::Trae { session, .. } => session.user_id = "synthetic-other".into(),
                _ => unreachable!(),
            }
            assert_eq!(save(&db, "browser-b", "账号B", &other, &LoginDetection::default()).unwrap(), "browser-b");
            match &mut other {
                Credential::Qoder { edition, session } => { *edition = "global".into(); session.user_id = Some("synthetic-user".into()); },
                Credential::Trae { edition, session } => { *edition = "global".into(); session.user_id = "synthetic-user".into(); },
                _ => unreachable!(),
            }
            assert_eq!(save(&db, "browser-global", "国际版", &other, &LoginDetection::default()).unwrap(), "browser-global");
            assert_eq!(crate::connections::list(&db).unwrap().len(), 3);
        }
    }
    #[test]
    fn late_verified_response_cannot_revive_cancelled_or_expired_attempts() {
        for expired in [false, true] {
            let attempt = synthetic_attempt();
            if expired { attempt.lock().unwrap().started = Instant::now() - LIFETIME; }
            else { attempt.lock().unwrap().cancelled.store(true, Ordering::Release); }
            ready(&attempt, synthetic_credential(), QuotaState::Ok { windows: vec![], plan: None, account: None });
            let state = attempt.lock().unwrap();
            assert!(!state.saveable());
            assert!(state.credential.is_none());
        }
    }
    fn saved_workbuddy_qoder_credentials() -> Vec<(String, Credential)> {
        assert!(!crate::validation::isolated(), "隔离验收不读取真实授权");
        let path = std::path::PathBuf::from(std::env::var_os("APPDATA").expect("缺少应用目录"))
            .join("dev.ningz.jolu/usage.db");
        let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap_or_else(|_| panic!("无法只读打开 Jolu 账号数据库"));
        let mut stmt = db.prepare("SELECT id,platform,secret FROM connections WHERE platform IN ('workbuddy','qoder') AND kind='auth' AND secret LIKE 'jolu-browser:v1:%'").unwrap();
        let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        drop(stmt);
        drop(db);
        assert!(!rows.is_empty(), "没有已保存的 WorkBuddy/Qoder 浏览器账号样本");
        rows.into_iter().map(|(id, platform, sealed)| {
            let credential = load(&id, &platform, &sealed).unwrap_or_else(|_| panic!("已保存凭证无法解封"));
            (platform, credential)
        }).collect()
    }
    #[test]
    #[ignore = "显式执行：只读已保存 WorkBuddy/Qoder 浏览器凭证并核实官方身份，不新登录、不领取"]
    fn saved_workbuddy_qoder_identities_read_only_probe() {
        let mut valid = true;
        for (platform, credential) in saved_workbuddy_qoder_credentials() {
            let (edition, verified) = match credential {
                Credential::Workbuddy { edition, session } => {
                    let verified = crate::workbuddy_browser_auth::verify_session(&edition, &session).is_ok();
                    (edition, verified)
                },
                Credential::Qoder { edition, session } => {
                    let verified = crate::qoder_desktop::verify_session(&edition, &session).is_ok();
                    (edition, verified)
                },
                _ => panic!("浏览器凭证平台不匹配"),
            };
            println!("{platform} {edition}: fresh_identity_verified={verified}");
            valid &= verified;
        }
        assert!(valid, "有已保存浏览器账号的实时身份验证未通过；账号和凭证不输出");
    }
    #[test]
    #[ignore = "显式执行：仅读取独立浏览器账号签到状态；WorkBuddy为官方只读状态POST，Qoder为GET，绝不领取"]
    fn saved_workbuddy_qoder_checkin_status_read_only_probe() {
        use crate::checkin_types::State;
        let mut valid = true;
        for (platform, credential) in saved_workbuddy_qoder_credentials() {
            let (edition, outcome) = match credential {
                Credential::Workbuddy { edition, session } => {
                    let outcome = match crate::checkin_workbuddy::Account::from_browser(&edition, &session) {
                        Ok(account) => account.status(), Err(outcome) => outcome,
                    };
                    (edition, outcome)
                },
                Credential::Qoder { edition, session } => {
                    let outcome = match crate::checkin_qoder::Account::from_browser(&edition, &session) {
                        Ok(account) => account.status(), Err(outcome) => outcome,
                    };
                    (edition, outcome)
                },
                _ => panic!("浏览器凭证平台不匹配"),
            };
            println!("{platform} {edition}: checkin_state={:?} reward={:?}", outcome.state, outcome.reward);
            valid &= matches!(outcome.state, State::Ready | State::Claimed | State::Inactive | State::Forbidden | State::Unsupported);
        }
        assert!(valid, "有独立浏览器账号的签到状态查询未通过；账号和凭证不输出");
    }
    #[test]
    fn sealed_credentials_are_bound_to_connection_and_not_exposed_in_lists() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let credential = synthetic_credential();
        let id = save(&db, "browser-test", "测试浏览器", &credential, &LoginDetection::default()).unwrap();
        let row = crate::connections::query_creds(&db, &id).unwrap().unwrap();
        let sealed = row.secret.as_deref().unwrap();
        assert!(!sealed.contains("synthetic-private-token"));
        assert!(load(&id, "workbuddy", sealed).is_ok());
        assert_eq!(crate::connections::list(&db).unwrap()[0].edition.as_deref(),Some("cn"));
        assert!(load("another-id", "workbuddy", sealed).is_err());
        assert!(load(&id, "qoder", sealed).is_err());
        assert!(!serde_json::to_string(&crate::connections::list(&db).unwrap()).unwrap().contains("synthetic-private-token"));
        crate::connections::set_paused(&db, &id, true).unwrap();
        assert!(crate::connections::query_creds(&db, &id).is_err());
        let paused=crate::connections::creds_of(&db,&id).unwrap().unwrap();
        assert!(is_browser(paused.secret.as_deref()));
        assert!(load(&id,"workbuddy",paused.secret.as_deref().unwrap()).is_ok());
        crate::connections::set_paused(&db,&id,false).unwrap();
        assert!(crate::connections::query_creds(&db,&id).unwrap().is_some());
    }
    #[test]
    fn regions_only_accept_known_official_targets() {
        assert!(supported("workbuddy", "cn"));
        assert!(supported("qoder", "global"));
        assert!(!supported("workbuddy", "unknown"));
        for platform in ["claude","codex","gemini","grok","opencode","zcode"] {assert!(supported(platform,"default"));assert!(!supported(platform,"cn"));}
        for platform in ["qoder","trae"] {assert!(supported(platform,"cn"));assert!(supported(platform,"global"));assert!(!supported(platform,"ai"));}
        assert!(!supported("custom","default"));
    }
    #[test]
    fn cancelled_expired_or_consumed_authorization_cannot_be_saved() {
        let credential=synthetic_credential();
        let mut attempt=Attempt{started:Instant::now(),cancelled:Arc::new(AtomicBool::new(false)),status:Status{state:"ready",detection:Some(LoginDetection::default()),message:None},credential:Some(credential)};
        assert!(attempt.saveable());
        attempt.cancelled.store(true,Ordering::Release);
        assert!(!attempt.saveable());
        attempt.cancelled.store(false,Ordering::Release);
        attempt.started=Instant::now()-LIFETIME;
        assert!(!attempt.saveable());
        attempt.started=Instant::now();
        attempt.credential=None;
        assert!(!attempt.saveable());
    }
    #[test]
    #[ignore="鼠鼠授权后显式执行：只创建未登录的官方设备授权，打印站点与状态，不打开浏览器、不输出授权参数"]
    fn official_authorization_begin_probe() {
        let platform=std::env::var("JOLU_AUTH_PROBE_PLATFORM").unwrap_or_else(|_|"grok".into());
        assert!(matches!(platform.as_str(),"grok"|"zcode"|"trae"));
        let edition=if platform=="trae" {std::env::var("JOLU_AUTH_PROBE_EDITION").unwrap_or_else(|_|"cn".into())}else{"default".into()};
        let mut pending=PendingLogin::begin(&platform,&edition).unwrap_or_else(|reason|panic!("{platform} official begin: {reason}"));
        let url=url::Url::parse(pending.login_url().unwrap()).unwrap();
        println!("{platform}: begin=ok host={}",url.host_str().unwrap_or("?"));
        assert!(pending.poll().unwrap_or_else(|reason|panic!("{platform} initial poll: {reason}")).is_none());
    }
}
