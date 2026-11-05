use serde_json::{Value, json};
use worklens_core::{CoreConfig, Raw, Section, Snapshot, build_snapshot, dependencies, guidance};

fn fixture() -> Value {
    let mut raw: Value = serde_json::from_str(include_str!("../../store/fixtures/raw.json")).unwrap();
    raw["bugs"][0]["flags"] = json!([]);
    raw["revisions"][0]["status"] = json!("accepted");
    raw["revisions"][0]["reviewers"] = json!([]);
    raw["tryPushes"] = json!({});
    raw
}

fn snapshot(raw: Value) -> Snapshot {
    build_snapshot(
        serde_json::from_value::<Raw>(raw).unwrap(),
        &CoreConfig::default(),
        "2026-11-03T20:00:00Z".parse().unwrap(),
    )
}

fn selected(s: &Snapshot) -> &worklens_core::Entry {
    s.entry(s.find("1995008").unwrap())
}

fn step(s: &Snapshot) -> String {
    s.next_step(selected(s)).unwrap().to_string()
}

#[test]
fn classification_moves_an_accepted_revision_out_of_ready_and_targets_phabricator() {
    let mut raw = fixture();
    raw["revisions"][0]["projects"] = json!(["PHID-PROJ-data"]);
    raw["users"]["PHID-PROJ-data"] = json!("#needs-data-classification");
    let s = snapshot(raw);
    let e = selected(&s);
    assert_eq!(e.placement.unwrap().section, Section::Waiting);
    assert_eq!(s.tag(e).to_string(), "chk");
    assert!(s.reason(e).to_string().contains("requires data classification"));
    assert_eq!(step(&s), "Resolve data classification on D321987 in Phabricator");
    assert_eq!(
        guidance::next_url(&s.raw, s.next_step(e).unwrap()).unwrap(),
        "https://phabricator.services.mozilla.com/D321987"
    );
}

#[test]
fn accepted_is_not_ready_while_try_is_failing_running_or_outdated() {
    for (state, failed, running, pushed, expected) in [
        ("fail", 1, 0, "2026-11-03T19:00:00Z", "failing jobs"),
        ("running", 0, 1, "2026-11-03T19:00:00Z", "still running"),
        ("pass", 0, 0, "2026-10-01T00:00:00Z", "older diff"),
    ] {
        let mut raw = fixture();
        raw["tryPushes"] = json!({"321987": {
            "revision":"abc", "pushedAt":pushed, "url":"https://treeherder.mozilla.org/jobs?repo=try&revision=abc",
            "builds":"pass", "lint":"pass", "tests":state, "failed":failed, "running":running
        }});
        let s = snapshot(raw);
        assert_eq!(selected(&s).placement.unwrap().section, Section::Waiting);
        assert!(s.reason(selected(&s)).to_string().contains(expected));
        assert!(step(&s).starts_with("Inspect D321987's try"));
    }
}

#[test]
fn immediate_parents_are_context_without_becoming_personal_queue_items() {
    let mut raw = fixture();
    let mut parent = raw["revisions"][0].clone();
    parent["id"] = json!(123);
    parent["phid"] = json!("PHID-DREV-parent");
    parent["status"] = json!("needs-revision");
    parent["author"] = json!("PHID-USER-alice");
    raw["users"]["PHID-USER-alice"] = json!("alice");
    raw["dependencies"] = json!({"parents":{"321987":["PHID-DREV-parent"]}, "revisions":{"PHID-DREV-parent":parent}});
    let s = snapshot(raw);
    let e = selected(&s);
    let deps = dependencies::of(&s, e);
    assert_eq!(deps[0].id, "D123");
    assert_eq!(deps[0].owner.as_deref(), Some("alice"));
    assert_eq!(deps[0].blocked, Some(true));
    assert_eq!(step(&s), "Inspect dependency D123 in Phabricator");
    assert!(s.find("D123").is_none());
    assert!(worklens_core::brief::render(&s, e).contains("D123:"));
}

#[test]
fn accepted_parents_can_land_together_but_missing_parents_are_unknown() {
    let mut raw = fixture();
    let mut parent = raw["revisions"][0].clone();
    parent["id"] = json!(123);
    raw["dependencies"] = json!({"parents":{"321987":["PHID-DREV-parent"]}, "revisions":{"PHID-DREV-parent":parent}});
    let s = snapshot(raw.clone());
    assert_eq!(selected(&s).placement.unwrap().section, Section::Land);
    assert_eq!(dependencies::of(&s, selected(&s))[0].blocked, Some(false));
    raw["dependencies"]["revisions"] = json!({});
    let s = snapshot(raw);
    assert_eq!(dependencies::of(&s, selected(&s))[0].blocked, None);
    assert_eq!(step(&s), "Check D321987's parent revisions in Phabricator");
}

#[test]
fn bug_dependencies_show_owner_and_unknown_visibility_without_implying_a_lando_block() {
    let mut raw = fixture();
    raw["bugs"][0]["depends_on"] = json!([123, 124, 123]);
    let mut bug = raw["bugs"][0].clone();
    bug["id"] = json!(123);
    bug["assigned_to"] = json!("alice@mozilla.com");
    raw["dependencies"] = json!({"bugs":{"123":bug}});
    let s = snapshot(raw);
    let deps = dependencies::of(&s, selected(&s));
    assert_eq!(deps[0].owner.as_deref(), Some("alice@mozilla.com"));
    assert_eq!(deps[0].blocked, Some(true));
    assert_eq!(deps[1].id, "bug 124");
    assert_eq!(deps[1].blocked, None);
    assert_eq!(selected(&s).placement.unwrap().section, Section::Land);
}

#[test]
fn a_failed_dependency_lookup_does_not_override_a_known_classification_requirement() {
    let mut raw = fixture();
    raw["revisions"][0]["projects"] = json!(["data"]);
    raw["users"]["data"] = json!("#needs-data-classification");
    raw["errors"] = json!([{"source":"phabricator-dependencies","message":"unavailable"}]);
    assert!(step(&snapshot(raw)).starts_with("Resolve data classification"));
}

#[test]
fn a_failed_review_lookup_does_not_treat_cached_acceptance_as_current_readiness() {
    let mut raw = fixture();
    raw["errors"] = json!([{"source":"phabricator","message":"unavailable"}]);
    let s = snapshot(raw);
    assert!(step(&s).starts_with("Check outstanding reviews"));
    assert!(s.reason(selected(&s)).to_string().contains("could not be refreshed"));
}
