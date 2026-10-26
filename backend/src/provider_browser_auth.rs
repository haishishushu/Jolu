//! 浏览器授权凭证仅在后端保管，验证成功后才允许用户保存连接。
use crate::{login_detect::LoginDetection, quota::QuotaState};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
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
    match load(id,platform,sealed)?.query() {
        QuotaState::Ok {..}=>Ok(()),
        QuotaState::Unsupported{reason}|QuotaState::Unauthorized{reason}|QuotaState::Forbidden{reason}|QuotaState::RateLimited{reason}|QuotaState::Failed{reason}=>Err(reason),
    }
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

fn save(conn: &Connection, id: &str, name: &str, credential: &Credential, detection: &LoginDetection) -> Result<String, String> {
    let name = crate::connections::validate_name(name)?;
    let plain = serde_json::to_vec(credential).map_err(|_| "无法保存浏览器授权")?;
    let protected = crate::auth_snapshot::protect(&plain, id.as_bytes(), true)?;
    let sealed = format!("{PREFIX}{}", STANDARD.encode(protected));
    conn.execute("INSERT INTO connections(id,platform,kind,name,label,secret,masked,status,created_ms,account_email) VALUES(?1,?2,'auth',?3,'浏览器授权',?4,'已加密保存','connected',?5,?6)",
        params![id,credential.platform(),name,sealed,chrono::Utc::now().timestamp_millis(),detection.account]).map_err(|_| "保存浏览器账号失败")?;
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
    let QuotaState::Ok { plan, account, .. } = state else { return };
    if let Ok(mut attempt) = attempt.lock() {
        if attempt.cancelled.load(Ordering::Acquire) || attempt.started.elapsed() >= LIFETIME || attempt.status.state != "waiting" { return; }
        let detection = LoginDetection { found:true, account:account.and_then(|a| a.email), plan, sources:vec!["浏览器授权 · 已验证查询".into()], masked:Some("已验证，保存后加密存储".into()), message:None };
        attempt.credential = Some(credential);
        attempt.status = Status { state:"ready", detection:Some(detection), message:None };
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
                    match tauri::async_runtime::spawn_blocking(move ||query.query()).await {
                        Ok(state @ QuotaState::Ok{..})=>ready(&shared,credential,state),
                        _=>fail(&shared,"官方登录已返回，但真实查询未通过；请检查账号权限后重新授权"),
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
pub(crate) async fn save_browser_connection(app: tauri::AppHandle, window: WebviewWindow, id: String, name: String) -> Result<String, String> {
    require_main(&window)?;
    let shared = attempt(&id)?;
    let connection_id = format!("browser:{id}");
    let pool = app.state::<crate::Db>().0.clone();
    let output = crate::with_database(pool, move |conn| {
        let mut state = shared.lock().map_err(|_| "授权状态不可用")?;
        if !state.saveable() { return Err("授权尚未完成或已失效".into()); }
        let credential = state.credential.as_ref().ok_or("未获取有效授权")?;
        let detection = state.status.detection.as_ref().ok_or("未获取账号状态")?;
        let output = save(conn,&connection_id,&name,credential,detection)?;
        state.credential = None;
        state.status.state = "saved";
        Ok(output)
    }).await?;
    ATTEMPTS.lock().map_err(|_| "授权状态不可用")?.remove(&id);
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
