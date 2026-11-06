use serde_json::{Value, json};
use worklens_core::{CoreConfig, Raw, Section, Snapshot, build_snapshot};

const NOW: &str = "2026-11-20T12:00:00Z";

fn ago(days: i64) -> String {
    let now: jiff::Timestamp = NOW.parse().unwrap();
    now.checked_sub(jiff::SignedDuration::from_hours(24 * days)).unwrap().to_string()
}

fn ticket(key: &str, created: &str, priority: &str, sprint: &Value) -> Value {
    json!({
        "key": key, "summary": format!("ticket {key}"), "status": "To Do", "statusCategory": "new",
        "assignee": "me@mozilla.com", "priority": priority, "updated": ago(1), "created": created, "points": null, "sprint": sprint,
    })
}

fn bug(id: u32, created: Option<&str>, priority: &str) -> Value {
    json!({
        "id": id, "summary": format!("bug {id}"), "status": "NEW", "resolution": "", "assigned_to": "me@mozilla.com",
        "severity": "S3", "priority": priority, "keywords": [], "whiteboard": "", "see_also": [],
        "last_change_time": ago(2), "creation_time": created, "product": "Core", "component": "ML", "via": ["assigned"],
    })
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

fn backlog(s: &Snapshot) -> Vec<(String, String)> {
    s.section(Section::Backlog, true)
        .map(|i| s.entry(i))
        .map(|e| (s.id(e).to_string(), s.reason(e).to_string()))
        .collect()
}

fn row(id: &str, reason: &str) -> (String, String) {
    (id.to_owned(), reason.to_owned())
}

#[test]
fn unstarted_tickets_and_bugs_sort_by_priority_then_age() {
    let s = snapshot(
        &[
            bug(10, Some(ago(41).as_str()), "P3"),
            bug(11, None, "P1"),
            bug(12, Some(ago(90).as_str()), "P3"),
        ],
        &[ticket("PROJ-1", &ago(41), "High", &Value::Null)],
        &[],
    );
    assert_eq!(
        backlog(&s),
        [
            row("11", "Assigned to you, not started"),
            row("PROJ-1", "Filed 41 days ago, not started"),
            row("12", "Filed 90 days ago, not started"),
            row("10", "Filed 41 days ago, not started"),
        ]
    );
    let e = s.entry(s.section(Section::Backlog, true).nth(1).unwrap());
    assert_eq!(
        s.next_step(e).and_then(|n| worklens_core::guidance::next_url(&s.raw, n)).as_deref(),
        Some("https://mozilla-hub.atlassian.net/browse/PROJ-1")
    );
}

#[test]
fn a_ticket_that_joins_a_sprint_leaves_for_the_sprint() {
    let s = snapshot(
        &[],
        &[ticket("PROJ-2", &ago(5), "P2", &json!({"name": "Sprint 9", "state": "active"}))],
        &[],
    );
    assert!(backlog(&s).is_empty());
    assert_eq!(s.section(Section::Sprint, true).count(), 1);
    let s = snapshot(
        &[],
        &[ticket("PROJ-3", &ago(5), "P2", &json!({"name": "Sprint 8", "state": "closed"}))],
        &[],
    );
    assert_eq!(
        backlog(&s),
        [row("PROJ-3", "Filed 5 days ago, not started")],
        "a closed sprint isn't open"
    );
}

#[test]
fn started_or_asked_bugs_stay_out() {
    let mut asked = bug(20, Some(ago(10).as_str()), "P2");
    asked["flags"] =
        json!([{ "name": "needinfo", "status": "?", "requestee": "me@mozilla.com", "setter": "a@mozilla.com", "creation_date": ago(1) }]);
    let mut linked = bug(21, Some(ago(10).as_str()), "P2");
    linked["see_also"] = json!(["https://mozilla-hub.atlassian.net/browse/PROJ-9"]);
    let mut progress = ticket("PROJ-9", &ago(10), "P2", &Value::Null);
    progress["statusCategory"] = json!("indeterminate");
    progress["status"] = json!("In Progress");
    let patched = bug(22, Some(ago(10).as_str()), "P2");
    let rev = json!({ "id": 5, "title": "Bug 22", "status": "draft", "author": "PHID-ME", "bug": 22, "modified": ago(1), "reviewers": [] });
    let s = snapshot(&[asked, linked, patched], &[progress], &[rev]);
    assert!(backlog(&s).is_empty(), "{:?}", backlog(&s));
    assert_eq!(s.section(Section::Waiting, true).count(), 1, "the needinfo still wins");
}

#[test]
fn a_bugzilla_only_first_run_has_a_backlog_of_bugs() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/bugzilla_only.json");
    let raw: Raw = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let s = build_snapshot(raw, &CoreConfig::default(), NOW.parse().unwrap());
    let ids: Vec<String> = s.section(Section::Backlog, true).map(|i| s.id(s.entry(i)).to_string()).collect();
    assert!(!ids.is_empty() && ids.iter().all(|id| id.parse::<u32>().is_ok()), "{ids:?}");
}

#[test]
fn a_linked_ticket_in_another_persons_sprint_is_not_my_sprint_work() {
    let mut linked = bug(91, Some(&ago(10)), "P3");
    linked["see_also"] = json!(["https://mozilla-hub.atlassian.net/browse/PROJ-19"]);
    let mut issue = ticket("PROJ-19", &ago(10), "P3", &json!({"name":"Sprint 9","state":"active"}));
    issue["assignee"] = json!("other@mozilla.com");
    issue["status"] = json!("In Progress");
    issue["statusCategory"] = json!("indeterminate");
    let s = snapshot(&[linked], &[issue], &[]);
    assert_eq!(s.section(Section::Sprint, true).count(), 0);
    assert_eq!(s.entries[0].placement.unwrap().section, Section::Other);
}

#[test]
fn every_known_priority_precedes_an_older_unknown_priority() {
    let mut bugs: Vec<Value> = [(5, "P5"), (4, "P4"), (3, "P3"), (2, "P2"), (1, "P1")]
        .into_iter()
        .map(|(id, priority)| bug(id, Some(&ago(10)), priority))
        .collect();
    bugs.push(bug(99, Some(&ago(100)), "unset"));
    let s = snapshot(&bugs, &[], &[]);
    let ids: Vec<String> = backlog(&s).into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, ["1", "2", "3", "4", "5", "99"]);
}
