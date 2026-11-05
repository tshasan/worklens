use std::collections::HashMap;

use jiff::Timestamp;
use serde::Deserialize;
use worklens_core::RevId;
use worklens_core::raw::TryResult;

use crate::http::{Http, Result, parse};

pub const TREEHERDER: &str = "https://treeherder.mozilla.org";

pub fn push_url(revision: &str) -> String {
    format!("{TREEHERDER}/jobs?repo=try&revision={revision}")
}

pub struct Push {
    pub revision: String,
    pub pushed_at: Timestamp,
}

pub struct Health {
    pub builds: TryResult,
    pub lint: TryResult,
    pub tests: TryResult,
    pub failed: u32,
    pub running: u32,
}

pub struct Treeherder<'a> {
    pub http: &'a Http,
}

impl Treeherder<'_> {
    pub async fn latest_pushes(&self, email: &str, revs: &[RevId]) -> Result<HashMap<RevId, Push>> {
        if revs.is_empty() {
            return Ok(HashMap::new());
        }
        let req = self
            .http
            .client
            .get(format!("{TREEHERDER}/api/project/try/push/"))
            .query(&[("author", email), ("count", "100")]);
        Ok(latest_for(parse_pushes(&self.http.send(req).await?)?, revs))
    }

    pub async fn health(&self, revision: &str) -> Result<Option<Health>> {
        let req = self
            .http
            .client
            .get(format!("{TREEHERDER}/api/project/try/push/health_summary/"))
            .query(&[("revision", revision)]);
        parse_health(&self.http.send(req).await?)
    }
}

#[derive(Deserialize)]
pub struct ThPush {
    revision: String,
    push_timestamp: i64,
    revisions: Vec<ThCommit>,
}

#[derive(Deserialize)]
struct ThCommit {
    #[serde(default)]
    comments: String,
}

pub fn parse_pushes(bytes: &[u8]) -> Result<Vec<ThPush>> {
    #[derive(Deserialize)]
    struct Page {
        results: Vec<ThPush>,
    }
    Ok(parse::<Page>("Treeherder pushes", bytes)?.results)
}

fn latest_for(mut pushes: Vec<ThPush>, revs: &[RevId]) -> HashMap<RevId, Push> {
    pushes.sort_by_key(|p| std::cmp::Reverse(p.push_timestamp));
    let mut out = HashMap::with_capacity(revs.len());
    for p in &pushes {
        for commit in &p.revisions {
            for d in d_numbers(&commit.comments) {
                if revs.contains(&d) && !out.contains_key(&d) {
                    let pushed_at = Timestamp::from_second(p.push_timestamp).unwrap_or_default();
                    out.insert(
                        d,
                        Push {
                            revision: p.revision.clone(),
                            pushed_at,
                        },
                    );
                }
            }
        }
    }
    out
}

pub fn d_numbers(message: &str) -> Vec<RevId> {
    const LINE: &str = "differential revision:";
    const HOST: &str = "phabricator.services.mozilla.com/d";
    let lower = message.to_ascii_lowercase();
    let mut out = Vec::new();
    for (at, _) in lower.match_indices(LINE) {
        let rest = lower[at + LINE.len()..].trim_start();
        let Some(rest) = rest.strip_prefix("https://").or_else(|| rest.strip_prefix("http://")) else {
            continue;
        };
        let Some(rest) = rest.strip_prefix(HOST) else { continue };
        let end = rest.bytes().take_while(u8::is_ascii_digit).count();
        if rest[end..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        if let Ok(n) = rest[..end].parse()
            && !out.contains(&RevId(n))
        {
            out.push(RevId(n));
        }
    }
    out
}

pub fn parse_health(bytes: &[u8]) -> Result<Option<Health>> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ThHealth {
        build_failure_count: u32,
        build_in_progress_count: u32,
        lint_failure_count: u32,
        linting_in_progress_count: u32,
        test_failure_count: u32,
        test_in_progress_count: u32,
        status: Counts,
        metrics: Metrics,
    }
    #[derive(Deserialize)]
    struct Counts {
        #[serde(default)]
        pending: u32,
        #[serde(default)]
        running: u32,
    }
    #[derive(Deserialize)]
    struct Metrics {
        builds: Metric,
        linting: Metric,
        tests: Metric,
    }
    #[derive(Deserialize)]
    struct Metric {
        result: String,
    }
    fn kind(failed: u32, running: u32, result: &str) -> TryResult {
        match (failed, running, result) {
            (1.., _, _) => TryResult::Fail,
            (0, 1.., _) => TryResult::Running,
            (_, _, "pass") => TryResult::Pass,
            (_, _, "fail") => TryResult::Fail,
            _ => TryResult::None,
        }
    }
    let list: Vec<ThHealth> = parse("Treeherder health", bytes)?;
    Ok(list.into_iter().next().map(|h| Health {
        builds: kind(h.build_failure_count, h.build_in_progress_count, &h.metrics.builds.result),
        lint: kind(h.lint_failure_count, h.linting_in_progress_count, &h.metrics.linting.result),
        tests: kind(h.test_failure_count, h.test_in_progress_count, &h.metrics.tests.result),
        failed: h
            .build_failure_count
            .saturating_add(h.lint_failure_count)
            .saturating_add(h.test_failure_count),
        running: h.status.running.saturating_add(h.status.pending),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{}/fixtures/treeherder/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    #[test]
    fn d_numbers_match_differential_revision_lines_including_mach_try_auto() {
        let auto =
            "Tasks automatically selected.\nBug 1 - Fix it r=a\n\nDifferential Revision: https://phabricator.services.mozilla.com/D336313";
        assert_eq!(d_numbers(auto), [RevId(336_313)]);
        let two = "Bug 2 - x\nDifferential Revision: https://phabricator.services.mozilla.com/D1\nDifferential Revision: https://phabricator.services.mozilla.com/D2";
        assert_eq!(d_numbers(two), [RevId(1), RevId(2)]);
        assert_eq!(d_numbers("try: -b o -p all"), []);
        assert_eq!(
            d_numbers("Differential Revision: https://phabricator.services.mozilla.com/D12x"),
            []
        );
    }

    #[test]
    fn the_newest_push_for_each_revision_wins() {
        let pushes = latest_for(
            parse_pushes(&fixture("pushes.json")).unwrap(),
            &[RevId(321_987), RevId(335_404), RevId(1)],
        );
        assert_eq!(pushes[&RevId(321_987)].revision, "newer");
        assert_eq!(pushes[&RevId(335_404)].revision, "older");
        assert!(!pushes.contains_key(&RevId(1)));
    }

    #[test]
    fn health_counts_failures_and_running_jobs() {
        let h = parse_health(&fixture("health.json")).unwrap().unwrap();
        assert_eq!((h.builds, h.lint, h.tests), (TryResult::Pass, TryResult::Running, TryResult::Fail));
        assert_eq!((h.failed, h.running), (2, 3));
        assert!(parse_health(b"[]").unwrap().is_none());
    }

    #[test]
    fn huge_health_counts_saturate() {
        let mut v: serde_json::Value = serde_json::from_slice(&fixture("health.json")).unwrap();
        v[0]["testFailureCount"] = u32::MAX.into();
        v[0]["status"]["pending"] = u32::MAX.into();
        let h = parse_health(&serde_json::to_vec(&v).unwrap()).unwrap().unwrap();
        assert_eq!((h.failed, h.running), (u32::MAX, u32::MAX));
    }
}
