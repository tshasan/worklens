use std::collections::{BTreeMap, HashSet};

use futures::{StreamExt, stream};
use jiff::Timestamp;
use serde::Deserialize;
use worklens_core::raw::{Backout, Landing, Landings, PushHealth, RepositoryLandings};
use worklens_core::{BugId, RevId};

use crate::MAX_IN_FLIGHT;
use crate::http::{Error, Http, Result, parse};
use crate::treeherder::{TREEHERDER, d_numbers, parse_health};

pub const PUSH_LIMIT: usize = 100;
const HEALTH_LIMIT: usize = 4;
const REPOSITORIES: [&str; 2] = ["autoland", "mozilla-central"];

#[derive(Deserialize)]
struct Page {
    meta: Meta,
    results: Vec<Push>,
}

#[derive(Deserialize)]
struct Meta {
    repository: String,
}

#[derive(Deserialize)]
struct Push {
    revision: String,
    push_timestamp: i64,
    revisions: Vec<Commit>,
}

#[derive(Deserialize)]
struct Commit {
    revision: String,
    comments: String,
}

fn hash(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_backout(subject: &str) -> bool {
    let subject = subject.trim_start().to_ascii_lowercase();
    subject.starts_with("backed out") || subject.starts_with("backout") || subject.starts_with("revert")
}

fn bugs_named(subject: &str) -> Vec<BugId> {
    let lower = subject.to_ascii_lowercase();
    let mut bugs = Vec::new();
    let mut rest = lower.as_str();
    while let Some(at) = rest.find("bug") {
        rest = &rest[at + 3..];
        let after = rest.strip_prefix('s').unwrap_or(rest).trim_start_matches([' ', '#']);
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(id) = digits.parse::<u32>()
            && id > 0
            && !bugs.contains(&BugId(id))
        {
            bugs.push(BugId(id));
        }
    }
    bugs
}

fn cause_of(subject: &str) -> Option<String> {
    let (_, cause) = subject.split_once(" for ")?;
    let cause = cause.trim();
    (!cause.is_empty()).then(|| cause.to_owned())
}

fn backout(message: &str) -> Option<(Vec<BugId>, Option<String>)> {
    let subject = message.lines().next()?;
    if !is_backout(subject) {
        return None;
    }
    let bugs = bugs_named(subject);
    (!bugs.is_empty() || !targets(message).is_empty()).then(|| (bugs, cause_of(subject)))
}

fn targets(message: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for line in message.lines() {
        let text = line.trim();
        let candidate = text
            .strip_prefix("This reverts commit ")
            .or_else(|| text.strip_prefix("Backed out changeset "))
            .or_else(|| text.strip_prefix("Backout changeset "));
        if let Some(candidate) = candidate {
            let value: String = candidate.chars().take_while(char::is_ascii_hexdigit).collect();
            if (12..=40).contains(&value.len()) {
                targets.push(value.to_ascii_lowercase());
            }
        }
    }
    targets
}

fn trailer(message: &str) -> Option<RevId> {
    if is_backout(message.lines().next()?) {
        return None;
    }
    let mut ids = message.lines().filter_map(|line| {
        let (label, _) = line.split_once(':')?;
        if !label.eq_ignore_ascii_case("Differential Revision") {
            return None;
        }
        let ids = d_numbers(line);
        (ids.len() == 1).then(|| ids[0])
    });
    let id = ids.next()?;
    ids.next().is_none().then_some(id)
}

pub fn parse_scan(bytes: &[u8], repository: &str, wanted: &[RevId], now: Timestamp) -> Result<RepositoryLandings> {
    let mut page: Page = parse("Treeherder integration pushes", bytes)?;
    if page.meta.repository != repository {
        return Err(Error::Api("Treeherder returned the wrong integration repository".into()));
    }
    if page.results.len() > PUSH_LIMIT {
        return Err(Error::Api("Treeherder exceeded the requested integration push limit".into()));
    }
    let scanned_pushes = page.results.len();
    page.results.sort_by_key(|p| std::cmp::Reverse(p.push_timestamp));
    let oldest_push = page
        .results
        .last()
        .map(|p| Timestamp::from_second(p.push_timestamp))
        .transpose()
        .map_err(|e| Error::Api(format!("Treeherder push timestamp: {e}")))?;
    let wanted: HashSet<RevId> = wanted.iter().copied().collect();
    let mut matches = BTreeMap::new();
    let mut backouts = Vec::new();
    for push in page.results {
        if !hash(&push.revision) {
            return Err(Error::Api("Treeherder push has no full commit hash".into()));
        }
        let pushed_at = Timestamp::from_second(push.push_timestamp).map_err(|e| Error::Api(format!("Treeherder push timestamp: {e}")))?;
        for commit in push.revisions {
            if let Some((bugs, cause)) = backout(&commit.comments)
                && hash(&commit.revision)
            {
                backouts.push(Backout {
                    bugs,
                    targets: targets(&commit.comments),
                    cause,
                    commit: commit.revision,
                    push_revision: push.revision.clone(),
                    url: push_url(repository, &push.revision),
                    pushed_at,
                });
                continue;
            }
            let Some(id) = trailer(&commit.comments).filter(|id| wanted.contains(id)) else {
                continue;
            };
            if !hash(&commit.revision) {
                return Err(Error::Api("Treeherder matched commit has no full hash".into()));
            }
            matches.entry((id, commit.revision.clone())).or_insert_with(|| Landing {
                revision: id,
                commit: commit.revision,
                push_revision: push.revision.clone(),
                pushed_at,
                url: push_url(repository, &push.revision),
                health: None,
                health_error: Some("Push health not fetched: bounded request budget".into()),
            });
        }
    }
    Ok(RepositoryLandings {
        retry_at: None,
        repository: repository.into(),
        checked_at: now,
        push_limit: PUSH_LIMIT,
        scanned_pushes,
        oldest_push,
        error: None,
        matches: matches.into_values().collect(),
        backouts,
    })
}

async fn scan(http: &Http, repository: &str, wanted: &[RevId]) -> RepositoryLandings {
    let response = async {
        let req = http
            .client
            .get(format!("{TREEHERDER}/api/project/{repository}/push/"))
            .query(&[("count", PUSH_LIMIT)]);
        parse_scan(&http.send(req).await?, repository, wanted, Timestamp::now())
    }
    .await;
    response.unwrap_or_else(|error| RepositoryLandings {
        retry_at: error.retry_at(),
        repository: repository.into(),
        checked_at: Timestamp::now(),
        push_limit: PUSH_LIMIT,
        scanned_pushes: 0,
        oldest_push: None,
        error: Some(error.summary()),
        matches: Vec::new(),
        backouts: Vec::new(),
    })
}

pub fn parse_push_health(bytes: &[u8], repository: &str, revision: &str) -> Result<Option<PushHealth>> {
    #[derive(Deserialize)]
    struct Identity {
        repository: String,
        revision: String,
    }
    let ids: Vec<Identity> = parse("Treeherder integration health identity", bytes)?;
    if ids.is_empty() {
        return Ok(None);
    }
    if ids.len() != 1 || ids[0].repository != repository || ids[0].revision != revision {
        return Err(Error::Api("Treeherder returned health for a different push or repository".into()));
    }
    Ok(parse_health(bytes)?.map(|h| PushHealth {
        builds: h.builds,
        lint: h.lint,
        tests: h.tests,
        failed: h.failed,
        running: h.running,
    }))
}

pub async fn fetch(http: &Http, revisions: &[RevId]) -> Landings {
    let (autoland, central) = futures::join!(scan(http, REPOSITORIES[0], revisions), scan(http, REPOSITORIES[1], revisions));
    let mut repositories = vec![autoland, central];
    let mut jobs = BTreeMap::new();
    for (index, repo) in repositories.iter().enumerate() {
        for landing in &repo.matches {
            jobs.entry((index, landing.push_revision.clone())).or_insert(landing.pushed_at);
        }
    }
    let mut jobs: Vec<_> = jobs.into_iter().collect();
    jobs.sort_by_key(|(_, time)| std::cmp::Reverse(*time));
    jobs.truncate(HEALTH_LIMIT);
    let results: Vec<_> = stream::iter(jobs)
        .map(|((index, revision), _)| async move {
            let repository = REPOSITORIES[index];
            let result = async {
                let req = http
                    .client
                    .get(format!("{TREEHERDER}/api/project/{repository}/push/health_summary/"))
                    .query(&[("revision", revision.as_str()), ("with_in_progress_tests", "true")]);
                parse_push_health(&http.send(req).await?, repository, &revision)
            }
            .await;
            (index, revision, result)
        })
        .buffer_unordered(MAX_IN_FLIGHT)
        .collect()
        .await;
    for (index, revision, result) in results {
        let (health, error) = match result {
            Ok(Some(health)) => (Some(health), None),
            Ok(None) => (None, Some("No push health result available".into())),
            Err(error) => (None, Some(error.summary())),
        };
        for landing in repositories[index].matches.iter_mut().filter(|m| m.push_revision == revision) {
            landing.health = health;
            landing.health_error.clone_from(&error);
        }
    }
    for repository in &mut repositories {
        repository.checked_at = Timestamp::now();
    }
    let mut revisions = revisions.to_vec();
    revisions.sort_unstable();
    revisions.dedup();
    Landings { revisions, repositories }
}

pub fn push_url(repository: &str, revision: &str) -> String {
    format!("{TREEHERDER}/jobs?repo={repository}&revision={revision}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use worklens_core::raw::TryResult;

    fn now() -> Timestamp {
        "2026-11-03T20:00:00Z".parse().unwrap()
    }

    #[test]
    fn an_explicit_reverted_commit_can_identify_a_patch_without_a_bug_in_the_subject() {
        let commit = "1".repeat(40);
        let backout = "2".repeat(40);
        let page = serde_json::json!({"meta":{"repository":"autoland"},"results":[
            {"revision": backout, "push_timestamp":1793710800, "revisions":[{"revision":backout,
                "comments":format!("Revert a changeset for failures\n\nThis reverts commit {commit}.\n")}]},
            {"revision":commit,"push_timestamp":1793700000,"revisions":[{"revision":commit,
                "comments":"Bug 1995008 - A patch\n\nDifferential Revision: https://phabricator.services.mozilla.com/D321987\n"}]}
        ]});
        let scan = parse_scan(&serde_json::to_vec(&page).unwrap(), "autoland", &[RevId(321987)], now()).unwrap();
        assert_eq!(scan.backouts.len(), 1);
        assert!(scan.backouts[0].bugs.is_empty());
        assert_eq!(scan.backouts[0].targets, [commit]);
        let mut raw: worklens_core::Raw = serde_json::from_str(include_str!("../../store/fixtures/raw.json")).unwrap();
        raw.revisions[0].status = worklens_core::raw::RevStatus::Published;
        raw.landings = Some(Landings {
            revisions: vec![RevId(321987)],
            repositories: vec![scan],
        });
        let s = worklens_core::build_snapshot(raw, &worklens_core::CoreConfig::default(), now());
        assert!(s.entries.iter().any(|e| {
            e.placement
                .is_some_and(|p| p.reason == worklens_core::rules::Reason::BackedOut(BugId(1995008)))
        }));
    }

    #[test]
    fn a_zero_bug_number_is_not_a_backout_candidate() {
        let mut response: serde_json::Value =
            serde_json::from_slice(include_bytes!("../fixtures/treeherder/revert-recorded.json")).unwrap();
        response["results"][0]["revisions"][0]["comments"] = serde_json::json!("Backout bug 0");
        let scan = parse_scan(&serde_json::to_vec(&response).unwrap(), "autoland", &[], now()).unwrap();
        assert!(scan.backouts.is_empty());
    }

    #[test]
    fn recorded_revert_names_the_affected_commit() {
        let scan = parse_scan(
            include_bytes!("../fixtures/treeherder/revert-recorded.json"),
            "autoland",
            &[],
            now(),
        )
        .unwrap();
        assert_eq!(scan.backouts.len(), 1);
        assert_eq!(scan.backouts[0].targets, ["dad5f0c681918eb9ca90dd155c1cdcdbdb4bfeb6"]);
        assert!(scan.matches.is_empty());
    }

    #[test]
    fn exact_trailers_find_stack_commits_without_using_the_push_tip() {
        let bytes = include_bytes!("../fixtures/treeherder/integration-pushes.json");
        let scan = parse_scan(bytes, "autoland", &[RevId(123), RevId(124), RevId(125)], now()).unwrap();
        assert_eq!(scan.scanned_pushes, 4);
        assert_eq!(scan.matches.len(), 3, "both commits of D123 and the stack top remain available");
        assert_eq!(scan.matches.iter().filter(|m| m.revision == RevId(123)).count(), 2);
        assert!(!scan.matches.iter().any(|m| m.revision == RevId(125)));
        let found = scan.matches.iter().find(|m| m.revision == RevId(123)).unwrap();
        assert_eq!(found.commit, "1".repeat(40));
        assert_eq!(found.push_revision, "e".repeat(40));
        assert!(found.url.contains("repo=autoland"));
        assert!(found.health.is_none());
    }

    #[test]
    fn wrong_repository_and_bad_commit_hashes_fail_instead_of_matching() {
        let bytes = include_bytes!("../fixtures/treeherder/integration-pushes.json");
        assert!(parse_scan(bytes, "mozilla-central", &[RevId(123)], now()).is_err());
        let mut page: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        page["results"][2]["revisions"][1]["revision"] = serde_json::json!("short");
        assert!(parse_scan(&serde_json::to_vec(&page).unwrap(), "autoland", &[RevId(123)], now()).is_err());
    }

    #[test]
    fn health_requires_exact_repository_and_push_identity() {
        let mut health: serde_json::Value = serde_json::from_slice(include_bytes!("../fixtures/treeherder/health.json")).unwrap();
        health[0]["repository"] = serde_json::json!("autoland");
        let bytes = serde_json::to_vec(&health).unwrap();
        let parsed = parse_push_health(&bytes, "autoland", "newer").unwrap().unwrap();
        assert_eq!(parsed.tests, TryResult::Fail);
        assert_eq!(parsed.running, 3);
        assert!(parse_push_health(&bytes, "mozilla-central", "newer").is_err());
        assert!(parse_push_health(&bytes, "autoland", "another-push").is_err());
        assert!(parse_push_health(b"[]", "autoland", "newer").unwrap().is_none());
    }

    #[test]
    fn backouts_are_kept_with_the_bugs_and_cause_their_subject_names() {
        let bytes = include_bytes!("../fixtures/treeherder/integration-backouts.json");
        let scan = parse_scan(bytes, "autoland", &[RevId(321987)], now()).unwrap();
        let found: Vec<(Vec<u32>, Option<&str>, i64)> = scan
            .backouts
            .iter()
            .map(|b| (b.bugs.iter().map(|x| x.0).collect(), b.cause.as_deref(), b.pushed_at.as_second()))
            .collect();
        assert_eq!(
            found,
            [
                (vec![1995012], None, 1793736000),
                (vec![1995010, 1995011], Some("causing build bustages. CLOSED TREE"), 1793732400),
                (vec![1995020], Some("breaking the build"), 1793728800),
                (vec![1995008], Some("causing bc failures in browser_viewsource.js"), 1793710800),
            ],
            "a message needs a bug candidate or an explicitly affected commit"
        );
        let backout = &scan.backouts[3];
        assert_eq!(backout.commit, "21".repeat(20));
        assert_eq!(backout.url, push_url("autoland", &"2".repeat(40)));
        assert_eq!(scan.matches.len(), 2, "both landings remain available for commit attribution");
        assert_eq!(
            scan.matches.iter().map(|m| m.pushed_at.as_second()).max().unwrap(),
            1793718000,
            "the newest landing, the re-land, is kept"
        );
    }

    #[test]
    fn ambiguous_and_embedded_trailers_are_not_landing_evidence() {
        assert!(trailer("Bug 1\nSee Differential Revision: https://phabricator.services.mozilla.com/D123").is_none());
        assert!(trailer("Bug 1\nDifferential Revision: https://phabricator.services.mozilla.com/D123\nDifferential Revision: https://phabricator.services.mozilla.com/D124").is_none());
    }
}
