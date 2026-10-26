//! Qoder 桌面端已登录账号的只读额度查询与官方设备授权。
use crate::quota::{AccountMetadata, QuotaState, QuotaWindow};
use aes_gcm::{aead::{Aead, KeyInit}, Aes256Gcm, Nonce};
use base64::{engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD}, Engine};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{io::Read, path::PathBuf};

/// 仅供后端受保护的凭证存储使用，不能作为前端 DTO 返回。
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<String>,
    pub account: Option<String>,
    pub user_id: Option<String>,
}

fn failed(reason: &str) -> QuotaState { QuotaState::Failed { reason: reason.into() } }
fn unauthorized(reason: &str) -> QuotaState { QuotaState::Unauthorized { reason: reason.into() } }
fn unsupported(reason: &str) -> QuotaState { QuotaState::Unsupported { reason: reason.into() } }

fn endpoints(edition: &str) -> Result<(&'static str, &'static str, &'static str), String> {
    match edition {
        "cn" => Ok(("https://qoder.cn", "https://openapi.qoder.com.cn", "com.qodercn.app.stable")),
        "global" => Ok(("https://qoder.com", "https://openapi.qoder.sh", "com.qoder.app.stable")),
        _ => Err("未知 Qoder 版本".into()),
    }
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::trim).filter(|s| !s.is_empty() && s.len() <= 320 && !s.chars().any(char::is_control)).map(str::to_owned)
}
fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key)?.as_f64().filter(|n| n.is_finite() && *n >= 0.0)
}
fn percent(value: &Value, key: &str) -> Option<f64> { number(value, key).filter(|n| *n <= 1.0).map(|n| n * 100.0) }
fn secret(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().filter(|s| !s.is_empty() && s.len() <= 65536 && !s.chars().any(char::is_control)).map(str::to_owned)
}
fn expiry(value: &Value) -> Option<String> {
    if let Some(s) = value.as_str() { return chrono::DateTime::parse_from_rfc3339(s).ok().map(|date| date.to_rfc3339()); }
    let timestamp = value.as_i64().filter(|n| *n > 0)?;
    let date = if timestamp >= 1_000_000_000_000 { chrono::DateTime::from_timestamp_millis(timestamp) }
        else { chrono::DateTime::from_timestamp(timestamp, 0) }?;
    Some(date.to_rfc3339())
}

fn bucket(value: &Value, key: String, name: String, total_key: &str) -> QuotaWindow {
    let unit = text(value, "unit").unwrap_or_else(|| "credits".into());
    QuotaWindow {
        key, window_name: name, used_percent: percent(value, "percentage"),
        amount_text: number(value, "used").zip(number(value, total_key)).map(|(used, total)| format!("{used} / {total} {unit}")),
        remaining_text: number(value, "remaining").map(|remaining| format!("剩余 {remaining} {unit}")),
        // expiresAt 是套餐/资源包到期，不是额度重置。
        resets_at: None,
    }
}

fn parse_usage(value: &Value) -> QuotaState {
    if value["displayMode"] == "enterprise" { return unsupported("Qoder 企业账号的额度由企业页面提供，请在原工具查看"); }
    if value["displayMode"] != "qoder" { return failed("Qoder 未返回可识别的账号额度"); }
    let Some(usage) = value.get("qoderUsage").filter(|v| v.is_object()) else { return failed("Qoder 账号额度格式无效"); };
    let Some(plan) = text(usage, "userType") else { return failed("Qoder 未返回账号套餐信息"); };
    let mut windows = Vec::new();
    if let Some(used_percent) = percent(usage, "totalUsagePercentage") {
        windows.push(QuotaWindow { key: "qoder:overall".into(), window_name: "总额度".into(), used_percent: Some(used_percent), amount_text: None, remaining_text: None, resets_at: None });
    }
    for (key, name, total_key) in [("userQuota", "套餐额度", "total"), ("addOnQuota", "加购额度", "total"), ("orgResourcePackage", "组织额度", "cap")] {
        if let Some(value) = usage.get(key).filter(|v| v.is_object()) { windows.push(bucket(value, format!("qoder:{key}"), name.into(), total_key)); }
    }
    if let Some(packages) = usage.get("dedicatedResourcePackages").and_then(Value::as_array) {
        for (index, package) in packages.iter().filter(|v| v.is_object()).enumerate() {
            windows.push(bucket(package, format!("qoder:dedicated:{}", text(package, "id").unwrap_or_else(|| index.to_string())), text(package, "name").unwrap_or_else(|| "专属额度".into()), "total"));
        }
    }
    QuotaState::Ok { windows, plan: Some(plan), account: Some(AccountMetadata {
        expires_at: usage.get("expiresAt").and_then(expiry), subscription_source: "live",
        observed_at: Some(chrono::Utc::now().to_rfc3339()), ..AccountMetadata::default()
    }) }
}

fn auth_path(edition: &str) -> Result<PathBuf, QuotaState> {
    let (_, _, directory) = endpoints(edition).map_err(|_| unsupported("未知 Qoder 版本"))?;
    let root = std::env::var_os("APPDATA").ok_or_else(|| unauthorized("未找到 Qoder 桌面端登录目录"))?;
    Ok(PathBuf::from(root).join(directory).join("auth.v1.dat"))
}

fn read_file(path: &std::path::Path) -> Result<Vec<u8>, QuotaState> {
    let file = std::fs::File::open(path).map_err(|_| unauthorized("无法读取 Qoder 桌面登录，请先在对应版本登录"))?;
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|_| failed("读取 Qoder 登录文件失败"))?;
    if bytes.len() > 1024 * 1024 { return Err(failed("Qoder 登录文件超过大小限制")); }
    Ok(bytes)
}

fn decrypt(encrypted: &[u8], key: &[u8]) -> Result<Value, QuotaState> {
    if encrypted.len() < 31 || !encrypted.starts_with(b"v10") { return Err(unsupported("当前 Qoder 登录加密格式不受支持，请使用浏览器授权")); }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| failed("Qoder 登录加密密钥无效"))?;
    let mut plaintext = cipher.decrypt(Nonce::from_slice(&encrypted[3..15]), &encrypted[15..]).map_err(|_| unauthorized("无法解锁 Qoder 登录，请使用原 Windows 用户重新登录"))?;
    let value = serde_json::from_slice(&plaintext).map_err(|_| failed("Qoder 登录数据格式无效"));
    plaintext.fill(0);
    value
}

pub(crate) fn local_session(edition: &str) -> Result<Session, QuotaState> {
    if crate::validation::isolated() { return Err(unauthorized("隔离环境不读取本机 Qoder 登录")); }
    if !cfg!(windows) { return Err(unsupported("本机 Qoder 登录解锁目前仅支持 Windows，请使用浏览器授权")); }
    let path = auth_path(edition)?;
    let encrypted = read_file(&path)?;
    let state: Value = serde_json::from_slice(&read_file(&path.with_file_name("Local State"))?).map_err(|_| failed("Qoder 本机加密配置无效"))?;
    let wrapped = state.pointer("/os_crypt/encrypted_key").and_then(Value::as_str).and_then(|s| STANDARD.decode(s).ok()).filter(|key| key.starts_with(b"DPAPI") && key.len() > 5)
        .ok_or_else(|| unsupported("Qoder 本机密钥格式不受支持，请使用浏览器授权"))?;
    let mut key = crate::auth_snapshot::protect(&wrapped[5..], &[], false).map_err(|_| unauthorized("Windows 无法解锁 Qoder 登录，请使用原 Windows 用户重新登录"))?;
    let result = decrypt(&encrypted, &key);
    key.fill(0);
    let value = result?;
    parse_local_auth(&value,chrono::Utc::now())
}

fn parse_local_auth(value:&Value,now:chrono::DateTime<chrono::Utc>) -> Result<Session,QuotaState> {
    if value["schemaVersion"] != 1 { return Err(unsupported("Qoder 本机登录格式不受支持，请使用浏览器授权")); }
    let expires_at=value.get("expiresAt").and_then(expiry).ok_or_else(||unauthorized("Qoder 本地登录有效期无法识别，请在原工具重新登录"))?;
    if chrono::DateTime::parse_from_rfc3339(&expires_at).is_ok_and(|expires|expires<=now) {return Err(unauthorized("Qoder 本地登录已过期，请在原工具登录或使用浏览器授权"));}
    let token = secret(&value, "token").ok_or_else(|| unauthorized("Qoder 桌面端尚未登录"))?;
    let user_id = text(&value["user"], "id").ok_or_else(|| unauthorized("Qoder 桌面登录缺少账号标识，请重新登录"))?;
    Ok(Session { token, refresh_token: secret(&value, "refreshToken"), expires_at: Some(expires_at),
        account: text(&value["user"], "email").or_else(|| text(&value["user"], "name")), user_id: Some(user_id) })
}

pub(crate) fn local_auth_missing(edition:&str) -> bool { auth_path(edition).is_ok_and(|path|matches!(path.try_exists(),Ok(false))) }

fn response_json(response: reqwest::blocking::Response) -> Result<Value, QuotaState> {
    match response.status().as_u16() {
        200..=299 => (),
        401 => return Err(unauthorized("Qoder 登录已过期，请在原工具重新登录或使用浏览器授权")),
        403 => return Err(QuotaState::Forbidden { reason: "Qoder 当前账号没有读取额度的权限".into() }),
        429 => return Err(QuotaState::RateLimited { reason: "Qoder 查询过于频繁，请稍后刷新".into() }),
        _ => return Err(failed("Qoder 服务返回异常，请稍后刷新")),
    }
    let mut bytes = Vec::new();
    response.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|_| failed("读取 Qoder 响应失败"))?;
    if bytes.len() > 2 * 1024 * 1024 { return Err(failed("Qoder 响应超过大小限制")); }
    serde_json::from_slice(&bytes).map_err(|_| failed("Qoder 响应格式无效"))
}

pub(crate) fn query(edition: &str, token: &str) -> QuotaState {
    (|| {
        let (_, api, _) = endpoints(edition).map_err(|_| unsupported("未知 Qoder 版本"))?;
        let response = crate::quota::client().map_err(|_| failed("无法创建 Qoder 查询客户端"))?.get(format!("{api}/sash/api/v2/me/usage"))
            .header("Accept", "application/json").bearer_auth(token).header("Cosy-ClientType", "10").header("User-Agent", "Qoder")
            .send().map_err(|_| failed("无法连接 Qoder 额度服务，请检查网络与代理"))?;
        Ok(parse_usage(&response_json(response)?))
    })().unwrap_or_else(|state| state)
}

fn source_edition(source_id: &str) -> Result<&str, QuotaState> {
    match source_id { "native:qoder:cn" => Ok("cn"), "native:qoder:global" => Ok("global"), _ => Err(unsupported("未知 Qoder 登录来源")) }
}

pub(crate) fn source_identity(source_id: &str) -> Result<String, QuotaState> {
    let edition = source_edition(source_id)?;
    let session = local_session(edition)?;
    let mut hash = Sha256::new();
    for part in [edition, session.token.as_str(), session.user_id.as_deref().unwrap_or("")] { hash.update(part.as_bytes()); hash.update([0]); }
    Ok(format!("{:x}", hash.finalize()))
}

pub(crate) fn source_quota(source_id: &str) -> QuotaState {
    (|| {
        let edition = source_edition(source_id)?;
        if edition == "global" && !crate::validation::isolated() && local_auth_missing(edition) {
            return Ok(crate::qoder_quota::local_qoder_quota());
        }
        let session = local_session(edition)?;
        let mut state = query(edition, &session.token);
        let current = local_session(edition)?;
        if current.token != session.token || current.user_id != session.user_id { return Err(failed("Qoder 登录在查询期间发生变化，请重新刷新")); }
        if let QuotaState::Ok { account: Some(account), .. } = &mut state { account.email = session.account; }
        Ok(state)
    })().unwrap_or_else(|state| state)
}

fn auth_argument(value: &str, minimum: usize, maximum: usize) -> bool {
    (minimum..=maximum).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
}

pub(crate) fn login_url(edition: &str, nonce: &str, verifier: &str, machine_id: &str) -> Result<String, String> {
    let (auth, _, _) = endpoints(edition)?;
    if !auth_argument(nonce, 16, 256) || !auth_argument(verifier, 43, 128) || !auth_argument(machine_id, 1, 256) { return Err("Qoder 授权参数无效".into()); }
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut device = url::Url::parse(&format!("{auth}/device/selectAccounts")).map_err(|_| "Qoder 授权地址无效")?;
    device.query_pairs_mut().extend_pairs([("challenge", challenge.as_str()), ("challenge_method", "S256"), ("nonce", nonce), ("machine_id", machine_id), ("client_id", "732aef47-9cf2-46a2-95fe-4cebb5d0d1fa")]);
    // 以设备轮询接收授权，不声明属于原工具的 qoder-app:// 回调。
    let mut login = url::Url::parse(&format!("{auth}/users/sign-in")).map_err(|_| "Qoder 授权地址无效")?;
    login.query_pairs_mut().extend_pairs([("biz_variant", "qoder"), ("oauth_callback", device.as_str())]);
    Ok(login.to_string())
}

pub(crate) fn poll_login(edition: &str, nonce: &str, verifier: &str) -> Result<Option<Session>, String> {
    let (_, api, _) = endpoints(edition)?;
    if !auth_argument(nonce, 16, 256) || !auth_argument(verifier, 43, 128) { return Err("Qoder 授权参数无效".into()); }
    let client = crate::quota::client().map_err(|_| "无法创建 Qoder 授权客户端")?;
    let response = client.get(format!("{api}/api/v1/deviceToken/poll")).query(&[("nonce", nonce), ("verifier", verifier), ("challenge_method", "S256")])
        .header("Accept", "application/json").send().map_err(|_| "无法连接 Qoder 授权服务，请检查网络")?;
    if response.status().as_u16() == 404 { return Ok(None); }
    let value = response_json(response).map_err(|_| "Qoder 授权服务返回异常，请重新发起授权")?;
    let Some(token) = secret(&value, "token") else { return Ok(None) };
    let Some(refresh_token) = secret(&value, "refresh_token") else { return Ok(None) };
    let response = client.get(format!("{api}/api/v1/userinfo")).header("Accept", "application/json").bearer_auth(&token)
        .send().map_err(|_| "无法核实 Qoder 授权账号，请重试")?;
    let user = response_json(response).map_err(|_| "Qoder 授权账号校验失败，请重新授权")?;
    let user_id = text(&user, "id").or_else(|| text(&user, "user_id")).or_else(|| text(&user, "uid")).ok_or("Qoder 授权账号缺少用户标识")?;
    let expires_at = value.get("expires_at").and_then(expiry).or_else(|| value["expires_in"].as_i64().filter(|s| *s > 0).and_then(chrono::TimeDelta::try_seconds).and_then(|delta| chrono::Utc::now().checked_add_signed(delta)).map(|date| date.to_rfc3339()));
    Ok(Some(Session { token, refresh_token: Some(refresh_token), expires_at, account: text(&user, "email").or_else(|| text(&user, "name")), user_id: Some(user_id) }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn local_login_requires_valid_unexpired_account_state() {
        let now=chrono::Utc::now();
        let mut value=json!({"schemaVersion":1,"token":"synthetic-token","expiresAt":(now+chrono::TimeDelta::hours(1)).to_rfc3339(),"user":{"id":"synthetic-user"}});
        assert!(parse_local_auth(&value,now).is_ok());
        value["expiresAt"]=json!((now-chrono::TimeDelta::seconds(1)).to_rfc3339());
        assert!(matches!(parse_local_auth(&value,now),Err(QuotaState::Unauthorized{..})));
        value["expiresAt"]=json!("invalid");
        assert!(parse_local_auth(&value,now).is_err());
    }

    #[test]
    fn raw_fraction_and_dedicated_packages_keep_real_zero_and_unknown() {
        let state = parse_usage(&json!({"displayMode":"qoder","qoderUsage":{
            "userType":"pro","expiresAt":1800000000000u64,"totalUsagePercentage":0.25,
            "userQuota":{"used":0,"total":100,"percentage":0},"addOnQuota":{},
            "dedicatedResourcePackages":[{"id":"dedicated","name":"专属资源","used":20,"total":80,"remaining":60,"percentage":0.25}]}}));
        let QuotaState::Ok { windows, account, .. } = state else { panic!("expected desktop quota") };
        assert_eq!(windows.len(), 4);
        assert_eq!(windows[0].used_percent, Some(25.0));
        assert_eq!(windows[1].used_percent, Some(0.0));
        assert_eq!(windows[1].amount_text.as_deref(), Some("0 / 100 credits"));
        assert_eq!(windows[1].remaining_text, None);
        assert_eq!(windows[2].used_percent, None);
        assert_eq!(windows[2].remaining_text, None);
        assert_eq!(windows[3].remaining_text.as_deref(), Some("剩余 60 credits"));
        assert_eq!(windows[3].used_percent, Some(25.0));
        assert!(windows.iter().all(|window| window.resets_at.is_none()));
        assert!(account.unwrap().expires_at.is_some());
    }

    #[test]
    fn enterprise_and_missing_usage_do_not_fabricate_quota() {
        assert!(matches!(parse_usage(&json!({"displayMode":"enterprise","enterpriseUsage":{"detailUrl":"https://example.test"}})), QuotaState::Unsupported { .. }));
        assert!(matches!(parse_usage(&json!({"displayMode":"qoder","qoderUsage":{}})), QuotaState::Failed { .. }));
        assert!(matches!(parse_usage(&json!({"usage":{"userType":"free"}})), QuotaState::Failed { .. }));
    }

    #[test]
    fn device_flow_uses_edition_bound_hosts_and_pkce_without_foreign_callback() {
        for (edition, host) in [("cn", "qoder.cn"), ("global", "qoder.com")] {
            let url = url::Url::parse(&login_url(edition, "1234567890123456", &"a".repeat(64), "machine-id").unwrap()).unwrap();
            assert_eq!(url.host_str(), Some(host));
            let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
            let callback = url::Url::parse(&query["oauth_callback"]).unwrap();
            assert_eq!(callback.host_str(), Some(host));
            assert_eq!(callback.path(), "/device/selectAccounts");
            let parameters: std::collections::HashMap<_, _> = callback.query_pairs().into_owned().collect();
            assert_eq!(parameters["challenge"], URL_SAFE_NO_PAD.encode(Sha256::digest("a".repeat(64).as_bytes())));
            assert_eq!(parameters["challenge_method"], "S256");
            assert!(!parameters.contains_key("redirect_uri"));
            assert!(!parameters.contains_key("verifier"));
        }
        assert!(login_url("unknown", "1234567890123456", &"a".repeat(64), "machine").is_err());
        assert!(login_url("cn", "short", &"a".repeat(64), "machine").is_err());
        assert!(matches!(source_quota("monitor:native:qoder:global"), QuotaState::Unsupported { .. }));
    }

    #[test]
    fn safe_storage_decrypt_rejects_tampering_and_unknown_versions() {
        let key = [7u8; 32];
        let nonce = [4u8; 12];
        let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
        let mut encrypted = b"v10".to_vec();
        encrypted.extend_from_slice(&nonce);
        encrypted.extend(cipher.encrypt(Nonce::from_slice(&nonce), br#"{"schemaVersion":1,"token":"synthetic"}"#.as_slice()).unwrap());
        assert_eq!(decrypt(&encrypted, &key).unwrap()["token"], "synthetic");
        encrypted[15] ^= 1;
        assert!(matches!(decrypt(&encrypted, &key), Err(QuotaState::Unauthorized { .. })));
        encrypted[2] = b'2';
        assert!(matches!(decrypt(&encrypted, &key), Err(QuotaState::Unsupported { .. })));
    }

    #[test]
    #[ignore = "鼠鼠授权后执行：只读本机国内登录和官方额度，不输出身份或凭证"]
    fn local_cn_read_only_smoke() {
        let session = local_session("cn").unwrap_or_else(|_| panic!("local session unavailable"));
        assert!(session.user_id.is_some());
        let value = serde_json::to_value(source_quota("native:qoder:cn")).unwrap();
        println!("Qoder CN desktop: state={} windows={}", value["state"].as_str().unwrap_or("?"), value["windows"].as_array().map_or(0, Vec::len));
        assert_eq!(value["state"], "ok");
    }
}
