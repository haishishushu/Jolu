//! Trae 官方系统浏览器授权。协议核对官方 CN 3.4.4 / Global 3.6.4 安装包
//! out/main.js 的 loginUrlBuilder、marscode/request、marscode/login。
//! 只创建 Jolu 独立的 PKCE 授权与临时设备公钥；不访问或修改 Trae 登录文件。
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};
use url::Url;

const CLIENT_ID: &str = "ono9krqynydwx5";
const ERROR: &str = "Trae 官方授权未通过，请检查账号或重新授权";
const MAX_BYTES: usize = 128 * 1024;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub(crate) token: String,
    pub(crate) host: String,
    pub(crate) expires_at: String,
    pub(crate) account: Option<String>,
    pub(crate) user_id: String,
}

pub(crate) struct Pending {
    pub(crate) login_url: String,
    listener: TcpListener,
    edition: String,
    state: String,
    verifier: String,
    device: String,
    machine: String,
    public_key: String,
    started: Instant,
    consumed: bool,
}

fn region(edition: &str, tag: &str) -> Result<&'static str, String> {
    match (edition, tag) {
        ("cn", "" | "cn") => Ok("https://api.trae.cn"),
        ("global", "" | "row") => Ok("https://growsg-normal.trae.ai"),
        ("global", "usttp") => Ok("https://grow-normal.traeapi.us"),
        _ => Err(ERROR.into()),
    }
}

fn website(edition: &str) -> Result<&'static str, String> {
    match edition {
        "cn" => Ok("https://www.trae.cn"),
        "global" => Ok("https://www.trae.ai"),
        _ => Err(ERROR.into()),
    }
}

#[cfg(windows)]
fn random(bytes: usize) -> Result<String, String> {
    use windows_sys::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };
    let mut data = vec![0; bytes];
    if unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            data.as_mut_ptr(),
            bytes as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    } < 0
    {
        return Err("无法生成 Trae 授权随机数".into());
    }
    Ok(URL_SAFE_NO_PAD.encode(data))
}
#[cfg(not(windows))]
fn random(_: usize) -> Result<String, String> {
    Err("Trae 浏览器授权当前仅支持 Windows".into())
}

// 官方 ExchangeToken 要求 DeviceInfo.DevicePublicKey。临时密钥仅在 CNG 内存中生成，
// 只导出公钥后销毁私钥；本实现不申请刷新，不在磁盘保留设备身份。
#[cfg(windows)]
fn device_public_key() -> Result<String, String> {
    use windows_sys::Win32::Security::Cryptography::*;
    unsafe {
        let mut algorithm = std::ptr::null_mut();
        if BCryptOpenAlgorithmProvider(
            &mut algorithm,
            BCRYPT_ECDSA_P256_ALGORITHM,
            std::ptr::null(),
            0,
        ) < 0
        {
            return Err(ERROR.into());
        }
        let mut key = std::ptr::null_mut();
        let result = (|| {
            if BCryptGenerateKeyPair(algorithm, &mut key, 256, 0) < 0
                || BCryptFinalizeKeyPair(key, 0) < 0
            {
                return Err(ERROR.into());
            }
            let mut blob = [0u8; 72];
            let mut written = 0;
            if BCryptExportKey(
                key,
                std::ptr::null_mut(),
                BCRYPT_ECCPUBLIC_BLOB,
                blob.as_mut_ptr(),
                blob.len() as u32,
                &mut written,
                0,
            ) < 0
                || written != 72
                || u32::from_le_bytes(blob[4..8].try_into().unwrap()) != 32
            {
                return Err(ERROR.into());
            }
            // id-ecPublicKey + prime256v1 SubjectPublicKeyInfo, uncompressed X/Y.
            let mut der = vec![
                0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06,
                0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00, 0x04,
            ];
            der.extend_from_slice(&blob[8..]);
            let encoded = STANDARD.encode(der);
            let lines = encoded
                .as_bytes()
                .chunks(64)
                .map(|c| std::str::from_utf8(c).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            Ok(format!(
                "-----BEGIN PUBLIC KEY-----\n{lines}\n-----END PUBLIC KEY-----\n"
            ))
        })();
        if !key.is_null() {
            BCryptDestroyKey(key);
        }
        BCryptCloseAlgorithmProvider(algorithm, 0);
        result
    }
}
#[cfg(not(windows))]
fn device_public_key() -> Result<String, String> {
    Err(ERROR.into())
}

fn login_url(
    edition: &str,
    port: u16,
    state: &str,
    verifier: &str,
    machine: &str,
    device: &str,
) -> Result<String, String> {
    let mut url = Url::parse(&format!("{}/authorization", website(edition)?)).map_err(|_| ERROR)?;
    // 官网只接受固定 /authorize；回传 loginTraceID，以此随机值绑定本次授权。
    // 官方公开 authorization/page.b9f91f9896.js 明确验证回调 URL 正则。
    let callback = format!("http://127.0.0.1:{port}/authorize");
    url.query_pairs_mut().extend_pairs([
        ("login_version", "1"),
        ("auth_from", "trae"),
        ("login_channel", "native_ide"),
        ("plugin_version", "jolu-0.1.0"),
        ("auth_type", "local"),
        ("client_id", CLIENT_ID),
        ("redirect", "0"),
        ("state", state),
        ("login_trace_id", state),
        ("auth_callback_url", &callback),
        ("machine_id", machine),
        ("device_id", device),
        ("x_device_id", device),
        ("x_machine_id", machine),
        ("x_device_type", "Windows"),
        ("x_app_version", version(edition)),
        ("x_app_type", "stable"),
        (
            "code_challenge",
            &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        ),
        ("code_challenge_method", "S256"),
    ]);
    Ok(url.into())
}

fn version(edition: &str) -> &'static str {
    if edition == "cn" {
        "3.4.4"
    } else {
        "3.6.4"
    }
}

pub(crate) fn begin(edition: &str) -> Result<Pending, String> {
    if crate::validation::isolated() {
        return Err("隔离环境不创建 Trae 真实授权".into());
    }
    website(edition)?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|_| "无法启动 Trae 本机授权回调")?;
    listener.set_nonblocking(true).map_err(|_| ERROR)?;
    let port = listener.local_addr().map_err(|_| ERROR)?.port();
    let state = random(32)?;
    let verifier = random(48)?;
    let machine = random(32)?;
    let device = random(32)?;
    let public_key = device_public_key()?;
    let login_url = login_url(edition, port, &state, &verifier, &machine, &device)?;
    Ok(Pending {
        login_url,
        listener,
        edition: edition.into(),
        state,
        verifier,
        device,
        machine,
        public_key,
        started: Instant::now(),
        consumed: false,
    })
}

fn pairs(raw: &str) -> Result<HashMap<String, String>, String> {
    let mut result = HashMap::new();
    for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
        if result
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Err(ERROR.into());
        }
    }
    Ok(result)
}

fn callback(target: &str, expected: &str, state: &str) -> Result<HashMap<String, String>, String> {
    let (path, query) = target.split_once('?').ok_or(ERROR)?;
    if path != expected || target.contains('#') {
        return Err(ERROR.into());
    }
    let values = pairs(query)?;
    if values.get("loginTraceID").map(String::as_str) != Some(state)
        || values.get("state").is_some_and(|v| v != state)
        || values.contains_key("error_code")
    {
        return Err(ERROR.into());
    }
    // 只接受 AuthCode + PKCE。不得接受回调直接提供的 userJwt / refreshToken。
    let auth: Value =
        serde_json::from_str(values.get("authCodeInfo").ok_or(ERROR)?).map_err(|_| ERROR)?;
    if text(&auth, "AuthCode").is_none() {
        return Err(ERROR.into());
    }
    Ok(values)
}

fn text(value: &Value, key: &str) -> Option<String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() < 64 * 1024 && !s.chars().any(char::is_control))
        .map(str::to_owned)
}

fn respond(stream: &mut TcpStream, status: &str, origin: Option<&str>, message: &str) {
    let cors = origin.map(|o| format!("Access-Control-Allow-Origin: {o}\r\nVary: Origin\r\nAccess-Control-Allow-Methods: GET, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n")).unwrap_or_default();
    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\n{cors}Content-Length: {}\r\n\r\n{message}", message.len());
}

fn receive(
    stream: &mut TcpStream,
    pending: &Pending,
) -> Result<Option<HashMap<String, String>>, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(|_| ERROR)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .map_err(|_| ERROR)?;
    let mut bytes = Vec::new();
    let until = Instant::now() + Duration::from_secs(2);
    while bytes.len() < MAX_BYTES && Instant::now() < until {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).map_err(|_| ERROR)?;
        if count == 0 {
            return Err(ERROR.into());
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let request = std::str::from_utf8(&bytes).map_err(|_| ERROR)?;
    let (head, _) = request.split_once("\r\n\r\n").ok_or(ERROR)?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next().ok_or(ERROR)?.split_whitespace();
    let method = first.next().ok_or(ERROR)?;
    let target = first.next().ok_or(ERROR)?;
    if first.next() != Some("HTTP/1.1") || first.next().is_some() || target.len() > MAX_BYTES / 2 {
        return Err(ERROR.into());
    }
    let mut headers = HashMap::new();
    for line in lines {
        let (key, value) = line.split_once(':').ok_or(ERROR)?;
        if headers
            .insert(key.to_ascii_lowercase(), value.trim())
            .is_some()
        {
            return Err(ERROR.into());
        }
    }
    let expected_host = format!(
        "127.0.0.1:{}",
        pending.listener.local_addr().map_err(|_| ERROR)?.port()
    );
    if headers.get("host") != Some(&expected_host.as_str()) {
        return Err(ERROR.into());
    }
    let origin = headers.get("origin").copied();
    if origin.is_some_and(|o| website(&pending.edition).as_deref() != Ok(o)) {
        return Err(ERROR.into());
    }
    let expected_path = "/authorize";
    if target.split('?').next() != Some(expected_path) {
        return Err(ERROR.into());
    }
    if method == "OPTIONS" {
        respond(stream, "204 No Content", origin, "");
        return Ok(None);
    }
    if method != "GET" {
        return Err(ERROR.into());
    }
    let values = callback(target, expected_path, &pending.state)?;
    respond(
        stream,
        "200 OK",
        origin,
        "Jolu 已收到授权回调，正在核实账号。请返回 Jolu 查看验证结果。",
    );
    Ok(Some(values))
}

fn post(host: &str, path: &str, body: &Value, token: Option<&str>) -> Result<Value, String> {
    let client = crate::quota::client().map_err(|_| ERROR)?;
    let mut request = client.post(format!("{host}{path}")).json(body);
    if let Some(token) = token {
        request = request.header("x-cloudide-token", token);
    }
    let response = request.send().map_err(|_| ERROR)?;
    if !response.status().is_success() {
        return Err(ERROR.into());
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ERROR)?;
    if bytes.len() > MAX_BYTES {
        return Err(ERROR.into());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| ERROR)?;
    if value["ResponseMetadata"]["Error"].is_object() {
        return Err(ERROR.into());
    }
    Ok(value)
}

fn exchange(pending: &Pending, values: &HashMap<String, String>) -> Result<Session, String> {
    let host = region(
        &pending.edition,
        values.get("userTag").map(String::as_str).unwrap_or(""),
    )?;
    // 不信任任何回调提供的 host、userInfo、token 或账号标签。
    let auth: Value =
        serde_json::from_str(values.get("authCodeInfo").ok_or(ERROR)?).map_err(|_| ERROR)?;
    let code = text(&auth, "AuthCode").ok_or(ERROR)?;
    let result = post(
        host,
        "/trae/api/v3/oauth/ExchangeToken",
        &json!({
            "ClientID": CLIENT_ID, "AuthCode": code, "CodeVerifier": pending.verifier,
            "IDEVersion": version(&pending.edition), "DeviceInfo": {
                "DeviceID": pending.device, "MachineID": pending.machine, "PlatformCode": "IDE_PC",
                "DeviceType": "PC", "DeviceName": "Jolu", "DeviceModel": "", "ClientVersion": version(&pending.edition),
                "DevicePublicKey": pending.public_key, "DeviceBrand": "", "DeviceCPU": "", "OSInfo": "Windows", "OSVersion": ""
            }
        }),
        None,
    )?;
    let token = text(&result["Result"], "Token").ok_or(ERROR)?;
    let millis = result["Result"]["TokenExpireAt"].as_i64().ok_or(ERROR)?;
    let expiry = chrono::DateTime::from_timestamp_millis(millis)
        .filter(|t| *t > chrono::Utc::now())
        .ok_or(ERROR)?;
    let user = post(
        host,
        "/cloudide/api/v3/trae/GetUserInfo",
        &json!({"ReqSource":"IDE", "IDEVersion":version(&pending.edition)}),
        Some(&token),
    )?;
    let user_id = text(&user["Result"], "UserID")
        .or_else(|| user["Result"]["UserID"].as_u64().map(|v| v.to_string()))
        .filter(|id| !id.trim().is_empty() && id.len() <= 320 && id != "0")
        .ok_or(ERROR)?;
    let account =
        text(&user["Result"], "NonPlainTextEmail").or_else(|| text(&user["Result"], "ScreenName"));
    Ok(Session {
        token,
        host: host.into(),
        expires_at: expiry.to_rfc3339(),
        account,
        user_id,
    })
}

/// 外部统一生命周期每次调用非阻塞检查；drop Pending 即关闭独立 loopback listener。
pub(crate) fn poll(pending: &mut Pending) -> Result<Option<Session>, String> {
    if pending.consumed || pending.started.elapsed() >= Duration::from_secs(600) {
        return Err("Trae 授权已结束或超时，请重新发起".into());
    }
    for _ in 0..4 {
        match pending.listener.accept() {
            Ok((mut stream, peer)) => {
                if !peer.ip().is_loopback() {
                    continue;
                }
                match receive(&mut stream, pending) {
                    Ok(Some(values)) => {
                        pending.consumed = true;
                        return exchange(pending, &values).map(Some);
                    }
                    Ok(None) => {}
                    Err(_) => respond(&mut stream, "400 Bad Request", None, "无效的授权回调"),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => return Err(ERROR.into()),
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn official_login_url_keeps_pkce_verifier_private_and_binds_callback() {
        let url = Url::parse(
            &login_url(
                "cn",
                3210,
                "synthetic-state",
                "synthetic-verifier",
                "synthetic-machine",
                "synthetic-device",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(url.origin().ascii_serialization(), "https://www.trae.cn");
        let query = pairs(url.query().unwrap()).unwrap();
        assert_eq!(
            query["auth_callback_url"],
            "http://127.0.0.1:3210/authorize"
        );
        assert_eq!(
            query["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(b"synthetic-verifier"))
        );
        assert!(!url.as_str().contains("synthetic-verifier"));
        assert_eq!(query["login_trace_id"], "synthetic-state");
        let global = Url::parse(&login_url("global", 3210, "s", "v", "m", "d").unwrap()).unwrap();
        assert_eq!(global.origin().ascii_serialization(), "https://www.trae.ai");
        assert!(login_url("private", 3210, "s", "v", "m", "d").is_err());
    }
    #[test]
    fn callback_rejects_wrong_state_duplicate_fields_and_direct_tokens() {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("authCodeInfo", r#"{"AuthCode":"synthetic-code"}"#)
            .append_pair("loginTraceID", "state")
            .finish();
        assert!(callback(&format!("/authorize?{query}"), "/authorize", "state").is_ok());
        assert!(callback(&format!("/authorize/other?{query}"), "/authorize", "state").is_err());
        assert!(callback(
            &format!("/authorize?{query}&state=other"),
            "/authorize",
            "state"
        )
        .is_err());
        assert!(callback(
            &format!("/authorize?{query}&{query}"),
            "/authorize",
            "state"
        )
        .is_err());
        assert!(callback("/authorize?userJwt=synthetic", "/authorize", "state").is_err());
        assert!(callback(
            &format!(
                "/authorize?{}",
                query.replace("loginTraceID=state", "loginTraceID=other")
            ),
            "/authorize",
            "state"
        )
        .is_err());
        assert!(callback(
            &format!("/authorize?{}", query.replace("&loginTraceID=state", "")),
            "/authorize",
            "state"
        )
        .is_err());
    }
    #[test]
    fn region_is_fixed_and_does_not_accept_callback_hosts() {
        assert_eq!(
            region("global", "row").unwrap(),
            "https://growsg-normal.trae.ai"
        );
        assert_eq!(
            region("global", "usttp").unwrap(),
            "https://grow-normal.traeapi.us"
        );
        assert!(region("cn", "usttp").is_err());
        assert!(region("global", "https://attacker.invalid").is_err());
    }
}
