use jiff::Timestamp;
use serde_json::{Value, json};
use worklens_core::marks::{Marks, apply, mark_all_seen};
use worklens_core::timeline::{events, ships, since};
use worklens_core::{CoreConfig, Raw, Snapshot, build_snapshot};

fn at(t: &str) -> Timestamp {
    t.parse().unwrap()
}

fn fixture() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/raw.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn snapshot(raw: Value) -> Snapshot {
    build_snapshot(
        serde_json::from_value::<Raw>(raw).unwrap(),
        &CoreConfig::default(),
        at("2026-11-03T20:00:00Z"),
    )
}

fn texts(s: &Snapshot) -> Vec<String> {
    let e = s.entries.iter().find(|e| s.id(e).to_string() == "1995008").unwrap();
    events(s, e).into_iter().map(|ev| format!("{} {}", ev.source, ev.text)).collect()
}

fn with_history() -> Value {
    let mut raw = fixture();
    raw["bugs"][0]["creation_time"] = json!("2026-10-01T09:00:00Z");
    raw["issues"]["AIPLAT-1416"]["created"] = json!("2026-10-02T09:00:00Z");
    raw["revisions"][0]["created"] = json!("2026-10-20T09:00:00Z");
    raw["landings"] = json!({"revisions":[321987],"repositories":[{
        "repository":"autoland","checkedAt":"2026-11-03T19:00:00Z","pushLimit":100,"scannedPushes":100,
        "oldestPush":"2026-11-02T00:00:00Z","error":null,"matches":[{
            "revision":321987,"commit":"1111","pushRevision":"2222","url":"https://treeherder.mozilla.org/x",
            "pushedAt":"2026-11-03T10:00:00Z","healthError":null,"health":null
        }]
    }]});
    raw
}

#[test]
fn events_from_every_tool_read_in_order() {
    assert_eq!(
        texts(&snapshot(with_history())),
        [
            "Bugzilla Bug 1995008 filed",
            "Jira AIPLAT-1416 created",
            "Phabricator D321987 posted",
            "Phabricator D321987 new diff",
            "Treeherder D321987 pushed to try",
            "Bugzilla needinfo to you from a@mozilla.com",
            "Treeherder D321987 landed on autoland",
        ]
    );
}

#[test]
fn an_old_cache_without_creation_times_still_has_a_timeline() {
    let t = texts(&snapshot(fixture()));
    assert!(t.contains(&"Phabricator D321987 new diff".to_owned()), "{t:?}");
    assert!(!t.iter().any(|x| x.contains("filed") || x.contains("created")), "{t:?}");
}

#[test]
fn a_last_change_apart_from_the_diff_is_said_with_the_status() {
    let mut raw = with_history();
    raw["revisions"][0]["modified"] = json!("2026-10-25T00:00:00Z");
    raw["revisions"][0]["status"] = json!("accepted");
    assert!(texts(&snapshot(raw)).contains(&"Phabricator D321987 last changed, now accepted".to_owned()));
}

fn ship_text(flags: Value) -> Option<String> {
    let mut raw = fixture();
    raw["bugs"][0]["release_flags"] = flags;
    let s = snapshot(raw);
    ships(&s, &s.entries[0])
}

#[test]
fn ships_says_which_release_from_the_lowest_fixed_version() {
    assert_eq!(
        ship_text(json!({"cf_status_firefox161":"fixed","cf_status_firefox_esr153":"fixed"})).as_deref(),
        Some("Fixed in Nightly 161; it becomes Beta on Nov 5 (Bugzilla status)")
    );
    assert_eq!(
        ship_text(json!({"cf_status_firefox161":"fixed","cf_status_firefox160":"fixed"})).as_deref(),
        Some("Fixed in Beta 160; Firefox 160 releases Nov 6 (Bugzilla status)")
    );
    assert_eq!(
        ship_text(json!({"cf_status_firefox159":"verified"})).as_deref(),
        Some("Firefox 159 has the fix and is released (Bugzilla status)")
    );
    assert_eq!(ship_text(json!({"cf_status_firefox161":"affected"})), None);
}

#[test]
fn since_counts_my_changed_and_new_items_once() {
    let mut marks = Marks::default();
    let before = snapshot(fixture());
    mark_all_seen(&mut marks, &before, at("2026-11-03T12:00:00Z"));
    marks.last_look = Some(at("2026-11-03T12:00:00Z"));
    let mut raw = fixture();
    raw["revisions"][0]["reviewers"][0]["status"] = json!("accepted");
    let mut after = snapshot(raw);
    apply(&mut after, &marks, at("2026-11-03T20:00:00Z"));
    let got = since(&after);
    assert_eq!((got.changed, got.new), (1, 0), "{got:?}");
    assert!(got.highlights[0].starts_with("1995008 "), "{got:?}");
    assert!(got.highlights[0].contains("accepted"), "{got:?}");

    let mut quiet = snapshot(fixture());
    apply(&mut quiet, &marks, at("2026-11-03T20:00:00Z"));
    assert_eq!(since(&quiet), Default::default());
}
