use serde_json::{Value, json};
use worklens_core::{CoreConfig, Raw, Section, build_snapshot};

fn snapshot(reviewers: &Value, away: &Value, errors: &Value) -> worklens_core::Snapshot {
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-03T12:00:00Z",
        "me": {"email": "me@example.com", "phid": "PHID-ME"},
        "bugs": [], "issues": {}, "toReview": [], "bugRevisions": {}, "lastComments": {},
        "revisions": [{"id": 1, "title": "A review", "author": "PHID-ME", "status": "needs-review",
            "modified": "2026-11-02T12:00:00Z", "reviewers": reviewers}],
        "away": away, "errors": errors, "users": {"PHID-USER-a":"Alice", "PHID-USER-b":"Bob"}
    }))
    .unwrap();
    build_snapshot(raw, &CoreConfig::default(), "2026-11-03T12:00:00Z".parse().unwrap())
}

fn check(reviewers: &Value, away: &Value, errors: &Value, expected: bool) {
    let s = snapshot(reviewers, away, errors);
    let e = s.entry(s.section(Section::Others, false).next().unwrap());
    assert_eq!(s.reason(e).to_string().contains("away until Nov 18"), expected, "{}", s.reason(e));
    let suggestion = s.next_step(e).map(|step| step.to_string()).unwrap_or_default();
    assert_eq!(suggestion.contains("Consider another reviewer"), expected, "{suggestion}");
    if expected {
        assert_eq!(
            s.next_step(e)
                .and_then(|step| worklens_core::guidance::next_url(&s.raw, step))
                .as_deref(),
            Some("https://phabricator.services.mozilla.com/D1")
        );
    }
}

#[test]
fn confirmed_away_is_a_fact_and_an_optional_suggestion() {
    check(
        &json!([{"phid":"PHID-USER-a","status":"added"}]),
        &json!({"PHID-USER-a":"2026-11-18T12:00:00Z"}),
        &json!([]),
        true,
    );
}

#[test]
fn mixed_unknown_groups_and_expired_lookups_do_not_claim_everyone_is_away() {
    let away = json!({"PHID-USER-a":"2026-11-18T12:00:00Z"});
    for other in ["PHID-USER-b", "PHID-PROJ-group"] {
        check(
            &json!([{"phid":"PHID-USER-a","status":"added"},{"phid":other,"status":"added"}]),
            &away,
            &json!([]),
            false,
        );
    }
    let reviewer = json!([{"phid":"PHID-USER-a","status":"added"}]);
    check(&reviewer, &json!({}), &json!([]), false);
    check(&reviewer, &json!({"PHID-USER-a":"2026-11-03T12:00:00Z"}), &json!([]), false);
    check(
        &reviewer,
        &away,
        &json!([{"source":"phabricator-availability","message":"lookup failed"}]),
        false,
    );
}

#[test]
fn blocking_reviewers_are_required_even_when_an_optional_reviewer_is_available() {
    check(
        &json!([{"phid":"PHID-USER-a","status":"blocking"},{"phid":"PHID-USER-b","status":"added"}]),
        &json!({"PHID-USER-a":"2026-11-18T12:00:00Z"}),
        &json!([]),
        true,
    );
    check(
        &json!([{"phid":"PHID-USER-a","status":"added"},{"phid":"PHID-USER-b","status":"blocking"}]),
        &json!({"PHID-USER-a":"2026-11-18T12:00:00Z"}),
        &json!([]),
        false,
    );
    check(
        &json!([{"phid":"PHID-USER-a","status":"blocking"},{"phid":"PHID-PROJ-group","status":"blocking"}]),
        &json!({"PHID-USER-a":"2026-11-18T12:00:00Z"}),
        &json!([]),
        false,
    );
}

#[test]
fn unrelated_source_errors_do_not_hide_confirmed_availability() {
    let reviewer = json!([{"phid":"PHID-USER-a","status":"added"}]);
    let away = json!({"PHID-USER-a":"2026-11-18T12:00:00Z"});
    check(
        &reviewer,
        &away,
        &json!([{"source":"phabricator","message":"lookup failed"}]),
        false,
    );
    check(
        &reviewer,
        &away,
        &json!([{"source":"jira","message":"not configured","unset":true}]),
        true,
    );
}

#[test]
fn multiple_away_reviewers_are_named_and_separated_without_a_leading_delimiter() {
    let s = snapshot(
        &json!([{"phid":"PHID-USER-a","status":"added"},{"phid":"PHID-USER-b","status":"added"}]),
        &json!({"PHID-USER-a":"2026-11-18T12:00:00Z","PHID-USER-b":"2026-11-19T12:00:00Z"}),
        &json!([]),
    );
    let e = s.entry(s.section(Section::Others, false).next().unwrap());
    assert_eq!(
        s.reason(e).to_string(),
        "D1 waiting on Alice, away until Nov 18; Bob, away until Nov 19"
    );
}
