use jiff::Timestamp;
use serde_json::{Value, json};
use worklens_core::rules::{NextStep, Reason};
use worklens_core::section::Section;
use worklens_core::{CoreConfig, Raw, Snapshot, build_snapshot};

const ME: &str = "me@mozilla.com";

fn bug(id: u32, status: &str, resolution: &str, severity: &str, needinfo_from: Option<&str>) -> Value {
    let flags: Vec<Value> = needinfo_from
        .map(|setter| {
            json!({ "name": "needinfo", "status": "?", "requestee": ME, "setter": setter, "creation_date": "2026-10-29T12:00:00Z" })
        })
        .into_iter()
        .collect();
    json!({
        "id": id, "summary": format!("bug {id}"), "status": status, "resolution": resolution, "assigned_to": "other@mozilla.com",
        "severity": severity, "priority": "P3", "keywords": [], "whiteboard": "", "see_also": [], "flags": flags,
        "last_change_time": "2026-11-19T10:00:00Z", "cf_last_resolved": "2026-11-19T10:00:00Z",
        "product": "Core", "component": "ML", "via": ["needinfo"],
    })
}

fn snapshot(bugs: &[Value]) -> Snapshot {
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-19T12:00:00Z", "me": { "email": ME, "phid": "PHID-ME" },
        "bugs": bugs, "issues": {}, "toReview": [500], "bugRevisions": {}, "lastComments": {}, "errors": [],
        "revisions": [{
            "id": 500, "title": "Bug 3 - review me", "status": "needs-review", "author": "PHID-OTHER", "bug": 3,
            "modified": "2026-11-19T09:00:00Z", "reviewers": [{ "phid": "PHID-ME", "status": "added" }],
        }],
    }))
    .unwrap();
    build_snapshot(raw, &CoreConfig::default(), "2026-11-19T12:00:00Z".parse::<Timestamp>().unwrap())
}

fn waiting(s: &Snapshot) -> Vec<(u32, Reason)> {
    s.section(Section::Waiting, false)
        .map(|i| s.entry(i))
        .map(|e| (s.bug(e.bug.unwrap()).id.0, e.placement.unwrap().reason))
        .collect()
}

#[test]
fn a_needinfo_on_a_resolved_bug_sinks_below_reviews_and_open_needinfos() {
    let s = snapshot(&[
        bug(1, "RESOLVED", "FIXED", "S1", Some("release-mgmt-account-bot@mozilla.tld")),
        bug(2, "NEW", "", "S3", Some("aryx@mozilla.com")),
        bug(3, "NEW", "", "S3", None),
    ]);
    let order = waiting(&s);
    assert_eq!(
        order.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        [2, 3, 1],
        "an S1 bug that is already fixed still ranks last: {order:?}"
    );
    assert_eq!(order[0].1, Reason::Needinfo);
    assert!(matches!(order[1].1, Reason::ReviewRequested(_)));
    assert_eq!(order[2].1, Reason::ClosedNeedinfo);
}

#[test]
fn it_says_the_bug_is_resolved_and_the_flag_is_still_set() {
    let s = snapshot(&[bug(1, "RESOLVED", "FIXED", "S3", Some("aryx@mozilla.com"))]);
    let i = s.section(Section::Waiting, false).next().unwrap();
    let e = s.entry(i);
    assert_eq!(
        s.reason(e).to_string(),
        "bug RESOLVED FIXED on Nov 19; needinfo from aryx@mozilla.com still set, 21 days"
    );
    assert_eq!(e.placement.unwrap().reason.tag(), "nix");
    assert_eq!(s.next_step(e), Some(NextStep::ClearNeedinfo));
}

#[test]
fn a_verified_bug_counts_as_closed_too() {
    let s = snapshot(&[bug(1, "VERIFIED", "FIXED", "S3", Some("aryx@mozilla.com"))]);
    assert_eq!(waiting(&s), [(1, Reason::ClosedNeedinfo)]);
}
