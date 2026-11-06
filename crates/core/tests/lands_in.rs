use jiff::civil::date;
use worklens_core::raw::Trains;
use worklens_core::train::{LandsIn, lands_in};
use worklens_core::{CoreConfig, Raw, Section, build_snapshot};

fn trains() -> Trains {
    Trains {
        nightly: 162,
        beta: 161,
        release: 160,
        esr: vec![153],
        next_merge: date(2026, 11, 10),
        next_release: date(2026, 11, 11),
    }
}

#[test]
fn merge_day_and_stale_calendars_do_not_predict_the_next_version() {
    let before = LandsIn {
        version: 162,
        if_by: Some(date(2026, 11, 9)),
        after: Some(163),
    };
    assert_eq!(lands_in(&trains(), date(2026, 11, 9)), Some(before));
    assert_eq!(lands_in(&trains(), date(2026, 11, 10)), None);
    assert_eq!(lands_in(&trains(), date(2026, 11, 11)), None);
    assert_eq!(lands_in(&trains(), date(2027, 1, 1)), None);
    let mut invalid = trains();
    invalid.next_release = date(2026, 11, 8);
    assert_eq!(lands_in(&invalid, date(2026, 11, 9)), None);
}

#[test]
fn only_accepted_entries_carry_it_and_never_without_trains() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/busy.json");
    let raw: Raw = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let s = build_snapshot(raw, &CoreConfig::default(), "2026-11-03T19:00:00Z".parse().unwrap());
    let accepted = s
        .section(Section::Land, true)
        .map(|i| s.entry(i))
        .find(|e| matches!(e.placement.unwrap().reason, worklens_core::rules::Reason::Accepted(_)));
    let e = accepted.expect("busy has an accepted revision");
    assert_eq!(
        s.lands_in(e),
        Some(LandsIn {
            version: 147,
            if_by: Some(date(2026, 11, 8)),
            after: Some(148)
        })
    );
    let waiting = s.entry(s.section(Section::Waiting, true).next().unwrap());
    assert_eq!(s.lands_in(waiting), None);
    let mut raw: Raw = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    raw.trains = None;
    let s = build_snapshot(raw, &CoreConfig::default(), "2026-11-03T19:00:00Z".parse().unwrap());
    assert!(s.entries.iter().all(|e| s.lands_in(e).is_none()), "absent, not guessed");
}

#[test]
fn status_flags_are_not_build_inclusion_and_versions_in_a_gap_are_unknown() {
    use worklens_core::train::{Train, position};
    let mut t = trains();
    t.beta = 160;
    t.release = 159;
    assert_eq!(position(161, &t, date(2026, 11, 9)), None);
    assert_eq!(position(159, &t, date(2026, 11, 9)), Some(Train::Released));
    assert_eq!(position(159, &t, date(2026, 11, 9)).map(Train::id), Some("released"));
    assert_eq!(position(162, &t, date(2026, 11, 10)), None);
}

#[test]
fn failed_or_carried_calendars_and_old_caches_do_not_predict_a_release() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/busy.json");
    for field in ["failed", "carried", "old", "future"] {
        let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        match field {
            "failed" => value["errors"] = serde_json::json!([{"source":"product-details", "message":"lookup failed"}]),
            "carried" => value["stale"] = serde_json::json!([{"source":"product-details", "since":"2026-10-01T00:00:00Z"}]),
            "old" => value["fetchedAt"] = serde_json::json!("2026-10-01T00:00:00Z"),
            _ => value["fetchedAt"] = serde_json::json!("2026-11-04T00:00:00Z"),
        }
        let raw: Raw = serde_json::from_value(value).unwrap();
        let s = build_snapshot(raw, &CoreConfig::default(), "2026-11-03T19:00:00Z".parse().unwrap());
        assert!(s.entries.iter().all(|e| s.lands_in(e).is_none()), "{field}");
    }
}

#[test]
fn cache_freshness_includes_48_hours_and_rejects_later_or_future_timestamps() {
    let raw: Raw = serde_json::from_str(include_str!("../../store/fixtures/busy.json")).unwrap();
    for (seconds, expected) in [(-1, false), (0, true), (48 * 60 * 60, true), (48 * 60 * 60 + 1, false)] {
        let now = raw.fetched_at.checked_add(jiff::SignedDuration::from_secs(seconds)).unwrap();
        assert_eq!(worklens_core::train::trusted(&raw, now), expected, "{seconds}");
    }
}

#[test]
fn release_day_and_duplicate_train_numbers_are_independently_unknown() {
    let today = date(2026, 11, 9);
    let mut t = trains();
    t.next_release = today;
    assert_eq!(lands_in(&t, today), None);
    let mut t = trains();
    t.release = t.beta;
    assert_eq!(lands_in(&t, today), None);
    let mut t = trains();
    t.beta = t.nightly;
    assert_eq!(lands_in(&t, today), None);
}
