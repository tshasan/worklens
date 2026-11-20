use jiff::Timestamp;
use serde_json::{Map, Value, json};
use worklens_core::section::Section;
use worklens_core::snapshot::{ItemId, Scope};
use worklens_core::{CoreConfig, Entry, Raw, Snapshot, build_snapshot};

const ME: &str = "me@mozilla.com";
const SPRINT: &str = "AI Platform 2026.24";

fn link(key: &str) -> String {
    format!("https://mozilla-hub.atlassian.net/browse/{key}")
}

fn bug(id: u32, keys: &[&str], via: &str) -> Value {
    json!({
        "id": id, "summary": format!("bug {id}"), "status": "ASSIGNED", "resolution": "", "assigned_to": ME,
        "severity": "S3", "priority": "P3", "keywords": [], "whiteboard": "",
        "see_also": keys.iter().map(|k| link(k)).collect::<Vec<_>>(), "flags": [],
        "last_change_time": "2026-11-18T12:00:00Z", "product": "Firefox", "component": "AI", "via": [via],
    })
}

fn issue(key: &str, kind: &str, status: &str, category: &str, assignee: Option<&str>, sprint: bool) -> Value {
    json!({
        "key": key, "summary": key, "status": status, "statusCategory": category, "assignee": assignee, "type": kind,
        "updated": "2026-11-18T12:00:00Z", "points": 3,
        "sprint": if sprint { json!({ "name": SPRINT, "state": "active" }) } else { Value::Null },
    })
}

fn rev(id: u32, bug: u32, status: &str) -> Value {
    json!({
        "id": id, "title": format!("Bug {bug} - part"), "status": status, "author": "PHID-ME", "bug": bug,
        "modified": "2026-11-18T12:00:00Z", "reviewers": [],
    })
}

fn snapshot(bugs: &[Value], issues: &[Value], revisions: &[Value], extra: Value, cfg: &CoreConfig) -> Snapshot {
    let issues: Map<String, Value> = issues.iter().map(|i| (i["key"].as_str().unwrap().to_owned(), i.clone())).collect();
    let mut raw = json!({
        "version": 1, "fetchedAt": "2026-11-19T12:00:00Z", "me": { "email": ME, "jiraEmail": ME, "phid": "PHID-ME" },
        "bugs": bugs, "issues": issues, "revisions": revisions, "toReview": [], "bugRevisions": {},
        "lastComments": {}, "errors": [],
    });
    if let Value::Object(more) = extra {
        raw.as_object_mut().unwrap().extend(more);
    }
    let raw: Raw = serde_json::from_value(raw).unwrap();
    build_snapshot(raw, cfg, "2026-11-19T12:00:00Z".parse::<Timestamp>().unwrap())
}

fn mine(s: &Snapshot, id: u32) -> &Entry {
    s.entries
        .iter()
        .find(|e| e.scope == Scope::Mine && matches!(e.id, ItemId::Bug(b) if b.0 == id))
        .unwrap()
}

fn messages(e: &Entry) -> Vec<&str> {
    e.drift.iter().map(|d| d.message.as_str()).collect()
}

fn ids(s: &Snapshot, section: Section) -> Vec<String> {
    s.section(section, false).map(|i| s.id(s.entry(i)).to_string()).collect()
}

fn closed(mut b: Value) -> Value {
    b["status"] = json!("RESOLVED");
    b["resolution"] = json!("FIXED");
    b
}

fn assigned_to(mut b: Value, who: &str) -> Value {
    b["assigned_to"] = json!(who);
    b
}

fn chosen(keys: &[&str], issues: &[Value]) -> String {
    let s = snapshot(
        &[bug(1, keys, "assigned")],
        issues,
        &[rev(10, 1, "needs-review")],
        json!({}),
        &CoreConfig::default(),
    );
    s.issue(mine(&s, 1).issue.unwrap()).key.0.clone()
}

#[test]
fn a_closed_sprint_is_no_plan_and_ties_keep_see_also_order() {
    let mut old = issue("AIPLAT-1", "Task", "Backlog", "new", None, true);
    old["sprint"]["state"] = json!("closed");
    let owned = issue("AIPLAT-2", "Task", "Backlog", "new", Some(ME), false);
    assert_eq!(chosen(&["AIPLAT-1", "AIPLAT-2"], &[old, owned]), "AIPLAT-2");
    let a = || issue("AIPLAT-1", "Task", "Backlog", "new", None, false);
    let b = || issue("AIPLAT-2", "Task", "Backlog", "new", None, false);
    assert_eq!(chosen(&["AIPLAT-1", "AIPLAT-2"], &[a(), b()]), "AIPLAT-1");
    assert_eq!(chosen(&["AIPLAT-2", "AIPLAT-1"], &[a(), b()]), "AIPLAT-2");
}

#[test]
fn a_meta_bugs_epic_is_its_own_ticket_but_a_child_bugs_epic_is_a_rollup() {
    let mut child = issue("AIPLAT-9", "Task", "In Progress", "indeterminate", Some(ME), false);
    child["parent"] = json!("AIPLAT-1022");
    let epic = issue("AIPLAT-1022", "Epic", "Closed", "done", Some(ME), false);
    let mut meta = bug(2052354, &["AIPLAT-1022"], "assigned");
    meta["keywords"] = json!(["meta"]);
    let mut titled = bug(2052355, &["AIPLAT-1022"], "assigned");
    titled["summary"] = json!("[Meta] Smart Window happy-path performance findings");
    let s = snapshot(
        &[meta, titled, bug(2092358, &["AIPLAT-1022"], "assigned")],
        &[epic, child],
        &[],
        json!({}),
        &CoreConfig::default(),
    );
    for id in [2052354, 2052355] {
        let e = mine(&s, id);
        assert_eq!(e.issue.map(|i| s.issue(i).key.0.clone()).as_deref(), Some("AIPLAT-1022"), "{id}");
        assert_eq!(messages(e), ["Jira is Closed but bug is still ASSIGNED"], "{id}");
    }
    assert!(mine(&s, 2092358).issue.is_none());
    assert!(
        !s.entries
            .iter()
            .any(|e| matches!(e.id, ItemId::Jira(_)) && e.issue.is_some_and(|i| s.issue(i).key.0 == "AIPLAT-1022")),
        "the epic its bugs link is not listed again on its own"
    );
}

#[test]
fn a_ticket_titled_meta_is_a_rollup_for_a_bug_that_is_not() {
    let mut meta = issue("AIPLAT-1607", "Task", "In Progress", "indeterminate", Some(ME), false);
    meta["summary"] = json!("[meta] Create a policy for captchas in PageExtractor");
    let s = snapshot(
        &[bug(1, &["AIPLAT-1607"], "assigned")],
        &[meta],
        &[rev(10, 1, "needs-review")],
        json!({}),
        &CoreConfig::default(),
    );
    assert!(mine(&s, 1).issue.is_none());
}

#[test]
fn a_rollup_linked_twice_is_named_once() {
    let s = snapshot(
        &[bug(2092358, &["AIPLAT-1607", "AIPLAT-1607"], "assigned")],
        &[issue("AIPLAT-1607", "Epic", "In Progress", "indeterminate", Some(ME), false)],
        &[rev(342405, 2092358, "needs-review")],
        json!({}),
        &CoreConfig::default(),
    );
    assert_eq!(
        messages(mine(&s, 2092358)),
        ["bug has revisions in review but no Jira ticket of its own; AIPLAT-1607 only rolls it up"]
    );
}

fn two_links(order: &[&str]) -> Snapshot {
    snapshot(
        &[bug(2091535, order, "assigned")],
        &[
            issue("AIDI-334", "Idea", "Backlog", "new", None, false),
            issue("AIPLAT-1674", "Task", "ENG REVIEW", "indeterminate", Some(ME), true),
        ],
        &[rev(343000, 2091535, "needs-review")],
        json!({}),
        &CoreConfig::default(),
    )
}

#[test]
fn drift_checks_the_engineering_ticket_not_the_idea_the_bug_also_links() {
    for order in [["AIDI-334", "AIPLAT-1674"], ["AIPLAT-1674", "AIDI-334"]] {
        let s = two_links(&order);
        let e = mine(&s, 2091535);
        assert_eq!(s.issue(e.issue.unwrap()).key.0, "AIPLAT-1674", "{order:?}");
        assert_eq!(messages(e), Vec::<&str>::new(), "{order:?}");
        assert_eq!(
            ids(&s, Section::Sprint),
            ["2091535"],
            "one row for the bug and its ticket: {order:?}"
        );
    }
}

#[test]
fn an_idea_known_from_the_roadmap_counts_as_a_rollup_even_unfetched() {
    let roadmap = json!({ "roadmap": { "parents": {}, "ideas": { "AIDI-334": {
        "key": "AIDI-334", "summary": "Smart Window captchas", "status": "Developing", "statusCategory": "indeterminate",
        "assignee": null, "updated": "2026-11-18T12:00:00Z",
    }}}});
    let s = snapshot(
        &[bug(2091535, &["AIDI-334", "AIPLAT-1674"], "assigned")],
        &[issue("AIPLAT-1674", "Task", "ENG REVIEW", "indeterminate", Some(ME), true)],
        &[rev(343000, 2091535, "needs-review")],
        roadmap,
        &CoreConfig::default(),
    );
    let e = mine(&s, 2091535);
    assert_eq!(s.issue(e.issue.unwrap()).key.0, "AIPLAT-1674");
    assert_eq!(
        s.idea(e.idea.unwrap()).key.0,
        "AIDI-334",
        "the idea it links is what it rolls up to"
    );
    assert!(e.drift.is_empty(), "{:?}", messages(e));
}

#[test]
fn the_ticket_in_the_sprint_and_assigned_to_the_bug_assignee_wins_over_an_older_one() {
    let s = snapshot(
        &[bug(1, &["AIPLAT-1", "AIPLAT-2"], "assigned")],
        &[
            issue("AIPLAT-1", "Task", "Backlog", "new", None, false),
            issue("AIPLAT-2", "Task", "ENG REVIEW", "indeterminate", Some(ME), true),
        ],
        &[rev(10, 1, "needs-review")],
        json!({}),
        &CoreConfig::default(),
    );
    assert_eq!(s.issue(mine(&s, 1).issue.unwrap()).key.0, "AIPLAT-2");
}

#[test]
fn work_in_review_without_a_ticket_is_drift() {
    let s = snapshot(
        &[
            bug(2093725, &[], "assigned"),
            bug(2092358, &["AIPLAT-1607"], "assigned"),
            bug(3, &[], "assigned"),
            bug(4, &[], "assigned"),
        ],
        &[issue("AIPLAT-1607", "Epic", "In Progress", "indeterminate", Some(ME), false)],
        &[
            rev(343316, 2093725, "needs-review"),
            rev(342405, 2092358, "needs-revision"),
            rev(30, 3, "published"),
        ],
        json!({}),
        &CoreConfig::default(),
    );
    assert_eq!(messages(mine(&s, 2093725)), ["bug has revisions in review but no Jira ticket"]);
    assert_eq!(
        messages(mine(&s, 2092358)),
        ["bug has revisions in review but no Jira ticket of its own; AIPLAT-1607 only rolls it up"]
    );
    assert!(mine(&s, 3).drift.is_empty(), "landed work has nothing in review");
    assert!(mine(&s, 4).drift.is_empty(), "not started");
    assert!(mine(&s, 2092358).issue.is_none(), "an epic is not the bug's ticket");
}

#[test]
fn a_mirrored_bug_that_only_links_an_epic_says_the_ticket_is_missing() {
    let cfg = CoreConfig {
        whiteboard: Some("[aiplatform]".into()),
        ..CoreConfig::default()
    };
    let mut b = bug(2092358, &["AIPLAT-1607"], "assigned");
    b["whiteboard"] = json!("[aiplatform]");
    let s = snapshot(
        &[b],
        &[issue("AIPLAT-1607", "Epic", "In Progress", "indeterminate", Some(ME), false)],
        &[rev(342405, 2092358, "needs-revision")],
        json!({}),
        &cfg,
    );
    assert_eq!(
        messages(mine(&s, 2092358)),
        ["no Jira ticket in See Also; AIPLAT-1607 only rolls it up"]
    );
}

#[test]
fn a_ticket_that_other_tickets_hang_under_is_a_rollup() {
    let mut child = issue("AIPLAT-9", "Task", "In Progress", "indeterminate", Some(ME), true);
    child["parent"] = json!("AIPLAT-1607");
    let s = snapshot(
        &[bug(2092358, &["AIPLAT-1607"], "assigned")],
        &[issue("AIPLAT-1607", "Task", "In Progress", "indeterminate", Some(ME), false), child],
        &[rev(342405, 2092358, "needs-review")],
        json!({}),
        &CoreConfig::default(),
    );
    assert!(mine(&s, 2092358).issue.is_none());
}

#[test]
fn the_sprint_counts_work_in_review_outside_it_without_adding_points() {
    let s = snapshot(
        &[
            bug(2093725, &[], "assigned"),
            bug(2093723, &["AIPLAT-1607"], "assigned"),
            bug(2091535, &["AIPLAT-1674"], "assigned"),
            bug(5, &[], "assigned"),
            closed(bug(6, &[], "assigned")),
            assigned_to(bug(7, &[], "needinfo"), "other@mozilla.com"),
        ],
        &[
            issue("AIPLAT-1607", "Epic", "In Progress", "indeterminate", None, false),
            issue("AIPLAT-1674", "Task", "ENG REVIEW", "indeterminate", Some(ME), true),
        ],
        &[
            rev(1, 2093725, "needs-review"),
            rev(2, 2093723, "accepted"),
            rev(3, 2091535, "needs-review"),
            rev(4, 5, "abandoned"),
            rev(6, 6, "accepted"),
            rev(7, 7, "needs-review"),
        ],
        json!({}),
        &CoreConfig::default(),
    );
    let sp = s.sprint.as_ref().unwrap();
    let outside: Vec<String> = sp.outside.iter().map(|i| s.id(s.entry(*i)).to_string()).collect();
    assert_eq!(outside, ["2093725", "2093723"], "not closed bugs, nor ones assigned to others");
    assert!(
        (sp.committed - 3.0).abs() < f32::EPSILON,
        "only AIPLAT-1674's points: {}",
        sp.committed
    );
}
