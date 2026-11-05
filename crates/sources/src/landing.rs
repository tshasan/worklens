use std::collections::{BTreeMap, HashSet};

use futures::{StreamExt, stream};
use jiff::Timestamp;
use serde::Deserialize;
use worklens_core::RevId;
use worklens_core::raw::{Landing, Landings, PushHealth, RepositoryLandings};

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

fn trailer(message: &str) -> Option<RevId> {
    let subject = message.lines().next()?.to_ascii_lowercase();
    if subject.starts_with("backed out") || subject.starts_with("backout") || subject.starts_with("revert ") {
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
    for push in page.results {
        if !hash(&push.revision) {
            return Err(Error::Api("Treeherder push has no full commit hash".into()));
        }
        let pushed_at = Timestamp::from_second(push.push_timestamp).map_err(|e| Error::Api(format!("Treeherder push timestamp: {e}")))?;
        for commit in push.revisions {
            let Some(id) = trailer(&commit.comments).filter(|id| wanted.contains(id)) else {
                continue;
            };
            if !hash(&commit.revision) {
                return Err(Error::Api("Treeherder matched commit has no full hash".into()));
            }
            matches.entry(id).or_insert_with(|| Landing {
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
        repository: repository.into(),
        checked_at: now,
        push_limit: PUSH_LIMIT,
        scanned_pushes,
        oldest_push,
        error: None,
        matches: matches.into_values().collect(),
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
        repository: repository.into(),
        checked_at: Timestamp::now(),
        push_limit: PUSH_LIMIT,
        scanned_pushes: 0,
        oldest_push: None,
        error: Some(error.summary()),
        matches: Vec::new(),
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
    fn exact_trailers_find_stack_commits_without_using_the_push_tip() {
        let bytes = include_bytes!("../fixtures/treeherder/integration-pushes.json");
        let scan = parse_scan(bytes, "autoland", &[RevId(123), RevId(124), RevId(125)], now()).unwrap();
        assert_eq!(scan.scanned_pushes, 4);
        assert_eq!(scan.matches.len(), 2, "backout, quoted and prefix-only matches must not count");
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
    fn ambiguous_and_embedded_trailers_are_not_landing_evidence() {
        assert!(trailer("Bug 1\nSee Differential Revision: https://phabricator.services.mozilla.com/D123").is_none());
        assert!(trailer("Bug 1\nDifferential Revision: https://phabricator.services.mozilla.com/D123\nDifferential Revision: https://phabricator.services.mozilla.com/D124").is_none());
    }
}
