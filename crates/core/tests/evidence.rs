use serde_json::{Value, json};
use worklens_core::evidence::{metric, workflow};
use worklens_core::{CoreConfig, Raw, Snapshot, build_snapshot};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../store/fixtures/raw.json")).unwrap()
}

fn snapshot(raw: Value) -> Snapshot {
    build_snapshot(
        serde_json::from_value::<Raw>(raw).unwrap(),
        &CoreConfig::default(),
        "2026-11-03T20:00:00Z".parse().unwrap(),
    )
}

#[test]
fn old_cache_does_not_claim_approval_or_landing() {
    let s = snapshot(fixture());
    let w = workflow(&s, &s.entries[0]);
    assert!(w.approvals.iter().any(|v| v.state == "unknown" && v.label.ends_with("data review")));
    assert!(w.approvals.iter().any(|v| v.state == "unconfirmed"));
    assert!(w.limitations.iter().any(|v| v.contains("telemetry coverage unknown")));
}

#[test]
fn accepted_revision_with_classification_requirement_is_not_cleared() {
    let mut raw = fixture();
    raw["revisions"][0]["status"] = json!("accepted");
    raw["revisions"][0]["projects"] = json!(["PHID-PROJ-data"]);
    raw["users"]["PHID-PROJ-data"] = json!("#needs-data-classification");
    let s = snapshot(raw);
    let w = workflow(&s, &s.entries[0]);
    assert!(w.approvals.iter().any(|v| v.state == "required"));
    assert!(w.approvals.iter().any(|v| v.state == "unconfirmed"));
    assert!(w.approvals.iter().all(|v| v.url.contains("D321987")));
}

#[test]
fn channel_fixed_and_uplift_approval_do_not_confirm_commits_or_shipment() {
    let mut raw = fixture();
    raw["trains"] = json!({"nightly":161,"beta":160,"release":159,"esr":[153],"nextMerge":"2026-11-05","nextRelease":"2026-11-06"});
    raw["bugs"][0]["release_flags"] =
        json!({"cf_status_firefox161":"fixed","cf_status_firefox160":"affected","cf_status_firefox_esr153":"unaffected"});
    raw["uplift"] = json!({"1995008":{"patch":true,"approvals":[["beta","+"],["beta","?"]]}});
    raw["revisions"][0]["status"] = json!("published");
    let s = snapshot(raw);
    let w = workflow(&s, &s.entries[0]);
    assert_eq!(
        w.channels.iter().map(|v| v.state).collect::<Vec<_>>(),
        ["reported_fixed", "affected", "unknown", "unaffected"]
    );
    assert!(w.channels[0].summary.contains("flag alone does not verify"));
    assert!(w.approvals.iter().any(|v| v.state == "reported_closed"));
    assert!(w.approvals.iter().any(|v| v.summary.contains("+, ?")));
}

fn with_landings() -> Value {
    let mut raw = fixture();
    raw["landings"] = json!({"revisions":[321987],"repositories":[{
        "repository":"autoland","checkedAt":"2026-11-03T19:00:00Z","pushLimit":100,"scannedPushes":100,
        "oldestPush":"2026-11-02T00:00:00Z","error":null,"matches":[{
            "revision":321987,"commit":"1111111111111111111111111111111111111111",
            "pushRevision":"2222222222222222222222222222222222222222",
            "url":"https://treeherder.mozilla.org/jobs?repo=autoland&revision=2222222222222222222222222222222222222222",
            "pushedAt":"2026-11-03T10:00:00Z","healthError":null,
            "health":{"builds":"pass","lint":"pass","tests":"pass","failed":0,"running":0}
        }]
    }]});
    raw
}

#[test]
fn observed_commit_and_health_are_distinct_from_current_presence_and_shipment() {
    let s = snapshot(with_landings());
    let w = workflow(&s, &s.entries[0]);
    assert_eq!(w.landings[0].state, "observed_landing_passed");
    assert!(w.landings[0].summary.contains("1111111111111111111111111111111111111111"));
    assert!(w.landings[0].summary.contains("later backouts and shipped builds unverified"));
    assert!(w.approvals.iter().any(|v| v.summary.contains("integration-push observations")));
}

#[test]
fn missing_results_failure_old_diff_and_missing_jobs_cannot_become_green() {
    for (change, want) in [
        ("no match", "not_observed"),
        ("failure", "lookup_failed"),
        ("old diff", "older_diff_landing"),
        ("no jobs", "observed_landing"),
        ("running", "observed_landing_running"),
        ("failed jobs", "observed_landing_failed"),
    ] {
        let mut raw = with_landings();
        match change {
            "no match" => raw["landings"]["repositories"][0]["matches"] = json!([]),
            "failure" => raw["landings"]["repositories"][0]["error"] = json!("source unavailable"),
            "old diff" => raw["revisions"][0]["diffCreated"] = json!("2026-11-03T12:00:00Z"),
            "no jobs" => raw["landings"]["repositories"][0]["matches"][0]["health"] = Value::Null,
            "running" => raw["landings"]["repositories"][0]["matches"][0]["health"]["running"] = json!(1),
            _ => raw["landings"]["repositories"][0]["matches"][0]["health"]["failed"] = json!(1),
        }
        let s = snapshot(raw);
        let w = workflow(&s, &s.entries[0]);
        assert_eq!(w.landings[0].state, want, "{change}");
    }
}

#[test]
fn expiry_is_version_specific_and_missing_trains_are_unknown() {
    let mut raw = fixture();
    raw["trains"] = json!({"nightly":161,"beta":160,"release":159,"esr":[],"nextMerge":"2026-11-05","nextRelease":"2026-11-06"});
    raw["telemetry"] = json!({"fetchedAt":"2026-11-03T19:00:00Z","metrics":[{
        "name":"example.counter","expires":"161","disabled":false,"notificationEmails":["me@mozilla.com"],
        "bugs":[1995008],"dataReviews":["https://bugzilla.mozilla.org/show_bug.cgi?id=42"],
        "sourceUrl":null,"definitionLastSeen":"2026-11-02 00:00:00"
    }]});
    let mut s = snapshot(raw);
    let report = s.raw.telemetry.as_ref().unwrap();
    let evidence = metric(&s, &report.metrics[0], report.fetched_at);
    assert_eq!(evidence.state, "expired_on_nightly");
    assert!(evidence.summary.contains("older channels may still collect"));
    assert_eq!(workflow(&s, &s.entries[0]).telemetry.len(), 1);
    s.raw.trains = None;
    let report = s.raw.telemetry.as_ref().unwrap();
    assert_eq!(metric(&s, &report.metrics[0], report.fetched_at).state, "unknown");
}
