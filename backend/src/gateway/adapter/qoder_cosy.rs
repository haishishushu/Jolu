//! Qoder 模型接口的 COSY 签名与请求体编码。
//! 移植自 magpie-community/plugins `packages/qoder`（MIT）；其协议来自
//! CLIProxyAPI 的 qoder 支持（https://github.com/ufec/CLIProxyAPI，MIT）。
use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use base64::{engine::general_purpose::STANDARD, Engine};
use md5::{Digest, Md5};
use rsa::{pkcs8::DecodePublicKey, Pkcs1v15Encrypt, RsaPublicKey};

/// 信封声明的客户端版本：Qoder 桌面端内置 worker 的版本（app 0.4.8 的 1.1.69），
/// 与 Cosy-ClientType 10（桌面）一致；跟随桌面端，而不是 CLI。
pub(super) const COSY_VERSION: &str = "1.1.69";

/// Qoder 客户端内置的 1024 位 RSA 公钥，包裹每次请求的 AES 密钥。
const RSA_KEY: &str = "-----BEGIN PUBLIC KEY-----
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDA8iMH5c02LilrsERw9t6Pv5Nc
4k6Pz1EaDicBMpdpxKduSZu5OANqUq8er4GM95omAGIOPOh+Nx0spthYA2BqGz+l
6HRkPJ7S236FZz73In/KVuLnwI8JJ2CbuJap8kvheCCZpmAWpb/cPx/3Vr/J6I17
XcW+ML9FoCI6AOvOzwIDAQAB
-----END PUBLIC KEY-----
";

/// 请求体编码用的乱序 Base64 字母表，'$' 补位。
const ALPHABET: &[u8; 64] = b"_doRTgHZBKcGVjlvpC,@aFSx#DPuNJme&i*MzLOEn)sUrthbf%Y^w.(kIQyXqWA!";

/// 签名所需的账号材料：access 是任务令牌（国内版换不到时为设备令牌）。
pub(super) struct Signer<'a> {
    pub uid: &'a str,
    pub name: &'a str,
    pub email: &'a str,
    pub access: &'a str,
    pub machine_id: &'a str,
}

fn segment_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        acc = ((acc << 8) | byte as u32) & 0xff_ffff;
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(ALPHABET[((acc >> bits) & 63) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (6 - bits)) & 63) as usize] as char);
    }
    while out.len() % 4 != 0 {
        out.push('$');
    }
    out
}

/// 前后三分之一互换；编码结果全是 ASCII，按字节切分即可。
fn swap_thirds(s: &str) -> String {
    let third = s.len() / 3;
    if third == 0 {
        return s.to_string();
    }
    format!("{}{}{}", &s[s.len() - third..], &s[third..s.len() - third], &s[..third])
}

/// agent_chat_generation 与模型列表的 `Encode=1` 请求体编码。
pub(super) fn encode_body(text: &str) -> String {
    swap_thirds(&segment_encode(text.as_bytes()))
}

#[cfg(test)]
fn decode_body(wire: &str) -> String {
    let s = swap_thirds(wire);
    let mut out = Vec::new();
    for group in s.as_bytes().chunks(4).filter(|g| g.len() == 4) {
        let values: Vec<u64> = group
            .iter()
            .filter(|c| **c != b'$')
            .map(|c| ALPHABET.iter().position(|a| a == c).unwrap_or(0) as u64)
            .collect();
        let x = values.iter().fold(0u64, |x, v| x * 64 + v);
        let n = 6 * values.len() / 8;
        for k in 0..n {
            out.push(((x >> (6 * values.len() - 8 * (k + 1))) & 255) as u8);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn random_bytes<const N: usize>() -> [u8; N] {
    use rsa::rand_core::RngCore;
    let mut bytes = [0u8; N];
    rsa::rand_core::OsRng.fill_bytes(&mut bytes);
    bytes
}

/// 带连字符的随机 UUID（v4），请求 ID 按 Qoder 签名器的格式。
pub(super) fn random_uuid() -> String {
    let mut b = random_bytes::<16>();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|v| format!("{v:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

/// 账号信息：AES-128-CBC 加密（密钥即 IV，取随机 UUID 的前 16 个十六进制字符），
/// 密钥再用 Qoder 公钥 PKCS#1 v1.5 包裹。
fn user_blob(u: &Signer) -> Result<(String, String), String> {
    let raw = serde_json::json!({"uid":u.uid,"aid":"","name":u.name,"email":u.email,"security_oauth_token":u.access}).to_string();
    let hex = random_uuid().replace('-', "");
    let key = &hex.as_bytes()[..16];
    let info = cbc::Encryptor::<aes::Aes128>::new_from_slices(key, key)
        .map_err(|_| "Qoder 签名密钥无效")?
        .encrypt_padded_vec_mut::<Pkcs7>(raw.as_bytes());
    let public = RsaPublicKey::from_public_key_pem(RSA_KEY).map_err(|_| "Qoder 签名公钥无效")?;
    let wrapped = public
        .encrypt(&mut rsa::rand_core::OsRng, Pkcs1v15Encrypt, key)
        .map_err(|_| "Qoder 签名失败")?;
    Ok((STANDARD.encode(info), STANDARD.encode(wrapped)))
}

/// Node 的 process.arch + "_" + process.platform：Windows 上为 x86_64_win32。
fn machine_os() -> String {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        "x86" => "ia32",
        other => other,
    };
    let platform = match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    };
    format!("{arch}_{platform}")
}

/// Cosy-MachineHostname：可打印 ASCII 原样（最多 96 字符），否则替换为安全字符；空名不发送。
fn machine_hostname() -> String {
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default();
    let safe: String = name
        .trim()
        .chars()
        .map(|c| if ('\x21'..='\x7e').contains(&c) { c } else { '-' })
        .collect();
    let safe = safe.trim_matches('-');
    safe.chars().take(96).collect()
}

fn sign(path: &str, u: &Signer, body: &str, ts: i64) -> Result<Vec<(&'static str, String)>, String> {
    if u.machine_id.is_empty() {
        return Err("Qoder 账号缺少设备标识，请重新登录".into());
    }
    let (info, key) = user_blob(u)?;
    let payload = STANDARD.encode(
        serde_json::json!({"version":"v1","requestId":random_uuid(),"info":info,"cosyVersion":COSY_VERSION,"ideVersion":""}).to_string(),
    );
    let path = path.strip_prefix("/algo").unwrap_or(path);
    let signature: String = Md5::digest(format!("{payload}\n{key}\n{ts}\n{body}\n{path}").as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let m = u.machine_id.to_string();
    Ok(vec![
        ("accept", "application/json".into()),
        ("accept-encoding", "identity".into()),
        ("content-type", "application/json".into()),
        ("authorization", format!("Bearer COSY.{payload}.{signature}")),
        ("cosy-business-product", "app".into()),
        ("cosy-business-type", "agent".into()),
        ("cosy-clientip", m.clone()),
        ("cosy-clienttype", "10".into()),
        ("cosy-data-policy", "disagree".into()),
        ("cosy-date", ts.to_string()),
        ("cosy-key", key),
        ("cosy-machineid", m.clone()),
        ("cosy-machinetoken", m),
        ("cosy-machinetype", "5".into()),
        ("cosy-machineos", machine_os()),
        ("cosy-scene", "app".into()),
        ("cosy-user", u.uid.into()),
        ("cosy-version", COSY_VERSION.into()),
        ("login-version", "v2".into()),
    ])
}

/// 非对话请求（模型列表等）的签名头。path 为 URL 的路径部分，不含查询串。
pub(super) fn cosy_headers(path: &str, u: &Signer, body: &str) -> Result<Vec<(&'static str, String)>, String> {
    sign(path, u, body, chrono::Utc::now().timestamp())
}

/// 对话请求的签名头：不带 ClientIp；没有本机原生身份时不带 MachineToken / MachineType；
/// 加上主机名与模型键。
pub(super) fn chat_headers(
    path: &str,
    u: &Signer,
    body: &str,
    model_key: &str,
    model_source: &str,
) -> Result<Vec<(&'static str, String)>, String> {
    let mut headers: Vec<(&'static str, String)> = sign(path, u, body, chrono::Utc::now().timestamp())?
        .into_iter()
        .filter(|(name, _)| !matches!(*name, "cosy-clientip" | "cosy-machinetoken" | "cosy-machinetype" | "accept"))
        .collect();
    let host = machine_hostname();
    if !host.is_empty() {
        headers.push(("cosy-machinehostname", host));
    }
    headers.push(("accept", "text/event-stream".into()));
    headers.push(("cache-control", "no-cache".into()));
    headers.push(("x-model-key", model_key.into()));
    headers.push(("x-model-source", model_source.into()));
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 期望值由 magpie 插件原实现（node）生成。
    #[test]
    fn body_codec_matches_magpie() {
        for (text, wire) in [
            ("", ""),
            ("a", "$p$#"),
            ("ab", "$SB#"),
            ("abc", "MSK#"),
            ("hello 你好", "&QPSWHq&QG(PHFru"),
            (
                r#"{"request_id":"x","messages":[{"role":"user","content":"hi"}]}"#,
                "N*BrBOjbuECLuEp*l*KnP,KWxxw$NkjiD(F^BM)umYKYu(%LBMn*JxjLmYKYDxg.Dxjwx(LzBMn*moBrBO.L",
            ),
        ] {
            assert_eq!(encode_body(text), wire, "{text}");
            assert_eq!(decode_body(wire), text);
        }
    }

    #[test]
    fn signature_follows_cosy_envelope() {
        let digest: String = Md5::digest(b"P\nK\n1700000000\nBODY\n/api/v2/model/list")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(digest, "64b1bf1ee289d16a55b8a2bb9c466987");
        let signer = Signer { uid: "u1", name: "", email: "", access: "job", machine_id: "m1" };
        let headers = sign("/algo/api/v2/model/list", &signer, "", 1_700_000_000).unwrap();
        let get = |name: &str| headers.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str());
        let auth = get("authorization").unwrap();
        let (payload, sig) = auth.strip_prefix("Bearer COSY.").unwrap().rsplit_once('.').unwrap();
        let key = get("cosy-key").unwrap();
        // 路径去掉 /algo 前缀参与签名
        let expect: String = Md5::digest(format!("{payload}\n{key}\n1700000000\n\n/api/v2/model/list").as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(sig, expect);
        let envelope: serde_json::Value = serde_json::from_slice(&STANDARD.decode(payload).unwrap()).unwrap();
        assert_eq!(envelope["cosyVersion"], COSY_VERSION);
        assert_eq!(STANDARD.decode(key).unwrap().len(), 128, "1024 位 RSA 密文");
        assert_eq!(get("cosy-machineid"), Some("m1"));
        assert_eq!(get("cosy-user"), Some("u1"));
        assert!(sign("/x", &Signer { machine_id: "", ..signer }, "", 0).is_err());
        let chat = chat_headers("/algo/api/v2/service/pro/sse/agent_chat_generation", &Signer { uid: "u1", name: "", email: "", access: "job", machine_id: "m1" }, "w", "gmodel", "system").unwrap();
        assert!(!chat.iter().any(|(n, _)| *n == "cosy-clientip" || *n == "cosy-machinetoken"));
        assert!(chat.iter().any(|(n, v)| *n == "accept" && v == "text/event-stream"));
        assert!(chat.iter().any(|(n, v)| *n == "x-model-key" && v == "gmodel"));
    }
}
