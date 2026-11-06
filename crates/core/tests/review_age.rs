use jiff::Timestamp;
use serde_json::{Value, json};
use worklens_core::{CoreConfig, Raw, Snapshot, build_snapshot};

const NOW: &str = "2026-11-03T12:00:00Z";

fn snapshot(revisions: &Value, to_review: &Value) -> Snapshot {
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": NOW,
        "me": { "email": "me@mozilla.com", "jiraEmail": "me@mozilla.com", "phid": "PHID-ME" },
        "bugs": [], "issues": {}, "revisions": revisions, "toReview": to_review,
        "bugRevisions": {}, "lastComments": {}, "errors": [],
        "users": { "PHID-THEM": "them" },
    }))
    .unwrap();
    build_snapshot(raw, &CoreConfig::default(), NOW.parse::<Timestamp>().unwrap())
}

fn revision(id: u32, author: &str, reviewer: &str, diff_created: Option<&str>) -> Value {
    let mut r = json!({
        "id": id, "title": format!("D{id}"), "status": "needs-review", "author": author,
        "modified": "2026-11-03T11:00:00Z",
        "reviewers": [{ "phid": reviewer, "status": "added" }],
    });
    if let Some(d) = diff_created {
        r["diffCreated"] = json!(d);
    }
    r
}

fn listed(s: &Snapshot) -> Vec<(String, String, bool)> {
    s.entries
        .iter()
        .filter(|e| e.placement.is_some())
        .map(|e| (s.id(e).to_string(), s.reason(e).to_string(), e.placement.unwrap().stale))
        .collect()
}

#[test]
fn a_comment_does_not_restart_the_review_clock() {
    let s = snapshot(
        &json!([
            revision(1, "PHID-ME", "PHID-THEM", Some("2026-10-26T12:00:00Z")),
            revision(2, "PHID-THEM", "PHID-ME", Some("2026-10-30T12:00:00Z")),
        ]),
        &json!([2]),
    );
    let rows = listed(&s);
    let mine = rows.iter().find(|r| r.0 == "D1").expect("my revision is listed");
    assert!(mine.2, "my revision waited on a week of business days: {mine:?}");
    let theirs = rows.iter().find(|r| r.0 == "D2").expect("the review asked of me is listed");
    assert!(
        theirs.1.contains("latest diff posted 4 days ago"),
        "age from the diff, not the comment: {theirs:?}"
    );
}

#[test]
fn without_a_diff_date_the_age_falls_back_to_last_activity() {
    let s = snapshot(&json!([revision(1, "PHID-ME", "PHID-THEM", None)]), &json!([]));
    let rows = listed(&s);
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].2, "an hour of activity isn't stale: {rows:?}");
    assert!(rows[0].1.ends_with("revision last modified today"), "{rows:?}");
}

#[test]
fn accepted_age_names_modification_without_claiming_when_acceptance_happened() {
    let mut r = revision(1, "PHID-ME", "PHID-THEM", Some("2026-10-01T12:00:00Z"));
    r["status"] = json!("accepted");
    r["modified"] = json!("2026-10-26T12:00:00Z");
    r["reviewers"][0]["status"] = json!("accepted");
    let s = snapshot(&json!([r]), &json!([]));
    let rows = listed(&s);
    assert!(rows[0].2, "the existing seven-day stale mechanism remains");
    assert!(rows[0].1.starts_with("D1 accepted by them;"), "{rows:?}");
    assert!(rows[0].1.contains("revision last modified 8 days ago"), "{rows:?}");
    assert!(!rows[0].1.contains("not landed for"), "{rows:?}");
}

#[test]
fn changed_revision_age_keeps_the_rejecting_reviewer_and_names_modification_time() {
    let mut r = revision(1, "PHID-ME", "PHID-THEM", None);
    r["status"] = json!("needs-revision");
    r["reviewers"][0]["status"] = json!("rejected");
    let s = snapshot(&json!([r]), &json!([]));
    let rows = listed(&s);
    assert_eq!(rows[0].1, "changes requested on D1 by them; revision last modified today");
}

#[test]
fn a_complete_day_is_not_reported_as_today() {
    let mut r = revision(1, "PHID-ME", "PHID-THEM", None);
    r["modified"] = json!("2026-11-02T12:00:00Z");
    let s = snapshot(&json!([r]), &json!([]));
    assert_eq!(listed(&s)[0].1, "D1 waiting on them; revision last modified 1 day ago");
}
