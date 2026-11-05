use jiff::Timestamp;
use serde_json::{Value, json};
use worklens_core::section::Section;
use worklens_core::{CoreConfig, Raw, Snapshot, build_snapshot};

const ME: &str = "me@mozilla.com";

fn fixed(id: u32, flags: &Value, whiteboard: &str) -> Value {
    let mut b = json!({
        "id": id, "summary": format!("bug {id}"), "status": "RESOLVED", "resolution": "FIXED",
        "assigned_to": ME, "severity": "S3", "priority": "P3", "keywords": [], "whiteboard": whiteboard,
        "see_also": [], "last_change_time": "2026-10-30T12:00:00Z", "product": "Core", "component": "ML",
        "via": ["landed"], "release_flags": {},
    });
    for (k, v) in flags.as_object().unwrap() {
        b["release_flags"][format!("cf_status_firefox{k}")] = v.clone();
    }
    b
}

fn snapshot(bugs: &[Value], uplift: &Value) -> Snapshot {
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-02T12:00:00Z",
        "me": { "email": ME, "jiraEmail": ME, "phid": "PHID-ME" },
        "bugs": bugs, "issues": {}, "revisions": [], "toReview": [], "bugRevisions": {}, "lastComments": {}, "errors": [],
        "trains": { "nightly": 161, "beta": 160, "release": 159, "esr": [153, 115], "nextMerge": "2026-11-05", "nextRelease": "2026-11-06" },
        "uplift": uplift,
        "crashes": { "50": { "count": 7, "since": "2026-10-28" }, "51": { "count": 2, "since": "2026-10-28" } },
    }))
    .unwrap();
    build_snapshot(raw, &CoreConfig::default(), "2026-11-02T12:00:00Z".parse::<Timestamp>().unwrap())
}

fn listed(s: &Snapshot) -> Vec<(String, String, String, String, String)> {
    s.entries
        .iter()
        .map(|e| {
            let section = e.placement.map_or("", |p| p.section.id()).to_owned();
            let next = s.next_step(e).map(|n| n.to_string()).unwrap_or_default();
            (section, s.id(e).to_string(), s.tag(e).to_string(), s.reason(e).to_string(), next)
        })
        .collect()
}

#[test]
fn each_uplift_state_is_one_turn() {
    let patch = |approvals: Value| json!({ "patch": true, "approvals": approvals });
    let s = snapshot(
        &[
            fixed(10, &json!({ "161": "fixed", "160": "affected" }), ""),
            fixed(11, &json!({ "161": "fixed", "160": "affected" }), ""),
            fixed(12, &json!({ "161": "fixed", "160": "affected" }), ""),
            fixed(13, &json!({ "161": "fixed", "160": "fixed" }), ""),
            fixed(14, &json!({ "161": "fixed", "160": "affected", "_esr153": "affected" }), ""),
            fixed(15, &json!({ "161": "fixed", "160": "affected" }), "[checkin-needed-beta]"),
            fixed(16, &json!({ "161": "fixed", "160": "affected" }), ""),
        ],
        &json!({
            "10": patch(json!([])),
            "11": patch(json!([["beta", "+"]])),
            "12": patch(json!([["beta", "?"]])),
            "13": patch(json!([])),
            "14": patch(json!([])),
            "15": patch(json!([])),
            "16": { "patch": false, "approvals": [] },
        }),
    );
    let row =
        |section: &str, id: &str, tag: &str, reason: &str, next: &str| (section.into(), id.into(), tag.into(), reason.into(), next.into());
    let request = "Request an uplift in Lando, or set the status flags if it isn't needed";
    assert_eq!(
        listed(&s),
        vec![
            row(
                "uplift",
                "10",
                "up",
                "fixed in 161; Beta 160 is affected and no uplift was requested",
                request
            ),
            row(
                "land",
                "11",
                "up+",
                "uplift to Beta 160 approved, not landed there yet, 3 days",
                "Land the uplift to Beta 160"
            ),
            row(
                "others",
                "12",
                "up?",
                "uplift to Beta 160 requested, waiting on release management, 3 days",
                ""
            ),
            row(
                "uplift",
                "14",
                "up",
                "fixed in 161; Beta 160 and ESR 153 are affected and no uplift was requested",
                request
            ),
        ],
        "13 is fixed everywhere, 15 is marked checkin-needed-beta, and 16 has no patch to uplift, so they're left out"
    );
    assert_eq!(s.section(Section::Uplift, false).count(), 2);
}

#[test]
fn a_fixed_bug_without_uplift_info_stays_out() {
    let s = snapshot(&[fixed(10, &json!({ "161": "fixed", "160": "affected" }), "")], &json!({}));
    assert!(s.entries.is_empty(), "no uplift info: nothing to say about a fixed bug");
}

#[test]
fn tracked_bugs_are_critical_and_say_for_which_version() {
    let mut bug = fixed(20, &json!({}), "");
    bug["status"] = json!("ASSIGNED");
    bug["resolution"] = json!("");
    bug["via"] = json!(["assigned"]);
    bug["release_flags"] =
        json!({ "cf_tracking_firefox160": "+", "cf_tracking_firefox161": "---", "cf_tracking_firefox_esr153": "blocking" });
    let s = snapshot(&[bug], &json!({}));
    let e = &s.entries[0];
    assert_eq!(e.placement.unwrap().tier, worklens_core::rules::Tier::Critical);
    assert_eq!(s.tracked_for(e), ["160", "esr153"]);
}

#[test]
fn a_regression_of_my_fix_is_waiting_on_me() {
    let mut regression = fixed(30, &json!({}), "");
    regression["status"] = json!("NEW");
    regression["resolution"] = json!("");
    regression["assigned_to"] = json!("nobody@mozilla.org");
    regression["via"] = json!(["regression"]);
    regression["regressed_by"] = json!([10]);
    let s = snapshot(
        &[fixed(10, &json!({ "161": "fixed", "160": "unaffected" }), ""), regression],
        &json!({ "10": { "patch": true, "approvals": [] } }),
    );
    let rows = listed(&s);
    assert_eq!(rows.len(), 1, "my fix itself has nothing to do: {rows:?}");
    assert_eq!(
        rows[0],
        (
            "waiting".into(),
            "30".into(),
            "reg".into(),
            "regression from your fix in bug 10, 3 days".into(),
            "Look at the regression from your fix in bug 10".into()
        )
    );
}

#[test]
fn a_question_i_asked_is_waiting_on_them() {
    let mut bug = fixed(40, &json!({}), "");
    bug["status"] = json!("NEW");
    bug["resolution"] = json!("");
    bug["assigned_to"] = json!("nobody@mozilla.org");
    bug["via"] = json!(["asked"]);
    bug["flags"] = json!([
        { "name": "needinfo", "status": "?", "requestee": "alice@mozilla.com", "setter": ME, "creation_date": "2026-10-23T12:00:00Z" },
    ]);
    let s = snapshot(&[bug], &json!({}));
    let rows = listed(&s);
    assert_eq!(
        rows,
        vec![(
            "others".into(),
            "40".into(),
            "ni>".into(),
            "you asked alice@mozilla.com, no answer for 10 days".into(),
            "Follow up on your question, or ask someone else".into()
        )]
    );
    assert!(s.entries[0].placement.unwrap().stale);
}

#[test]
fn a_fix_that_still_crashes_is_waiting_on_me() {
    let s = snapshot(&[fixed(50, &json!({}), ""), fixed(51, &json!({}), "")], &json!({}));
    assert_eq!(
        listed(&s),
        vec![(
            "waiting".into(),
            "50".into(),
            "cr".into(),
            "still crashing after your fix: 7 Nightly crashes in builds since Oct 28".into(),
            "Check whether the fix worked: look at the crashes on crash-stats".into()
        )],
        "2 crashes is under BugBot's threshold of 5"
    );
}

#[test]
fn crash_signatures_are_split() {
    let mut bug = fixed(1, &json!({}), "");
    bug["cf_crash_signature"] = json!("[@ a::b ]\r\n[@ <c as d>::e]");
    let bug: worklens_core::raw::Bug = serde_json::from_value(bug).unwrap();
    assert_eq!(worklens_core::bugs::crash_signatures(&bug), ["a::b", "<c as d>::e"]);
}

#[test]
fn a_fix_with_no_beta_status_needs_one_set() {
    let s = snapshot(
        &[fixed(60, &json!({ "161": "fixed", "160": "---" }), "")],
        &json!({ "60": { "patch": true, "approvals": [] } }),
    );
    assert_eq!(
        listed(&s),
        vec![(
            "uplift".into(),
            "60".into(),
            "st?".into(),
            "fixed in 161; the Beta 160 status isn't set, so nobody can tell if it needs an uplift".into(),
            "Set status-firefox160 to affected or unaffected".into()
        )]
    );
}

#[test]
fn the_comment_that_came_with_a_needinfo_is_found() {
    let mut bug = fixed(70, &json!({}), "");
    bug["status"] = json!("NEW");
    bug["resolution"] = json!("");
    bug["via"] = json!(["needinfo"]);
    bug["flags"] =
        json!([{ "name": "needinfo", "status": "?", "requestee": ME, "setter": "a@mozilla.com", "creation_date": "2026-10-30T10:00:30Z" }]);
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-02T12:00:00Z", "me": { "email": ME, "jiraEmail": ME, "phid": "PHID-ME" },
        "bugs": [bug], "issues": {}, "revisions": [], "toReview": [], "bugRevisions": {}, "lastComments": {}, "errors": [],
        "threads": { "70": { "total": 2, "comments": [
            { "author": "x@mozilla.com", "time": "2026-10-01T00:00:00Z", "text": "description" },
            { "author": "a@mozilla.com", "time": "2026-10-30T10:00:00Z", "text": "Can you check this?" },
        ] } },
    }))
    .unwrap();
    let s = build_snapshot(raw, &CoreConfig::default(), "2026-11-02T12:00:00Z".parse::<Timestamp>().unwrap());
    assert_eq!(
        s.needinfo_comment(&s.entries[0]).map(|c| c.text.as_str()),
        Some("Can you check this?")
    );
}

#[test]
fn group_reviews_wait_on_me_after_direct_ones() {
    let mut bug = fixed(80, &json!({}), "");
    bug["status"] = json!("NEW");
    bug["resolution"] = json!("");
    bug["via"] = json!(["revision"]);
    let rev = |id: u32, bug: Option<u32>, days: u32| {
        json!({
            "id": id, "title": format!("D{id}"), "status": "needs-review", "author": "PHID-T", "bug": bug,
            "modified": format!("2026-10-{:02}T12:00:00Z", 30 - days), "reviewers": [],
        })
    };
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-02T12:00:00Z", "me": { "email": ME, "jiraEmail": ME, "phid": "PHID-ME" },
        "bugs": [bug], "issues": {}, "toReview": [91], "bugRevisions": {}, "lastComments": {}, "errors": [],
        "revisions": [rev(90, Some(80), 9), rev(91, None, 1)],
        "users": { "PHID-T": "them", "PHID-PROJ": "#ai-platform-reviewers" },
        "groupReview": { "90": "PHID-PROJ" },
    }))
    .unwrap();
    let s = build_snapshot(raw, &CoreConfig::default(), "2026-11-02T12:00:00Z".parse::<Timestamp>().unwrap());
    let waiting: Vec<(String, String, String)> = s
        .section(Section::Waiting, false)
        .map(|i| s.entry(i))
        .map(|e| (s.id(e).to_string(), s.tag(e).to_string(), s.reason(e).to_string()))
        .collect();
    assert_eq!(
        waiting,
        [
            ("D91".into(), "r?".into(), "review requested on D91 by them, 4 days".into()),
            (
                "80".into(),
                "r?#".into(),
                "review requested of #ai-platform-reviewers on D90 by them, 12 days".into()
            ),
        ],
        "the direct review comes first though the group one is older"
    );
}

fn needs_uplift(s: &Snapshot) -> Vec<String> {
    s.section(Section::Uplift, true).map(|i| s.id(s.entry(i)).to_string()).collect()
}

#[test]
fn a_needinfo_on_the_assignee_holds_the_uplift() {
    let mut asked = fixed(10, &json!({ "161": "fixed", "160": "affected" }), "");
    asked["assigned_to"] = json!("dev@mozilla.com");
    asked["flags"] = json!([
        { "name": "needinfo", "status": "?", "requestee": "dev@mozilla.com", "setter": "qa@mozilla.com", "creation_date": "2026-10-31T12:00:00Z" },
    ]);
    let mut elsewhere = fixed(11, &json!({ "161": "fixed", "160": "affected" }), "");
    elsewhere["flags"] = json!([
        { "name": "needinfo", "status": "?", "requestee": "qa@mozilla.com", "setter": "dev@mozilla.com", "creation_date": "2026-10-31T12:00:00Z" },
    ]);
    let patch = json!({ "patch": true, "approvals": [] });
    let s = snapshot(&[asked, elsewhere], &json!({ "10": patch, "11": patch }));
    assert_eq!(
        needs_uplift(&s),
        ["11"],
        "10's assignee has an open question; 11's needinfo is on someone else"
    );
}

#[test]
fn an_open_regression_holds_the_uplift() {
    let regression = |id: u32, of: u32, open: bool| {
        let mut b = fixed(id, &json!({}), "");
        if open {
            b["status"] = json!("NEW");
            b["resolution"] = json!("");
        }
        b["assigned_to"] = json!("nobody@mozilla.org");
        b["via"] = json!(["regression"]);
        b["regressed_by"] = json!([of]);
        b
    };
    let patch = json!({ "patch": true, "approvals": [] });
    let s = snapshot(
        &[
            fixed(10, &json!({ "161": "fixed", "160": "affected" }), ""),
            fixed(11, &json!({ "161": "fixed", "160": "affected" }), ""),
            regression(30, 10, true),
            regression(31, 11, false),
        ],
        &json!({ "10": patch, "11": patch }),
    );
    assert_eq!(needs_uplift(&s), ["11"], "10 caused open bug 30; 11's regression 31 is fixed");
    let waiting: Vec<String> = s.section(Section::Waiting, true).map(|i| s.id(s.entry(i)).to_string()).collect();
    assert_eq!(waiting, ["30"], "the regression itself is still mine to look at");
}

#[test]
fn only_supported_esrs_need_uplifts() {
    let patch = json!({ "patch": true, "approvals": [] });
    let s = snapshot(
        &[
            fixed(10, &json!({ "161": "fixed", "160": "unaffected", "_esr140": "affected" }), ""),
            fixed(11, &json!({ "161": "fixed", "160": "unaffected", "_esr153": "affected" }), ""),
        ],
        &json!({ "10": patch, "11": patch }),
    );
    assert_eq!(needs_uplift(&s), ["11"], "ESR 140 isn't a supported train, so 10 has nowhere to go");
    let e = s.entries.iter().find(|e| s.id(e).to_string() == "11").unwrap();
    assert_eq!(
        s.reason(e).to_string(),
        "fixed in 161; ESR 153 is affected and no uplift was requested"
    );
}

#[test]
fn the_brief_shows_evidence_where_it_decides_the_next_step() {
    let mut asked = fixed(70, &json!({}), "");
    asked["status"] = json!("NEW");
    asked["resolution"] = json!("");
    asked["via"] = json!(["needinfo"]);
    asked["flags"] =
        json!([{ "name": "needinfo", "status": "?", "requestee": ME, "setter": "a@mozilla.com", "creation_date": "2026-10-30T10:00:30Z" }]);
    let s = snapshot(
        &[fixed(10, &json!({ "161": "fixed", "160": "affected" }), ""), asked],
        &json!({ "10": { "patch": true, "approvals": [] } }),
    );
    let brief = |id: &str| {
        let e = s.entries.iter().find(|e| s.id(e).to_string() == id).unwrap();
        worklens_core::brief::render(&s, e)
    };
    let heading = "## Approvals, fix presence, and telemetry";

    let uplift = brief("10");
    assert!(uplift.contains(heading), "{uplift}");
    assert!(uplift.contains("- Beta 160: Bugzilla reports affected"), "{uplift}");
    assert!(
        uplift.contains("- Nightly 161: Bugzilla reports fixed"),
        "the channels decide the uplift: {uplift}"
    );
    assert!(uplift.contains("\n- Unknown: telemetry\n"), "limitations on one line: {uplift}");
    assert!(!uplift.contains("Glean definitions not fetched"), "{uplift}");

    let needinfo = brief("70");
    assert!(!needinfo.contains(heading), "nothing known that bears on a needinfo: {needinfo}");
}
