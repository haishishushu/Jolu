//! Trae 本机登录只读额度查询。CN 使用 v2，国际版使用 v1；不刷新或写回凭据。
//! tc 信封格式核对本机官方安装包 byteCrypto.js，公开盐表参考 CreditDaddy (MIT)。
//! 只读取 icube.cloudide；不读取 icube-dc 设备密钥，不使用 refreshToken。
// Public tc tables: Copyright (c) 2026 CreditDaddy contributors.
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
// of the Software, and to permit persons to whom the Software is furnished to do
// so, subject to the following conditions:
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

use crate::quota::{AccountMetadata, QuotaAmountRange, QuotaState, QuotaWindow};
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;
use sha2::{Digest, Sha256, Sha512};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

const TABLE_A: [u8; 64] = [
    82, 9, 106, 213, 48, 54, 165, 56, 191, 64, 163, 158, 129, 243, 215, 251, 124, 227, 57, 130,
    155, 47, 255, 135, 52, 142, 67, 68, 196, 222, 233, 203, 84, 123, 148, 50, 166, 194, 35, 61,
    238, 76, 149, 11, 66, 250, 195, 78, 8, 46, 161, 102, 40, 217, 36, 178, 118, 91, 162, 73, 109,
    139, 209, 37,
];
const TABLE_B: [u8; 64] = [
    31, 221, 168, 51, 136, 7, 199, 49, 177, 18, 16, 89, 39, 128, 236, 95, 96, 81, 127, 169, 25,
    181, 74, 13, 45, 229, 122, 159, 147, 201, 156, 239, 160, 224, 59, 77, 174, 42, 245, 176, 200,
    235, 187, 60, 131, 83, 153, 97, 23, 43, 4, 126, 186, 119, 214, 38, 225, 105, 20, 99, 85, 33,
    12, 125,
];
const MAX_BYTES: u64 = 2 * 1024 * 1024;

fn failed(reason: &str) -> QuotaState {
    QuotaState::Failed {
        reason: reason.into(),
    }
}
fn unauthorized(reason: &str) -> QuotaState {
    QuotaState::Unauthorized {
        reason: reason.into(),
    }
}

// 只接受经实机验证的普通 tc 格式；未知版本/私有模式不猜测、不暴力尝试。
pub(crate) fn decrypt_auth(encoded: &str) -> Option<Value> {
    if encoded.len() > 128 * 1024 {
        return None;
    }
    let raw = STANDARD.decode(encoded).ok()?;
    if raw.len() < 54 || raw[..6] != [116, 99, 5, 16, 0, 0] {
        return None;
    }
    let mut material = Sha512::digest(&raw[6..38]).to_vec();
    material.extend(TABLE_A.iter().zip(TABLE_B).map(|(a, b)| a ^ b));
    let key = Sha512::digest(&material);
    let mut cipher = raw[38..].to_vec();
    let plain = cbc::Decryptor::<aes::Aes128>::new_from_slices(&key[..16], &key[16..32])
        .ok()?
        .decrypt_padded_mut::<Pkcs7>(&mut cipher)
        .ok()?;
    if plain.len() <= 64 || Sha512::digest(&plain[64..]).as_slice() != &plain[..64] {
        return None;
    }
    serde_json::from_slice(&plain[64..]).ok()
}

fn label(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.len() <= 320 && !v.chars().any(char::is_control))
        .map(str::to_string)
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value
        .get(key)?
        .as_f64()
        .filter(|v| v.is_finite() && *v >= 0.0)
}

// 主机来自官方安装包 bootConfig.ug；不得仅以域名后缀信任任意地址。
fn endpoint(host: &str) -> Option<&'static str> {
    match host.trim_end_matches('/') {
        "https://api.trae.cn" => Some("https://api.trae.cn/trae/api/v2/pay/ide_user_ent_usage"),
        "https://grow-normal.trae.ai" => {
            Some("https://grow-normal.trae.ai/trae/api/v1/pay/ide_user_ent_usage")
        }
        "https://growsg-normal.trae.ai" => {
            Some("https://growsg-normal.trae.ai/trae/api/v1/pay/ide_user_ent_usage")
        }
        "https://growva-normal.trae.ai" => {
            Some("https://growva-normal.trae.ai/trae/api/v1/pay/ide_user_ent_usage")
        }
        "https://grow-normal.traeapi.us" => {
            Some("https://grow-normal.traeapi.us/trae/api/v1/pay/ide_user_ent_usage")
        }
        _ => None,
    }
}

fn window(
    key: String,
    name: String,
    used: Option<f64>,
    total: Option<f64>,
    unit: &str,
) -> QuotaWindow {
    QuotaWindow {
        key,
        window_name: name,
        used_percent: used
            .zip(total)
            .filter(|(u, t)| *t > 0.0 && u <= t)
            .map(|(u, t)| u / t * 100.0),
        amount_text: match (used, total) {
            (Some(u), Some(t)) => Some(format!("{u} / {t} {unit}")),
            (Some(u), None) => Some(format!("已用 {u} {unit}")),
            (None, Some(t)) => Some(format!("总额度 {t} {unit}；已用未知")),
            _ => None,
        },
        // 不把总量相减伪装成来源直接返回的剩余量；权益到期不是重置时间。
        remaining_text: None,
        resets_at: None,
    }
}

fn amount_range(
    key: String,
    name: String,
    used: Option<f64>,
    total: Option<f64>,
    unit: &str,
    unlimited: bool,
) -> QuotaAmountRange {
    QuotaAmountRange {
        key,
        name,
        total,
        used,
        unlimited,
        remaining: None,
        calculated_remaining: if unlimited { None } else {
            total.zip(used).filter(|(t, u)| u <= t).map(|(t, u)| t - u)
        },
        unit: unit.into(),
    }
}

fn parse_usage(value: &Value) -> QuotaState {
    if let Some(code) = value.get("code") {
        if code.as_i64() != Some(0) {
            return if code.as_i64() == Some(1001) {
                unauthorized("Trae 登录已失效，请在原工具刷新登录")
            } else {
                failed("Trae 额度服务返回异常状态，请稍后刷新")
            };
        }
    }
    let Some(packs) = value
        .get("user_entitlement_pack_list")
        .and_then(Value::as_array)
    else {
        return failed("Trae 额度响应格式无效");
    };
    // display_desc 是服务端真实标签。不能将积分包的到期时间当作套餐到期。
    let plan = packs
        .iter()
        .find(|p| p["entitlement_base_info"]["product_extra"]["subscription_extra"].is_object())
        .and_then(|p| label(p, "display_desc"));
    let mut windows = Vec::new();
    let mut amount_ranges = Vec::new();
    if let Some(summary) = value.get("usage_summary").filter(|v| v.is_object()) {
        let used = number(summary, "consumed_amount");
        let total = number(summary, "total_amount");
        if used.is_some() || total.is_some() {
            amount_ranges.push(amount_range(
                "trae:credits".into(), "积分汇总".into(), used, total, "积分", false,
            ));
            windows.push(window(
                "trae:credits".into(),
                "积分额度".into(),
                used,
                total,
                "积分",
            ));
        }
    }
    let has_summary = !windows.is_empty();
    {
        let dollar_billing = value["is_dollar_usage_billing"].as_bool() == Some(true);
        for (index, pack) in packs.iter().enumerate() {
            let quota = &pack["entitlement_base_info"]["quota"];
            let usage = &pack["usage"];
            let title = label(pack, "display_desc").unwrap_or_else(|| "账号额度".into());
            let fields: &[(&str, &str, &str, &str)] = if dollar_billing {
                &[
                    (
                        "basic_usage_limit",
                        "basic_usage_amount",
                        "基础用量",
                        "美元",
                    ),
                    (
                        "bonus_usage_limit",
                        "bonus_usage_amount",
                        "奖励用量",
                        "美元",
                    ),
                ]
            } else {
                &[
                    ("credits_limit", "credits_amount", "积分", "积分"),
                    (
                        "premium_model_fast_request_limit",
                        "premium_model_fast_request_usage",
                        "快速请求",
                        "次",
                    ),
                    (
                        "premium_model_slow_request_limit",
                        "premium_model_slow_request_usage",
                        "慢速请求",
                        "次",
                    ),
                    (
                        "advanced_model_request_limit",
                        "advanced_model_request_usage",
                        "高级模型请求",
                        "次",
                    ),
                    (
                        "auto_completion_limit",
                        "auto_completion_usage",
                        "自动补全",
                        "次",
                    ),
                ]
            };
            for &(limit_key, usage_key, name, unit) in fields {
                let total = number(quota, limit_key);
                let unlimited = quota[limit_key].as_i64() == Some(-1);
                let used = if usage_key == "premium_model_fast_request_usage" {
                    number(usage, "premium_model_fast_amount").or_else(|| number(usage, usage_key))
                } else {
                    number(usage, usage_key)
                };
                if total.is_some() || used.is_some() || unlimited {
                    amount_ranges.push(amount_range(
                        format!("trae:{index}:{limit_key}"),
                        format!("{title} · {name}"), used, total, unit, unlimited,
                    ));
                }
                if has_summary {
                    continue;
                }
                if unlimited {
                    windows.push(QuotaWindow {
                        key: format!("trae:{index}:{limit_key}"),
                        window_name: format!("{title} · {name}"),
                        used_percent: None,
                        amount_text: Some(
                            used.map(|u| format!("已用 {u} {unit}；不限量"))
                                .unwrap_or_else(|| "不限量".into()),
                        ),
                        remaining_text: None,
                        resets_at: None,
                    });
                } else if total.is_some() || used.is_some() {
                    windows.push(window(
                        format!("trae:{index}:{limit_key}"),
                        format!("{title} · {name}"),
                        used,
                        total,
                        unit,
                    ));
                }
            }
        }
    }
    if windows.is_empty() && plan.is_none() {
        return failed("Trae 未返回可识别的套餐或数值额度");
    }
    QuotaState::Ok {
        windows,
        plan,
        account: Some(AccountMetadata {
            subscription_source: "live",
            observed_at: Some(chrono::Utc::now().to_rfc3339()),
            amount_ranges: Some(amount_ranges),
            ..AccountMetadata::default()
        }),
    }
}

fn query_auth(auth: &Value) -> QuotaState {
    let Some(host) = auth.get("host").and_then(Value::as_str).and_then(endpoint) else {
        return QuotaState::Unsupported {
            reason: "当前 Trae 登录区域或企业私有域名尚未支持额度查询".into(),
        };
    };
    let Some(token) = auth
        .get("token")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() < 64 * 1024)
    else {
        return unauthorized("Trae 本地登录记录缺少有效凭据，请在原工具登录");
    };
    if let Some(expiry) = auth
        .get("expiredAt")
        .and_then(Value::as_str)
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
    {
        if expiry <= chrono::Utc::now() {
            return unauthorized("Trae 本地登录已过期，请在原工具刷新登录");
        }
    }
    let client = match crate::quota::client() {
        Ok(client) => client,
        Err(_) => return failed("无法创建 Trae 额度查询客户端"),
    };
    let response = match client
        .post(host)
        .header("Authorization", format!("Cloud-IDE-JWT {token}"))
        .json(&serde_json::json!({"require_usage":true,"req_source":1}))
        .send()
    {
        Ok(response) => response,
        Err(_) => return failed("Trae 额度查询失败，请检查网络后刷新"),
    };
    match response.status().as_u16() {
        401 => return unauthorized("Trae 登录已失效，请在原工具刷新登录"),
        403 => {
            return QuotaState::Forbidden {
                reason: "当前 Trae 账号无权读取额度".into(),
            }
        }
        429 => {
            return QuotaState::RateLimited {
                reason: "Trae 额度查询过于频繁，请稍后刷新".into(),
            }
        }
        200 => {}
        _ => return failed("Trae 额度服务响应异常，请稍后刷新"),
    }
    let mut body = Vec::new();
    if response.take(MAX_BYTES + 1).read_to_end(&mut body).is_err() || body.len() as u64 > MAX_BYTES
    {
        return failed("Trae 额度响应读取失败");
    }
    let Ok(value) = serde_json::from_slice(&body) else {
        return failed("Trae 额度响应格式无效");
    };
    let mut state = parse_usage(&value);
    if let QuotaState::Ok {
        account: Some(account),
        ..
    } = &mut state
    {
        account.email = label(&auth["account"], "email");
    }
    state
}

/// 只查询 Jolu 浏览器独立授权；不读取或刷新原工具登录。
pub(crate) fn authorized_quota(edition: &str, session: &crate::trae_browser_auth::Session) -> QuotaState {
    let allowed = matches!((edition, session.host.as_str()),
        ("cn", "https://api.trae.cn") |
        ("global", "https://growsg-normal.trae.ai" | "https://grow-normal.traeapi.us"));
    if !allowed { return failed("Trae 授权区域无效，请重新授权"); }
    query_auth(&serde_json::json!({
        "host":session.host,"token":session.token,"expiredAt":session.expires_at,
        "account":{"email":session.account}
    }))
}

fn read_auth(root: &Path) -> Result<Value, QuotaState> {
    let file =
        std::fs::File::open(root.join("User/globalStorage/storage.json")).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                unauthorized("未找到本机 Trae 登录，请先在 Trae 中登录")
            } else {
                failed("无法读取 Trae 本地登录文件")
            }
        })?;
    let mut data = Vec::new();
    if file.take(MAX_BYTES + 1).read_to_end(&mut data).is_err() || data.len() as u64 > MAX_BYTES {
        return Err(failed("Trae 本地登录文件读取失败"));
    }
    let storage: Value =
        serde_json::from_slice(&data).map_err(|_| failed("Trae 本地登录文件格式无效"))?;
    let encoded = storage
        .get("iCubeAuthInfo://icube.cloudide")
        .and_then(Value::as_str)
        .ok_or_else(|| unauthorized("未找到本机 Trae 登录，请先在 Trae 中登录"))?;
    decrypt_auth(encoded).ok_or_else(|| failed("Trae 本地登录格式无法识别，请在原工具检查登录状态"))
}

fn validate_auth(auth: &Value) -> Result<(), String> {
    if auth
        .get("host")
        .and_then(Value::as_str)
        .and_then(endpoint)
        .is_none()
    {
        return Err("当前 Trae 登录区域或企业私有域名尚未支持额度查询".into());
    }
    if !auth
        .get("token")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty() && s.len() < 64 * 1024 && !s.chars().any(char::is_control))
    {
        return Err("Trae 本地登录记录缺少有效凭据，请在原工具登录".into());
    }
    if let Some(expiry) = auth
        .get("expiredAt")
        .and_then(Value::as_str)
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
    {
        if expiry <= chrono::Utc::now() {
            return Err("Trae 本地登录已过期，请在原工具刷新登录".into());
        }
    }
    Ok(())
}

/// root 为变体用户数据根（%APPDATA%/Trae 或 %APPDATA%/Trae CN）；无联网和写入。
pub fn validate_source(root: &Path) -> Result<(), String> {
    let auth = read_auth(root).map_err(|state| match state {
        QuotaState::Unauthorized { reason } | QuotaState::Failed { reason } => reason,
        _ => "Trae 本地登录无法读取".into(),
    })?;
    validate_auth(&auth)
}

fn same_login(before: &Value, after: &Value) -> bool {
    // token 可能因正常续期而变化；此时也丢弃旧响应，等待使用新 token 的下一轮查询。
    ["host", "token"].iter().all(|key| {
        before.get(key).and_then(Value::as_str).is_some()
            && before.get(key).and_then(Value::as_str) == after.get(key).and_then(Value::as_str)
    })
}

fn auth_identity(auth: &Value) -> String {
    let mut hash = Sha256::new();
    for key in ["host", "token"] {
        let value = auth[key].as_str().unwrap_or_default().as_bytes();
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value);
    }
    format!("{:x}", hash.finalize())
}

pub(crate) fn source_auth(source_id: &str) -> Result<(PathBuf, Value), QuotaState> {
    if crate::validation::isolated() {
        return Err(unauthorized("隔离环境不读取本机 Trae 登录"));
    }
    let (variant, cn) = match source_id {
        "native:trae:ide" => ("Trae", false),
        "native:trae:cn" => ("Trae CN", true),
        _ => {
            return Err(QuotaState::Unsupported {
                reason: "未知 Trae 登录来源".into(),
            })
        }
    };
    let Some(root) = std::env::var_os("APPDATA") else {
        return Err(unauthorized("未找到本机 Trae 用户数据目录"));
    };
    let root = Path::new(&root).join(variant);
    let auth = read_auth(&root)?;
    if auth
        .get("host")
        .and_then(Value::as_str)
        .and_then(endpoint)
        .is_none()
    {
        return Err(QuotaState::Unsupported {
            reason: "当前 Trae 登录区域或企业私有域名尚未支持额度查询".into(),
        });
    }
    if let Err(reason) = validate_auth(&auth) {
        return Err(unauthorized(&reason));
    }
    if (auth["host"]
        .as_str()
        .unwrap_or_default()
        .trim_end_matches('/')
        == "https://api.trae.cn")
        != cn
    {
        return Err(failed(
            "Trae 登录区域与所选来源不一致，请在对应原工具检查登录",
        ));
    }
    Ok((root, auth))
}

/// 仅供后端缓存归属判断；摘要不进入日志、前端响应或统计导出。
pub fn source_identity(source_id: &str) -> Result<String, QuotaState> {
    source_auth(source_id).map(|(_, auth)| auth_identity(&auth))
}

/// 本机账号发现只检查原工具登录有效性和区域，不发网络请求或刷新凭据。
pub(crate) fn source_account(source_id: &str) -> Result<Option<String>, QuotaState> {
    source_auth(source_id).map(|(_, auth)| label(&auth["account"], "email")
        .or_else(|| label(&auth["account"], "username")))
}

/// 不跨变体回退；一个来源只查询自身账号。
pub fn source_quota(source_id: &str) -> QuotaState {
    // source_auth 在任何文件访问前执行隔离检查，身份查询也使用相同入口。
    let (root, auth) = match source_auth(source_id) {
        Ok(source) => source,
        Err(state) => return state,
    };
    let state = query_auth(&auth);
    if !read_auth(&root).is_ok_and(|current| same_login(&auth, &current)) {
        return failed("Trae 原工具登录在查询期间发生变化，请刷新后重试");
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn source_identity_changes_only_for_bound_credentials() {
        let before = json!({"host":"https://api.trae.cn","token":"synthetic-a","account":{"email":"a@example.invalid"}});
        let mut after = before.clone();
        after["account"]["email"] = json!("b@example.invalid");
        assert_eq!(auth_identity(&before), auth_identity(&after));
        after["token"] = json!("synthetic-b");
        assert_ne!(auth_identity(&before), auth_identity(&after));
        after["token"] = before["token"].clone();
        after["host"] = json!("https://grow-normal.trae.ai");
        assert_ne!(auth_identity(&before), auth_identity(&after));
    }

    #[test]
    fn cn_v2_summary_preserves_real_zero_without_inventing_reset() {
        let result = parse_usage(
            &json!({"usage_summary":{"consumed_amount":0,"consumption_ratio":0,"total_amount":4600},"user_entitlement_pack_list":[]}),
        );
        let QuotaState::Ok { windows, .. } = result else {
            panic!("v2 summary must be recognized")
        };
        assert_eq!(windows[0].used_percent, Some(0.0));
        assert_eq!(windows[0].amount_text.as_deref(), Some("0 / 4600 积分"));
        assert!(windows[0].resets_at.is_none());
    }

    #[test]
    fn amount_ranges_preserve_summary_and_each_package_with_explicit_calculation() {
        let result = parse_usage(&json!({
            "usage_summary":{"consumed_amount":250.125,"total_amount":4700},
            "user_entitlement_pack_list":[
                {"display_desc":"每日签到","entitlement_base_info":{"quota":{"credits_limit":100}},"usage":{"credits_amount":20.125}},
                {"display_desc":"注册奖励","entitlement_base_info":{"quota":{"credits_limit":4000}},"usage":{"credits_amount":230}},
                {"display_desc":"快速请求","entitlement_base_info":{"quota":{"premium_model_fast_request_limit":10}},"usage":{"premium_model_fast_amount":3}}
            ]
        }));
        let value = serde_json::to_value(result).unwrap();
        let ranges = value["account"]["amount_ranges"].as_array().expect("must expose raw quota ranges");
        assert_eq!(ranges.len(), 4);
        for (range, total, used, calculated) in [
            (&ranges[0], 4700.0, 250.125, 4449.875),
            (&ranges[1], 100.0, 20.125, 79.875),
            (&ranges[2], 4000.0, 230.0, 3770.0),
            (&ranges[3], 10.0, 3.0, 7.0),
        ] {
            assert_eq!(range["total"], json!(total));
            assert_eq!(range["used"], json!(used));
            assert_eq!(range["remaining"], Value::Null, "calculated values must not masquerade as direct source fields");
            assert_eq!(range["calculated_remaining"], json!(calculated));
        }
    }

    #[test]
    fn amount_ranges_keep_missing_and_incompatible_usage_unknown() {
        for (summary, expected) in [
            (json!({"total_amount":4700}), None),
            (json!({"total_amount":0,"consumed_amount":0}), Some(0.0)),
            (json!({"total_amount":10,"consumed_amount":11}), None),
            (json!({"total_amount":10,"consumed_amount":-1}), None),
        ] {
            let value = serde_json::to_value(parse_usage(&json!({"usage_summary":summary,"user_entitlement_pack_list":[]}))).unwrap();
            let ranges = value["account"]["amount_ranges"].as_array().expect("must preserve incomplete raw range");
            assert_eq!(ranges[0]["calculated_remaining"], json!(expected));
            assert_eq!(ranges[0]["remaining"], Value::Null);
        }
    }

    #[test]
    fn amount_ranges_preserve_unlimited_packages_with_and_without_summary_or_usage() {
        for summary in [None, Some(json!({"total_amount":100,"consumed_amount":0}))] {
            for used in [None, Some(3.0)] {
                let mut body = json!({"user_entitlement_pack_list":[{
                    "display_desc":"不限量包",
                    "entitlement_base_info":{"quota":{"credits_limit":-1,"premium_model_fast_request_limit":-1}},
                    "usage":{}
                }]});
                if let Some(summary) = &summary { body["usage_summary"] = summary.clone(); }
                if let Some(used) = used { body["user_entitlement_pack_list"][0]["usage"]["credits_amount"] = json!(used); }
                let value = serde_json::to_value(parse_usage(&body)).unwrap();
                let ranges = value["account"]["amount_ranges"].as_array().expect("unlimited quotas must have explicit ranges");
                let credit = ranges.iter().find(|range| range["key"] == "trae:0:credits_limit")
                    .expect("unlimited credit quota must be retained");
                let requests = ranges.iter().find(|range| range["key"] == "trae:0:premium_model_fast_request_limit")
                    .expect("unlimited request quota must be retained");
                for range in [credit, requests] {
                    assert_eq!(range["unlimited"], json!(true));
                    assert!(range["total"].is_null());
                    assert!(range["remaining"].is_null());
                    assert!(range["calculated_remaining"].is_null());
                }
                assert_eq!(credit["used"], json!(used));
                assert!(requests["used"].is_null());
            }
        }
    }

    #[test]
    fn summary_missing_usage_is_unknown_and_zero_total_has_no_percentage() {
        for summary in [
            json!({"total_amount":4600}),
            json!({"total_amount":0,"consumed_amount":0}),
        ] {
            let QuotaState::Ok { windows, .. } =
                parse_usage(&json!({"usage_summary":summary,"user_entitlement_pack_list":[]}))
            else {
                panic!()
            };
            assert_eq!(windows[0].used_percent, None);
        }
        assert!(matches!(
            parse_usage(&json!({"user_entitlement_pack_list":[]})),
            QuotaState::Failed { .. }
        ));
        assert!(matches!(
            parse_usage(&json!({"code":1001,"message":"secret"})),
            QuotaState::Unauthorized { .. }
        ));
    }

    #[test]
    fn international_dollar_billing_does_not_invent_request_usage_or_expiry() {
        let body = json!({"is_dollar_usage_billing":true,"user_entitlement_pack_list":[{
            "status":1,"display_desc":"Free plan","entitlement_base_info":{"end_time":1793491199,
            "product_extra":{"subscription_extra":{}},"quota":{"basic_usage_limit":1,"premium_model_fast_request_limit":10}},
            "usage":{"basic_usage_amount":0}}]});
        let QuotaState::Ok {
            windows,
            plan,
            account,
        } = parse_usage(&body)
        else {
            panic!()
        };
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].amount_text.as_deref(), Some("0 / 1 美元"));
        assert_eq!(plan.as_deref(), Some("Free plan"));
        assert!(account.unwrap().expires_at.is_none());
    }

    #[test]
    fn public_tc_format_matches_independently_generated_node_fixture() {
        const FIXTURE: &str = "dGMFEAAAKioqKioqKioqKioqKioqKioqKioqKioqKioqKioqKiojCe0EmCtuzd3TW5rJBTdgAsz1IY0L/5c3Iq1RK33eU8toRGHV0ON4mP3uxdgsy0zVPI8kL4TkMKvWEiUgy3ftYnJYSix3CmdezsNNgZJXCNvQRv9X7/E757KXgt0xdjrupQzyKj4jM26b3VZdrsPLmC0woULyyyBmqhnX/Oej8Je1JrmAWlZdbYDAHr9xpvRLCyirflgCJ7RT4vuJNrEAQSurISflkKDmo105tcGwZ3ST5ogff0nWg4r+xPIsWjjwZpLBtKIiwtDLd92dDqD/";
        let auth = decrypt_auth(FIXTURE).unwrap();
        assert_eq!(auth["token"], "synthetic-test-token");
        assert!(validate_auth(&auth).is_ok());
        let mut corrupt = STANDARD.decode(FIXTURE).unwrap();
        corrupt[50] ^= 1;
        assert!(decrypt_auth(&STANDARD.encode(&corrupt)).is_none());
        corrupt[0] = 0;
        assert!(decrypt_auth(&STANDARD.encode(&corrupt)).is_none());
        assert!(decrypt_auth("invalid").is_none());
    }

    #[test]
    fn host_allowlist_expiry_and_unknown_source_fail_closed() {
        for host in [
            "http://api.trae.cn",
            "https://api.trae.cn.evil.invalid",
            "https://api.trae.cn/path",
            "https://user@api.trae.cn",
            "https://api.trae.cn:443",
        ] {
            assert!(endpoint(host).is_none());
        }
        assert!(endpoint("https://grow-normal.traeapi.us").is_some());
        assert!(validate_auth(&json!({"token":"synthetic", "host":"https://api.trae.cn", "expiredAt":"2000-01-01T00:00:00Z"})).is_err());
        assert!(matches!(
            source_quota("unknown"),
            QuotaState::Unsupported { .. }
        ));
    }

    #[test]
    fn unlimited_and_unknown_are_not_reported_as_zero() {
        let QuotaState::Ok { windows, .. } = parse_usage(&json!({"user_entitlement_pack_list":[{
            "entitlement_base_info":{"quota":{"credits_limit":-1,"premium_model_fast_request_limit":10}},"usage":{"credits_amount":3}}]}))
        else {
            panic!()
        };
        assert_eq!(
            windows[0].amount_text.as_deref(),
            Some("已用 3 积分；不限量")
        );
        assert_eq!(windows[0].used_percent, None);
        assert_eq!(windows[1].used_percent, None);
        assert!(windows[1]
            .amount_text
            .as_ref()
            .unwrap()
            .contains("已用未知"));
    }

    #[test]
    fn login_switch_guard_ignores_profile_updates_but_rejects_changed_credentials() {
        let before = json!({"host":"https://api.trae.cn","token":"synthetic-a","account":{"email":"a@example.invalid"}});
        let mut after = before.clone();
        after["account"]["email"] = json!("b@example.invalid");
        assert!(same_login(&before, &after));
        after["token"] = json!("synthetic-b");
        assert!(!same_login(&before, &after));
        after["token"] = before["token"].clone();
        after["host"] = json!("https://grow-normal.trae.ai");
        assert!(!same_login(&before, &after));
        assert!(!same_login(&Value::Null, &Value::Null));
    }

    #[test]
    #[ignore = "显式执行：只读本机两版登录和官方额度，不刷新凭据"]
    fn local_variants_read_only_smoke() {
        for source in ["native:trae:cn", "native:trae:ide"] {
            match source_quota(source) {
                QuotaState::Ok { windows, plan, .. } => {
                    println!(
                        "{source}: ok windows={} has_plan={}",
                        windows.len(),
                        plan.is_some()
                    );
                    assert!(!windows.is_empty());
                }
                _ => panic!("Trae 只读真机查询未成功；原始认证与响应不输出"),
            }
        }
    }

    #[test]
    #[ignore = "显式执行：仅只读国内官方额度字段，输出数值与字段是否存在，不输出身份和原始响应"]
    fn cn_credit_fields_read_only_smoke() {
        let (_, auth) = source_auth("native:trae:cn")
            .unwrap_or_else(|_| panic!("Trae 国内只读登录未就绪"));
        let client = crate::quota::client().expect("Trae 国内只读客户端未就绪");
        for full_data in [false, true] {
            let response = client
                .post(endpoint("https://api.trae.cn").unwrap())
                .header("Authorization", format!("Cloud-IDE-JWT {}", auth["token"].as_str().unwrap()))
                .json(&json!({"require_usage":true,"req_source":1,"full_data":full_data}))
                .send()
                .expect("Trae 国内只读额度请求失败");
            assert_eq!(response.status().as_u16(), 200, "Trae 国内只读额度状态异常");
            let mut body = Vec::new();
            response.take(MAX_BYTES + 1).read_to_end(&mut body).expect("Trae 国内只读额度读取失败");
            assert!(body.len() as u64 <= MAX_BYTES);
            let value: Value = serde_json::from_slice(&body).expect("Trae 国内只读额度格式异常");
            assert!(value.get("code").is_none_or(|v| v.as_i64() == Some(0)), "Trae 国内只读额度业务异常");
            let summary = &value["usage_summary"];
            println!("Trae CN full_data={full_data}: summary total={:?} consumed={:?} remaining={:?} available={:?}",
                number(summary, "total_amount"), number(summary, "consumed_amount"),
                number(summary, "remaining_amount"), number(summary, "available_amount"));
            let packs = value["user_entitlement_pack_list"].as_array().expect("Trae 国内只读额度包列表缺失");
            println!("Trae CN: packs={} has_history={} has_records={}", packs.len(),
                value.get("history").is_some(), value.get("records").is_some());
            let QuotaState::Ok { account: Some(account), .. } = parse_usage(&value) else {
                panic!("Trae 国内只读额度解析未成功");
            };
            let ranges = account.amount_ranges.expect("Trae 国内只读额度未保留原始范围");
            let summary_range = ranges.iter().find(|range| range.key == "trae:credits")
                .expect("Trae 国内只读额度未保留积分汇总");
            assert!(summary_range.remaining.is_none(), "Trae 没有直接返回余额，不得伪装来源字段");
            assert_eq!(summary_range.calculated_remaining,
                summary_range.total.zip(summary_range.used).filter(|(t, u)| u <= t).map(|(t, u)| t - u));
            println!("Trae CN parsed: ranges={} calculated_remaining={:?}", ranges.len(), summary_range.calculated_remaining);
            for (index, pack) in packs.iter().enumerate() {
                println!("Trae CN pack={index}: total={:?} used={:?} remaining={:?} available={:?} source_type={:?} has_start={} has_end={} status={:?}",
                    number(&pack["entitlement_base_info"]["quota"], "credits_limit"),
                    number(&pack["usage"], "credits_amount"),
                    number(&pack["usage"], "credits_remaining"),
                    number(&pack["usage"], "credits_available"),
                    pack["entitlement_base_info"]["product_extra"]["package_extra"]["package_source_type"].as_i64(),
                    pack["entitlement_base_info"].get("start_time").is_some(),
                    pack["entitlement_base_info"].get("end_time").is_some(), pack["status"].as_i64());
            }
        }
    }
}
