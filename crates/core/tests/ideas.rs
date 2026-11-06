use jiff::Timestamp;
use serde_json::json;
use worklens_core::rules::NextStep;
use worklens_core::section::Section;
use worklens_core::{CoreConfig, Snapshot, build_snapshot};

const ME: &str = "me@mozilla.com";

fn issue(key: &str, status: &str, category: &str, parent: Option<&str>, ideas: &[&str]) -> serde_json::Value {
    json!({
        "key": key, "summary": format!("{key} summary"), "status": status, "statusCategory": category,
        "assignee": ME, "parent": parent, "updated": "2026-11-01T12:00:00Z", "points": 3, "sprint": null,
        "ideas": ideas,
    })
}

fn snapshot() -> Snapshot {
    snapshot_with(|_| {}, &CoreConfig::default())
}

fn snapshot_with(change: impl FnOnce(&mut serde_json::Value), cfg: &CoreConfig) -> Snapshot {
    let mut raw = json!({
        "version": 1, "fetchedAt": "2026-11-04T12:00:00Z",
        "me": { "email": ME, "jiraEmail": ME, "phid": "PHID-ME" },
        "bugs": [{
            "id": 1, "summary": "Read Google Docs", "status": "ASSIGNED", "resolution": "", "assigned_to": ME,
            "severity": "S3", "priority": "P3", "keywords": [], "whiteboard": "",
            "see_also": ["https://mozilla-hub.atlassian.net/browse/AIPLAT-1416"], "flags": [],
            "last_change_time": "2026-11-01T12:00:00Z", "product": "Core", "component": "ML", "via": ["assigned"],
        }],
        "issues": {
            "AIPLAT-1416": issue("AIPLAT-1416", "ENG REVIEW", "indeterminate", Some("AIPLAT-1040"), &[]),
            "AIPLAT-1612": issue("AIPLAT-1612", "In Progress", "indeterminate", Some("AIPLAT-1040"), &[]),
            "AIPLAT-51": issue("AIPLAT-51", "Backlog", "new", None, &[]),
        },
        "revisions": [], "toReview": [], "bugRevisions": {}, "lastComments": {}, "errors": [],
        "roadmap": {
            "parents": {
                "AIPLAT-1040": {
                    "key": "AIPLAT-1040", "summary": "Example epic: text extraction", "status": "In Progress",
                    "statusCategory": "indeterminate", "assignee": "pm@mozilla.com", "type": "Epic",
                    "updated": "2026-11-01T12:00:00Z", "points": null, "sprint": null, "ideas": ["AIDI-79"],
                },
            },
            "ideas": {
                "AIDI-79": {
                    "key": "AIDI-79", "summary": "Example idea: page extraction", "status": "Developing",
                    "statusCategory": "indeterminate", "assignee": "pm@mozilla.com",
                    "updated": "2026-11-04T09:00:00Z", "weeklyStatus": "Dev on track",
                    "target": "Fx158.0.1 (Oct 20)", "quarter": "Q4 | 2026",
                },
            },
        },
    });
    change(&mut raw);
    let raw = serde_json::from_value(raw).unwrap();
    build_snapshot(raw, cfg, "2026-11-04T12:00:00Z".parse::<Timestamp>().unwrap())
}

fn ids(s: &Snapshot, section: Section) -> Vec<String> {
    s.section(section, true).map(|i| s.id(s.entry(i)).to_string()).collect()
}

#[test]
fn an_idea_is_one_row_in_its_own_tab_and_never_a_turn() {
    let s = snapshot();
    assert_eq!(ids(&s, Section::Ideas), ["AIDI-79"]);
    for turn in [Section::Waiting, Section::Land, Section::Returned, Section::Others, Section::Other] {
        assert!(!ids(&s, turn).contains(&"AIDI-79".to_owned()), "{turn:?}");
    }
    let idea = s.entry(s.find("AIDI-79").unwrap());
    assert_eq!(
        s.reason(idea).to_string(),
        "Developing, Dev on track, target Fx158.0.1 (Oct 20); 2 of your items roll up to it"
    );
    let work: Vec<String> = s.work_of(idea.idea.unwrap()).map(|i| s.id(s.entry(i)).to_string()).collect();
    assert_eq!(work, ["1", "AIPLAT-1612"], "the bug stands for its ticket; AIPLAT-51 has no idea");
}

#[test]
fn a_passed_target_is_drift_once_per_idea_not_per_ticket() {
    let s = snapshot();
    assert_eq!(ids(&s, Section::Drift), ["AIDI-79"]);
    let idea = s.entry(s.find("AIDI-79").unwrap());
    let messages: Vec<(&str, &str)> = idea.drift.iter().map(|d| (d.message.as_str(), d.cause.as_str())).collect();
    assert_eq!(
        messages,
        [(
            "AIDI-79 targets Fx158.0.1 (Oct 20), 15 days ago",
            "bug 1 is still ASSIGNED, and 1 more of yours is open"
        )]
    );
    assert!(idea.drift.iter().all(|d| d.fix.is_none()), "worklens never edits the idea");
    assert_eq!(s.next_step(idea), Some(NextStep::TellIdeaOwner));
}

#[test]
fn work_shows_its_epic_and_idea() {
    let s = snapshot();
    let bug = s.entry(s.find("1").unwrap());
    assert_eq!(s.parent(bug).map(|p| p.key.0.as_str()), Some("AIPLAT-1040"));
    assert_eq!(bug.idea.map(|i| s.idea(i).key.0.as_str()), Some("AIDI-79"));
    let loose = s.entry(s.find("AIPLAT-51").unwrap());
    assert!(loose.idea.is_none() && s.parent(loose).is_none());
}

#[test]
fn a_closed_epic_with_open_work_under_it_is_drift_on_that_work() {
    let s = snapshot_with(
        |raw| {
            let epic = &mut raw["roadmap"]["parents"]["AIPLAT-1040"];
            epic["status"] = "Closed".into();
            epic["statusCategory"] = "done".into();
        },
        &CoreConfig::default(),
    );
    let messages = |id: &str| -> Vec<String> {
        let e = s.entry(s.find(id).unwrap());
        e.drift.iter().map(|d| format!("{}: {}", d.message, d.cause)).collect()
    };
    assert_eq!(messages("1"), ["AIPLAT-1416's epic AIPLAT-1040 is Closed: bug 1 is still ASSIGNED"]);
    assert_eq!(
        messages("AIPLAT-1612"),
        ["AIPLAT-1612's epic AIPLAT-1040 is Closed: AIPLAT-1612 is still In Progress"]
    );
    assert!(messages("AIPLAT-51").is_empty());
}

#[test]
fn the_update_for_an_idea_states_what_each_item_is_doing() {
    let s = snapshot_with(
        |raw| {
            raw["issues"]["AIPLAT-1612"]["status"] = "Closed".into();
            raw["issues"]["AIPLAT-1612"]["statusCategory"] = "done".into();
        },
        &CoreConfig::default(),
    );
    let text = worklens_core::brief::render(&s, s.entry(s.find("AIDI-79").unwrap()));
    assert_eq!(
        text,
        "## AIDI-79 Example idea: page extraction: engineering update, 2026-11-04\n\n\
         Jira: Developing · Dev on track · target Fx158.0.1 (Oct 20)\n\n\
         ### In progress\n\n\
         - Bug 1 (AIPLAT-1416) Read Google Docs: ASSIGNED\n\n\
         ### Done\n\n\
         - AIPLAT-1612 AIPLAT-1612 summary: Closed\n\n\
         ### Doesn't match Jira\n\n\
         - AIDI-79 targets Fx158.0.1 (Oct 20), 15 days ago: bug 1 is still ASSIGNED\n\n\
         _From worklens: my Bugzilla, Phabricator and Jira items under AIDI-79 as of 2026-11-04 12:00 UTC. \
         Other people's work isn't included._\n"
    );
}

#[test]
fn the_team_tab_names_ideas_the_team_is_late_on() {
    let cfg = CoreConfig {
        whiteboard: Some("[aiplatform]".into()),
        ..CoreConfig::default()
    };
    let s = snapshot_with(
        |raw| {
            raw["bugs"].as_array_mut().unwrap().push(json!({
                "id": 2, "summary": "Teammate's part", "status": "NEW", "resolution": "", "assigned_to": "x@mozilla.com",
                "severity": "S3", "priority": "P3", "keywords": [], "whiteboard": "[aiplatform]",
                "see_also": ["https://mozilla-hub.atlassian.net/browse/AIPLAT-1612"], "flags": [],
                "last_change_time": "2026-11-01T12:00:00Z", "product": "Core", "component": "ML", "via": ["team"],
            }));
        },
        &cfg,
    );
    let team: Vec<String> = s.section(Section::Team, true).map(|i| s.id(s.entry(i)).to_string()).collect();
    assert!(team.contains(&"AIDI-79".to_owned()), "{team:?}");
    let idea = s
        .section(Section::Team, true)
        .map(|i| s.entry(i))
        .find(|e| s.id(e).to_string() == "AIDI-79")
        .unwrap();
    assert_eq!(idea.drift[0].cause, "bug 2 is still NEW");
    assert_eq!(s.work_of(idea.idea.unwrap()).count(), 2, "my own work under the idea is unchanged");
}
