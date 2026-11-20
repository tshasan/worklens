use jiff::Timestamp;
use serde_json::{Value, json};
use worklens_core::section::Section;
use worklens_core::{CoreConfig, Raw, Snapshot, build_snapshot};

const ME: &str = "me@mozilla.com";
const TITLE: &str = "Bug 2093725 - Stop blaming a headless read on an off-site load that never committed";

fn rev(id: u32, title: &str, status: &str, author: &str) -> Value {
    json!({
        "id": id, "title": title, "status": status, "author": author, "bug": 2093725,
        "modified": "2026-11-18T12:00:00Z", "reviewers": [],
    })
}

fn snapshot(revisions: &[Value]) -> Snapshot {
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-19T12:00:00Z", "me": { "email": ME, "phid": "PHID-ME" },
        "bugs": [{
            "id": 2093725, "summary": "Headless read blamed on off-site load", "status": "ASSIGNED", "resolution": "",
            "assigned_to": ME, "severity": "S3", "priority": "P2", "keywords": [], "whiteboard": "", "see_also": [],
            "flags": [], "last_change_time": "2026-11-18T12:00:00Z", "product": "Firefox", "component": "AI",
            "via": ["assigned"],
        }],
        "issues": {}, "revisions": revisions, "toReview": [], "bugRevisions": {}, "lastComments": {},
        "errors": [{ "source": "jira", "message": "not set up", "unset": true }],
    }))
    .unwrap();
    build_snapshot(raw, &CoreConfig::default(), "2026-11-19T12:00:00Z".parse::<Timestamp>().unwrap())
}

fn messages(s: &Snapshot) -> Vec<&str> {
    s.entries[0].drift.iter().map(|d| d.message.as_str()).collect()
}

#[test]
fn two_open_revisions_with_one_title_are_flagged_without_picking_the_stale_one() {
    let s = snapshot(&[
        rev(343322, TITLE, "needs-review", "PHID-ME"),
        rev(343316, TITLE, "needs-review", "PHID-ME"),
    ]);
    assert_eq!(messages(&s), ["D343316 and D343322 have the same title; one may be stale"]);
    assert_eq!(s.section(Section::Drift, false).count(), 1, "listed in Drift even with Jira off");
    assert!(!s.off(Section::Drift));
    assert_eq!(s.not_checked(Section::Drift), None);
}

#[test]
fn three_copies_are_one_warning() {
    let s = snapshot(&[
        rev(1, TITLE, "needs-review", "PHID-ME"),
        rev(2, &format!("  {TITLE} "), "needs-revision", "PHID-ME"),
        rev(3, TITLE, "accepted", "PHID-ME"),
    ]);
    assert_eq!(messages(&s), ["D1, D2 and D3 have the same title; one may be stale"]);
}

#[test]
fn closed_foreign_and_differently_titled_revisions_are_left_alone() {
    for other in [
        rev(343316, TITLE, "abandoned", "PHID-ME"),
        rev(343316, TITLE, "published", "PHID-ME"),
        rev(343316, TITLE, "needs-review", "PHID-SOMEONE"),
        rev(343316, "Bug 2093725 - part 2", "needs-review", "PHID-ME"),
    ] {
        let s = snapshot(&[rev(343322, TITLE, "needs-review", "PHID-ME"), other]);
        assert_eq!(messages(&s), Vec::<&str>::new());
        assert!(s.off(Section::Drift), "Jira is off and nothing drifted");
    }
}
