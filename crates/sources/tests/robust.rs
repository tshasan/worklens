use jiff::Timestamp;
use proptest::prelude::*;
use serde_json::Value;
use worklens_core::{BugId, RevId};
use worklens_sources::jira::JiraConfig;
use worklens_sources::{JiraAuth, bugzilla, crashstats, jira, landing, phab, telemetry, trains, treeherder};
use worklens_testkit::{Mutation, fixture, mutated, mutations, text};

const FIXTURES: &[&str] = &[
    "bugzilla/bugs.json",
    "bugzilla/comments.json",
    "bugzilla/attachments.json",
    "jira/search.json",
    "jira/epic.json",
    "jira/ideas.json",
    "phabricator/revision.search.json",
    "phabricator/edge.search.json",
    "phabricator/phid.query.json",
    "phabricator/availability-recorded.json",
    "treeherder/revert-recorded.json",
    "treeherder/pushes.json",
    "treeherder/health.json",
    "treeherder/integration-pushes.json",
    "treeherder/recorded-autoland-health.json",
    "treeherder/recorded-autoland-push.json",
    "glean.json",
    "product-details.json",
];

fn cfg() -> JiraConfig {
    JiraConfig {
        site: "https://mozilla-hub.atlassian.net".into(),
        token: String::new(),
        auth: JiraAuth::Bearer,
        points_field: "customfield_10037".into(),
        sprint_field: "customfield_10020".into(),
    }
}

fn now() -> Timestamp {
    "2026-11-03T20:00:00Z".parse().unwrap()
}

fn every_parser(bytes: &[u8]) {
    let _ = bugzilla::parse_bugs(bytes);
    let _ = bugzilla::parse_uplift(bytes);
    let _ = bugzilla::parse_comments(bytes);
    let _ = bugzilla::parse_attachments(bytes);
    let _ = crashstats::parse_total(bytes);
    let _ = jira::parse_search(bytes, &cfg());
    let _ = jira::parse_fields(bytes);
    let _ = phab::parse_revisions(bytes);
    let _ = phab::parse_availability(bytes);
    let _ = phab::parse_parents(bytes, &["PHID-DREV-1".into()]);
    let _ = telemetry::parse_metrics(bytes, "me@mozilla.com", &[BugId(1), BugId(1995008)]);
    let _ = trains::parse_trains(bytes);
    let _ = treeherder::parse_pushes(bytes);
    let _ = treeherder::parse_health(bytes);
    let _ = landing::parse_scan(bytes, "autoland", &[RevId(1), RevId(321987)], now());
    let _ = landing::parse_push_health(bytes, "autoland", "abc");
}

fn seed() -> impl Strategy<Value = (Value, Vec<Mutation>)> {
    (prop::sample::select(FIXTURES), mutations(12)).prop_map(|(name, ms)| (fixture(&format!("crates/sources/fixtures/{name}")), ms))
}

proptest! {
    #![proptest_config(worklens_testkit::config(256))]

    #[test]
    fn parsers_never_panic_on_bytes(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        every_parser(&bytes);
    }

    #[test]
    fn parsers_never_panic_on_mutated_fixtures((value, ms) in seed()) {
        let bytes = serde_json::to_vec(&mutated(&value, &ms)).unwrap();
        every_parser(&bytes);
    }

    #[test]
    fn commit_messages_never_panic(message in text()) {
        let _ = treeherder::d_numbers(&message);
        let _ = treeherder::push_url(&message);
        let _ = landing::push_url(&message, &message);
    }
}
