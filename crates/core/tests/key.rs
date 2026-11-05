use jiff::Timestamp;
use serde_json::{Value, json};
use worklens_core::key::{EntryKey, Whereabouts};
use worklens_core::marks::{Hidden, Marks, apply, mark_done};
use worklens_core::{CoreConfig, Raw, Section, Snapshot, build_snapshot};

const ME: &str = "me@mozilla.com";

fn now() -> Timestamp {
    "2026-11-02T12:00:00Z".parse().unwrap()
}

fn bug(id: u32) -> Value {
    json!({
        "id": id, "summary": format!("bug {id}"), "status": "ASSIGNED", "resolution": "",
        "assigned_to": ME, "severity": "S3", "priority": "P3", "keywords": [], "whiteboard": "",
        "see_also": [], "last_change_time": "2026-11-01T12:00:00Z",
        "product": "Core", "component": "ML", "via": ["assigned"],
        "flags": [{ "name": "needinfo", "status": "?", "requestee": ME, "setter": "a@mozilla.com",
                    "creation_date": "2026-10-30T12:00:00Z" }],
    })
}

fn snapshot(ids: &[u32]) -> Snapshot {
    let bugs: Vec<Value> = ids.iter().map(|id| bug(*id)).collect();
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-02T12:00:00Z",
        "me": { "email": ME, "jiraEmail": ME, "phid": "PHID-ME" },
        "bugs": bugs, "issues": {}, "revisions": [], "toReview": [], "bugRevisions": {},
        "lastComments": {}, "errors": [],
    }))
    .unwrap();
    build_snapshot(raw, &CoreConfig::default(), now())
}

#[test]
fn a_key_finds_the_same_entry_in_a_later_snapshot() {
    let before = snapshot(&[100, 200]);
    let key = EntryKey::of(&before, &before.entries[1]);
    assert_eq!(key.to_string(), "200");
    assert!(!key.is(&before, &before.entries[0]));
    let after = snapshot(&[200, 300, 100]);
    let found = key.find(&after).expect("still there");
    assert_eq!(after.id(after.entry(found)).to_string(), "200");
    assert!(key.find(&snapshot(&[100])).is_none());
}

#[test]
fn whereabouts_says_section_or_local_mark() {
    let mut s = snapshot(&[100]);
    assert!(matches!(s.entries[0].whereabouts(), Whereabouts::In(Section::Waiting)));
    let mut marks = Marks::default();
    mark_done(&mut marks, &s, &s.entries[0], now());
    apply(&mut s, &marks, now());
    assert!(matches!(s.entries[0].whereabouts(), Whereabouts::Hidden(Hidden::Done)));
}

fn reviewed(asked: bool) -> Snapshot {
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-02T12:00:00Z",
        "me": { "email": ME, "jiraEmail": ME, "phid": "PHID-ME" },
        "bugs": [bug(100)], "issues": {},
        "revisions": [{ "id": 9, "title": "D9", "status": "needs-review", "author": "PHID-THEM", "bug": 100,
                        "modified": "2026-11-01T12:00:00Z",
                        "reviewers": [{ "phid": "PHID-ME", "status": "added" }] }],
        "toReview": if asked { json!([9]) } else { json!([]) },
        "bugRevisions": { "100": [9] }, "lastComments": {}, "errors": [],
    }))
    .unwrap();
    build_snapshot(raw, &CoreConfig::default(), now())
}

#[test]
fn a_finished_review_keeps_the_bugs_key() {
    let before = reviewed(true);
    let e = before
        .entries
        .iter()
        .find(|e| e.review.is_some())
        .expect("the bug's entry carries the review");
    let key = EntryKey::of(&before, e);
    assert_eq!(key.to_string(), "100");
    let after = reviewed(false);
    let found = key.find(&after).expect("the bug is still listed after the review is done");
    assert_eq!(after.id(after.entry(found)).to_string(), "100");
}

fn mine_and_team() -> Snapshot {
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-02T12:00:00Z",
        "me": { "email": ME, "jiraEmail": ME, "phid": "PHID-ME" },
        "bugs": [{
            "id": 1, "summary": "bug 1", "status": "RESOLVED", "resolution": "FIXED", "assigned_to": ME,
            "severity": "S3", "priority": "P3", "keywords": [], "whiteboard": "[aiplatform]",
            "see_also": ["https://mozilla-hub.atlassian.net/browse/AI-1"], "flags": [],
            "last_change_time": "2026-11-01T12:00:00Z", "product": "Core", "component": "ML",
            "via": ["assigned", "team"],
        }],
        "issues": { "AI-1": {
            "key": "AI-1", "summary": "AI-1", "status": "In Progress", "statusCategory": "indeterminate",
            "assignee": ME, "updated": "2026-11-01T12:00:00Z", "points": 3, "sprint": null,
        } },
        "revisions": [], "toReview": [], "bugRevisions": {}, "lastComments": {}, "errors": [],
    }))
    .unwrap();
    let cfg = CoreConfig {
        whiteboard: Some("[aiplatform]".to_owned()),
        ..CoreConfig::default()
    };
    build_snapshot(raw, &cfg, now())
}

#[test]
fn my_entry_and_the_team_entry_of_one_bug_have_different_keys() {
    let s = mine_and_team();
    let both: Vec<usize> = (0..s.entries.len()).filter(|i| s.id(&s.entries[*i]).to_string() == "1").collect();
    assert_eq!(both.len(), 2, "bug 1 is listed for me and in Team drift");
    for i in both {
        let key = EntryKey::of(&s, &s.entries[i]);
        assert_eq!(key.find(&s).map(|f| f.get()), Some(i), "each key finds its own entry");
        assert_eq!(key.to_string(), "1");
    }
}
