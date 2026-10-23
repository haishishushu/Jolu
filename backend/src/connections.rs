//! 连接管理（§6.3）
//!
//! 连接只有两种：官方订阅（`auth`）与 API Key（`api`）。
//! **本地会话记录不是连接**——它是统计来源，由后台自动发现，不入此表。
//!
//! ## 凭证纪律
//!
//! `secret` 只在后端查额度/余额时读取；所有跨进程边界的结构体（`ConnectionDto`）
//! 只带 `masked`，前端拿不到原值，日志也不打印原值。
//! 脱敏格式保留前缀与末 4 位，足以区分多个 Key 又不泄露内容。

use rusqlite::{params, Connection as Sql, OptionalExtension};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod manual_models_tests {
    use super::*;

    #[test]
    fn models_endpoint_normalizes_base_and_rejects_non_http() {
        assert_eq!(models_endpoint("https://gw.test", "openai", None).unwrap(), "https://gw.test/v1/models");
        assert_eq!(models_endpoint("https://gw.test/", "openai", None).unwrap(), "https://gw.test/v1/models");
        assert_eq!(models_endpoint("https://gw.test/v1", "openai", None).unwrap(), "https://gw.test/v1/models");
        assert_eq!(models_endpoint("https://gw.test/v1/", "openai", Some("claude-sonnet-4-5")).unwrap(), "https://gw.test/v1/models/claude-sonnet-4-5");
        assert_eq!(
            models_endpoint("https://generativelanguage.googleapis.com/v1beta/openai/", "openai", None).unwrap(),
            "https://generativelanguage.googleapis.com/v1beta/openai/models"
        );
        assert!(models_endpoint("ftp://gw.test", "openai", None).is_err());
        assert!(models_endpoint("not a url", "openai", None).is_err());
    }
}

#[cfg(test)]
mod gateway_tests {
    use super::*;

    #[test]
    fn official_base_urls_are_not_misrouted_to_gateway_usage() {
        for (platform, host) in [("claude", "api.anthropic.com"), ("codex", "api.openai.com")] {
            assert_eq!(normalized_base(platform, "api", Some(format!("https://{host}/v1"))), None);
            let custom = format!("https://{host}.example.test/v1");
            assert_eq!(normalized_base(platform, "api", Some(custom.clone())), Some(custom));
            assert_eq!(normalized_base(platform, "auth", Some("https://gateway.test".into())), None);
        }
    }

    #[test]
    fn local_registration_is_deduplicated_and_does_not_enable_connections() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let input = |key: &str| NewConnection { platform: "claude".into(), kind: "api".into(), name: "local".into(), secret: Some(key.into()), base_url: Some("https://example.test".into()), model: None, effort: None, context_1m: None, protocol: None };
        let (id, added) = register_local(&db, input("sk-first-1234")).unwrap();
        assert!(added);
        assert!(selectable_creds(&db, &id).is_err());
        assert_eq!(register_local(&db, input("sk-first-1234")).unwrap(), (id.clone(), false));
        let (other, added) = register_local(&db, input("sk-other-1234")).unwrap();
        assert!(added); assert_ne!(other, id);
        assert_eq!(list(&db).unwrap().len(), 2);
        set_paused(&db, &id, false).unwrap();
        register_local(&db, input("sk-first-1234")).unwrap();
        assert!(selectable_creds(&db, &id).is_ok());
        set_paused(&db, &id, true).unwrap();
        register_local(&db, input("sk-first-1234")).unwrap();
        assert!(creds_of(&db, &id).unwrap().unwrap().paused);
        // 「当前使用」按平台 + 类型 + 凭证原值比对
        assert_eq!(find_by_secret(&db, "claude", "api", "sk-other-1234").unwrap(), Some(other));
        assert_eq!(find_by_secret(&db, "claude", "auth", "sk-other-1234").unwrap(), None);
        assert_eq!(find_by_secret(&db, "codex", "api", "sk-first-1234").unwrap(), None);
    }

    #[test]
    fn paused_connection_cannot_be_reactivated_by_late_query_results() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let id = add(&db, NewConnection {
            platform: "claude".into(), kind: "api".into(), name: "pause test".into(),
            secret: Some("synthetic-pause-key".into()), base_url: None,
            model: None, effort: None, context_1m: None, protocol: None,
        }).unwrap();
        db.execute("UPDATE connections SET status = 'paused' WHERE id = ?1", [&id]).unwrap();
        touch_sync(&db, &id).unwrap();
        assert_eq!(list(&db).unwrap()[0].status, "paused");
        assert!(!record_query_status(&db, &id, true).unwrap());
        assert_eq!(list(&db).unwrap()[0].status, "paused");
        set_paused(&db, &id, false).unwrap();
        assert!(record_query_status(&db, &id, false).unwrap());
        assert!(!record_query_status(&db, &id, false).unwrap());
        assert!(record_query_status(&db, &id, true).unwrap());
        set_paused(&db, &id, true).unwrap();
        mark_expired(&db, &id).unwrap();
        assert_eq!(list(&db).unwrap()[0].status, "paused");
    }

    #[test]
    fn pause_preserves_saved_credentials_and_blocks_queries_until_resumed() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let id = add(&db, NewConnection {
            platform: "codex".into(), kind: "api".into(), name: "saved connection".into(),
            secret: Some("synthetic-preserved-key".into()), base_url: Some("https://example.test".into()),
            model: None, effort: None, context_1m: None, protocol: None,
        }).unwrap();
        set_paused(&db, &id, true).unwrap();
        assert!(query_creds(&db, &id).is_err());
        let saved = creds_of(&db, &id).unwrap().unwrap();
        assert!(saved.paused);
        assert_eq!(saved.secret.as_deref(), Some("synthetic-preserved-key"));
        assert_eq!(saved.base_url.as_deref(), Some("https://example.test"));
        assert!(!fetch_local(&db, &id).unwrap().ok);
        assert_eq!(list(&db).unwrap().len(), 1);
        set_paused(&db, &id, false).unwrap();
        assert!(query_creds(&db, &id).unwrap().is_some());
        assert_eq!(list(&db).unwrap()[0].status, "connected");
        assert!(set_paused(&db, "missing", true).is_err());
    }

    #[test]
    fn official_native_keys_require_explicit_credentials_and_supported_platform() {
        let input=|platform: &str, kind: &str, secret: Option<&str>| NewConnection {
            platform:platform.into(), kind:kind.into(), name:"official test".into(),
            secret:secret.map(str::to_string),base_url:None,model:None,effort:None,context_1m:None,protocol:None,
        };
        for platform in ["gemini","grok"] {
            assert!(prepare_new(input(platform,"api",Some("synthetic-key"))).is_ok());
            assert!(prepare_new(input(platform,"api",None)).is_err());
            assert!(prepare_new(input(platform,"auth",Some("synthetic-key"))).is_err());
        }
        assert!(prepare_new(input("trae","api",Some("synthetic-key"))).is_err());
    }

    #[test]
    fn gateway_accepts_http_and_https_domains_but_rejects_other_schemes() {
        let connection = |base: &str| NewConnection {
            platform: "claude".into(), kind: "api".into(), name: "HTTP gateway test".into(),
            secret: Some("synthetic-test-key".into()), base_url: Some(base.into()),
            model: None, effort: None, context_1m: None, protocol: None,
        };
        for base in ["http://api.example.com", "http://192.168.1.2:8080", "http://localhost:8000", "https://api.example.com", "HTTP://api.example.com"] {
            assert!(prepare_new(connection(base)).is_ok(), "should accept {base}");
        }
        for base in ["ftp://api.example.com", "file:///tmp/key", "not-a-url", "http://"] {
            assert!(prepare_new(connection(base)).is_err(), "should reject {base}");
        }
    }
}

#[cfg(test)]
mod rename_tests {
    use super::*;

    fn db() -> rusqlite::Connection {
        crate::db::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn claude_api(name: &str, secret: &str) -> NewConnection {
        NewConnection {
            platform: "claude".into(),
            kind: "api".into(),
            name: name.into(),
            secret: Some(secret.into()),
            base_url: None,
            model: None,
            effort: None,
            context_1m: None,
            protocol: None,
        }
    }

    #[test]
    fn rename_updates_name_only_and_keeps_credentials() {
        let db = db();
        let id = add(&db, claude_api("旧名称", "sk-rename-1234")).unwrap();
        rename_connection(&db, &id, "  新名称  ").unwrap();
        let row = &list(&db).unwrap()[0];
        assert_eq!(row.name, "新名称");
        // 改名只动名称：凭证脱敏标识与状态保持不变
        assert_eq!(row.masked, mask("sk-rename-1234"));
        assert_eq!(row.status, "connected");
        // 改名是纯元数据操作，断开的连接同样允许改名
        set_paused(&db, &id, true).unwrap();
        rename_connection(&db, &id, "断开也能改名").unwrap();
        assert_eq!(list(&db).unwrap()[0].name, "断开也能改名");
    }

    #[test]
    fn rename_rejects_blank_overlong_and_missing_connections() {
        let db = db();
        let id = add(&db, claude_api("原名", "sk-rename-5678")).unwrap();
        assert!(rename_connection(&db, &id, "   ").is_err());
        assert!(rename_connection(&db, &id, &"长".repeat(81)).is_err());
        assert!(rename_connection(&db, "missing", "任意").is_err());
        // 校验失败后原名保持不变
        assert_eq!(list(&db).unwrap()[0].name, "原名");
    }

    #[test]
    fn custom_name_overrides_local_inputs_only_when_valid() {
        let input = |name: &str| NewConnection {
            platform: "claude".into(),
            kind: "api".into(),
            name: name.into(),
            secret: Some("sk-custom-1234".into()),
            base_url: None,
            model: None,
            effort: None,
            context_1m: None,
            protocol: None,
        };
        // 用户填了名称：覆盖自动生成名，并去首尾空白
        let renamed = with_custom_name(vec![input("Claude API Key")], Some("  我的连接  ".into())).unwrap();
        assert_eq!(renamed[0].name, "我的连接");
        // 只填空白：视为未填，保留自动生成名
        let kept = with_custom_name(vec![input("Claude API Key")], Some("   ".into())).unwrap();
        assert_eq!(kept[0].name, "Claude API Key");
        // 未填：原样返回
        let none = with_custom_name(vec![input("Claude API Key")], None).unwrap();
        assert_eq!(none[0].name, "Claude API Key");
        // 超长名称：整体拒绝，不让半合法输入落到库里
        assert!(with_custom_name(vec![input("Claude API Key")], Some("长".repeat(81))).is_err());
    }
}

#[cfg(test)]
mod local_sync_tests {
    use super::*;

    fn db() -> rusqlite::Connection {
        crate::db::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn claude_api(name: &str, secret: &str, base: Option<&str>) -> NewConnection {
        NewConnection {
            platform: "claude".into(),
            kind: "api".into(),
            name: name.into(),
            secret: Some(secret.into()),
            base_url: base.map(str::to_string),
            model: None,
            effort: None,
            context_1m: None,
            protocol: None,
        }
    }

    #[test]
    fn update_refreshes_secret_and_base_url_in_place() {
        let db = db();
        let id = add(&db, claude_api("旧中转", "sk-old-1234", Some("https://old.example.test"))).unwrap();
        let result = apply_local(
            &db,
            &id,
            Some(LocalPair { secret: "sk-new-4567".into(), base_url: Some("https://new.example.test".into()) }),
            vec![],
        )
        .unwrap();
        assert!(result.ok && result.changed, "{result:?}");
        assert_eq!(result.masked.as_deref(), Some("sk-****4567"));
        let row = &list(&db).unwrap()[0];
        assert_eq!(row.masked, "sk-****4567");
        assert_eq!(row.base_url.as_deref(), Some("https://new.example.test"));
        assert_eq!(row.status, "connected");
    }

    #[test]
    fn update_reports_no_change_when_local_matches_after_normalization() {
        let db = db();
        let id = add(&db, claude_api("同一中转", "sk-same-1234", Some("https://same.example.test"))).unwrap();
        let result = apply_local(
            &db,
            &id,
            Some(LocalPair { secret: "sk-same-1234".into(), base_url: Some("https://same.example.test/".into()) }),
            vec![],
        )
        .unwrap();
        assert!(result.ok && !result.changed, "{result:?}");
    }

    #[test]
    fn update_without_local_secret_keeps_connection_untouched() {
        let db = db();
        let id = add(&db, claude_api("不变化", "sk-keep-1234", Some("https://keep.example.test"))).unwrap();
        let result = apply_local(&db, &id, None, vec![]).unwrap();
        assert!(!result.ok && !result.changed, "{result:?}");
        let row = &list(&db).unwrap()[0];
        assert_eq!(row.masked, mask("sk-keep-1234"));
        assert_eq!(row.base_url.as_deref(), Some("https://keep.example.test"));
    }

    #[test]
    fn update_refuses_when_another_same_kind_connection_holds_a_different_key() {
        let db = db();
        add(&db, claude_api("A", "sk-aaa-1111", None)).unwrap();
        let b = add(&db, claude_api("B", "sk-bbb-2222", None)).unwrap();
        let result = apply_local(&db, &b, Some(LocalPair { secret: "sk-ccc-3333".into(), base_url: None }), vec![]).unwrap();
        assert!(!result.ok && !result.changed, "{result:?}");
        let row = list(&db).unwrap().into_iter().find(|c| c.id == b).unwrap();
        assert_eq!(row.masked, mask("sk-bbb-2222"));
    }

    #[test]
    fn sync_updates_the_unambiguous_connection_and_reports_changed() {
        let db = db();
        add(&db, claude_api("Claude API Key", "sk-old-9999", None)).unwrap();
        let changed = sync_from_local_with(&db, &|platform, kind| {
            matches!((platform, kind), ("claude", "api"))
                .then(|| LocalPair { secret: "sk-new-8888".into(), base_url: None })
        })
        .unwrap();
        assert!(changed);
        assert_eq!(list(&db).unwrap()[0].masked, mask("sk-new-8888"));
    }

    #[test]
    fn sync_skips_when_local_missing_ambiguous_or_paused() {
        let db = db();
        let local = |secret: &str| Some(LocalPair { secret: secret.into(), base_url: None });
        let a = add(&db, claude_api("A", "sk-aaa-1111", None)).unwrap();
        // 本机没读到：不动
        assert!(!sync_from_local_with(&db, &|_, _| None).unwrap());
        // 同类型两条：分不清本机配置属于哪条，不动
        let b = add(&db, claude_api("B", "sk-bbb-2222", None)).unwrap();
        assert!(!sync_from_local_with(&db, &|_, _| local("sk-new-3333")).unwrap());
        // 暂停的连接不参与，也不会因此误更新另一条
        set_paused(&db, &b, true).unwrap();
        assert!(!sync_from_local_with(&db, &|_, _| local("sk-new-4444")).unwrap());
        assert_eq!(creds_of(&db, &a).unwrap().unwrap().secret.as_deref(), Some("sk-aaa-1111"));
        assert_eq!(creds_of(&db, &b).unwrap().unwrap().secret.as_deref(), Some("sk-bbb-2222"));
    }
}

// 任务 3.4 开始：自定义供应商与协议鉴权
#[cfg(test)]
mod custom_provider_tests {
    use super::*;

    fn custom(protocol: Option<&str>, base: Option<&str>, secret: Option<&str>) -> NewConnection {
        NewConnection {
            platform: "custom".into(),
            kind: "api".into(),
            name: "公司网关".into(),
            secret: secret.map(Into::into),
            base_url: base.map(Into::into),
            model: Some("gpt-4.1-mini".into()),
            effort: None,
            context_1m: None,
            protocol: protocol.map(Into::into),
        }
    }

    fn headers(protocol: &str) -> reqwest::header::HeaderMap {
        let client = reqwest::blocking::Client::new();
        apply_auth(client.get("https://gw.test/v1/models"), protocol, "synthetic-key").build().unwrap().headers().clone()
    }

    #[test]
    fn anthropic_protocol_headers() {
        let h = headers("anthropic");
        assert_eq!(h.get("x-api-key").unwrap(), "synthetic-key");
        assert!(h.contains_key("anthropic-version"));
        assert!(!h.contains_key("authorization"));
        let h = headers("openai");
        assert_eq!(h.get("authorization").unwrap(), "Bearer synthetic-key");
        assert!(!h.contains_key("x-api-key"));
    }

    /// 本机一次性 HTTP 桩：按给定状态与响应体回复，并把收到的请求原文（含请求体）交回给测试检查。
    fn serve_once(status: &'static str, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        serve_once_with(status, body, String::new())
    }

    /// 返回 3xx 与 Location 的桩，用于确认客户端不跟随重定向
    fn serve_once_with_location(status: &'static str, location: String) -> (String, std::thread::JoinHandle<String>) {
        serve_once_with(status, "{}", format!("Location: {location}\r\n"))
    }

    fn serve_once_with(status: &'static str, body: &'static str, extra_headers: String) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            let header_end = loop {
                if let Some(at) = request.windows(4).position(|w| w == b"\r\n\r\n") { break at + 4; }
                let n = stream.read(&mut buf).unwrap();
                if n == 0 { break request.len(); }
                request.extend_from_slice(&buf[..n]);
            };
            let head = String::from_utf8_lossy(&request[..header_end]).to_lowercase();
            let length = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
            while request.len() < header_end + length {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 { break; }
                request.extend_from_slice(&buf[..n]);
            }
            let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            stream.write_all(reply.as_bytes()).unwrap();
            stream.write_all(body.as_bytes()).unwrap();
            String::from_utf8_lossy(&request).to_lowercase()
        });
        (base, handle)
    }

    fn serve_models_once() -> (String, std::thread::JoinHandle<String>) {
        serve_once("200 OK", r#"{"data":[{"id":"model-a"},{"id":"model-b"},{"id":"model-a"}]}"#)
    }

    #[test]
    fn verify_model_sends_one_minimal_request_per_protocol() {
        let (base, server) = serve_once("200 OK", r#"{"id":"msg_1"}"#);
        assert_eq!(verify_model("anthropic", &base, "synthetic-key", " claude-x "), Ok(()));
        let request = server.join().unwrap();
        assert!(request.starts_with("post /v1/messages "), "{request}");
        assert!(request.contains("x-api-key: synthetic-key") && !request.contains("authorization:"));
        assert!(request.contains(r#""model":"claude-x""#) && request.contains(r#""max_tokens":1"#));

        let (base, server) = serve_once("200 OK", r#"{"id":"chatcmpl-1"}"#);
        assert_eq!(verify_model("openai", &format!("{base}/v1"), "synthetic-key", "gpt-x"), Ok(()));
        let request = server.join().unwrap();
        assert!(request.starts_with("post /v1/chat/completions "), "{request}");
        assert!(request.contains("authorization: bearer synthetic-key") && request.contains(r#""model":"gpt-x""#));
    }

    #[test]
    fn verify_model_explains_rejections_without_echoing_the_key() {
        let (base, server) = serve_once("401 Unauthorized", r#"{"error":{"message":"invalid api key"}}"#);
        let err = verify_model("openai", &base, "synthetic-key", "gpt-x").unwrap_err();
        server.join().unwrap();
        assert!(err.contains("401") && err.contains("API Key") && !err.contains("synthetic-key"), "{err}");

        let (base, server) = serve_once("404 Not Found", r#"{"error":{"message":"model not found"}}"#);
        let err = verify_model("openai", &base, "synthetic-key", "gpt-x").unwrap_err();
        server.join().unwrap();
        assert!(err.contains("404") && err.contains("model not found"), "{err}");

        assert!(verify_model("openai", "https://gw.test", "synthetic-key", "  ").is_err());
    }

    #[test]
    fn versioned_openai_bases_are_not_given_another_v1() {
        let models = |base: &str, protocol: &str| models_endpoint(base, protocol, None).unwrap();
        // OpenAI 兼容：以 /v<数字> 结尾视为已带版本（智谱 /paas/v4、火山 /api/v3 等）
        assert_eq!(models("https://open.example.cn/api/paas/v4", "openai"), "https://open.example.cn/api/paas/v4/models");
        assert_eq!(models("https://open.example.cn/api/paas/v4/", "openai"), "https://open.example.cn/api/paas/v4/models");
        assert_eq!(models("https://generativelanguage.googleapis.com/v1beta/openai", "openai"), "https://generativelanguage.googleapis.com/v1beta/openai/models");
        assert_eq!(models("https://gw.test", "openai"), "https://gw.test/v1/models");
        assert_eq!(models("https://gw.test/api/v3beta", "openai"), "https://gw.test/api/v3beta/v1/models");
        assert_eq!(api_endpoint("https://ark.example.com/api/v3", "openai", "/chat/completions").unwrap(), "https://ark.example.com/api/v3/chat/completions");
        // Anthropic 兼容保持原规则：只有 /v1（或 /openai）结尾不再补 /v1
        assert_eq!(models("https://open.example.cn/api/anthropic", "anthropic"), "https://open.example.cn/api/anthropic/v1/models");
        assert_eq!(models("https://gw.test/v1", "anthropic"), "https://gw.test/v1/models");
        assert_eq!(models("https://gw.test/api/v4", "anthropic"), "https://gw.test/api/v4/v1/models");
    }

    #[test]
    fn official_prefilled_bases_keep_their_models_endpoint() {
        // providerCatalog.ts 的官方预填地址；协议与 fetch_models 一样由 auth_protocol 决定
        let models = |platform: &str, base: &str| models_endpoint(base, auth_protocol(platform, None), None).unwrap();
        assert_eq!(models("codex", "https://api.openai.com/v1"), "https://api.openai.com/v1/models");
        assert_eq!(models("grok", "https://api.x.ai/v1"), "https://api.x.ai/v1/models");
        assert_eq!(models("claude", "https://api.anthropic.com"), "https://api.anthropic.com/v1/models");
    }

    #[test]
    fn redirects_are_refused_without_following_or_leaking_the_key() {
        // 跳转目标：若客户端跟随重定向，这里会收到带 Key 的第二次请求
        let target = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let location = format!("http://{}/v1/models", target.local_addr().unwrap());
        let (base, server) = serve_once_with_location("302 Found", location.clone());
        let err = fetch_models("custom", Some("anthropic"), &base, "synthetic-key").unwrap_err();
        server.join().unwrap();
        assert!(err.contains("重定向") && err.contains("302") && err.contains("最终地址"), "{err}");
        assert!(!err.contains(&location) && !err.contains("synthetic-key"), "{err}");

        let (base, server) = serve_once_with_location("307 Temporary Redirect", location.clone());
        let err = verify_model("openai", &base, "synthetic-key", "gpt-x").unwrap_err();
        server.join().unwrap();
        assert!(err.contains("重定向") && err.contains("307") && !err.contains(&location), "{err}");
        assert!(matches!(target.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock), "不应向跳转目标发送请求");
    }

    #[test]
    fn service_error_message_is_scrubbed_of_the_key() {
        let (base, server) = serve_once("400 Bad Request", r#"{"error":{"message":"bad key synthetic-key for model"}}"#);
        let err = verify_model("openai", &base, "synthetic-key", "gpt-x").unwrap_err();
        server.join().unwrap();
        assert!(err.contains("bad key **** for model") && !err.contains("synthetic-key"), "{err}");
        // 连接失败的报错不带 URL（URL 里可能有 userinfo / query 凭证）；端口 9 在本机无人监听
        let err = fetch_models("custom", Some("openai"), "http://user:pass@127.0.0.1:9/v1?key=secret-q", "synthetic-key").unwrap_err();
        assert!(!err.contains("pass") && !err.contains("secret-q"), "{err}");
        let err = verify_model("openai", "http://user:pass@127.0.0.1:9/v1?key=secret-q", "synthetic-key", "gpt-x").unwrap_err();
        assert!(!err.contains("pass") && !err.contains("secret-q"), "{err}");
    }

    #[test]
    fn custom_key_change_requires_a_new_model() {
        assert!(check_custom_key_change("custom", None).is_err());
        assert!(check_custom_key_change("custom", Some("  ")).is_err());
        assert_eq!(check_custom_key_change("custom", Some("gpt-x")), Ok(()));
        assert_eq!(check_custom_key_change("codex", None), Ok(()));
    }

    #[test]
    fn fetch_models_sends_selected_protocol_headers_to_custom_service() {
        let (base, server) = serve_models_once();
        assert_eq!(fetch_models("custom", Some("anthropic"), &base, "synthetic-key").unwrap(), vec!["model-a", "model-b"]);
        let request = server.join().unwrap();
        assert!(request.starts_with("get /v1/models "), "{request}");
        assert!(request.contains("x-api-key: synthetic-key") && request.contains("anthropic-version:"));
        assert!(!request.contains("authorization:"));

        let (base, server) = serve_models_once();
        assert_eq!(fetch_models("custom", Some("openai"), &format!("{base}/v1/"), "synthetic-key").unwrap().len(), 2);
        let request = server.join().unwrap();
        assert!(request.starts_with("get /v1/models "), "{request}");
        assert!(request.contains("authorization: bearer synthetic-key") && !request.contains("x-api-key"));
    }

    #[test]
    fn auth_protocol_follows_platform_then_saved_protocol() {
        assert_eq!(auth_protocol("claude", None), "anthropic");
        assert_eq!(auth_protocol("codex", None), "openai");
        assert_eq!(auth_protocol("custom", Some("anthropic")), "anthropic");
        assert_eq!(auth_protocol("custom", Some("openai")), "openai");
        assert_eq!(auth_protocol("custom", None), "openai");
    }

    #[test]
    fn custom_connection_requires_protocol_base_and_explicit_key() {
        assert!(prepare_new(custom(Some("openai"), Some("https://gw.test/v1/"), Some(" sk-custom-1234 "))).is_ok());
        assert!(prepare_new(custom(None, Some("https://gw.test/v1"), Some("sk-custom-1234"))).is_err());
        assert!(prepare_new(custom(Some("gemini"), Some("https://gw.test/v1"), Some("sk-custom-1234"))).is_err());
        assert!(prepare_new(custom(Some("openai"), None, Some("sk-custom-1234"))).is_err());
        assert!(prepare_new(custom(Some("openai"), Some("https://gw.test/v1"), None)).is_err());
        assert!(prepare_new(NewConnection { kind: "auth".into(), ..custom(Some("openai"), Some("https://gw.test"), Some("sk-custom-1234")) }).is_err());
    }

    #[test]
    fn custom_connection_saves_protocol_and_model() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let input = prepare_new(custom(Some("anthropic"), Some("https://gw.test/"), Some("sk-custom-1234"))).unwrap();
        let id = add(&db, input).unwrap();
        let dto = list(&db).unwrap().into_iter().find(|c| c.id == id).unwrap();
        assert_eq!((dto.platform.as_str(), dto.protocol.as_deref(), dto.base_url.as_deref()), ("custom", Some("anthropic"), Some("https://gw.test")));
        assert_eq!(dto.model.as_deref(), Some("gpt-4.1-mini"));
        // 恢复查询 / 更换 Key 的校验靠保存的模型（D-11）
        assert_eq!(creds_of(&db, &id).unwrap().unwrap().model.as_deref(), Some("gpt-4.1-mini"));
    }
}
// 任务 3.4 结束

/// 返回给前端的连接。**不含** secret 字段——这是刻意的，不要加。
#[derive(Debug, Serialize)]
pub struct ConnectionDto {
    /// 已核对凭证的账号邮箱；未知为 None，不从名称或 Key 推断。
    pub account_email: Option<String>,
    pub id: String,
    pub platform: String,
    pub kind: String,
    pub name: String,
    pub label: String,
    /// 脱敏标识，如 `sk-ant-****3f9a`；Auth 连接为套餐名或空
    pub masked: String,
    pub status: String,
    /// 相对时间文案，如「刚刚」「12 秒前」；从未同步过为 None
    pub last_sync_text: Option<String>,
    /// 自建网关（sub2api）的部署地址；官方直连为 None。
    /// 地址不是凭证，可以回显，便于用户核对填错没有
    pub base_url: Option<String>,
    /// 手动添加时保存的默认参数：模型 / 思考强度 / 1M 上下文。
    /// 本机读取的连接与历史连接为 None；参数随连接保存，供请求与成本估算使用
    pub model: Option<String>,
    pub effort: Option<String>,
    pub context_1m: Option<bool>,
    /// 自定义供应商的接口协议：openai / anthropic；其他平台为 None
    pub protocol: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NewConnection {
    pub platform: String,
    pub kind: String,
    pub name: String,
    /// 仅 kind = "api" 时有值。落库后前端不再回读
    pub secret: Option<String>,
    /// sub2api 等自建网关的部署地址；官方直连留空
    pub base_url: Option<String>,
    /// 手动添加的可选默认参数；缺失时为 None
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub context_1m: Option<bool>,
    /// 接口协议：openai（Bearer）或 anthropic（x-api-key）；自定义供应商必填
    #[serde(default)]
    pub protocol: Option<String>,
}

/// 脱敏：保留可识别的前缀与末 4 位，中间固定 4 个星号。
/// 太短的 Key（异常输入）整体打码，避免反而暴露大部分内容。
pub fn mask(secret: &str) -> String {
    let s = secret.trim();
    if s.len() <= 8 {
        return "****".into();
    }
    // 常见前缀 sk-ant- / sk- 保留，便于用户辨认是哪一类 Key
    let prefix_len = if s.starts_with("sk-ant-") {
        7
    } else if s.starts_with("sk-") {
        3
    } else {
        4
    };
    let head: String = s.chars().take(prefix_len).collect();
    let tail: String = s.chars().skip(s.chars().count().saturating_sub(4)).collect();
    format!("{head}****{tail}")
}

/// 毫秒时间戳 → 相对时间文案（§6.3 的「最近同步」列）
fn relative(ms: i64, now: i64) -> String {
    let d = (now - ms).max(0) / 1000;
    match d {
        0..=4 => "刚刚".into(),
        5..=59 => format!("{d} 秒前"),
        60..=3599 => format!("{} 分钟前", d / 60),
        3600..=86399 => format!("{} 小时前", d / 3600),
        _ => format!("{} 天前", d / 86400),
    }
}

fn row_to_dto(r: &rusqlite::Row, now: i64) -> rusqlite::Result<ConnectionDto> {
    let last: Option<i64> = r.get("last_sync_ms")?;
    let platform: String = r.get("platform")?;
    let kind: String = r.get("kind")?;
    let base_url = normalized_base(&platform, &kind, r.get("base_url")?);
    Ok(ConnectionDto {
        account_email: r.get("account_email")?,
        id: r.get("id")?,
        platform: r.get("platform")?,
        kind: r.get("kind")?,
        name: r.get("name")?,
        label: r.get("label")?,
        masked: r.get("masked")?,
        status: r.get("status")?,
        last_sync_text: last.map(|m| relative(m, now)),
        base_url,
        model: r.get("model")?,
        effort: r.get("effort")?,
        context_1m: r.get("context_1m")?,
        protocol: r.get("protocol")?,
    })
}

pub fn list(conn: &Sql) -> rusqlite::Result<Vec<ConnectionDto>> {
    backfill_account_emails(conn)?;
    let now = chrono::Local::now().timestamp_millis();
    let mut stmt = conn.prepare(
        "SELECT id, platform, kind, name, label, masked, status, last_sync_ms, base_url, model, effort, context_1m, protocol, account_email
         FROM connections ORDER BY created_ms ASC",
    )?;
    let rows = stmt
        .query_map([], |r| row_to_dto(r, now))?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// 旧连接只补充未知身份；切换本机账号后保留已保存的邮箱，不能串号。
fn backfill_account_emails(conn: &Sql) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("SELECT id, platform, secret FROM connections WHERE kind='auth' AND account_email IS NULL AND platform IN ('claude','codex')")?;
    let pending = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, platform, secret) in pending {
        if let Some(email) = secret.as_deref().and_then(|s| crate::login_detect::account_for_secret(&platform, s)) {
            conn.execute("UPDATE connections SET account_email=?2 WHERE id=?1 AND account_email IS NULL", params![id, email])?;
        }
    }
    Ok(())
}

/// 连接名称校验：新增与改名共用同一口径（去首尾空白，1–80 个字符）。
fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 80 {
        return Err("连接名称应为 1 到 80 个字符".into());
    }
    Ok(name)
}

/// 校验连接输入，显式凭证不触发本机读取。
pub fn prepare_new(mut c: NewConnection) -> Result<NewConnection, String> {
    if !crate::platforms::known(&c.platform) {
        return Err("不支持的平台标识".into());
    }
    // 自定义供应商（D-11）：只接受用户显式填写的地址与 Key，不读取本机凭证
    if c.platform == "custom" {
        if c.kind != "api" { return Err("自定义供应商只支持 API Key 连接".into()); }
        if c.secret.as_deref().map_or(true, |s| s.trim().is_empty()) { return Err("请填写 API Key".into()); }
        if c.base_url.as_deref().map_or(true, |s| s.trim().is_empty()) { return Err("请填写 Base URL".into()); }
    }
    c.protocol = valid_protocol(&c.platform, c.protocol.take())?;
    if crate::platforms::native(&c.platform) && !(c.kind == "api" && crate::provider_key::supported(&c.platform)) { return Err("该平台请通过本机来源获取，不保存猜测的登录凭证".into()); }
    // 仅获取型平台（画布 18 后续）：凭证只能来自本机读取
    if crate::creds::is_fetch_only_platform(&c.platform) && c.kind != "auth" && !crate::provider_key::supported(&c.platform) {
        return Err("该平台只支持从本机获取凭证，不支持手动填写".into());
    }
    if !matches!(c.kind.as_str(), "auth" | "api") {
        return Err("接入方式必须是 auth 或 api".into());
    }
    c.name = validate_name(&c.name)?;
    let using_local = c.secret.as_deref().map_or(true, |s| s.trim().is_empty());
    if using_local && c.base_url.as_deref().map_or(true, |s| s.trim().is_empty()) {
        c.base_url = crate::creds::local_base_url(&c.platform, &c.kind);
    }
    c.secret = c.secret
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| crate::creds::read_secret(&c.platform, &c.kind));
    if c.secret.is_none() {
        return Err("未找到可保存的凭证；请粘贴 API Key，或先在对应 CLI 中登录".into());
    }
    c.base_url = c.base_url
        .take()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty());
    if let Some(base) = c.base_url.as_deref() {
        let url = reqwest::Url::parse(base).map_err(|_| "网关地址格式无效".to_string())?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("网关地址必须使用 HTTP 或 HTTPS".into());
        }
    }
    if crate::provider_key::supported(&c.platform) {
        crate::provider_key::check_base(&c.platform, c.base_url.as_deref())?;
        c.model = None; c.effort = None; c.context_1m = None;
    }
    c.base_url = normalized_base(&c.platform, &c.kind, c.base_url);
    Ok(c)
}

/// 协议只认 openai / anthropic；自定义供应商必须选择其一。
fn valid_protocol(platform: &str, protocol: Option<String>) -> Result<Option<String>, String> {
    match protocol.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p @ ("openai" | "anthropic")) => Ok(Some(p.to_string())),
        Some(_) => Err("协议只能是 OpenAI 兼容或 Anthropic 兼容".into()),
        None if platform == "custom" => Err("请选择接口协议".into()),
        None => Ok(None),
    }
}

fn normalized_base(platform: &str, kind: &str, base: Option<String>) -> Option<String> {
    if kind == "auth" { return None; }
    base.filter(|value| {
        let Ok(url) = reqwest::Url::parse(value) else { return true };
        let host = match platform { "claude" => "api.anthropic.com", "codex" => "api.openai.com", _ => return true };
        !(url.scheme() == "https" && url.host_str() == Some(host) && url.port_or_known_default() == Some(443)
            && matches!(url.path().trim_end_matches('/'), "" | "/v1") && url.query().is_none() && url.fragment().is_none()
            && url.username().is_empty() && url.password().is_none())
    })
}

/// 编辑命令用：api 连接的地址归一（官方地址返回 None，不落库）。
pub(crate) fn normalized_base_for(platform: &str, base: Option<String>) -> Option<String> {
    normalized_base(platform, "api", base)
}

/// 仅写入已经完成凭证解析与网络验证的连接。
pub fn add(conn: &Sql, c: NewConnection) -> Result<String, String> {
    if c.secret.as_deref().map_or(true, |secret| secret.is_empty()) {
        return Err("连接尚未验证".into());
    }
    let now = chrono::Local::now().timestamp_millis();
    let suffix: String = conn.query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    let id = format!("{}-{suffix}", c.platform);
    let masked = c.secret.as_deref().map(mask).unwrap_or_default();
    let label = if c.kind == "api" { "API Key" } else { "官方订阅" };
    let base = c.base_url;
    let account_email = if c.kind == "auth" { c.secret.as_deref().and_then(|s| crate::login_detect::account_for_secret(&c.platform, s)) } else { None };

    conn.execute(
        "INSERT INTO connections
           (id, platform, kind, name, label, secret, masked, base_url, model, effort, context_1m, status, last_sync_ms, created_ms, protocol, account_email)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'connected', ?12, ?12, ?13, ?14)",
        params![id, c.platform, c.kind, c.name, label, c.secret, masked, base, c.model, c.effort, c.context_1m, now, c.protocol, account_email],
    ).map_err(|e| e.to_string())?;
    Ok(id)
}

/// 移除连接：只删凭证与配置，**历史统计保留**（§6.3）。
/// requests 表按 platform 存储、不关联 connection_id，因此这里天然不会碰到统计数据。
/// 「获取模型」：从网关拉取可用模型列表。
/// Claude 与 Anthropic 兼容的自定义供应商走 x-api-key + anthropic-version，OpenAI 兼容网关走 Bearer；
/// 两家的 /v1/models 响应同为 {"data": [{"id": ...}]}，取 id 即可。
pub fn fetch_models(platform: &str, protocol: Option<&str>, base_url: &str, secret: &str) -> Result<Vec<String>, String> {
    let protocol = auth_protocol(platform, protocol);
    let url = models_endpoint(base_url, protocol, None)?;
    let resp = apply_auth(keyed_client(10)?.get(&url), protocol, secret)
        .send()
        .map_err(|e| format!("无法连接网关：{}", e.without_url()))?;
    let status = resp.status();
    if status.is_redirection() {
        return Err(redirect_error(status));
    }
    if !status.is_success() {
        return Err(format!("获取模型失败：HTTP {status}，请检查 Key 与地址"));
    }
    let body: serde_json::Value = resp.json().map_err(|e| format!("响应解析失败：{}", e.without_url()))?;
    let ids = body
        .get("data")
        .and_then(|d| d.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(str::to_string))
                .fold(Vec::new(), |mut ids, id| {
                    // 模型 ID 去重（Jolu.pen 画板 29），保留服务端返回的顺序
                    if !ids.contains(&id) { ids.push(id); }
                    ids
                })
        });
    match ids {
        Some(ids) if !ids.is_empty() => Ok(ids),
        _ => Err("网关返回了空模型列表".into()),
    }
}

/// 「获取强度」已移除：思考强度没有网关端点，由用户按模型直接选择
/// off/low/medium/high（与请求日志 effort 列同口径），无需网络请求。

/// {base} → {base}/v1/models[/{model}]。用户可能填 https://gw.test
/// 或已带 /v1 的地址，两种都归一；仅接受 HTTP(S)。
/// {base} → {base}/v1/models[/{model}]。用户可能填 https://gw.test
/// 或已带 /v1 的地址，两种都归一；仅接受 HTTP(S)。
/// 「获取模型」与「获取强度」（effort_map）共用。
pub(crate) fn models_endpoint(base_url: &str, protocol: &str, model: Option<&str>) -> Result<String, String> {
    let tail = match model {
        Some(m) => format!("/models/{m}"),
        None => "/models".to_string(),
    };
    api_endpoint(base_url, protocol, &tail)
}

/// {base} → {base}/v1{tail}；已带 /v1（或 Gemini 的 /openai）时直接拼接。仅接受 HTTP(S)。
/// OpenAI 兼容地址以 /v<数字> 结尾（智谱 /paas/v4、火山 /api/v3 等）视为已带版本，不再补 /v1；
/// Anthropic 兼容保持只认 /v1 的规则。
fn api_endpoint(base_url: &str, protocol: &str, tail: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(trimmed).map_err(|_| "网关地址格式无效".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("网关地址必须使用 HTTP 或 HTTPS".into());
    }
    let versioned = protocol != "anthropic" && trimmed.rsplit('/').next().is_some_and(|last| {
        last.len() > 1 && last.starts_with('v') && last[1..].bytes().all(|b| b.is_ascii_digit())
    });
    // Gemini 的 OpenAI 兼容地址以 /openai 结尾，模型列表直接挂在其下（/v1beta/openai/models）
    if versioned || trimmed.ends_with("/v1") || trimmed.ends_with("/openai") {
        Ok(format!("{trimmed}{tail}"))
    } else {
        Ok(format!("{trimmed}/v1{tail}"))
    }
}

/// 「验证模型」（画板 27「接口不支持模型列表」）：模型接口不可用、用户手填模型 ID 时，
/// 由用户确认后发送**一次**最小请求（max_tokens = 1）确认地址、密钥与模型可用。可能产生少量 API 用量，
/// 只在用户点击并确认后调用；Key 只用于本次请求，错误信息不回显 Key。
pub fn verify_model(protocol: &str, base_url: &str, secret: &str, model: &str) -> Result<(), String> {
    let model = model.trim();
    if model.is_empty() {
        return Err("请填写模型 ID".into());
    }
    let anthropic = protocol == "anthropic";
    let protocol = if anthropic { "anthropic" } else { "openai" };
    let url = api_endpoint(base_url, protocol, if anthropic { "/messages" } else { "/chat/completions" })?;
    let body = serde_json::json!({ "model": model, "max_tokens": 1, "messages": [{ "role": "user", "content": "ping" }] });
    let resp = apply_auth(keyed_client(30)?.post(&url), protocol, secret)
        .json(&body)
        .send()
        .map_err(|e| format!("无法连接服务：{}", e.without_url()))?;
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    if status.is_redirection() {
        return Err(redirect_error(status));
    }
    // 服务返回的错误说明（截断）有助于判断是模型 ID 还是参数问题；服务若回显了 Key，先打码
    let detail = resp.json::<serde_json::Value>().ok()
        .and_then(|v| v.pointer("/error/message").and_then(|m| m.as_str()).map(str::to_string))
        .map(|m| if secret.is_empty() { m } else { m.replace(secret, "****") })
        .map(|m| format!("：{}", m.chars().take(160).collect::<String>()))
        .unwrap_or_default();
    Err(match status.as_u16() {
        401 | 403 => format!("验证失败：HTTP {status}，API Key 无效或没有该模型的权限{detail}"),
        404 => format!("验证失败：HTTP {status}，服务未找到该模型或接口，请检查模型 ID 与 Base URL{detail}"),
        429 => format!("验证失败：HTTP {status}，服务限流或额度不足，请稍后再试{detail}"),
        _ => format!("验证失败：HTTP {status}{detail}"),
    })
}

/// 带 Key 请求用的客户端：**不跟随重定向**。reqwest 跨主机跳转只删 Authorization 等头、不删 x-api-key，
/// https 跳到同主机 http 时还会明文发出 Key（先例：provider_key.rs、quota.rs）。
fn keyed_client(timeout_secs: u64) -> Result<reqwest::blocking::Client, String> {
    // 代理按应用联网方式（任务 2.3）
    crate::net::blocking_builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.without_url().to_string())
}

/// 3xx 一律不跟随，也不回显 Location（目标地址可能带凭证）
fn redirect_error(status: reqwest::StatusCode) -> String {
    format!("服务返回重定向（HTTP {status}），请填写最终地址")
}

/// 自定义供应商更换 API Key 时必须同时带上模型：后端校验只看「有模型」（D-11），
/// 新 Key 的可达性由前端用新 Key 获取模型或「验证模型」保证（画板 27）。
pub(crate) fn check_custom_key_change(platform: &str, model: Option<&str>) -> Result<(), String> {
    if platform != "custom" || model.is_some_and(|m| !m.trim().is_empty()) {
        return Ok(());
    }
    Err("自定义供应商更换 API Key 后，请在「编辑」中用新密钥重新获取模型或验证手填模型，再保存".into())
}

/// 鉴权协议：Claude 固定为 anthropic；其他平台按保存的协议，未记录时按 OpenAI 兼容。
pub(crate) fn auth_protocol(platform: &str, protocol: Option<&str>) -> &'static str {
    if platform == "claude" || protocol == Some("anthropic") { "anthropic" } else { "openai" }
}

/// 鉴权头：anthropic 走 x-api-key + anthropic-version，其余（OpenAI 兼容）走 Bearer。
pub(crate) fn apply_auth(req: reqwest::blocking::RequestBuilder, protocol: &str, secret: &str) -> reqwest::blocking::RequestBuilder {
    if protocol == "anthropic" {
        req.header("x-api-key", secret).header("anthropic-version", "2023-06-01")
    } else {
        req.header("Authorization", format!("Bearer {secret}"))
    }
}

/// 按凭证原值找连接（只在后端比对）；同一凭证重复保存时取最早的一条。
pub fn find_by_secret(conn: &Sql, platform: &str, kind: &str, secret: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT id FROM connections WHERE platform=?1 AND kind=?2 AND secret=?3 ORDER BY created_ms LIMIT 1",
        params![platform, kind, secret],
        |r| r.get(0),
    ).optional().map_err(|e| e.to_string())
}

pub fn register_local(conn: &Sql, input: NewConnection) -> Result<(String, bool), String> {
    let c = prepare_new(input)?;
    let mut stmt = conn.prepare("SELECT id, base_url FROM connections WHERE platform=?1 AND kind=?2 AND secret=?3").map_err(|e| e.to_string())?;
    let matches = stmt.query_map(params![c.platform, c.kind, c.secret], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))
        .map_err(|e| e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())?;
    let existing = matches.into_iter().find(|(_, base)| normalized_base(&c.platform, &c.kind, base.clone()) == c.base_url).map(|(id, _)| id);
    if let Some(id) = existing { return Ok((id, false)); }
    let id = add(conn, c)?;
    conn.execute("UPDATE connections SET status='paused', last_sync_ms=NULL WHERE id=?1", [&id]).map_err(|e| e.to_string())?;
    Ok((id, true))
}

pub(crate) fn selectable_creds(conn: &Sql, id: &str) -> Result<Option<Creds>, String> {
    let status: Option<String> = conn.query_row("SELECT status FROM connections WHERE id=?1", [id], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
    if status.as_deref() != Some("connected") { return Err("请先连接成功，再启用到灵动岛".into()); }
    creds_of(conn, id).map_err(|e| e.to_string())
}

pub fn remove(conn: &Sql, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM connections WHERE id = ?1", params![id])?;
    Ok(())
}

/// 暂停只改变连接状态，保留凭证、配置与所有本机历史。
pub fn set_paused(conn: &Sql, id: &str, paused: bool) -> Result<(), String> {
    let changed = conn.execute(
        "UPDATE connections SET status = ?2 WHERE id = ?1",
        params![id, if paused { "paused" } else { "connected" }],
    ).map_err(|e| e.to_string())?;
    if changed == 0 { return Err("连接不存在".into()); }
    Ok(())
}

/// 取出连接的凭证与地址，**仅供后端查额度/余额使用**。
/// 返回值含凭证原值，绝不可序列化给前端或写进日志。
pub(crate) struct Creds {
    pub paused: bool,
    pub name: String,
    pub platform: String,
    pub kind: String,
    pub secret: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
}

pub(crate) fn creds_of(conn: &Sql, id: &str) -> rusqlite::Result<Option<Creds>> {
    conn.query_row(
        "SELECT name, platform, kind, secret, base_url, status, model FROM connections WHERE id = ?1",
        params![id],
        |r| {
            let platform: String = r.get(1)?;
            let kind: String = r.get(2)?;
            let base_url = normalized_base(&platform, &kind, r.get(4)?);
            Ok(Creds {
                paused: r.get::<_, String>(5)? == "paused",
                name: r.get(0)?,
                platform: r.get(1)?,
                kind: r.get(2)?,
                secret: r.get(3)?,
                base_url,
                model: r.get(6)?,
            })
        },
    )
    .optional()
}

/// 本地代理按凭证归账用：API Key 连接的 (id, platform, secret)。只在内存里做哈希比对，不返回给前端。
pub(crate) fn api_key_secrets(conn: &Sql) -> rusqlite::Result<Vec<(String, String, String)>> {
    let mut stmt = conn.prepare("SELECT id, platform, secret FROM connections WHERE kind = 'api' AND secret IS NOT NULL AND secret != ''")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 所有远程查询在命中缓存或发送请求之前，都必须经过此检查。
pub(crate) fn query_creds(conn: &Sql, id: &str) -> Result<Option<Creds>, String> {
    let current = creds_of(conn, id).map_err(|e| e.to_string())?;
    if current.as_ref().is_some_and(|c| c.paused) {
        return Err("连接已断开，请先重新连接".into());
    }
    Ok(current)
}

/// 记录一次成功查询的时间，用于「最近同步」列
pub(crate) fn touch_sync(conn: &Sql, id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE connections SET last_sync_ms = ?2, status = 'connected' WHERE id = ?1 AND status != 'paused'",
        params![id, chrono::Local::now().timestamp_millis()],
    )?;
    Ok(())
}

/// 把连接标记为凭证失效，供额度查询返回 401 时使用
pub(crate) fn mark_expired(conn: &Sql, id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE connections SET status = 'expired' WHERE id = ?1 AND status != 'paused'",
        params![id],
    )?;
    Ok(())
}

pub(crate) fn record_query_status(conn: &Sql, id: &str, success: bool) -> Result<bool, String> {
    let old: Option<String> = conn.query_row("SELECT status FROM connections WHERE id=?1", [id], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
    if old.is_none() || old.as_deref() == Some("paused") { return Ok(false); }
    if success { touch_sync(conn, id) } else { mark_expired(conn, id) }.map_err(|e| e.to_string())?;
    Ok(old.as_deref() != Some(if success { "connected" } else { "expired" }))
}

pub fn replace_api_key(conn: &Sql, id: &str, secret: &str) -> Result<(), String> {
    let changed = conn.execute(
        "UPDATE connections
         SET secret = ?2, masked = ?3, status = 'connected', last_sync_ms = ?4
         WHERE id = ?1 AND kind = 'api' AND status != 'paused'",
        params![id, secret, mask(secret), chrono::Local::now().timestamp_millis()],
    ).map_err(|e| e.to_string())?;
    if changed == 0 { return Err("API Key 连接不存在".into()); }
    Ok(())
}

/// 重命名连接：只改显示名称，凭证、脱敏标识与状态一概不动。
/// 纯元数据操作，断开的连接同样允许改名。
pub fn rename_connection(conn: &Sql, id: &str, name: &str) -> Result<(), String> {
    let name = validate_name(name)?;
    let changed = conn
        .execute("UPDATE connections SET name = ?2 WHERE id = ?1", params![id, name])
        .map_err(|e| e.to_string())?;
    if changed == 0 { return Err("连接不存在".into()); }
    Ok(())
}

/// 统一编辑（画布 18）：更新 API 连接的网关地址与默认参数（模型/思考强度/1M）。
/// 地址传入前须已经过 normalized_base 归一；名称与凭证走各自的函数。
pub fn update_params(
    conn: &Sql,
    id: &str,
    base_url: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
    context_1m: Option<bool>,
) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE connections SET base_url = ?2, model = ?3, effort = ?4, context_1m = ?5
             WHERE id = ?1 AND kind = 'api'",
            params![id, base_url, model, effort, context_1m],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 { return Err("API Key 连接不存在".into()); }
    Ok(())
}

/// 「添加连接」读取本机配置时应用用户填写的连接名称。
/// 名称只填了空白视为未填，保留自动生成名；超长名称整体拒绝。
pub fn with_custom_name(
    inputs: Vec<NewConnection>,
    name: Option<String>,
) -> Result<Vec<NewConnection>, String> {
    let Some(name) = name.as_deref().map(str::trim).filter(|n| !n.is_empty()) else {
        return Ok(inputs);
    };
    let name = validate_name(name)?;
    Ok(inputs
        .into_iter()
        .map(|input| NewConnection { name: name.clone(), ..input })
        .collect())
}

/// 「获取」的结果（§6.3 的四种状态里，后端只负责成功/未找到）
#[derive(Debug, Serialize)]
pub struct FetchResult {
    pub ok: bool,
    /// 成功时为新的脱敏标识
    pub masked: Option<String>,
    /// 失败原因，直接作为界面提示文案
    pub message: String,
    /// 扫描过的位置，界面需要说明读取范围，不静默读取
    pub scanned: Vec<String>,
    /// 本次是否真的写入了变更；「已更新」与「已是最新」靠它区分
    pub changed: bool,
}

/// 本机当前应用的凭证与地址，是「更新」与自动同步共用的最小变更集。
#[derive(Debug, Clone)]
pub(crate) struct LocalPair {
    pub secret: String,
    pub base_url: Option<String>,
}

/// 把本机当前配置原位应用到一个已有连接上。
///
/// local 为 None 表示本机没有读到凭证：只读不写，原状态与凭证保持不变。
/// 更新范围是 secret、masked 与 base_url——cc-switch 切换中转时两者一起变，
/// 只刷 Key 会把新 Key 发往旧网关。同平台同类型有多条连接且凭证原值对不上
/// 时拒绝覆盖，让用户显式选择，绝不静默改错行。
pub(crate) fn apply_local(
    conn: &Sql,
    id: &str,
    local: Option<LocalPair>,
    scanned: Vec<String>,
) -> rusqlite::Result<FetchResult> {
    let row: Option<(String, String, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT platform, kind, secret, base_url FROM connections WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((platform, kind, current_secret, current_base)) = row else {
        return Ok(FetchResult { ok: false, masked: None, message: "连接不存在".into(), scanned, changed: false });
    };
    // 失败不得清空或破坏现有连接：这里只读不写
    let Some(local) = local else {
        return Ok(FetchResult { ok: false, masked: None, message: "未找到本机凭证".into(), scanned, changed: false });
    };
    let masked = mask(&local.secret);
    // 与 prepare_new 同一清洗口径：去首尾空白、去结尾斜杠，官方地址归一成 None
    let cleaned_base = local
        .base_url
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty());
    let base_url = normalized_base(&platform, &kind, cleaned_base);
    let same_kind_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM connections WHERE platform = ?1 AND kind = ?2",
        params![platform, kind],
        |row| row.get(0),
    )?;
    if same_kind_count > 1 && current_secret.is_some() && current_secret.as_deref() != Some(local.secret.as_str()) {
        return Ok(FetchResult {
            ok: false,
            masked: Some(masked),
            message: "本机当前登录账号与此命名连接不同，已拒绝覆盖；请新增连接或切回对应 CLI 账号".into(),
            scanned,
            changed: false,
        });
    }
    let changed = current_secret.as_deref() != Some(local.secret.as_str())
        || normalized_base(&platform, &kind, current_base) != base_url;
    if changed {
        let now = chrono::Local::now().timestamp_millis();
        let email = if kind == "auth" { crate::login_detect::account_for_secret(&platform, &local.secret) } else { None };
        conn.execute(
            "UPDATE connections
                SET secret = ?2, masked = ?3, base_url = ?4, status = 'connected', last_sync_ms = ?5,
                    account_email = CASE WHEN secret = ?2 THEN account_email ELSE ?6 END
              WHERE id = ?1",
            params![id, local.secret, masked, base_url, now, email],
        )?;
    }
    Ok(FetchResult {
        ok: true,
        masked: Some(masked),
        message: if changed { "已更新凭证".into() } else { "本机凭证未变化".into() },
        scanned,
        changed,
    })
}

/// 「更新」按钮：重读本机配置刷新**已存在**连接的 secret 与 base_url。
/// 不新建连接、不改名称与类型（§6.3）。
pub fn fetch_local(conn: &Sql, id: &str) -> rusqlite::Result<FetchResult> {
    if creds_of(conn, id)?.is_some_and(|c| c.paused) {
        return Ok(FetchResult { ok: false, masked: None, message: "连接已断开，请先重新连接".into(), scanned: vec![], changed: false });
    }
    let row: Option<(String, String)> = conn
        .query_row("SELECT platform, kind FROM connections WHERE id = ?1", params![id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    let Some((platform, kind)) = row else {
        return Ok(FetchResult { ok: false, masked: None, message: "连接不存在".into(), scanned: vec![], changed: false });
    };
    let scanned = crate::creds::scanned_locations(&platform, &kind);
    let local = crate::creds::read_local_pair(&platform, &kind)
        .map(|(secret, base_url)| LocalPair { secret, base_url });
    apply_local(conn, id, local, scanned)
}

/// 自动同步（默认常开）：把本机当前应用的配置原位写回已有连接。
///
/// 只处理无歧义的连接：同平台同类型恰好一条时才尝试原位更新；本机没读到、
/// 多条同类型（分不清本机配置属于哪条）时一律不动。paused 的连接会被
/// apply_local 拒绝，不会被打断或改写。返回是否有实际变更，供调用方
/// 决定是否推送事件。
pub(crate) fn sync_from_local_with(
    conn: &Sql,
    read: &dyn Fn(&str, &str) -> Option<LocalPair>,
) -> Result<bool, String> {
    let rows: Vec<(String, String, String)> = conn
        .prepare("SELECT id, platform, kind FROM connections")
        .and_then(|mut stmt| {
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|e| e.to_string())?;
    let mut changed = false;
    for (id, platform, kind) in rows {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM connections WHERE platform = ?1 AND kind = ?2",
                params![platform, kind],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if count != 1 { continue; }
        let Some(local) = read(&platform, &kind) else { continue };
        let result = apply_local(conn, &id, Some(local), vec![]).map_err(|e| e.to_string())?;
        changed |= result.changed;
    }
    Ok(changed)
}

/// 自动同步入口：读本机真实配置。应用启动时与配置文件变化时各调一次。
pub fn sync_from_local(conn: &Sql) -> Result<bool, String> {
    sync_from_local_with(conn, &|platform, kind| {
        crate::creds::read_local_pair(platform, kind)
            .map(|(secret, base_url)| LocalPair { secret, base_url })
    })
}

#[cfg(test)]
mod account_email_tests {
    use super::*;
    #[test]
    fn account_email_survives_rename_pause_and_dto_has_no_secret() {
        let db = crate::db::open(std::path::Path::new(":memory:")).unwrap();
        let id = add(&db, NewConnection { platform:"codex".into(), kind:"auth".into(), name:"Work".into(), secret:Some("synthetic-identity-only".into()), base_url:None, model:None, effort:None, context_1m:None, protocol:None }).unwrap();
        db.execute("UPDATE connections SET account_email='person@example.test', name='Renamed' WHERE id=?1", [&id]).unwrap();
        set_paused(&db, &id, true).unwrap();
        let rows = list(&db).unwrap();
        assert_eq!(rows[0].account_email.as_deref(), Some("person@example.test"));
        assert_eq!(rows[0].name, "Renamed");
        let dto = serde_json::to_value(&rows[0]).unwrap();
        assert!(dto.get("secret").is_none());
        assert_eq!(dto["account_email"], "person@example.test");
        apply_local(&db, &id, Some(LocalPair { secret: "synthetic-replacement-identity".into(), base_url: None }), vec![]).unwrap();
        assert_eq!(list(&db).unwrap()[0].account_email, None);

    }
}
