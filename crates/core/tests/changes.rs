use jiff::Timestamp;
use serde_json::{Value, json};
use worklens_core::marks::{Marks, apply, mark_all_seen};
use worklens_core::{CoreConfig, Raw, Snapshot, build_snapshot};

fn at(t: &str) -> Timestamp {
    t.parse().unwrap()
}

fn fixture() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/raw.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn snapshot(raw: Value, now: &str) -> Snapshot {
    let raw: Raw = serde_json::from_value(raw).unwrap();
    build_snapshot(raw, &CoreConfig::default(), at(now))
}

fn changes_after(edit: impl FnOnce(&mut Value)) -> Vec<String> {
    let mut marks = Marks::default();
    let before = snapshot(fixture(), "2026-11-03T12:00:00Z");
    mark_all_seen(&mut marks, &before, at("2026-11-03T12:00:00Z"));
    marks.last_look = Some(at("2026-11-03T12:00:00Z"));
    let mut raw = fixture();
    edit(&mut raw);
    let mut after = snapshot(raw, "2026-11-03T13:00:00Z");
    apply(&mut after, &marks, at("2026-11-03T13:00:00Z"));
    let e = after
        .entries
        .iter()
        .find(|e| after.id(e).to_string() == "1995008")
        .expect("bug 1995008 is listed");
    e.changes.clone()
}

fn revision(raw: &mut Value) -> &mut Value {
    raw["revisions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|r| r["id"] == 321987)
        .unwrap()
}

#[test]
fn nothing_changed_lists_nothing() {
    assert_eq!(changes_after(|_| {}), Vec::<String>::new());
}

#[test]
fn changes_across_the_three_tools_are_each_said_once() {
    let changes = changes_after(|raw| {
        revision(raw)["reviewers"][0]["status"] = "accepted".into();
        raw["issues"]["AIPLAT-1416"]["status"] = "In Progress".into();
        raw["lastComments"]["1995008"] = json!({ "author": "b@mozilla.com", "time": "2026-11-03T12:30:00Z", "text": "Any news?" });
    });
    assert_eq!(
        changes,
        vec![
            "D321987: quillfox blocking → accepted",
            "AIPLAT-1416: ENG REVIEW → In Progress",
            "new drift: Jira is In Progress but a revision is in review; expected ENG REVIEW",
            "new comment from b@mozilla.com",
        ],
    );
}

#[test]
fn an_answered_needinfo_moves_the_item_and_says_so() {
    let changes = changes_after(|raw| {
        for bug in raw["bugs"].as_array_mut().unwrap() {
            if bug["id"] == 1995008 {
                bug["flags"] = json!([]);
            }
        }
    });
    assert!(changes.iter().any(|c| c.starts_with("moved: Waiting on you → ")), "{changes:?}");
    assert!(
        changes.contains(&"needinfo from a@mozilla.com answered or cleared".to_owned()),
        "{changes:?}"
    );
}

#[test]
fn my_own_comment_is_not_a_change() {
    let changes = changes_after(|raw| {
        raw["lastComments"]["1995008"] = json!({ "author": "me@mozilla.com", "time": "2026-11-03T12:30:00Z", "text": "Done." });
    });
    assert_eq!(changes, Vec::<String>::new());
}

#[test]
fn a_failed_source_is_not_reported_as_removed_work() {
    let changes = changes_after(|raw| {
        raw["issues"] = json!({});
        raw["errors"] = json!([{ "source": "jira", "message": "timed out" }]);
    });
    assert!(!changes.iter().any(|c| c.contains("Jira")), "{changes:?}");
}

#[test]
fn an_item_never_seen_has_no_baseline_to_compare() {
    let mut s = snapshot(fixture(), "2026-11-03T13:00:00Z");
    let marks = Marks {
        last_look: Some(at("2026-11-03T12:00:00Z")),
        ..Marks::default()
    };
    apply(&mut s, &marks, at("2026-11-03T13:00:00Z"));
    let e = s.entries.iter().find(|e| s.id(e).to_string() == "1995008").unwrap();
    assert!(e.marks.is_new);
    assert!(e.changes.is_empty(), "{:?}", e.changes);
}

#[test]
fn a_new_diff_and_try_push_are_changes_even_when_the_status_stays_the_same() {
    let changes = changes_after(|raw| {
        revision(raw)["diffPHID"] = json!("PHID-DIFF-new");
        raw["tryPushes"]["321987"]["revision"] = json!("new-try-commit");
    });
    assert!(changes.iter().any(|c| c == "D321987: new diff"), "{changes:?}");
    assert!(changes.iter().any(|c| c.starts_with("D321987 try push:")), "{changes:?}");
}

#[test]
fn release_approvals_and_landing_health_are_changes_without_a_section_transition() {
    let changes = changes_after(|raw| {
        raw["bugs"][0]["release_flags"]["cf_status_firefox160"] = json!("fixed");
        raw["uplift"]["1995008"] = json!({"patch":true,"approvals":[["beta", "+"]]});
        raw["landings"] = json!({"revisions":[321987],"repositories":[{
            "repository":"autoland","checkedAt":"2026-11-03T13:00:00Z","pushLimit":100,"scannedPushes":1,
            "oldestPush":"2026-11-03T12:30:00Z","error":null,"matches":[{
                "revision":321987,"commit":"commit","pushRevision":"push","url":"https://treeherder.mozilla.org/jobs?repo=autoland",
                "pushedAt":"2026-11-03T12:30:00Z","healthError":null,
                "health":{"builds":"pass","lint":"pass","tests":"fail","failed":1,"running":0}
            }]
        }]});
    });
    for label in ["cf_status_firefox160", "uplift approvals", "autoland landing", "autoland jobs"] {
        assert!(changes.iter().any(|c| c.contains(label)), "{label}: {changes:?}");
    }
}

#[test]
fn failed_lookups_do_not_report_disappeared_try_or_release_evidence() {
    let changes = changes_after(|raw| {
        raw["tryPushes"] = json!({});
        raw["uplift"] = json!({});
        raw["errors"] = json!([
            {"source":"treeherder","message":"unavailable"},
            {"source":"bugzilla","message":"unavailable"},
            {"source":"treeherder-landings","message":"unavailable"}
        ]);
    });
    assert!(changes.is_empty(), "{changes:?}");
}

#[test]
fn new_diff_wakes_both_done_and_snoozed_items() {
    use worklens_core::marks::{mark_done, mark_snoozed};
    let before = snapshot(fixture(), "2026-11-03T12:00:00Z");
    let e = before.entry(before.find("1995008").unwrap());
    for snooze in [false, true] {
        let mut marks = Marks::default();
        mark_all_seen(&mut marks, &before, before.now);
        if snooze {
            mark_snoozed(&mut marks, &before, e, None, before.now);
        } else {
            mark_done(&mut marks, &before, e, before.now);
        }
        let mut raw = fixture();
        revision(&mut raw)["diffPHID"] = json!("PHID-DIFF-new");
        let mut after = snapshot(raw, "2026-11-03T13:00:00Z");
        apply(&mut after, &marks, before.now);
        let e = after.entry(after.find("1995008").unwrap());
        assert!(e.marks.hidden.is_none());
        assert!(e.marks.is_new);
    }
}

#[test]
fn legacy_marks_acquire_a_baseline_then_detect_a_later_diff_without_waking_early() {
    use worklens_core::marks::{Signature, mark_done, upgrade};
    let mut before = snapshot(fixture(), "2026-11-03T12:00:00Z");
    let e = before.entry(before.find("1995008").unwrap());
    let mut marks = Marks::default();
    mark_done(&mut marks, &before, e, before.now);
    let sig = &mut marks.items.get_mut("1995008").unwrap().done.as_mut().unwrap().sig;
    let mut old = Signature::parse(sig).unwrap();
    old.facts = None;
    *sig = old.to_sig();
    assert!(upgrade(&mut marks, &before));
    assert!(!upgrade(&mut marks, &before));
    let now = before.now;
    apply(&mut before, &marks, now);
    assert!(before.entry(before.find("1995008").unwrap()).marks.hidden.is_some());
    let mut raw = fixture();
    revision(&mut raw)["diffPHID"] = json!("PHID-DIFF-new");
    let mut after = snapshot(raw, "2026-11-03T13:00:00Z");
    apply(&mut after, &marks, now);
    assert!(after.entry(after.find("1995008").unwrap()).marks.hidden.is_none());
}

#[test]
fn a_section_worklens_no_longer_has_is_not_a_move() {
    use std::borrow::Cow;
    use worklens_core::changes::{Failed, changes};
    use worklens_core::marks::Signature;
    let sig = |section: &'static str| Signature {
        section: Some(Cow::Borrowed(section)),
        ..Signature::default()
    };
    assert!(changes(&sig("uplift"), &sig("landed"), Failed::default()).is_empty());
    assert_eq!(
        changes(&sig("waiting"), &sig("landed"), Failed::default()),
        ["moved: Waiting on you → Landed"]
    );
}
