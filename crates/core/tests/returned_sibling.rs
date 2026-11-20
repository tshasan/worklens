use jiff::Timestamp;
use serde_json::{Value, json};
use worklens_core::rules::Reason;
use worklens_core::section::Section;
use worklens_core::{CoreConfig, Raw, Snapshot, build_snapshot};

const ME: &str = "me@mozilla.com";

fn rev(id: u32, status: &str, author: &str) -> Value {
    json!({
        "id": id, "title": format!("Bug 2092358 - part {id}"), "status": status, "author": author, "bug": 2092358,
        "modified": "2026-11-18T12:00:00Z", "reviewers": [{ "phid": "PHID-R", "status": "added" }],
    })
}

fn snapshot(revisions: &[Value]) -> Snapshot {
    build(raw(revisions))
}

fn build(v: Value) -> Snapshot {
    let raw: Raw = serde_json::from_value(v).unwrap();
    build_snapshot(raw, &CoreConfig::default(), "2026-11-19T12:00:00Z".parse::<Timestamp>().unwrap())
}

fn raw(revisions: &[Value]) -> Value {
    json!({
        "version": 1, "fetchedAt": "2026-11-19T12:00:00Z", "me": { "email": ME, "phid": "PHID-ME" },
        "bugs": [{
            "id": 2092358, "summary": "Continue Smart Window chat after a captcha", "status": "ASSIGNED", "resolution": "",
            "assigned_to": ME, "severity": "S3", "priority": "P2", "keywords": [], "whiteboard": "", "see_also": [],
            "flags": [], "last_change_time": "2026-11-18T12:00:00Z", "product": "Firefox", "component": "AI",
            "via": ["assigned"],
        }],
        "issues": {}, "revisions": revisions, "toReview": [], "bugRevisions": {}, "lastComments": {}, "errors": [],
        "tryPushes": { "342404": {
            "revision": "abc", "pushedAt": "2026-11-18T13:00:00Z", "url": "https://treeherder.mozilla.org/x",
            "builds": "fail", "lint": "pass", "tests": "pass", "failed": 2, "running": 0,
        }},
    })
}

fn ids(s: &Snapshot, section: Section) -> Vec<String> {
    s.section(section, false).map(|i| s.id(s.entry(i)).to_string()).collect()
}

#[test]
fn changes_requested_on_one_revision_shows_even_when_an_accepted_sibling_places_the_bug() {
    let s = snapshot(&[
        rev(342404, "accepted", "PHID-ME"),
        rev(342405, "needs-revision", "PHID-ME"),
        rev(342406, "needs-review", "PHID-ME"),
    ]);
    let e = &s.entries[0];
    assert!(matches!(e.placement.unwrap().reason, Reason::LandingBlocked(_)));
    assert_eq!(ids(&s, Section::Waiting), ["2092358"]);
    assert_eq!(ids(&s, Section::Returned), ["2092358"], "D342405 is my turn");
    assert_eq!(
        s.reason(e).to_string(),
        "D342404 accepted; try has failing jobs; inspect before landing; also D342405 changes requested"
    );
}

#[test]
fn every_revision_that_needs_me_is_named_once() {
    let s = snapshot(&[
        rev(342404, "needs-revision", "PHID-ME"),
        rev(342405, "needs-revision", "PHID-ME"),
        rev(342407, "accepted", "PHID-ME"),
        rev(342408, "needs-revision", "PHID-SOMEONE"),
    ]);
    let e = &s.entries[0];
    assert!(matches!(e.placement.unwrap().reason, Reason::Accepted(_)));
    let text = s.reason(e).to_string();
    assert!(
        text.ends_with("; also D342404 changes requested, D342405 changes requested"),
        "{text}"
    );
    assert!(!text.contains("D342408"), "not mine: {text}");
    assert_eq!(ids(&s, Section::Returned), ["2092358"]);
}

#[test]
fn a_bug_already_placed_in_returned_is_listed_there_once() {
    let s = snapshot(&[rev(342405, "needs-revision", "PHID-ME"), rev(342406, "needs-review", "PHID-ME")]);
    assert_eq!(ids(&s, Section::Returned), ["2092358"]);
    assert_eq!(
        s.reason(&s.entries[0]).to_string(),
        "changes requested on D342405 by a reviewer; revision last modified 1 day ago"
    );
}

#[test]
fn an_accepted_revision_behind_a_needinfo_is_named_as_accepted() {
    let mut v = raw(&[rev(342404, "accepted", "PHID-ME")]);
    v["bugs"][0]["flags"] = json!([{
        "name": "needinfo", "status": "?", "requestee": ME, "setter": "aryx@mozilla.com",
        "creation_date": "2026-11-17T12:00:00Z",
    }]);
    let s = build(v);
    assert_eq!(
        s.reason(&s.entries[0]).to_string(),
        "needinfo from aryx@mozilla.com, 2 days; also D342404 accepted"
    );
}

#[test]
fn someone_elses_needs_revision_is_not_my_turn() {
    let s = snapshot(&[rev(342404, "accepted", "PHID-ME"), rev(342405, "needs-revision", "PHID-SOMEONE")]);
    assert!(ids(&s, Section::Returned).is_empty());
}
