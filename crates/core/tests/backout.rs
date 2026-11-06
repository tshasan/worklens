use serde_json::{Value, json};
use worklens_core::evidence::workflow;
use worklens_core::rules::{NextStep, Reason};
use worklens_core::{BugId, CoreConfig, Raw, Section, Snapshot, build_snapshot};

const BUG: u32 = 1995008;
const PUSH_URL: &str = "https://treeherder.mozilla.org/jobs?repo=autoland&revision=";

fn hex(c: char) -> String {
    c.to_string().repeat(40)
}

fn landing(rev: u32, push: char, at: &str, failed: u32) -> Value {
    json!({
        "revision": rev, "commit": hex(push), "pushRevision": hex(push), "url": format!("{PUSH_URL}{}", hex(push)),
        "pushedAt": at, "healthError": null,
        "health": {"builds": "pass", "lint": "pass", "tests": if failed > 0 { "fail" } else { "pass" }, "failed": failed, "running": 0}
    })
}

fn backout(bugs: &[u32], cause: Option<&str>, push: char, at: &str) -> Value {
    json!({"bugs": bugs, "targets": [hex('1')], "cause": cause, "commit": hex(push), "pushRevision": hex(push), "url": format!("{PUSH_URL}{}", hex(push)), "pushedAt": at})
}

fn raw(matches: &[Value], backouts: &[Value]) -> Value {
    let mut raw: Value = serde_json::from_str(include_str!("../../store/fixtures/raw.json")).unwrap();
    raw["revisions"][0]["status"] = json!("published");
    raw["landings"] = json!({"revisions": [321987], "repositories": [{
        "repository": "autoland", "checkedAt": "2026-11-03T19:00:00Z", "pushLimit": 100, "scannedPushes": 100,
        "oldestPush": "2026-11-02T00:00:00Z", "error": null, "matches": matches, "backouts": backouts
    }]});
    raw
}

fn snapshot(raw: Value) -> Snapshot {
    build_snapshot(
        serde_json::from_value::<Raw>(raw).unwrap(),
        &CoreConfig::default(),
        "2026-11-03T20:00:00Z".parse().unwrap(),
    )
}

fn first_waiting(s: &Snapshot) -> &worklens_core::Entry {
    s.entry(s.section(Section::Waiting, false).next().expect("something waiting"))
}

#[test]
fn a_backout_after_my_landing_says_where_how_soon_and_why() {
    let s = snapshot(raw(
        &[landing(321987, '1', "2026-11-03T10:00:00Z", 0)],
        &[backout(&[BUG], Some("causing bc failures"), '2', "2026-11-03T13:00:00Z")],
    ));
    let e = first_waiting(&s);
    let p = e.placement.unwrap();
    assert_eq!(
        p.reason,
        Reason::BackedOut(BugId(BUG)),
        "the backout outranks the sheriff's needinfo"
    );
    assert_eq!(
        s.reason(e).to_string(),
        "D321987 backed out of autoland 3h after landing: causing bc failures"
    );
    assert_eq!(s.next_step(e), Some(NextStep::OpenBackout(BugId(BUG))));
    assert_eq!(
        worklens_core::guidance::next_url(&s.raw, NextStep::OpenBackout(BugId(BUG))),
        Some(format!("{PUSH_URL}{}", hex('2')))
    );
    assert_eq!(s.tag(e).to_string(), "bo");
}

#[test]
fn failing_landing_jobs_do_not_prove_the_backout_cause() {
    let s = snapshot(raw(
        &[landing(321987, '1', "2026-11-03T10:00:00Z", 4)],
        &[backout(&[BUG], Some("causing bc failures"), '2', "2026-11-03T13:00:00Z")],
    ));
    let e = first_waiting(&s);
    assert_eq!(s.next_step(e), Some(NextStep::OpenBackout(BugId(BUG))));
    let url = worklens_core::guidance::next_url(&s.raw, s.next_step(e).unwrap()).unwrap();
    assert_eq!(url, format!("{PUSH_URL}{}", hex('2')));
}

fn relanded(backouts: &[Value]) -> Value {
    let mut raw = raw(
        &[
            landing(321987, '1', "2026-11-03T10:00:00Z", 0),
            landing(321987, '3', "2026-11-03T15:00:00Z", 0),
        ],
        backouts,
    );
    let mut again = raw["revisions"][0].clone();
    again["id"] = json!(321987);
    raw["revisions"].as_array_mut().unwrap().push(again);
    raw
}

#[test]
fn a_reland_clears_it_and_a_second_backout_brings_it_back() {
    let backed = backout(&[BUG], Some("causing bc failures"), '2', "2026-11-03T13:00:00Z");
    let s = snapshot(relanded(std::slice::from_ref(&backed)));
    assert_eq!(
        first_waiting(&s).placement.unwrap().reason,
        Reason::Needinfo,
        "re-landed, so only the needinfo is left"
    );
    let mut again = backout(&[BUG], Some("causing wpt failures"), '4', "2026-11-03T17:30:00Z");
    again["targets"] = json!([hex('3')]);
    let s = snapshot(relanded(&[backed, again]));
    let e = first_waiting(&s);
    assert_eq!(
        s.reason(e).to_string(),
        "D321987 backed out of autoland 2h after landing: causing wpt failures"
    );
}

#[test]
fn a_backout_of_someone_elses_patch_on_my_bug_does_not_blame_me() {
    let mut raw = raw(
        &[
            landing(321987, '1', "2026-11-03T10:00:00Z", 0),
            landing(400, '5', "2026-11-03T12:00:00Z", 0),
        ],
        &[backout(&[BUG], Some("causing bc failures"), '2', "2026-11-03T13:00:00Z")],
    );
    let mut theirs = raw["revisions"][0].clone();
    theirs["id"] = json!(400);
    theirs["author"] = json!("PHID-USER-other");
    raw["revisions"].as_array_mut().unwrap().push(theirs);
    raw["landings"]["repositories"][0]["backouts"][0]["targets"] = json!([hex('5')]);
    let s = snapshot(raw);
    assert_eq!(first_waiting(&s).placement.unwrap().reason, Reason::Needinfo);
}

#[test]
fn a_backout_without_a_cause_says_so_and_a_multi_bug_backout_matches_each() {
    let s = snapshot(raw(
        &[landing(321987, '1', "2026-11-03T10:00:00Z", 0)],
        &[backout(&[2000001, BUG], None, '2', "2026-11-03T10:40:00Z")],
    ));
    assert_eq!(
        s.reason(first_waiting(&s)).to_string(),
        "D321987 backed out of autoland 40m after landing: cause not stated"
    );
}

#[test]
fn evidence_says_what_the_scan_found_about_backouts() {
    let s = snapshot(raw(&[landing(321987, '1', "2026-11-03T10:00:00Z", 0)], &[]));
    let w = workflow(&s, &s.entries[0]);
    assert!(
        w.landings[0].summary.contains("no attributed later backout in the scanned pushes"),
        "{}",
        w.landings[0].summary
    );
    assert!(!w.landings[0].summary.contains("later backouts and shipped builds unverified"));
    let s = snapshot(raw(
        &[landing(321987, '1', "2026-11-03T10:00:00Z", 0)],
        &[backout(&[BUG], Some("causing bc failures"), '2', "2026-11-03T13:00:00Z")],
    ));
    let w = workflow(&s, &s.entries[0]);
    assert!(
        w.landings[0]
            .summary
            .contains(&format!("backed out in autoland push {} at 2026-11-03T13:00:00Z", hex('2'))),
        "{}",
        w.landings[0].summary
    );
}

#[test]
fn another_patch_or_tree_does_not_clear_an_unresolved_backout() {
    let backed = backout(&[BUG], None, '2', "2026-11-03T13:00:00Z");
    let mut input = relanded(std::slice::from_ref(&backed));
    input["landings"]["repositories"][0]["matches"][1]["revision"] = json!(321988);
    let s = snapshot(input.clone());
    assert_eq!(first_waiting(&s).placement.unwrap().reason, Reason::BackedOut(BugId(BUG)));
    let mut other = input["landings"]["repositories"][0].clone();
    other["repository"] = json!("mozilla-central");
    other["backouts"] = json!([]);
    input["landings"]["repositories"][0]["matches"].as_array_mut().unwrap().pop();
    input["landings"]["repositories"].as_array_mut().unwrap().push(other);
    let s = snapshot(input);
    assert_eq!(first_waiting(&s).placement.unwrap().reason, Reason::BackedOut(BugId(BUG)));
}

#[test]
fn bug_numbers_alone_and_other_authors_never_attribute_a_backout() {
    let mut backed = backout(&[BUG], None, '2', "2026-11-03T13:00:00Z");
    backed.as_object_mut().unwrap().remove("targets");
    let s = snapshot(raw(&[landing(321987, '1', "2026-11-03T10:00:00Z", 0)], &[backed]));
    assert_eq!(first_waiting(&s).placement.unwrap().reason, Reason::Needinfo);
}

#[test]
fn either_the_revision_bug_field_or_the_bug_attachment_can_link_a_patch() {
    for link in ["revision", "attachment"] {
        let mut value = raw(
            &[landing(321987, '1', "2026-11-03T10:00:00Z", 0)],
            &[backout(&[BUG], None, '2', "2026-11-03T13:00:00Z")],
        );
        if link == "revision" {
            value["bugRevisions"] = json!({});
        } else {
            value["revisions"][0]["bug"] = Value::Null;
        }
        let s = snapshot(value);
        assert_eq!(first_waiting(&s).placement.unwrap().reason, Reason::BackedOut(BugId(BUG)), "{link}");
    }
}

#[test]
fn a_same_second_backout_is_not_cleared_by_its_original_landing() {
    let s = snapshot(raw(
        &[landing(321987, '1', "2026-11-03T10:00:00Z", 0)],
        &[backout(&[BUG], None, '2', "2026-11-03T10:00:00Z")],
    ));
    assert_eq!(first_waiting(&s).placement.unwrap().reason, Reason::BackedOut(BugId(BUG)));
    assert_eq!(
        s.reason(first_waiting(&s)).to_string(),
        "D321987 backed out of autoland 0m after landing: cause not stated"
    );
}

#[test]
fn evidence_requires_the_affected_commit_repository_and_later_time() {
    for case in ["another-commit", "earlier", "another-tree"] {
        let mut b = backout(&[BUG], None, '2', "2026-11-03T13:00:00Z");
        if case == "another-commit" {
            b["targets"] = json!([hex('5')]);
        } else if case == "earlier" {
            b["pushedAt"] = json!("2026-11-03T09:00:00Z");
        }
        let mut value = raw(&[landing(321987, '1', "2026-11-03T10:00:00Z", 0)], &[b]);
        if case == "another-tree" {
            let mut other = value["landings"]["repositories"][0].clone();
            other["repository"] = json!("mozilla-central");
            other["matches"] = json!([]);
            value["landings"]["repositories"][0]["backouts"] = json!([]);
            value["landings"]["repositories"].as_array_mut().unwrap().push(other);
        }
        let s = snapshot(value);
        assert!(
            workflow(&s, &s.entries[0]).landings[0]
                .summary
                .contains("no attributed later backout"),
            "{case}"
        );
    }
    let s = snapshot(raw(
        &[landing(321987, '1', "2026-11-03T10:00:00Z", 0)],
        &[backout(&[BUG], None, '2', "2026-11-03T10:00:00Z")],
    ));
    assert!(workflow(&s, &s.entries[0]).landings[0].summary.contains("backed out in autoland"));
}

#[test]
fn backout_elapsed_time_has_explicit_minute_hour_and_day_boundaries() {
    for (seconds, expected) in [(0, "0m"), (3599, "59m"), (3600, "1h"), (172799, "47h"), (172800, "2 days")] {
        assert_eq!(worklens_core::backout::Gap(seconds).to_string(), expected);
    }
}

#[test]
fn a_verified_backout_precedes_older_critical_needinfos() {
    let value: Value = serde_json::from_str(include_str!("../../store/fixtures/busy.json")).unwrap();
    let s = snapshot(value);
    let e = first_waiting(&s);
    assert_eq!(s.id(e).to_string(), "2010001");
    assert_eq!(e.placement.unwrap().reason, Reason::BackedOut(BugId(2010001)));
}
