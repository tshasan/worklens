use jiff::Timestamp;
use serde_json::{Value, json};
use worklens_core::section::Section;
use worklens_core::{CoreConfig, Raw, Snapshot, build_snapshot};

const ME: &str = "me@mozilla.com";

fn bug(assigned_to: &str, revisions: &[&str]) -> (Value, Vec<Value>) {
    let bug = json!({
        "id": 1, "summary": "bug 1", "status": "ASSIGNED", "resolution": "", "assigned_to": assigned_to,
        "severity": "S3", "priority": "P3", "keywords": [], "whiteboard": "[aiplatform]",
        "see_also": ["https://mozilla-hub.atlassian.net/browse/AI-1"], "flags": [],
        "last_change_time": "2026-11-01T12:00:00Z", "product": "Core", "component": "ML", "via": ["assigned"],
    });
    let revs = revisions
        .iter()
        .enumerate()
        .map(|(i, status)| {
            json!({
                "id": 100 + i, "title": "D", "status": status, "author": "PHID-ME", "bug": 1,
                "modified": "2026-11-01T12:00:00Z", "reviewers": [],
            })
        })
        .collect();
    (bug, revs)
}

fn issue(status: &str, category: &str, assignee: Option<&str>) -> Value {
    json!({
        "key": "AI-1", "summary": "AI-1", "status": status, "statusCategory": category, "assignee": assignee,
        "updated": "2026-11-01T12:00:00Z", "points": 3, "sprint": null,
    })
}

fn snapshot(me: &Value, (bug, revisions): (Value, Vec<Value>), issue: &Value, cfg: &CoreConfig) -> Snapshot {
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-02T12:00:00Z", "me": me,
        "bugs": [bug], "issues": { "AI-1": issue }, "revisions": revisions, "toReview": [], "bugRevisions": {},
        "lastComments": {}, "errors": [],
    }))
    .unwrap();
    build_snapshot(raw, cfg, "2026-11-02T12:00:00Z".parse::<Timestamp>().unwrap())
}

fn drift(s: &Snapshot) -> Vec<String> {
    s.entries[0].drift.iter().map(|d| d.message.clone()).collect()
}

#[test]
fn me_under_my_jira_email_is_the_same_assignee() {
    let me = json!({ "email": ME, "jiraEmail": "thasan@mozilla.com", "phid": "PHID-ME" });
    let s = snapshot(
        &me,
        bug(ME, &[]),
        &issue("In Progress", "indeterminate", Some("THasan@mozilla.com")),
        &CoreConfig::default(),
    );
    assert_eq!(drift(&s), Vec::<String>::new(), "my two emails are one person");
}

#[test]
fn assignees_compare_without_case() {
    let me = json!({ "email": ME, "jiraEmail": ME, "phid": "PHID-ME" });
    let s = snapshot(
        &me,
        bug(ME, &[]),
        &issue("In Progress", "indeterminate", Some("Me@Mozilla.com")),
        &CoreConfig::default(),
    );
    assert_eq!(drift(&s), Vec::<String>::new());
}

#[test]
fn someone_else_in_jira_is_still_drift() {
    let me = json!({ "email": ME, "jiraEmail": "thasan@mozilla.com", "phid": "PHID-ME" });
    let s = snapshot(
        &me,
        bug(ME, &[]),
        &issue("In Progress", "indeterminate", Some("other@mozilla.com")),
        &CoreConfig::default(),
    );
    assert_eq!(drift(&s), ["assignee: Bugzilla me@mozilla.com, Jira other@mozilla.com"]);
}

fn me() -> Value {
    json!({ "email": ME, "jiraEmail": ME, "phid": "PHID-ME" })
}

fn in_review(status: &str, category: &str, cfg: &CoreConfig) -> Vec<String> {
    drift(&snapshot(
        &me(),
        bug(ME, &["needs-review"]),
        &issue(status, category, Some(ME)),
        cfg,
    ))
}

#[test]
fn the_default_workflow_is_ai_platforms() {
    let cfg = CoreConfig::default();
    assert_eq!(
        in_review("In Progress", "indeterminate", &cfg),
        ["Jira is In Progress but a revision is in review; expected ENG REVIEW"]
    );
    assert!(in_review("ENG REVIEW", "indeterminate", &cfg).is_empty());
    assert!(
        in_review("Eng Review", "indeterminate", &cfg).is_empty(),
        "names match without case"
    );
    assert!(in_review("blocked", "indeterminate", &cfg).is_empty());
}

#[test]
fn an_unknown_status_falls_back_to_its_category() {
    let cfg = CoreConfig::default();
    assert!(
        in_review("In QA", "indeterminate", &cfg).is_empty(),
        "an in-progress step this config doesn't know isn't drift"
    );
    assert_eq!(
        in_review("Backlog", "new", &cfg),
        ["Jira is Backlog but a revision is in review; expected ENG REVIEW"],
        "a to-do status while in review still is"
    );
}

#[test]
fn another_teams_workflow() {
    let names = |xs: &[&str]| xs.iter().map(|x| (*x).to_owned()).collect();
    let cfg = CoreConfig {
        jira_statuses: worklens_core::drift::JiraStatuses {
            done: names(&["Done"]),
            review: names(&["In Review"]),
            in_progress: names(&["In Progress"]),
            blocked: names(&["On Hold"]),
        },
        ..CoreConfig::default()
    };
    assert!(in_review("In Review", "indeterminate", &cfg).is_empty());
    assert!(in_review("On Hold", "indeterminate", &cfg).is_empty());
    assert_eq!(
        in_review("In Progress", "indeterminate", &cfg),
        ["Jira is In Progress but a revision is in review; expected In Review"]
    );

    let (mut fixed, revs) = bug(ME, &["published"]);
    fixed["status"] = json!("RESOLVED");
    fixed["resolution"] = json!("FIXED");
    let s = snapshot(&me(), (fixed, revs), &issue("In Review", "indeterminate", Some(ME)), &cfg);
    let fix = s.entries[0].drift[0].fix.as_ref().unwrap();
    assert_eq!((fix.field, fix.value.as_str()), ("status", "Done"));
}

#[test]
fn without_a_whiteboard_tag_there_is_no_team_to_check() {
    let (mut mine, _) = bug(ME, &[]);
    mine["see_also"] = json!([]);
    let (mut team, _) = bug("other@mozilla.com", &[]);
    team["id"] = json!(2);
    team["via"] = json!(["team"]);
    team["status"] = json!("RESOLVED");
    team["resolution"] = json!("FIXED");
    let build = |whiteboard: Option<&str>| {
        let raw: Raw = serde_json::from_value(json!({
            "version": 1, "fetchedAt": "2026-11-02T12:00:00Z", "me": me(),
            "bugs": [mine, team], "issues": { "AI-1": issue("In Progress", "indeterminate", Some("other@mozilla.com")) },
            "revisions": [], "toReview": [], "bugRevisions": {}, "lastComments": {}, "errors": [],
        }))
        .unwrap();
        let cfg = CoreConfig {
            whiteboard: whiteboard.map(str::to_owned),
            ..CoreConfig::default()
        };
        let s = build_snapshot(raw, &cfg, "2026-11-02T12:00:00Z".parse::<Timestamp>().unwrap());
        let ids = |section| s.section(section, true).map(|i| s.id(s.entry(i)).to_string()).collect::<Vec<_>>();
        (ids(Section::Drift), ids(Section::Team))
    };
    assert_eq!(build(Some("[aiplatform]")), (vec!["1".to_owned()], vec!["2".to_owned()]));
    assert_eq!(build(None), (vec![], vec![]), "no tag: no mirror expected, no team");
    assert_eq!(build(Some("")), (vec![], vec![]), "an empty tag is no tag");
}

fn sprint_snapshot(revisions: &[&str], ai1_sprint: Option<&str>, other_sprint: Option<&str>) -> Snapshot {
    let me = json!({ "email": ME, "jiraEmail": ME, "phid": "PHID-ME" });
    let (bug, revisions) = bug(ME, revisions);
    let sprint = |name: Option<&str>| name.map(|n| json!({ "name": n, "state": "active" }));
    let mut ai1 = issue("ENG REVIEW", "indeterminate", Some(ME));
    ai1["sprint"] = json!(sprint(ai1_sprint));
    let mut ai2 = issue("In Progress", "indeterminate", Some(ME));
    ai2["key"] = json!("AI-2");
    ai2["sprint"] = json!(sprint(other_sprint));
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-02T12:00:00Z", "me": me,
        "bugs": [bug], "issues": { "AI-1": ai1, "AI-2": ai2 }, "revisions": revisions, "toReview": [],
        "bugRevisions": {}, "lastComments": {}, "errors": [],
    }))
    .unwrap();
    build_snapshot(raw, &CoreConfig::default(), "2026-11-02T12:00:00Z".parse::<Timestamp>().unwrap())
}

#[test]
fn a_revision_in_review_on_an_unplanned_ticket_is_untracked_work() {
    let s = sprint_snapshot(&["needs-review"], None, Some("162 Core"));
    let e = &s.entries[0];
    assert_eq!(drift(&s), vec!["AI-1 is in no sprint, but the bug has an open revision"]);
    let fix = e.drift[0].fix.as_ref().expect("a fix to copy");
    assert_eq!((fix.field, fix.value.as_str()), ("sprint", "162 Core"), "the sprint I'm in");
}

#[test]
fn untracked_work_needs_sprints_an_open_revision_and_no_sprint_on_the_ticket() {
    assert_eq!(drift(&sprint_snapshot(&["needs-review"], None, None)), Vec::<String>::new());
    assert_eq!(
        drift(&sprint_snapshot(&["needs-review"], Some("162 Core"), Some("162 Core"))),
        Vec::<String>::new()
    );
    assert_eq!(
        drift(&sprint_snapshot(&["abandoned"], None, Some("162 Core"))),
        Vec::<String>::new()
    );
}

fn titles(bug_title: &str, jira_title: &str, cfg: &CoreConfig) -> Vec<String> {
    let me = json!({ "email": ME, "jiraEmail": ME, "phid": "PHID-ME" });
    let (mut b, revs) = bug(ME, &[]);
    b["summary"] = json!(bug_title);
    let mut i = issue("In Progress", "indeterminate", Some(ME));
    i["summary"] = json!(jira_title);
    drift(&snapshot(&me, (b, revs), &i, cfg))
}

#[test]
fn title_drift_is_checked_only_where_jbi_syncs_the_title() {
    let syncs = CoreConfig {
        jbi_syncs: vec![worklens_core::drift::Synced::Summary],
        ..CoreConfig::default()
    };
    assert_eq!(
        titles("Fix the sidebar", "Sidebar work", &CoreConfig::default()),
        Vec::<String>::new()
    );
    assert_eq!(
        titles("Fix the sidebar", "Sidebar work", &syncs),
        vec!["title: Jira's differs from the bug's"]
    );
    assert_eq!(
        titles("Fix the sidebar focus ring", "Fix the sidebar", &syncs),
        Vec::<String>::new()
    );
    assert_eq!(
        titles("Fix the sidebar", "Fix the side", &syncs),
        vec!["title: Jira's differs from the bug's"]
    );
}
