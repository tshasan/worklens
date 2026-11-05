use jiff::Timestamp;
use serde_json::{Value, json};
use worklens_core::{CoreConfig, Raw, Section, Snapshot, build_snapshot, present};

const ME: &str = "me@mozilla.com";
const NOW: &str = "2026-11-03T12:00:00Z";

fn snapshot(bugs: &Value, issues: &Value, revisions: &Value) -> Snapshot {
    with(bugs, issues, revisions, &json!({}))
}

fn with(bugs: &Value, issues: &Value, revisions: &Value, extra: &Value) -> Snapshot {
    let mut raw = json!({
        "version": 1, "fetchedAt": NOW,
        "me": { "email": ME, "jiraEmail": ME, "phid": "PHID-ME" },
        "bugs": bugs, "issues": issues, "revisions": revisions, "toReview": [],
        "bugRevisions": {}, "lastComments": {}, "errors": [],
    });
    for (k, v) in extra.as_object().unwrap() {
        raw[k] = v.clone();
    }
    let raw: Raw = serde_json::from_value(raw).unwrap();
    build_snapshot(raw, &CoreConfig::default(), NOW.parse::<Timestamp>().unwrap())
}

fn bug(changed: &str) -> Value {
    json!({
        "id": 1, "summary": "bug 1", "status": "ASSIGNED", "resolution": "", "assigned_to": ME,
        "severity": "S3", "priority": "P3", "keywords": [], "whiteboard": "", "see_also": [], "flags": [],
        "last_change_time": changed, "product": "Core", "component": "ML", "via": ["assigned"],
    })
}

fn revision(id: u32, status: &str) -> Value {
    json!({
        "id": id, "title": format!("D{id}"), "status": status, "author": "PHID-ME", "bug": 1,
        "modified": "2026-11-01T12:00:00Z", "reviewers": [],
    })
}

fn push(commit: &str, failed: u32) -> Value {
    json!({
        "revision": commit, "url": format!("https://treeherder.mozilla.org/jobs?repo=try&revision={commit}"), "builds": "pass",
        "lint": "pass", "tests": if failed > 0 { "fail" } else { "pass" }, "failed": failed, "running": 0,
        "pushedAt": "2026-11-02T12:00:00Z",
    })
}

fn entry<'a>(s: &'a Snapshot, id: &str) -> &'a worklens_core::Entry {
    s.entries.iter().find(|e| s.id(e).to_string() == id).expect("listed")
}

#[test]
fn a_ticket_with_no_bug_is_as_old_as_its_last_update() {
    let s = snapshot(
        &json!([]),
        &json!({ "AI-7": {
            "key": "AI-7", "summary": "a ticket", "status": "In Progress", "statusCategory": "indeterminate",
            "assignee": ME, "updated": "2026-10-30T12:00:00Z", "points": 2, "sprint": null,
        } }),
        &json!([]),
    );
    let e = entry(&s, "AI-7");
    assert_eq!(present::age_since(&s, e), Some("2026-10-30T12:00:00Z".parse().unwrap()));
}

#[test]
fn try_is_shown_for_my_live_revision_even_once_accepted() {
    let s = with(
        &json!([bug("2026-11-01T12:00:00Z")]),
        &json!({}),
        &json!([revision(10, "published"), revision(11, "accepted")]),
        &json!({ "tryPushes": { "10": push("a", 0), "11": push("b", 2) } }),
    );
    let e = entry(&s, "1");
    let r = present::try_revision(&s, e).expect("D11 has a try push");
    assert_eq!(s.rev(r).id.to_string(), "D11");
    let live: Vec<String> = present::live_revisions(&s, e).map(|r| s.rev(r).id.to_string()).collect();
    assert_eq!(live, ["D11"]);
}

#[test]
fn an_unassigned_bug_is_owned_by_its_tickets_assignee() {
    let mut b = bug("2026-11-01T12:00:00Z");
    b["assigned_to"] = json!("nobody@mozilla.org");
    b["see_also"] = json!(["https://mozilla-hub.atlassian.net/browse/AI-1"]);
    b["flags"] = json!([{ "name": "needinfo", "status": "?", "requestee": ME, "setter": "a@mozilla.com",
                          "creation_date": "2026-10-30T12:00:00Z" }]);
    let s = snapshot(
        &json!([b]),
        &json!({ "AI-1": {
            "key": "AI-1", "summary": "AI-1", "status": "In Progress", "statusCategory": "indeterminate",
            "assignee": "them@mozilla.com", "updated": "2026-11-01T12:00:00Z", "points": 3, "sprint": null,
        } }),
        &json!([]),
    );
    assert_eq!(present::owner(&s, entry(&s, "1")), Some("them@mozilla.com"));
}

#[test]
fn a_reviewer_who_resigned_is_not_a_reviewer() {
    let mut r = revision(10, "needs-review");
    r["reviewers"] = json!([
        { "phid": "PHID-A", "status": "accepted" },
        { "phid": "PHID-B", "status": "resigned" },
        { "phid": "PHID-C", "status": "added" },
    ]);
    let s = snapshot(&json!([bug("2026-11-01T12:00:00Z")]), &json!({}), &json!([r]));
    let rev = s.rev(entry(&s, "1").revisions[0]);
    let phids: Vec<&str> = present::reviewers(rev).map(|v| v.phid.as_str()).collect();
    assert_eq!(phids, ["PHID-A", "PHID-C"]);
}

#[test]
fn a_snooze_date_reads_like_javascripts_date_string() {
    let t: Timestamp = "2026-11-10T09:00:00Z".parse().unwrap();
    assert_eq!(present::date(t, &jiff::tz::TimeZone::UTC), "Tue Nov 10 2026");
}

#[test]
fn a_drift_fix_reads_as_one_change_to_make() {
    use worklens_core::drift::{Fix, Side};
    let fix = Fix {
        side: Side::Jira,
        field: "Status",
        value: "In Progress".to_owned(),
    };
    assert_eq!(present::FixText(&fix).to_string(), "set Jira Status to In Progress");
    let fix = Fix {
        side: Side::Bugzilla,
        field: "assignee",
        value: "me@mozilla.com".to_owned(),
    };
    assert_eq!(present::FixText(&fix).to_string(), "set Bugzilla assignee to me@mozilla.com");
}

#[test]
fn links_name_every_place_the_entry_lives() {
    use present::LinkKind;
    let s = with(
        &json!([bug("2026-11-01T12:00:00Z")]),
        &json!({}),
        &json!([revision(10, "needs-review"), revision(11, "accepted")]),
        &json!({ "tryPushes": { "10": push("a", 0), "11": push("b", 2) } }),
    );
    let links = present::links(&s, entry(&s, "1"));
    let got: Vec<(LinkKind, &str)> = links.iter().map(|l| (l.kind, l.url.as_str())).collect();
    assert_eq!(
        got,
        [
            (LinkKind::Bug, "https://bugzilla.mozilla.org/show_bug.cgi?id=1"),
            (LinkKind::Revision, "https://phabricator.services.mozilla.com/D10"),
            (LinkKind::Revision, "https://phabricator.services.mozilla.com/D11"),
            (LinkKind::Try, "https://treeherder.mozilla.org/jobs?repo=try&revision=a"),
            (LinkKind::Lando, "https://lando.moz.tools/D11/"),
            (LinkKind::Try, "https://treeherder.mozilla.org/jobs?repo=try&revision=b"),
        ]
    );
}

fn words(segments: &[present::Segment]) -> Vec<&str> {
    segments.iter().map(|s| s.text.as_str()).collect()
}

#[test]
fn reviewers_group_by_what_they_did_and_name_groups_and_older_diffs() {
    use present::Signal;
    let mut r = revision(10, "needs-review");
    r["reviewers"] = json!([
        { "phid": "PHID-USER-a", "status": "accepted" },
        { "phid": "PHID-USER-b", "status": "rejected" },
        { "phid": "PHID-USER-c", "status": "accepted-older" },
        { "phid": "PHID-USER-d", "status": "blocking" },
        { "phid": "PHID-PROJ-g", "status": "added" },
        { "phid": "PHID-ME", "status": "added" },
        { "phid": "PHID-USER-e", "status": "resigned" },
    ]);
    let s = with(
        &json!([bug("2026-11-01T12:00:00Z")]),
        &json!({}),
        &json!([r]),
        &json!({ "users": { "PHID-USER-a": "alice", "PHID-USER-b": "bob", "PHID-USER-c": "carol",
                            "PHID-USER-d": "dave", "PHID-PROJ-g": "fluent-reviewers", "PHID-USER-e": "erin" } }),
    );
    let segments = present::review_segments(&s, s.rev(entry(&s, "1").revisions[0]));
    assert_eq!(
        words(&segments),
        [
            "alice accepted",
            "bob requested changes",
            "carol accepted an older diff",
            "waiting on dave (blocking), fluent-reviewers (group), you",
            "erin resigned",
        ]
    );
    let signals: Vec<Signal> = segments.iter().map(|s| s.signal).collect();
    assert_eq!(
        signals,
        [Signal::Good, Signal::Bad, Signal::Pending, Signal::Known, Signal::Unknown]
    );
}

#[test]
fn try_reads_kind_by_kind_and_running_is_not_passed() {
    let mut running = push("a", 0);
    running["tests"] = json!("running");
    running["lint"] = json!("none");
    running["running"] = json!(4);
    let s = with(
        &json!([bug("2026-11-01T12:00:00Z")]),
        &json!({}),
        &json!([revision(10, "needs-review"), revision(11, "needs-review")]),
        &json!({ "tryPushes": { "10": running, "11": push("b", 1) } }),
    );
    let e = entry(&s, "1");
    let state = |n: usize| s.try_state(e.revisions[n]);
    assert_eq!(
        words(&present::try_segments(state(0))),
        ["builds passed", "no lint jobs", "tests running", "4 jobs left"]
    );
    assert_eq!(
        words(&present::try_segments(state(1))),
        ["builds passed", "lint passed", "tests failed", "1 failing job"]
    );
}

#[test]
fn try_says_nothing_when_unknown_and_no_push_when_there_is_none() {
    let s = with(
        &json!([bug("2026-11-01T12:00:00Z")]),
        &json!({}),
        &json!([revision(10, "needs-review")]),
        &json!({ "tryPushes": {} }),
    );
    let rev = entry(&s, "1").revisions[0];
    assert_eq!(words(&present::try_segments(s.try_state(rev))), ["no try push"]);
    let s = snapshot(
        &json!([bug("2026-11-01T12:00:00Z")]),
        &json!({}),
        &json!([revision(10, "needs-review")]),
    );
    let rev = entry(&s, "1").revisions[0];
    assert!(present::try_segments(s.try_state(rev)).is_empty(), "Treeherder not read");
}

#[test]
fn a_bug_whose_sprint_ticket_is_someone_elses_still_shows() {
    let mut b = bug("2026-11-01T12:00:00Z");
    b["assigned_to"] = json!("nobody@mozilla.org");
    b["via"] = json!(["revision"]);
    b["see_also"] = json!(["https://mozilla-hub.atlassian.net/browse/AI-1"]);
    let s = snapshot(
        &json!([b]),
        &json!({ "AI-1": {
            "key": "AI-1", "summary": "bug 1", "status": "In Progress", "statusCategory": "indeterminate",
            "assignee": "them@mozilla.com", "updated": "2026-11-01T12:00:00Z", "points": 3,
            "sprint": { "name": "Sprint 9", "state": "active" },
        } }),
        &json!([]),
    );
    let shown: Vec<Section> = Section::ALL
        .into_iter()
        .filter(|sec| s.section(*sec, false).any(|i| s.id(s.entry(i)).to_string() == "1"))
        .collect();
    assert!(!shown.is_empty(), "bug 1 is in no section");
    assert!(!shown.contains(&Section::Sprint), "not your sprint ticket: {shown:?}");
}

#[test]
fn sections_say_why_they_were_not_checked() {
    let unset = json!({ "errors": [
        { "source": "jira", "message": "not set up", "unset": true },
        { "source": "phabricator", "message": "503" },
    ] });
    let s = with(&json!([]), &json!({}), &json!([]), &unset);
    assert!(s.empty_text(Section::Drift).starts_with("Off: Jira isn't set up"));
    assert!(s.empty_text(Section::Team).starts_with("Off: Jira isn't set up"));
    assert!(s.empty_text(Section::Land).starts_with("Not checked: Phabricator"));
    assert_eq!(s.empty_text(Section::Uplift), Section::Uplift.empty());
    assert_eq!(s.raw.failures().count(), 1);

    let s = snapshot(&json!([]), &json!({}), &json!([]));
    assert_eq!(s.empty_text(Section::Drift), Section::Drift.empty());
    assert!(s.empty_text(Section::Team).contains("whiteboard tag"));
}
