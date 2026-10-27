//! 仅依据本机成功领取记录计算签到统计，不读取凭证或推算奖励。
use crate::checkin_types::{Outcome, State};
use chrono::{Datelike, Duration, FixedOffset, NaiveDate, TimeZone};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Serialize)]
pub(crate) struct CheckinStatistics {
    pub has_records: bool,
    pub unit: String,
    pub week: CheckinTotals,
    pub month: CheckinTotals,
    pub all: CheckinTotals,
}

#[derive(Debug, Serialize)]
pub(crate) struct CheckinTotals {
    pub days: u32,
    pub reward: Option<f64>,
}

pub(crate) fn read(
    conn: &Connection,
    account_key: &str,
    now_ms: i64,
) -> rusqlite::Result<CheckinStatistics> {
    let timezone = FixedOffset::east_opt(8 * 3600).unwrap();
    let date = |ms| {
        timezone
            .timestamp_millis_opt(ms)
            .single()
            .map(|time| time.date_naive())
            .ok_or(rusqlite::Error::InvalidQuery)
    };
    let today = date(now_ms)?;
    let monday = today
        .checked_sub_signed(Duration::days(i64::from(
            today.weekday().num_days_from_monday(),
        )))
        .ok_or(rusqlite::Error::InvalidQuery)?;
    let next_monday = monday
        .checked_add_signed(Duration::days(7))
        .ok_or(rusqlite::Error::InvalidQuery)?;
    let mut week = Bucket::new();
    let mut month = Bucket::new();
    let mut all = Bucket::new();
    let mut periods = HashSet::new();
    let mut units = HashSet::new();
    // 最早成功的记录决定奖励与日期，重复连接不能补金额或把旧周期移到本周。
    let mut stmt = conn.prepare("SELECT outcome,created_ms FROM daily_checkin_records WHERE account_key=?1 ORDER BY created_ms,id")?;
    let rows = stmt.query_map([account_key], |row| {
        let raw: String = row.get(0)?;
        let outcome: Outcome =
            serde_json::from_str(&raw).map_err(|_| rusqlite::Error::InvalidQuery)?;
        Ok((outcome, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (outcome, created_ms) = row?;
        if outcome.state != State::Claimed || created_ms > now_ms {
            continue;
        }
        let day = date(created_ms)?;
        let period = match outcome
            .period_key
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            Some(value) => (true, value.to_owned()),
            None => (false, day.to_string()),
        };
        if !periods.insert(period) {
            continue;
        }
        units.insert(outcome.unit);
        let reward = outcome
            .reward
            .filter(|value| value.is_finite() && *value >= 0.0);
        all.add(day, reward);
        if day >= monday && day < next_monday {
            week.add(day, reward);
        }
        if day.year() == today.year() && day.month() == today.month() {
            month.add(day, reward);
        }
    }
    let unit = if units.len() == 1 {
        units.into_iter().next().unwrap()
    } else {
        String::new()
    };
    if unit.trim().is_empty() && !all.days.is_empty() {
        // DTO 只有一个公共单位，混合/缺失单位时任何桶都不能给出误导性金额。
        week.reward = None;
        month.reward = None;
        all.reward = None;
    }
    Ok(CheckinStatistics {
        has_records: !all.days.is_empty(),
        unit,
        week: week.totals(),
        month: month.totals(),
        all: all.totals(),
    })
}

struct Bucket {
    days: HashSet<NaiveDate>,
    reward: Option<f64>,
}

impl Bucket {
    fn new() -> Self {
        Self {
            days: HashSet::new(),
            reward: Some(0.0),
        }
    }

    fn add(&mut self, day: NaiveDate, reward: Option<f64>) {
        self.days.insert(day);
        self.reward = self
            .reward
            .zip(reward)
            .map(|(sum, value)| sum + value)
            .filter(|sum| sum.is_finite());
    }

    fn totals(self) -> CheckinTotals {
        CheckinTotals {
            days: self.days.len() as u32,
            reward: self.reward,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkin_types::{Outcome, State};
    use chrono::{FixedOffset, TimeZone};
    use rusqlite::params;

    fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::daily_checkin::init(&conn).unwrap();
        conn
    }

    fn ms(value: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(value)
            .unwrap()
            .timestamp_millis()
    }

    fn insert(
        conn: &Connection,
        connection: &str,
        key: &str,
        when: &str,
        state: State,
        reward: Option<f64>,
        period: Option<&str>,
        unit: &str,
    ) {
        insert_ms(conn, connection, key, ms(when), state, reward, period, unit);
    }

    fn insert_ms(
        conn: &Connection,
        connection: &str,
        key: &str,
        when: i64,
        state: State,
        reward: Option<f64>,
        period: Option<&str>,
        unit: &str,
    ) {
        let mut outcome = Outcome::new(state, "合成结果");
        outcome.reward = reward;
        outcome.period_key = period.map(str::to_owned);
        outcome.unit = unit.into();
        conn.execute(
            "INSERT INTO daily_checkin_records(connection_id,name,platform,edition,account_key,outcome,created_ms) VALUES(?1,'合成账号','qoder','cn',?2,?3,?4)",
            params![connection, key, serde_json::to_string(&outcome).unwrap(), when],
        ).unwrap();
    }

    #[test]
    fn beijing_week_crosses_year_and_month_uses_natural_month() {
        let conn = database();
        for (when, reward) in [
            ("2025-12-28T23:59:59+08:00", 1.0),
            ("2025-12-29T00:00:00+08:00", 2.0),
            ("2025-12-31T23:59:59+08:00", 3.0),
            // UTC 仍为 12 月，但北京时间已是元旦。
            ("2025-12-31T16:00:00Z", 4.0),
            ("2026-01-02T10:00:00+08:00", 5.0),
        ] {
            insert(
                &conn,
                "a",
                "account",
                when,
                State::Claimed,
                Some(reward),
                None,
                "积分",
            );
        }
        let stats = read(&conn, "account", ms("2026-01-02T12:00:00+08:00")).unwrap();
        assert!(stats.has_records);
        assert_eq!((stats.week.days, stats.week.reward), (4, Some(14.0)));
        assert_eq!((stats.month.days, stats.month.reward), (2, Some(9.0)));
        assert_eq!((stats.all.days, stats.all.reward), (5, Some(15.0)));
    }

    #[test]
    fn monday_and_new_month_exclude_previous_period_and_future_records() {
        let conn = database();
        for (when, reward) in [
            ("2026-01-31T23:59:59+08:00", 1.0),
            ("2026-02-01T00:00:00+08:00", 2.0),
            ("2026-02-02T00:00:00+08:00", 3.0),
            ("2026-03-01T00:00:00+08:00", 4.0),
        ] {
            insert(
                &conn,
                "a",
                "account",
                when,
                State::Claimed,
                Some(reward),
                None,
                "积分",
            );
        }
        let stats = read(&conn, "account", ms("2026-02-02T12:00:00+08:00")).unwrap();
        assert_eq!((stats.week.days, stats.week.reward), (1, Some(3.0)));
        assert_eq!((stats.month.days, stats.month.reward), (2, Some(5.0)));
        assert_eq!((stats.all.days, stats.all.reward), (3, Some(6.0)));
    }

    #[test]
    fn future_day_in_the_same_week_and_month_is_excluded_from_every_bucket() {
        let conn = database();
        let now = ms("2026-10-27T12:00:00+08:00");
        insert(
            &conn,
            "a",
            "account",
            "2026-10-28T10:00:00+08:00",
            State::Claimed,
            Some(7.0),
            None,
            "积分",
        );
        let stats = read(&conn, "account", now).unwrap();
        assert!(!stats.has_records);
        for totals in [stats.week, stats.month, stats.all] {
            assert_eq!((totals.days, totals.reward), (0, Some(0.0)));
        }
        let count: i64 = conn
            .query_row("SELECT count(*) FROM daily_checkin_records", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn future_millisecond_is_excluded_but_exact_snapshot_time_is_included() {
        let conn = database();
        let now = ms("2026-10-27T12:00:00+08:00");
        insert_ms(
            &conn,
            "a",
            "account",
            now + 1,
            State::Claimed,
            Some(7.0),
            Some("future"),
            "积分",
        );
        let stats = read(&conn, "account", now).unwrap();
        assert!(!stats.has_records);
        for totals in [stats.week, stats.month, stats.all] {
            assert_eq!((totals.days, totals.reward), (0, Some(0.0)));
        }
        insert_ms(
            &conn,
            "a",
            "account",
            now,
            State::Claimed,
            Some(2.0),
            Some("completed"),
            "积分",
        );
        let stats = read(&conn, "account", now).unwrap();
        assert!(stats.has_records);
        for totals in [stats.week, stats.month, stats.all] {
            assert_eq!((totals.days, totals.reward), (1, Some(2.0)));
        }
    }

    #[test]
    fn failed_unknown_and_ready_records_never_count_as_success() {
        let conn = database();
        for state in [
            State::Ready,
            State::Failed,
            State::Unknown,
            State::Unauthorized,
        ] {
            insert(
                &conn,
                "a",
                "account",
                "2026-10-27T10:00:00+08:00",
                state,
                Some(999.0),
                None,
                "积分",
            );
        }
        let now = ms("2026-10-27T12:00:00+08:00");
        let empty = read(&conn, "account", now).unwrap();
        assert!(!empty.has_records);
        assert_eq!((empty.all.days, empty.all.reward), (0, Some(0.0)));
        insert(
            &conn,
            "a",
            "account",
            "2026-10-26T10:00:00+08:00",
            State::Claimed,
            Some(2.0),
            None,
            "积分",
        );
        let stats = read(&conn, "account", now).unwrap();
        assert_eq!((stats.all.days, stats.all.reward), (1, Some(2.0)));
    }

    #[test]
    fn unknown_reward_only_invalidates_its_buckets_and_real_zero_is_preserved() {
        let conn = database();
        insert(
            &conn,
            "a",
            "account",
            "2026-09-30T10:00:00+08:00",
            State::Claimed,
            None,
            None,
            "积分",
        );
        insert(
            &conn,
            "a",
            "account",
            "2026-10-27T10:00:00+08:00",
            State::Claimed,
            Some(0.0),
            None,
            "积分",
        );
        let stats = read(&conn, "account", ms("2026-10-27T12:00:00+08:00")).unwrap();
        assert_eq!((stats.week.days, stats.week.reward), (1, Some(0.0)));
        assert_eq!((stats.month.days, stats.month.reward), (1, Some(0.0)));
        assert_eq!((stats.all.days, stats.all.reward), (2, None));
        insert(
            &conn,
            "a",
            "account",
            "2026-10-26T10:00:00+08:00",
            State::Claimed,
            None,
            None,
            "积分",
        );
        let stats = read(&conn, "account", ms("2026-10-27T12:00:00+08:00")).unwrap();
        assert_eq!((stats.week.days, stats.week.reward), (2, None));
        assert_eq!((stats.month.days, stats.month.reward), (2, None));
    }

    #[test]
    fn duplicate_connections_share_official_period_and_use_first_claim_date() {
        let conn = database();
        insert(
            &conn,
            "native",
            "account",
            "2026-09-30T23:59:59+08:00",
            State::Claimed,
            None,
            Some("official-1"),
            "积分",
        );
        insert(
            &conn,
            "browser",
            "account",
            "2026-10-27T10:00:00+08:00",
            State::Claimed,
            Some(100.0),
            Some("official-1"),
            "积分",
        );
        insert(
            &conn,
            "native",
            "account",
            "2026-10-27T10:01:00+08:00",
            State::Claimed,
            Some(3.0),
            Some("official-2"),
            "积分",
        );
        insert(
            &conn,
            "browser",
            "account",
            "2026-10-27T10:02:00+08:00",
            State::Claimed,
            Some(3.0),
            Some("official-2"),
            "积分",
        );
        let stats = read(&conn, "account", ms("2026-10-27T12:00:00+08:00")).unwrap();
        assert_eq!((stats.week.days, stats.week.reward), (1, Some(3.0)));
        assert_eq!((stats.month.days, stats.month.reward), (1, Some(3.0)));
        assert_eq!((stats.all.days, stats.all.reward), (2, None));
    }

    #[test]
    fn missing_period_uses_beijing_day_and_same_day_counts_once() {
        let conn = database();
        insert(
            &conn,
            "native",
            "account",
            "2026-10-26T16:01:00Z",
            State::Claimed,
            Some(5.0),
            None,
            "积分",
        );
        insert(
            &conn,
            "browser",
            "account",
            "2026-10-27T08:00:00+08:00",
            State::Claimed,
            Some(5.0),
            None,
            "积分",
        );
        let stats = read(&conn, "account", ms("2026-10-27T12:00:00+08:00")).unwrap();
        assert_eq!((stats.all.days, stats.all.reward), (1, Some(5.0)));
    }

    #[test]
    fn full_history_is_not_limited_to_recent_fifty_records_and_identity_is_isolated() {
        let conn = database();
        let first = FixedOffset::east_opt(8 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 1, 1, 10, 0, 0)
            .unwrap()
            .timestamp_millis();
        for day in 0..65 {
            insert_ms(
                &conn,
                "same-connection",
                "account-a",
                first + day * 86_400_000,
                State::Claimed,
                Some(1.0),
                None,
                "Credits",
            );
        }
        insert(
            &conn,
            "same-connection",
            "account-b",
            "2026-10-27T10:00:00+08:00",
            State::Claimed,
            Some(900.0),
            None,
            "积分",
        );
        let stats = read(&conn, "account-a", ms("2026-10-27T12:00:00+08:00")).unwrap();
        assert_eq!((stats.all.days, stats.all.reward), (65, Some(65.0)));
        assert_eq!(stats.unit, "Credits");
        assert_eq!((stats.week.days, stats.week.reward), (0, Some(0.0)));
        let other = read(&conn, "account-b", ms("2026-10-27T12:00:00+08:00")).unwrap();
        assert_eq!((other.all.days, other.all.reward), (1, Some(900.0)));
        let serialized = serde_json::to_string(&stats).unwrap();
        assert!(!serialized.contains("account-a") && !serialized.contains("same-connection"));
    }

    #[test]
    fn no_records_remains_explicitly_empty() {
        let stats = read(&database(), "missing", ms("2026-10-27T12:00:00+08:00")).unwrap();
        assert!(!stats.has_records);
        assert_eq!(
            (stats.week.days, stats.month.days, stats.all.days),
            (0, 0, 0)
        );
    }

    #[test]
    fn mixed_units_never_produce_an_added_reward_or_single_unit_label() {
        let conn = database();
        insert(
            &conn,
            "a",
            "account",
            "2026-10-26T10:00:00+08:00",
            State::Claimed,
            Some(2.0),
            None,
            "Credits",
        );
        insert(
            &conn,
            "a",
            "account",
            "2026-10-27T10:00:00+08:00",
            State::Claimed,
            Some(3.0),
            None,
            "积分",
        );
        let stats = read(&conn, "account", ms("2026-10-27T12:00:00+08:00")).unwrap();
        assert!(stats.unit.is_empty());
        assert_eq!((stats.week.days, stats.week.reward), (2, None));
        assert_eq!(stats.month.reward, None);
        assert_eq!(stats.all.reward, None);
    }

    #[test]
    fn unreadable_record_is_reported_as_a_statistics_error() {
        let conn = database();
        conn.execute("INSERT INTO daily_checkin_records(connection_id,name,platform,edition,account_key,outcome,created_ms) VALUES('a','test','qoder','cn','account','broken',1)", []).unwrap();
        assert!(read(&conn, "account", ms("2026-10-27T12:00:00+08:00")).is_err());
    }
}
