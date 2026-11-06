use jiff::Timestamp;
use serde_json::{Value, json};
use worklens_core::{CoreConfig, Raw, Section, Snapshot, build_snapshot};

fn busy() -> Snapshot {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/busy.json");
    let raw: Raw = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let cfg = CoreConfig {
        whiteboard: Some("[aiplatform]".into()),
        ..CoreConfig::default()
    };
    build_snapshot(raw, &cfg, "2026-11-03T19:00:00Z".parse::<Timestamp>().unwrap())
}

fn landed(s: &Snapshot) -> Vec<(String, String)> {
    s.section(Section::Landed, true)
        .map(|i| s.entry(i))
        .map(|e| (s.id(e).to_string(), s.reason(e).to_string()))
        .collect()
}

#[test]
fn each_train_position_reads_from_the_status_flags_and_the_calendar() {
    let s = busy();
    let rows = landed(&s);
    let today = jiff::civil::date(2026, 11, 3);
    for (id, expected) in [
        (2010105, Some("autoland")),
        (2010101, Some("nightly")),
        (2010102, Some("beta")),
        (2010103, Some("released")),
        (2010104, None),
    ] {
        let bug = s.raw.bugs.iter().find(|b| b.id.0 == id).unwrap();
        assert_eq!(
            worklens_core::train::of_landed(&s.raw, bug, today).map(worklens_core::train::Train::id),
            expected,
            "{id}"
        );
    }
    let reason = |id: &str| rows.iter().find(|(x, _)| x == id).map(|(_, r)| r.as_str());
    assert_eq!(
        reason("2010105"),
        Some("Landing observed on autoland; Nightly inclusion unverified")
    );
    assert_eq!(
        reason("2010101"),
        Some("Bugzilla marks Firefox 147 fixed; build inclusion unverified")
    );
    assert_eq!(
        reason("2010102"),
        Some("Bugzilla marks Firefox 146 fixed; build inclusion unverified"),
        "lowest fixed version wins"
    );
    assert_eq!(
        reason("2010103"),
        Some("Bugzilla marks Firefox 145 fixed; build inclusion unverified")
    );
    assert_eq!(
        reason("2010104"),
        Some("Resolved FIXED; no Firefox status flag set; build inclusion unverified"),
        "no guess from dates"
    );
    assert_eq!(reason("2010106"), None, "fixed 32 days ago, so it has left");
    assert!(s.entries.iter().all(|e| s.id(e).to_string() != "2010106"));
}

#[test]
fn landed_is_newest_first_and_opens_the_bug() {
    assert_eq!(Section::Landed.id(), "landed");
    assert_eq!(Section::Landed.title(), "Landed");
    assert_eq!(Section::Landed.short(), "Landed");
    assert_eq!(Section::Landed.empty(), "No recent fixes or observed landings in this cache.");
    let s = busy();
    let since: Vec<Timestamp> = s
        .section(Section::Landed, true)
        .filter_map(|i| s.entry(i).placement.and_then(|p| p.since))
        .collect();
    assert!(since.len() >= 6, "{since:?}");
    assert!(since.windows(2).all(|w| w[0] >= w[1]), "{since:?}");
    let e = s.entry(s.section(Section::Landed, true).next().unwrap());
    assert_eq!(
        s.next_step(e)
            .and_then(|step| worklens_core::guidance::next_url(&s.raw, step))
            .as_deref(),
        Some("https://bugzilla.mozilla.org/show_bug.cgi?id=2010105")
    );
    assert_eq!(
        s.next_step(e).map(|n| n.to_string()),
        Some(format!("Open bug {} in Bugzilla", s.id(e)))
    );
}

fn fixed_in(id: u32, version: &str) -> Value {
    json!({
        "id": id, "summary": format!("bug {id}"), "status": "RESOLVED", "resolution": "FIXED",
        "assigned_to": "me@mozilla.com", "severity": "S3", "priority": "P3", "keywords": [], "whiteboard": "",
        "see_also": [], "last_change_time": "2026-11-01T12:00:00Z", "cf_last_resolved": "2026-11-01T12:00:00Z",
        "product": "Core", "component": "ML", "via": ["landed"],
        "release_flags": { format!("cf_status_firefox{version}"): "fixed" },
    })
}

fn small(now: &str, bugs: &[Value], issues: &Value) -> Snapshot {
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-02T12:00:00Z",
        "me": { "email": "me@mozilla.com", "jiraEmail": "me@mozilla.com", "phid": "PHID-ME" },
        "bugs": bugs, "issues": issues, "revisions": [], "toReview": [], "bugRevisions": {}, "lastComments": {}, "errors": [],
        "trains": { "nightly": 161, "beta": 160, "release": 159, "esr": [153], "nextMerge": "2026-11-05", "nextRelease": "2026-11-06" },
    }))
    .unwrap();
    build_snapshot(raw, &CoreConfig::default(), now.parse().unwrap())
}

#[test]
fn a_landed_bug_whose_ticket_disagrees_is_still_in_drift() {
    let mut bug = fixed_in(4, "161");
    bug["see_also"] = json!(["https://mozilla-hub.atlassian.net/browse/PROJ-1"]);
    let issue = json!({ "PROJ-1": {
        "key": "PROJ-1", "summary": "ticket", "status": "In Progress", "statusCategory": "indeterminate",
        "assignee": "me@mozilla.com", "updated": "2026-11-01T00:00:00Z", "points": null, "sprint": null,
    }});
    let s = small("2026-11-03T12:00:00Z", &[bug], &issue);
    let drift: Vec<String> = s.section(Section::Drift, true).map(|i| s.id(s.entry(i)).to_string()).collect();
    assert_eq!(drift, ["4"], "the disagreement isn't hidden by Landed");
}

fn on(now: &str) -> Vec<(String, String)> {
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-02T12:00:00Z",
        "me": { "email": "me@mozilla.com", "jiraEmail": null, "phid": "PHID-ME" },
        "bugs": [fixed_in(1, "161"), fixed_in(2, "160"), fixed_in(3, "159")],
        "issues": {}, "revisions": [], "toReview": [], "bugRevisions": {}, "lastComments": {}, "errors": [],
        "trains": { "nightly": 161, "beta": 160, "release": 159, "esr": [153], "nextMerge": "2026-11-05", "nextRelease": "2026-11-06" },
    }))
    .unwrap();
    let mut rows = landed(&build_snapshot(raw, &CoreConfig::default(), now.parse().unwrap()));
    rows.sort();
    rows
}

#[test]
fn merge_day_is_still_before_the_merge_and_the_day_after_is_not() {
    let row = |id: &str, r: &str| (id.to_owned(), r.to_owned());
    assert_eq!(
        on("2026-11-05T23:00:00Z"),
        [
            row("1", "Bugzilla marks Firefox 161 fixed; build inclusion unverified"),
            row("2", "Bugzilla marks Firefox 160 fixed; build inclusion unverified"),
            row("3", "Bugzilla marks Firefox 159 fixed; build inclusion unverified"),
        ]
    );
    assert_eq!(
        on("2026-11-06T01:00:00Z"),
        [
            row("1", "Bugzilla marks Firefox 161 fixed; build inclusion unverified"),
            row("2", "Bugzilla marks Firefox 160 fixed; build inclusion unverified"),
            row("3", "Bugzilla marks Firefox 159 fixed; build inclusion unverified"),
        ]
    );
}

#[test]
fn recent_fixed_status_requires_the_landed_source_a_closed_bug_and_fixed_resolution() {
    for case in ["open", "wontfix", "another-source"] {
        let mut bug = fixed_in(1, "161");
        match case {
            "open" => {
                bug["status"] = json!("ASSIGNED");
                bug["resolution"] = json!("");
            }
            "wontfix" => bug["resolution"] = json!("WONTFIX"),
            _ => bug["via"] = json!(["assigned"]),
        }
        let s = small("2026-11-03T12:00:00Z", &[bug], &json!({}));
        assert_eq!(s.section(Section::Landed, true).count(), 0, "{case}");
    }
}

#[test]
fn the_thirtieth_day_is_included_and_the_next_day_leaves_the_reference_list() {
    let s = small("2026-12-01T12:00:00Z", &[fixed_in(1, "161")], &json!({}));
    assert_eq!(s.section(Section::Landed, true).count(), 1);
    let s = small("2026-12-02T12:00:00Z", &[fixed_in(1, "161")], &json!({}));
    assert_eq!(s.section(Section::Landed, true).count(), 0);
}

#[test]
fn the_release_matrix_is_unavailable_without_the_calendar() {
    let mut raw = busy().raw;
    raw.trains = None;
    let s = build_snapshot(raw, &CoreConfig::default(), "2026-11-03T19:00:00Z".parse().unwrap());
    let e = s.entry(s.section(Section::Landed, true).next().unwrap());
    assert!(!worklens_core::release::shown(&s, e));
}

#[test]
fn another_authors_landing_on_my_bug_does_not_become_my_autoland_reference() {
    let mut raw = busy().raw;
    let rev = raw.revisions.iter_mut().find(|r| r.id.0 == 360105).unwrap();
    rev.author = "PHID-USER-other".into();
    let bug = raw.bugs.iter().find(|b| b.id.0 == 2010105).unwrap();
    assert_eq!(worklens_core::train::on_autoland(&raw, bug), None);
    let s = build_snapshot(raw, &CoreConfig::default(), "2026-11-03T19:00:00Z".parse().unwrap());
    assert!(!s.section(Section::Landed, true).any(|i| s.id(s.entry(i)).to_string() == "2010105"));
}
