use jiff::civil::{Date, date};
use serde_json::json;
use worklens_core::drift_log::{DriftLog, KEEP_DAYS, counts};
use worklens_core::{CoreConfig, Raw, build_snapshot};

fn day(d: i8) -> Date {
    date(2026, 11, d)
}

#[test]
fn one_point_per_day_kept_in_order() {
    let mut log = DriftLog::default();
    log.record(day(3), 4, 0);
    log.record(day(1), 6, 0);
    log.record(day(3), 3, 0);
    let dates: Vec<(Date, usize)> = log.days.iter().map(|d| (d.date, d.mine)).collect();
    assert_eq!(dates, vec![(day(1), 6), (day(3), 3)]);
}

#[test]
fn old_days_drop_off() {
    let mut log = DriftLog::default();
    let start = date(2026, 1, 1);
    for n in 0..(KEEP_DAYS as i64 + 5) {
        log.record(start.checked_add(jiff::Span::new().days(n)).unwrap(), 1, 0);
    }
    assert_eq!(log.days.len(), KEEP_DAYS);
    assert_eq!(log.days[0].date, start.checked_add(jiff::Span::new().days(5)).unwrap());
}

#[test]
fn the_baseline_is_the_oldest_point_in_two_weeks_before_today() {
    let mut log = DriftLog::default();
    log.record(date(2026, 10, 1), 9, 0);
    log.record(date(2026, 10, 25), 6, 0);
    log.record(day(1), 5, 0);
    log.record(day(3), 4, 0);
    assert_eq!(log.baseline(day(3)).map(|d| (d.date, d.mine)), Some((date(2026, 10, 25), 6)));
    assert_eq!(DriftLog::default().baseline(day(3)), None);
}

#[test]
fn a_failed_bugzilla_or_jira_records_nothing() {
    let snapshot = |errors: serde_json::Value| {
        let raw: Raw = serde_json::from_value(json!({
            "version": 1, "fetchedAt": "2026-11-02T12:00:00Z",
            "me": { "email": "me@mozilla.com", "jiraEmail": "me@mozilla.com", "phid": "PHID-ME" },
            "bugs": [], "issues": {}, "revisions": [], "toReview": [], "bugRevisions": {}, "lastComments": {},
            "errors": errors,
        }))
        .unwrap();
        build_snapshot(raw, &CoreConfig::default(), "2026-11-02T12:00:00Z".parse().unwrap())
    };
    assert_eq!(counts(&snapshot(json!([]))), Some((0, 0)));
    assert_eq!(counts(&snapshot(json!([{ "source": "jira", "message": "timed out" }]))), None);
    assert_eq!(counts(&snapshot(json!([{ "source": "bugzilla", "message": "401" }]))), None);
    assert_eq!(
        counts(&snapshot(json!([{ "source": "treeherder", "message": "502" }]))),
        Some((0, 0))
    );
}
