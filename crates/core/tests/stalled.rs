use serde_json::{Value, json};
use worklens_core::{CoreConfig, Raw, Snapshot, build_snapshot};

const NOW: &str = "2026-11-20T12:00:00Z";

fn ago(days: i64) -> String {
    let now: jiff::Timestamp = NOW.parse().unwrap();
    now.checked_sub(jiff::SignedDuration::from_hours(24 * days)).unwrap().to_string()
}

fn ticket(key: &str, category: &str, updated: &str, sprint: bool) -> Value {
    json!({
        "key": key, "summary": format!("ticket {key}"), "status": if category == "new" { "To Do" } else { "In Progress" },
        "statusCategory": category, "assignee": "me@mozilla.com", "updated": updated, "points": 1,
        "sprint": if sprint { json!({"name": "Sprint 9", "state": "active"}) } else { Value::Null },
    })
}

fn bug_with_revision(bug_changed: &str, rev_modified: &str) -> (Value, Value) {
    let bug = json!({
        "id": 7, "summary": "bug 7", "status": "ASSIGNED", "resolution": "", "assigned_to": "me@mozilla.com",
        "severity": "S3", "priority": "P3", "keywords": [], "whiteboard": "", "see_also": [],
        "last_change_time": bug_changed, "product": "Core", "component": "ML", "via": ["assigned"],
    });
    let rev = json!({
        "id": 123, "title": "Bug 7 - x", "status": "changes-planned", "author": "PHID-ME", "bug": 7,
        "modified": rev_modified, "reviewers": [],
    });
    (bug, rev)
}

fn snapshot(bugs: &[Value], issues: &[Value], revisions: &[Value]) -> Snapshot {
    let issues: serde_json::Map<String, Value> = issues.iter().map(|i| (i["key"].as_str().unwrap().to_owned(), i.clone())).collect();
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": NOW,
        "me": { "email": "me@mozilla.com", "jiraEmail": "me@mozilla.com", "phid": "PHID-ME" },
        "bugs": bugs, "issues": issues, "revisions": revisions, "toReview": [], "bugRevisions": {}, "lastComments": {}, "errors": [],
    }))
    .unwrap();
    build_snapshot(raw, &CoreConfig::default(), NOW.parse().unwrap())
}

fn row(s: &Snapshot, id: &str) -> (bool, String) {
    let e = s
        .entries
        .iter()
        .find(|e| s.id(e).to_string() == id)
        .unwrap_or_else(|| panic!("no {id}"));
    (e.placement.unwrap().stale, s.reason(e).to_string())
}

#[test]
fn ticket_age_alone_does_not_apply_an_unrelated_inactivity_policy() {
    let s = snapshot(
        &[],
        &[
            ticket("PROJ-12", "indeterminate", &ago(15), true),
            ticket("PROJ-13", "indeterminate", &ago(13), true),
            ticket("PROJ-14", "indeterminate", &ago(15), false),
            ticket("PROJ-15", "new", &ago(30), false),
        ],
        &[],
    );
    assert_eq!(row(&s, "PROJ-12"), (false, "PROJ-12 is In Progress in Sprint 9".into()));
    assert_eq!(row(&s, "PROJ-13"), (false, "PROJ-13 is In Progress in Sprint 9".into()));
    assert_eq!(row(&s, "PROJ-14"), (false, "PROJ-14 is In Progress".into()));
    assert!(!row(&s, "PROJ-15").0, "not started is not stalled");
}

#[test]
fn bug_and_revision_age_alone_do_not_manufacture_an_obligation() {
    let (bug, rev) = bug_with_revision(&ago(15), &ago(15));
    let s = snapshot(&[bug], &[], &[rev]);
    assert_eq!(row(&s, "7"), (false, "bug is ASSIGNED".into()));
    let (bug, rev) = bug_with_revision(&ago(20), &ago(13));
    let s = snapshot(&[bug], &[], &[rev]);
    assert!(!row(&s, "7").0, "the revision moved 13 days ago: {:?}", row(&s, "7"));
    let (bug, mut rev) = bug_with_revision(&ago(40), &ago(40));
    rev["status"] = json!("published");
    let s = snapshot(&[bug], &[], &[rev]);
    assert!(!row(&s, "7").0, "no open revision, so nothing stalled");
}

#[test]
fn an_old_open_revision_stays_in_its_existing_section() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/busy.json");
    let raw: Raw = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let s = build_snapshot(raw, &CoreConfig::default(), "2026-11-03T19:00:00Z".parse().unwrap());
    assert_eq!(row(&s, "2010201"), (false, "bug is ASSIGNED".into()));
}
