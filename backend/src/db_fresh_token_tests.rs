//! 「本机今日 Token（新鲜口径）」聚合规则的回归测试。
//!
//! 单独成文件的原因与 `db_breakdown_tests.rs` 相同：这批断言围绕同一条约定展开——
//! 未知行不参与求和、也不得把一整天打成未知；只有当天一条已知行都没有才是未知，
//! 真实的零与未知必须分开。
//!
//! 旧实现用 `MIN(total_known) = 1` 守卫，等于「一行未知污染全天」：本机 zcode 当天有
//! 20,253,295 Token，却因为一条抓不到用量的失败请求返回 null。

use super::*;

/// 构造一条记录；`None` 表示来源没提供该字段（入库后对应的 `*_known = 0`）。
/// 缓存两项固定给 0，避免 Claude 的退化行（缺缓存字段 + 全 0）被聚合跳过。
fn rec(
    platform: &str,
    key: &str,
    ts: i64,
    input: Option<i64>,
    output: Option<i64>,
    total: Option<i64>,
) -> RequestRecord {
    RequestRecord {
        platform: platform.into(),
        source: "local".into(),
        dedup_key: key.into(),
        session_id: None,
        ts,
        model: Some("m1".into()),
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: Some(0),
        cache_write_tokens: Some(0),
        total_tokens: total,
        effort: None,
    }
}

fn seeded(records: &[RequestRecord]) -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    init(&conn).unwrap();
    insert_records(&mut conn, records).unwrap();
    conn
}

fn now() -> i64 {
    Local::now().timestamp_millis()
}

/// ① 全部行已知 → 求和
#[test]
fn all_rows_known_sums_the_day() {
    let now = now();
    let conn = seeded(&[
        rec("zcode", "a", now - 3_000, Some(1_000), Some(500), Some(1_500)),
        rec("zcode", "b", now - 2_000, Some(2_000), Some(1_000), Some(3_000)),
        rec("zcode", "c", now - 1_000, Some(4_000), Some(2_000), Some(6_000)),
    ]);
    assert_eq!(today_fresh_tokens(&conn, "zcode").unwrap(), Some(10_500));
}

/// ② 关键用例：1 条未知 + N 条已知 → 只累加已知行（复现本机 zcode 场景）。
/// 同时确认平台与日期区间仍然生效：跨平台、昨天与未来的行都不参与。
#[test]
fn one_unknown_row_does_not_null_the_whole_day() {
    let now = now();
    let day = 86_400_000;
    let conn = seeded(&[
        rec("zcode", "known-1", now - 5_000, Some(8_000_000), Some(0), Some(8_000_000)),
        // 失败请求：来源没给用量，入库后 total_known = 0
        rec("zcode", "failed", now - 4_000, None, None, None),
        rec("zcode", "known-2", now - 3_000, Some(12_253_295), Some(0), Some(12_253_295)),
        rec("claude", "other-platform", now - 2_000, Some(999), Some(999), Some(999)),
        rec("zcode", "yesterday", now - 2 * day, Some(7_000_000), Some(0), Some(7_000_000)),
    ]);

    assert_eq!(
        today_fresh_tokens(&conn, "zcode").unwrap(),
        Some(20_253_295),
        "未知行只把自己排除在外，其余已知行照常求和"
    );
    // 非原生平台走「输入 + 输出」的新鲜口径，同样只累加已知行
    assert_eq!(today_fresh_tokens(&conn, "claude").unwrap(), Some(1_998));
}

/// ③ 全部行未知 → NULL（未知），不是 0
#[test]
fn day_with_only_unknown_rows_is_unknown() {
    let now = now();
    let conn = seeded(&[
        rec("zcode", "boom-1", now - 2_000, None, None, None),
        rec("claude", "boom-2", now - 1_000, None, None, None),
    ]);
    assert_eq!(today_fresh_tokens(&conn, "zcode").unwrap(), None);
    assert_eq!(today_fresh_tokens(&conn, "claude").unwrap(), None);
}

/// ④ 当天无行 → 沿用既有约定：原生来源未知（None），非原生平台是真实的 0
#[test]
fn empty_day_follows_the_existing_convention() {
    let conn = seeded(&[]);
    assert_eq!(today_fresh_tokens(&conn, "zcode").unwrap(), None);
    assert_eq!(today_fresh_tokens(&conn, "claude").unwrap(), Some(0));
}

/// ⑤ 0 Token 的真实行 → 求和为 0，而不是未知
#[test]
fn real_zero_rows_sum_to_zero_not_unknown() {
    let now = now();
    let conn = seeded(&[
        rec("zcode", "zero-1", now - 2_000, Some(0), Some(0), Some(0)),
        rec("zcode", "zero-2", now - 1_000, Some(0), Some(0), Some(0)),
    ]);
    assert_eq!(today_fresh_tokens(&conn, "zcode").unwrap(), Some(0));

    let conn = seeded(&[rec("claude", "zero", now - 1_000, Some(0), Some(0), Some(0))]);
    assert_eq!(
        today_fresh_tokens(&conn, "claude").unwrap(),
        Some(0),
        "真实的零不能退化成未知"
    );
}

/// 改写后的嵌套 CASE 不得破坏 Codex 的缓存扣除，也不让未知行混进求和
#[test]
fn codex_subtracts_cached_tokens_for_known_rows_only() {
    let now = now();
    let mut known = rec("codex", "known", now - 2_000, Some(1_000), Some(200), Some(1_000));
    known.cache_read_tokens = Some(800);
    let conn = seeded(&[known, rec("codex", "failed", now - 1_000, None, None, None)]);

    // 1000 - 800 - 0 + 200 = 400
    assert_eq!(today_fresh_tokens(&conn, "codex").unwrap(), Some(400));
}
