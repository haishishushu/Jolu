//! 一键签到：账号归属、领取前检查、并发去重和本机脱敏记录。
use crate::checkin_stats::CheckinStatistics;
use crate::checkin_types::{Outcome, State};
use rusqlite::{params, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    sync::{LazyLock, Mutex},
};
use tauri::{Emitter, Manager, WebviewWindow};

pub(crate) fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS daily_checkin_records (
        id INTEGER PRIMARY KEY, connection_id TEXT NOT NULL, name TEXT NOT NULL,
        platform TEXT NOT NULL, edition TEXT NOT NULL, account_key TEXT NOT NULL,
        outcome TEXT NOT NULL, created_ms INTEGER NOT NULL
    ); CREATE INDEX IF NOT EXISTS idx_daily_checkin_records_time ON daily_checkin_records(created_ms DESC, id DESC);
    CREATE INDEX IF NOT EXISTS idx_daily_checkin_records_account_time ON daily_checkin_records(account_key,created_ms);")
}

fn native_edition(id: &str, platform: &str) -> Option<&'static str> {
    match (id, platform) {
        ("monitor:native:workbuddy:main", "workbuddy")
        | ("monitor:native:qoder:cn", "qoder")
        | ("monitor:native:trae:cn", "trae") => Some("cn"),
        ("monitor:native:workbuddy:ai", "workbuddy") => Some("ai"),
        ("monitor:native:qoder:global", "qoder") | ("monitor:native:trae:ide", "trae") => {
            Some("global")
        }
        _ => None,
    }
}

fn account_key(platform: &str, edition: &str, identity: &str) -> String {
    let mut hash = Sha256::new();
    for part in [platform, edition, identity] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

fn perform(
    status: impl FnOnce() -> Outcome,
    recheck: impl FnOnce() -> Result<(), Outcome>,
    claim: impl FnOnce() -> Outcome,
) -> (Outcome, bool) {
    let current = status();
    if current.state != State::Ready {
        return (current, false);
    }
    if let Err(outcome) = recheck() {
        return (outcome, false);
    }
    (claim(), true)
}

fn check_snapshot(
    conn: &Connection,
    id: &str,
    expected: &crate::connections::Creds,
) -> Result<(), Outcome> {
    crate::connections::record_query_result(conn, id, expected, None)
        .map(|_| ())
        .map_err(|_| Outcome::new(State::Unknown, "账号连接已变化，已取消本次领取，请刷新状态"))
}
fn check_expected_key(actual: &str, expected: Option<&str>) -> Result<(), Outcome> {
    if expected.is_some_and(|key| !key.is_empty() && key == actual) {
        Ok(())
    } else {
        Err(Outcome::new(
            State::Unknown,
            "账号身份与页面查询时不同，已取消领取，请刷新状态",
        ))
    }
}

static IN_FLIGHT: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
struct Flight(String);
impl Flight {
    fn acquire(key: String) -> Result<Self, String> {
        if !IN_FLIGHT
            .lock()
            .map_err(|_| "签到状态暂不可用")?
            .insert(key.clone())
        {
            return Err("此账号正在签到，请等待本次操作完成".into());
        }
        Ok(Self(key))
    }
}
impl Drop for Flight {
    fn drop(&mut self) {
        if let Ok(mut keys) = IN_FLIGHT.lock() {
            keys.remove(&self.0);
        }
    }
}

#[derive(Clone, Serialize)]
pub(crate) struct AccountDto {
    id: String,
    platform: String,
    edition: Option<String>,
    name: String,
    account: Option<String>,
    connection_status: String,
}

#[derive(Serialize)]
pub(crate) struct RecordDto {
    id: i64,
    connection_id: String,
    name: String,
    platform: String,
    edition: String,
    outcome: Outcome,
    created_ms: i64,
}

#[derive(Serialize)]
pub(crate) struct Overview {
    accounts: Vec<AccountDto>,
    records: Vec<RecordDto>,
}

#[derive(Serialize)]
pub(crate) struct Update {
    id: String,
    account_key: Option<String>,
    account: Option<String>,
    outcome: Outcome,
    checked_at: i64,
    statistics: Option<CheckinStatistics>,
}

fn overview(conn: &Connection) -> Result<Overview, String> {
    let accounts = crate::connections::list(conn)
        .map_err(|_| "读取签到账号失败")?
        .into_iter()
        .filter(|c| {
            c.kind == "auth" && matches!(c.platform.as_str(), "workbuddy" | "qoder" | "trae")
        })
        .map(|c| {
            let account = display_account(conn, &c);
            AccountDto {
            id: c.id,
            platform: c.platform,
            edition: c.edition,
            name: c.name,
            account,
            connection_status: c.status,
        }})
        .collect();
    Ok(Overview {
        accounts,
        records: records(conn)?,
    })
}

fn display_account(conn: &Connection, row: &crate::connections::ConnectionDto) -> Option<String> {
    let saved = crate::connections::query_creds(conn, &row.id).ok().flatten();
    if let Some(edition) = native_edition(&row.id, &row.platform) {
        // monitor 指向原工具当前登录，旧 account_email 快照不能覆盖换号后的身份。
        let source = row.id.strip_prefix("monitor:")?;
        let current = match row.platform.as_str() {
            "workbuddy" => crate::workbuddy_quota::source_account(source).ok().flatten(),
            "trae" => crate::trae_quota::source_account(source).ok().flatten(),
            "qoder" => crate::qoder_desktop::local_session(edition).ok().and_then(|session| session.account),
            _ => None,
        };
        return current.or_else(|| {
            let (_, account) = resolve(&row.id, saved.as_ref()?).ok()?;
            let key = account_key(&row.platform, edition, account.identity());
            Some(format!("账号 · {}", &key[..16]))
        });
    }
    if let Some(secret) = saved.as_ref().and_then(|value| value.secret.as_deref())
        .filter(|secret| crate::provider_browser_auth::is_browser(Some(secret))) {
        return crate::provider_browser_auth::load(&row.id, &row.platform, secret).ok()?.account_label();
    }
    row.account_email.clone()
}

fn records(conn: &Connection) -> Result<Vec<RecordDto>, String> {
    let mut stmt = conn.prepare("SELECT id,connection_id,name,platform,edition,outcome,created_ms FROM daily_checkin_records ORDER BY created_ms DESC,id DESC LIMIT 50")
        .map_err(|_| "读取签到记录失败")?;
    let rows = stmt
        .query_map([], |r| {
            let raw: String = r.get(5)?;
            let outcome = serde_json::from_str(&raw).map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok(RecordDto {
                id: r.get(0)?,
                connection_id: r.get(1)?,
                name: r.get(2)?,
                platform: r.get(3)?,
                edition: r.get(4)?,
                outcome,
                created_ms: r.get(6)?,
            })
        })
        .map_err(|_| "读取签到记录失败")?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|_| "签到记录格式无效".into())
}

fn record(
    conn: &Connection,
    id: &str,
    snapshot: &crate::connections::Creds,
    edition: &str,
    key: &str,
    outcome: &Outcome,
) -> Result<(), String> {
    let raw = serde_json::to_string(outcome).map_err(|_| "无法保存签到结果")?;
    conn.execute("INSERT INTO daily_checkin_records(connection_id,name,platform,edition,account_key,outcome,created_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![id,snapshot.name,snapshot.platform,edition,key,raw,chrono::Utc::now().timestamp_millis()])
        .map_err(|_| "签到结果已返回，但本机记录保存失败，请刷新状态核实")?;
    Ok(())
}

fn persist_snapshot(
    conn: &Connection,
    id: &str,
    snapshot: &crate::connections::Creds,
    edition: &str,
    key: Option<&str>,
    outcome: &Outcome,
    attempted: bool,
) -> Result<Option<CheckinStatistics>, String> {
    let validation = crate::connections::record_query_result(conn, id, snapshot, None);
    if attempted {
        record(
            conn,
            id,
            snapshot,
            edition,
            key.unwrap_or_default(),
            outcome,
        )?;
    }
    // 领取已发生时，即使另一个入口移除了连接也保留原身份审计记录；页面仍按核验失败裁剪。
    validation?;
    // 统计不可读与领取结果分别表达，不能把已确认成功改成失败而诱发重试。
    Ok(key.and_then(|key| {
        crate::checkin_stats::read(conn, key, chrono::Utc::now().timestamp_millis()).ok()
    }))
}

enum Account {
    Workbuddy(crate::checkin_workbuddy::Account),
    Qoder(crate::checkin_qoder::Account),
    Trae(crate::checkin_trae::Account),
}
impl Account {
    fn account_label(&self) -> Option<&str> {
        match self {
            Self::Workbuddy(account) => account.account_label(),
            Self::Qoder(account) => account.session_account(),
            Self::Trae(account) => account.account_label(),
        }
    }
    fn identity(&self) -> &str {
        match self {
            Self::Workbuddy(a) => a.identity(),
            Self::Qoder(a) => a.identity(),
            Self::Trae(a) => a.identity(),
        }
    }
    fn status(&self) -> Outcome {
        match self {
            Self::Workbuddy(a) => a.status(),
            Self::Qoder(a) => a.status(),
            Self::Trae(a) => a.status(),
        }
    }
    fn claim(&self, before_post: impl FnOnce() -> Result<(), Outcome>) -> Outcome {
        match self {
            Self::Workbuddy(a) => a.claim(before_post),
            Self::Qoder(a) => a.claim(before_post),
            Self::Trae(a) => a.claim(before_post),
        }
    }
}

fn resolve(id: &str, c: &crate::connections::Creds) -> Result<(String, Account), Outcome> {
    if crate::validation::isolated() {
        return Err(Outcome::new(
            State::Unauthorized,
            "隔离验收不读取真实登录或请求官方签到接口",
        ));
    }
    if c.kind != "auth" || c.paused {
        return Err(Outcome::new(State::Forbidden, "请先连接可签到的订阅账号"));
    }
    if let Some(edition) = native_edition(id, &c.platform) {
        let account = match c.platform.as_str() {
            "workbuddy" => {
                Account::Workbuddy(crate::checkin_workbuddy::Account::from_native(edition)?)
            }
            "qoder" => Account::Qoder(crate::checkin_qoder::Account::from_native(edition)?),
            "trae" => Account::Trae(crate::checkin_trae::Account::from_native(edition)?),
            _ => unreachable!(),
        };
        return Ok((edition.into(), account));
    }
    let sealed = c
        .secret
        .as_deref()
        .filter(|s| crate::provider_browser_auth::is_browser(Some(s)))
        .ok_or_else(|| {
            Outcome::new(
                State::Unsupported,
                "此连接没有可用的签到账号来源，请在供应商页重新连接对应版本",
            )
        })?;
    let credential = crate::provider_browser_auth::load(id, &c.platform, sealed).map_err(|_| {
        Outcome::new(
            State::Unauthorized,
            "此账号授权无法读取，请在供应商页重新授权",
        )
    })?;
    match credential {
        crate::provider_browser_auth::Credential::Workbuddy { edition, session } => {
            let account = Account::Workbuddy(crate::checkin_workbuddy::Account::from_browser(
                &edition, &session,
            )?);
            Ok((edition, account))
        }
        crate::provider_browser_auth::Credential::Qoder { edition, session } => {
            let account = Account::Qoder(crate::checkin_qoder::Account::from_browser(
                &edition, &session,
            )?);
            Ok((edition, account))
        }
        crate::provider_browser_auth::Credential::Trae { edition, session } => {
            let account = Account::Trae(crate::checkin_trae::Account::from_browser(
                &edition, &session,
            )?);
            Ok((edition, account))
        }
        _ => Err(Outcome::new(State::Unsupported, "此供应商未提供每日签到")),
    }
}

fn require_main(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("只能在主窗口使用一键签到".into())
    }
}

#[tauri::command]
pub(crate) async fn daily_checkin_overview(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> Result<Overview, String> {
    require_main(&window)?;
    crate::with_reader(app.state::<crate::DbRead>().0.clone(), overview).await
}

fn finish_update(
    id: String,
    mut key: Option<String>,
    label: Option<String>,
    mut outcome: Outcome,
    attempted: bool,
    source_current: bool,
    validated: Result<Option<CheckinStatistics>, String>,
) -> Update {
    let statistics = match validated {
        Ok(statistics) if source_current => statistics,
        Ok(_) => {
            key = None;
            None
        }
        Err(message) => {
            key = None;
            // POST 可能已到达服务端；只返回待核实，不提示直接重复领取。
            outcome = Outcome::new(
                State::Unknown,
                if attempted {
                    "本次领取未能确认，账号连接或本机记录发生变化，请刷新状态核实".to_string()
                } else {
                    message
                },
            );
            None
        }
    };
    if key.is_none() { outcome.official_statistics = None; }
    let account = if key.is_some() { label } else { None };
    Update {
        id,
        account_key: key,
        account,
        outcome,
        checked_at: chrono::Utc::now().timestamp_millis(),
        statistics,
    }
}

async fn execute(
    app: tauri::AppHandle,
    id: String,
    claim: bool,
    expected_account_key: Option<String>,
) -> Result<Update, String> {
    let pool = app.state::<crate::Db>().0.clone();
    let lookup_id = id.clone();
    let snapshot = crate::with_database(pool.clone(), move |conn| {
        crate::connections::query_creds(conn, &lookup_id)?
            .ok_or_else(|| "账号连接已删除，请刷新列表".to_string())
    })
    .await?;
    let current = snapshot.clone();
    let target_id = id.clone();
    let before_post_pool = pool.clone();
    let (edition, key, label, outcome, attempted, source_current) =
        tauri::async_runtime::spawn_blocking(move || {
            let (edition, account) = match resolve(&target_id, &current) {
                Ok(value) => value,
                Err(outcome) => return Ok((None, None, None, outcome, false, false)),
            };
            if account.identity().trim().is_empty() {
                return Err("未能核实签到账号身份，请重新授权".to_string());
            }
            let key = account_key(&current.platform, &edition, account.identity());
            let label = account.account_label().filter(|label| !label.trim().is_empty() && !label.chars().any(char::is_control))
                .map(|label| label.trim().to_string())
                .or_else(|| current.secret.as_deref().filter(|secret| crate::provider_browser_auth::is_browser(Some(secret)))
                    .and_then(|secret| crate::provider_browser_auth::load(&target_id, &current.platform, secret).ok())
                    .and_then(|credential| credential.account_label()))
                .or_else(|| Some(format!("账号 · {}", &key[..16])));
            if claim {
                if let Err(outcome) = check_expected_key(&key, expected_account_key.as_deref()) {
                    return Ok((Some(edition), Some(key), label, outcome, false, true));
                }
            }
            let _flight = if claim {
                Some(Flight::acquire(key.clone())?)
            } else {
                None
            };
            let recheck = || {
                if native_edition(&target_id, &current.platform).is_some() {
                    let same = resolve(&target_id, &current).is_ok_and(|(next_edition, next)| {
                        next_edition == edition && next.identity() == account.identity()
                    });
                    if !same {
                        return Err(Outcome::new(
                            State::Unknown,
                            "原工具账号已变化，已取消领取，请刷新状态",
                        ));
                    }
                }
                // 原工具读取可能需要时间；数据库快照必须放在最后一次检查。
                let conn = before_post_pool.lock().map_err(|_| {
                    Outcome::new(State::Unknown, "暂时无法核实账号连接，已取消领取")
                })?;
                check_snapshot(&conn, &target_id, &current)?;
                Ok(())
            };
            let (mut result, attempted) = if claim {
                perform(|| account.status(), recheck, || account.claim(recheck))
            } else {
                (account.status(), false)
            };
            if attempted && result.state == State::Claimed && result.official_statistics.is_none() {
                // 只读补查最新官方汇总；实际领取金额仍取领取响应，不用状态奖励替换。
                let latest = account.status();
                if latest.state == State::Claimed { result.official_statistics = latest.official_statistics; }
            }
            // 原工具登录可在请求途中改变；不把旧账号结果应用到新账号。
            let source_current = if native_edition(&target_id, &current.platform).is_some() {
                resolve(&target_id, &current).is_ok_and(|(next_edition, next)| {
                    next_edition == edition && next.identity() == account.identity()
                })
            } else {
                true
            };
            if !source_current {
                result = Outcome::new(
                    State::Unknown,
                    "原工具账号已变化，本次结果归属需重新核实，请刷新状态",
                );
            }
            Ok((Some(edition), Some(key), label, result, attempted, source_current))
        })
        .await
        .map_err(|_| "签到查询执行异常，请刷新状态")??;
    let changed_id = id.clone();
    let saved_outcome = outcome.clone();
    // 本机记录始终归属原请求身份；页面键稍后按后置身份与数据库核验单独裁剪。
    let save_key = key.clone();
    let validated = crate::with_database(pool, move |conn| {
        persist_snapshot(
            conn,
            &changed_id,
            &snapshot,
            edition.as_deref().unwrap_or_default(),
            save_key.as_deref(),
            &saved_outcome,
            claim && attempted,
        )
    })
    .await;
    if claim && attempted {
        crate::invalidate_network_cache(&id);
        let _ = app.emit("daily-checkin-changed", id.clone());
    }
    let update = finish_update(
        id,
        key,
        label,
        outcome,
        claim && attempted,
        source_current,
        validated,
    );
    Ok(update)
}

#[tauri::command]
pub(crate) async fn daily_checkin_status(
    app: tauri::AppHandle,
    window: WebviewWindow,
    id: String,
) -> Result<Update, String> {
    require_main(&window)?;
    execute(app, id, false, None).await
}

#[tauri::command]
pub(crate) async fn daily_checkin_claim(
    app: tauri::AppHandle,
    window: WebviewWindow,
    id: String,
    expected_account_key: String,
) -> Result<Update, String> {
    require_main(&window)?;
    execute(app, id, true, Some(expected_account_key)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn native_sources_require_matching_platform_and_explicit_edition() {
        assert_eq!(
            native_edition("monitor:native:workbuddy:main", "workbuddy"),
            Some("cn")
        );
        assert_eq!(
            native_edition("monitor:native:workbuddy:ai", "workbuddy"),
            Some("ai")
        );
        assert_eq!(
            native_edition("monitor:native:qoder:global", "qoder"),
            Some("global")
        );
        assert_eq!(native_edition("monitor:native:trae:cn", "trae"), Some("cn"));
        assert_eq!(native_edition("monitor:native:trae:ide", "qoder"), None);
        assert_eq!(
            native_edition("monitor:native:qoder:invented", "qoder"),
            None
        );
    }

    #[test]
    fn preflight_only_posts_when_ready_and_never_retries_a_claim() {
        for state in [
            State::Claimed,
            State::Unknown,
            State::Inactive,
            State::Unauthorized,
            State::Forbidden,
            State::RateLimited,
            State::Failed,
            State::Unsupported,
        ] {
            let calls = Cell::new(0);
            let (outcome, posted) = perform(
                || Outcome::new(state, "status"),
                || Ok(()),
                || {
                    calls.set(calls.get() + 1);
                    Outcome::new(State::Claimed, "claim")
                },
            );
            assert_eq!(outcome.state, state);
            assert!(!posted);
            assert_eq!(calls.get(), 0);
        }
        let calls = Cell::new(0);
        let (outcome, posted) = perform(
            || Outcome::new(State::Ready, "ready"),
            || Ok(()),
            || {
                calls.set(calls.get() + 1);
                Outcome::new(State::Unknown, "结果待核实")
            },
        );
        assert!(posted);
        assert_eq!(calls.get(), 1);
        assert_eq!(outcome.state, State::Unknown);
    }

    #[test]
    fn stable_identity_lock_deduplicates_connections_and_releases_on_drop() {
        let key = account_key("qoder", "cn", "synthetic-user-1");
        assert!(!key.contains("synthetic-user-1"));
        assert_ne!(key, account_key("qoder", "global", "synthetic-user-1"));
        assert_ne!(key, account_key("trae", "cn", "synthetic-user-1"));
        let first = Flight::acquire(key.clone()).unwrap();
        assert!(Flight::acquire(key.clone()).is_err());
        drop(first);
        assert!(Flight::acquire(key).is_ok());
    }

    #[test]
    fn schema_is_additive_and_safe_to_initialize_twice() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE original_data(value TEXT); INSERT INTO original_data VALUES('keep');",
        )
        .unwrap();
        init(&conn).unwrap();
        init(&conn).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM daily_checkin_records", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        let preserved: String = conn
            .query_row("SELECT value FROM original_data", [], |r| r.get(0))
            .unwrap();
        assert_eq!(preserved, "keep");
    }

    #[test]
    fn persisted_records_roundtrip_without_credentials() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        conn.execute_batch("INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms) VALUES('synthetic','qoder','auth','test','never-store-this-secret','****',1)").unwrap();
        let expected = crate::connections::query_creds(&conn, "synthetic")
            .unwrap()
            .unwrap();
        let mut outcome = Outcome::new(State::Claimed, "领取成功");
        outcome.reward = Some(0.0);
        record(
            &conn,
            "synthetic",
            &expected,
            "cn",
            &account_key("qoder", "cn", "private-id"),
            &outcome,
        )
        .unwrap();
        let saved = records(&conn).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].outcome.state, State::Claimed);
        assert_eq!(saved[0].outcome.reward, Some(0.0));
        let dto = serde_json::to_string(&saved).unwrap();
        assert!(!dto.contains("never-store-this-secret"));
        assert!(!dto.contains("private-id"));
        assert!(!dto.contains("account_key"));
    }

    #[test]
    fn saved_success_is_included_in_statistics_in_the_same_snapshot() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        conn.execute_batch("INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms) VALUES('synthetic','qoder','auth','test','private-secret','****',1)").unwrap();
        let snapshot = crate::connections::query_creds(&conn, "synthetic")
            .unwrap()
            .unwrap();
        let mut outcome = Outcome::new(State::Claimed, "领取成功");
        outcome.reward = Some(0.0);
        let statistics = persist_snapshot(
            &conn,
            "synthetic",
            &snapshot,
            "cn",
            Some("synthetic-key"),
            &outcome,
            true,
        )
        .unwrap()
        .unwrap();
        assert!(statistics.has_records);
        assert_eq!(statistics.all.days, 1);
        assert_eq!(statistics.all.reward, Some(0.0));
        let dto = serde_json::to_string(&statistics).unwrap();
        assert!(!dto.contains("private-secret") && !dto.contains("synthetic-key"));
    }

    #[test]
    fn deleting_a_connection_does_not_lose_an_already_attempted_claim_record() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        conn.execute_batch("INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms) VALUES('removed','qoder','auth','old','private-secret','****',1)").unwrap();
        let snapshot = crate::connections::query_creds(&conn, "removed").unwrap().unwrap();
        crate::connections::remove(&conn, "removed").unwrap();
        let mut outcome = Outcome::new(State::Claimed, "领取成功");
        outcome.reward = Some(100.0);
        assert!(persist_snapshot(&conn, "removed", &snapshot, "cn", Some("original-identity"), &outcome, true).is_err());
        let saved = records(&conn).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].outcome.state, State::Claimed);
        assert_eq!(crate::checkin_stats::read(&conn, "original-identity", chrono::Utc::now().timestamp_millis()).unwrap().all.reward, Some(100.0));
        assert!(crate::connections::query_creds(&conn,"removed").unwrap().is_none());
    }

    #[test]
    fn statistics_error_does_not_fail_a_saved_success_and_missing_identity_skips_statistics() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        conn.execute_batch("INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms) VALUES('synthetic','qoder','auth','test','private-secret','****',1);
            INSERT INTO daily_checkin_records(connection_id,name,platform,edition,account_key,outcome,created_ms) VALUES('synthetic','test','qoder','cn','synthetic-key','broken',1);").unwrap();
        let snapshot = crate::connections::query_creds(&conn, "synthetic")
            .unwrap()
            .unwrap();
        let outcome = Outcome::new(State::Claimed, "领取成功");
        assert!(persist_snapshot(
            &conn,
            "synthetic",
            &snapshot,
            "cn",
            Some("synthetic-key"),
            &outcome,
            true
        )
        .unwrap()
        .is_none());
        let saved: String = conn
            .query_row(
                "SELECT outcome FROM daily_checkin_records ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Outcome>(&saved).unwrap().state,
            State::Claimed
        );
        conn.execute_batch("DROP TABLE daily_checkin_records;")
            .unwrap();
        assert!(
            persist_snapshot(&conn, "synthetic", &snapshot, "cn", None, &outcome, false)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn postflight_identity_change_hides_old_statistics_but_keeps_original_record_key() {
        let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let id = "monitor:native:qoder:cn";
        conn.execute("INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms) VALUES(?1,'qoder','auth','test','native-source','****',1)", [id]).unwrap();
        let snapshot = crate::connections::query_creds(&conn, id).unwrap().unwrap();
        let key_a = account_key("qoder", "cn", "synthetic-a");
        let key_b = account_key("qoder", "cn", "synthetic-b");
        let mut claimed = Outcome::new(State::Claimed, "领取成功");
        claimed.reward = Some(3.0);
        record(&conn, id, &snapshot, "cn", &key_a, &claimed).unwrap();
        let current = finish_update(
            id.into(),
            Some(key_a.clone()),
            Some("first@example.test".into()),
            claimed,
            false,
            true,
            persist_snapshot(
                &conn,
                id,
                &snapshot,
                "cn",
                Some(&key_a),
                &Outcome::new(State::Claimed, "已领取"),
                false,
            ),
        );
        assert_eq!(current.account_key.as_deref(), Some(key_a.as_str()));
        assert_eq!(current.account.as_deref(), Some("first@example.test"));
        assert_eq!(current.statistics.unwrap().all.reward, Some(3.0));

        let changed = Outcome::new(State::Unknown, "原工具账号已变化，请刷新状态");
        let validated = persist_snapshot(&conn, id, &snapshot, "cn", Some(&key_a), &changed, true);
        let update = finish_update(
            id.into(),
            Some(key_a.clone()),
            Some("first@example.test".into()),
            changed,
            true,
            key_a == key_b,
            validated,
        );
        assert!(update.account_key.is_none());
        assert!(update.account.is_none());
        assert!(update.statistics.is_none());
        assert_eq!(update.outcome.state, State::Unknown);
        let (saved_key, saved_outcome): (String, String) = conn
            .query_row(
                "SELECT account_key,outcome FROM daily_checkin_records ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(saved_key, key_a);
        assert_eq!(
            serde_json::from_str::<Outcome>(&saved_outcome)
                .unwrap()
                .state,
            State::Unknown
        );
        assert!(!serde_json::to_string(&update).unwrap().contains(&key_a));
    }

    #[test]
    fn final_snapshot_identity_change_hides_old_key_and_statistics_for_status_and_claim() {
        for attempted in [false, true] {
            let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
            conn.execute_batch("INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms) VALUES('synthetic','qoder','auth','test','old-source','****',1)").unwrap();
            let snapshot = crate::connections::query_creds(&conn, "synthetic")
                .unwrap()
                .unwrap();
            let key_a = account_key("qoder", "cn", "synthetic-a");
            let claimed = Outcome::new(State::Claimed, "已领取");
            record(&conn, "synthetic", &snapshot, "cn", &key_a, &claimed).unwrap();
            conn.execute(
                "UPDATE connections SET secret='new-source' WHERE id='synthetic'",
                [],
            )
            .unwrap();
            let validated = persist_snapshot(
                &conn,
                "synthetic",
                &snapshot,
                "cn",
                Some(&key_a),
                &claimed,
                attempted,
            );
            assert!(validated.is_err());
            let update = finish_update(
                "synthetic".into(),
                Some(key_a.clone()),
                Some("first@example.test".into()),
                claimed,
                attempted,
                true,
                validated,
            );
            assert!(update.account_key.is_none());
            assert!(update.account.is_none());
            assert!(update.statistics.is_none());
            assert_eq!(update.outcome.state, State::Unknown);
            let saved_key: String = conn
                .query_row("SELECT account_key FROM daily_checkin_records", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(saved_key, key_a);
        }
    }

    #[test]
    fn connection_changes_during_status_cancel_post_before_it_is_sent() {
        for mutation in [
            "UPDATE connections SET status='paused'",
            "DELETE FROM connections",
            "UPDATE connections SET secret='synthetic-new'",
        ] {
            let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
            conn.execute_batch("INSERT INTO connections(id,platform,kind,name,secret,masked,created_ms) VALUES('synthetic','qoder','auth','test','synthetic-old','****',1)").unwrap();
            let expected = crate::connections::query_creds(&conn, "synthetic")
                .unwrap()
                .unwrap();
            let calls = Cell::new(0);
            let (result, posted) = perform(
                || {
                    conn.execute_batch(mutation).unwrap();
                    Outcome::new(State::Ready, "ready")
                },
                || check_snapshot(&conn, "synthetic", &expected),
                || {
                    calls.set(calls.get() + 1);
                    Outcome::new(State::Claimed, "claim")
                },
            );
            assert_eq!(result.state, State::Unknown);
            assert!(!posted);
            assert_eq!(calls.get(), 0);
        }
    }

    #[test]
    fn switching_native_identity_since_ui_status_cancels_claim() {
        let a = account_key("workbuddy", "cn", "synthetic-user-a");
        let b = account_key("workbuddy", "cn", "synthetic-user-b");
        let calls = Cell::new(0);
        let (result, posted) = perform(
            || Outcome::new(State::Ready, "ready"),
            || check_expected_key(&b, Some(&a)),
            || {
                calls.set(1);
                Outcome::new(State::Claimed, "claim")
            },
        );
        assert_eq!(result.state, State::Unknown);
        assert!(!posted);
        assert_eq!(calls.get(), 0);
        assert!(check_expected_key(&a, None).is_err());
        assert!(check_expected_key(&a, Some(&a)).is_ok());
    }
}
