use jiff::Timestamp;
use worklens_core::raw::{Landings, SourceError, Via};
use worklens_core::{Raw, RevId};

use crate::fetch::Config;
use crate::http::Http;

const TELEMETRY_SECONDS: i64 = 24 * 60 * 60;
const LANDING_SECONDS: i64 = 30 * 60;
const FAILURE_SECONDS: i64 = 5 * 60;

fn due(last: Option<Timestamp>, now: Timestamp, interval: i64) -> bool {
    last.is_none_or(|last| now.as_second().saturating_sub(last.as_second()) >= interval)
}

fn failed(raw: &Raw, source: &str) -> bool {
    raw.errors.iter().any(|error| error.source == source)
}

pub(crate) fn restore(raw: &mut Raw, previous: Option<&Raw>) {
    let Some(previous) = previous.filter(|p| p.me.email.eq_ignore_ascii_case(&raw.me.email)) else {
        return;
    };
    raw.telemetry.clone_from(&previous.telemetry);
    raw.supplemental_cache.telemetry_attempted_at = previous.supplemental_cache.telemetry_attempted_at;
    let same_review_identity = previous.me.phid == raw.me.phid;
    if same_review_identity {
        raw.landings.clone_from(&previous.landings);
        raw.supplemental_cache.landings_attempted_at = previous.supplemental_cache.landings_attempted_at;
    }
    raw.errors.extend(
        previous
            .errors
            .iter()
            .filter(|e| e.source == "probeinfo" || (same_review_identity && e.source == "treeherder-landings"))
            .map(|e| SourceError {
                source: e.source.clone(),
                message: e.message.clone(),
                unset: e.unset,
                retry_at: e.retry_at,
            }),
    );
}

fn telemetry_due(raw: &Raw, now: Timestamp) -> bool {
    let last = raw
        .supplemental_cache
        .telemetry_attempted_at
        .or_else(|| raw.telemetry.as_ref().map(|r| r.fetched_at));
    due(
        last,
        now,
        if failed(raw, "probeinfo") {
            FAILURE_SECONDS
        } else {
            TELEMETRY_SECONDS
        },
    )
}

fn landings_due(raw: &Raw, now: Timestamp) -> bool {
    let last = raw.supplemental_cache.landings_attempted_at.or_else(|| {
        raw.landings
            .as_ref()
            .and_then(|r| r.repositories.iter().map(|repo| repo.checked_at).max())
    });
    let error = failed(raw, "treeherder-landings")
        || raw
            .landings
            .as_ref()
            .is_some_and(|r| r.repositories.iter().any(|repo| repo.error.is_some()));
    due(last, now, if error { FAILURE_SECONDS } else { LANDING_SECONDS })
}

fn apply_landings(raw: &mut Raw, mut report: Landings) {
    raw.errors.retain(|e| e.source != "treeherder-landings");
    for repository in &mut report.repositories {
        if let Some(error) = &repository.error {
            raw.errors.push(SourceError {
                source: "treeherder-landings".into(),
                message: format!("{}: {error}", repository.repository),
                unset: false,
                retry_at: repository.retry_at,
            });
            if let Some(old) = raw
                .landings
                .as_ref()
                .and_then(|r| r.repositories.iter().find(|r| r.repository == repository.repository))
            {
                repository.matches.clone_from(&old.matches);
                repository.backouts.clone_from(&old.backouts);
                repository.checked_at = old.checked_at;
                repository.scanned_pushes = old.scanned_pushes;
                repository.oldest_push = old.oldest_push;
            }
        }
    }
    raw.landings = Some(report);
}

pub(crate) async fn refresh(http: &Http, cfg: &Config, raw: &mut Raw, touched: &[RevId], log: impl Fn(&str), now: Timestamp) {
    if !cfg.telemetry {
        raw.telemetry = None;
        raw.supplemental_cache.telemetry_attempted_at = None;
        raw.errors.retain(|e| e.source != "probeinfo");
    } else if telemetry_due(raw, now) {
        raw.supplemental_cache.telemetry_attempted_at = Some(now);
        raw.errors.retain(|e| e.source != "probeinfo");
        log("Reading daily Glean definitions");
        let ids: Vec<_> = raw
            .bugs
            .iter()
            .filter(|b| !b.via.iter().all(|v| *v == Via::Team))
            .map(|b| b.id)
            .collect();
        match crate::telemetry::fetch(http, &raw.me.email, &ids).await {
            Ok(report) => raw.telemetry = Some(report),
            Err(error) => raw.errors.push(SourceError {
                source: "probeinfo".into(),
                message: error.summary(),
                unset: false,
                retry_at: error.retry_at(),
            }),
        }
    }
    if !cfg.landing_checks {
        raw.landings = None;
        raw.supplemental_cache.landings_attempted_at = None;
        raw.errors.retain(|e| e.source != "treeherder-landings");
    } else if landings_due(raw, now) {
        let mut ids: Vec<_> = raw
            .revisions
            .iter()
            .filter(|r| {
                Some(&r.author) == raw.me.phid.as_ref()
                    || raw.to_review.contains(&r.id)
                    || touched.contains(&r.id)
                    || r.bug
                        .is_some_and(|id| raw.bugs.iter().any(|b| b.id == id && !b.via.iter().all(|v| *v == Via::Team)))
            })
            .map(|r| r.id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        if !ids.is_empty() {
            raw.supplemental_cache.landings_attempted_at = Some(now);
            log("Checking half-hourly integration evidence");
            let report = crate::landing::fetch(http, &ids).await;
            apply_landings(raw, report);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use worklens_core::raw::{RepositoryLandings, Telemetry};

    fn fixture() -> Raw {
        serde_json::from_str(include_str!("../../store/fixtures/raw.json")).unwrap()
    }

    fn now() -> Timestamp {
        "2026-11-04T00:00:00Z".parse().unwrap()
    }

    #[test]
    fn cadence_survives_serialization_and_repeated_refreshes() {
        let mut raw = fixture();
        raw.supplemental_cache.telemetry_attempted_at = Some(now());
        raw.supplemental_cache.landings_attempted_at = Some(now());
        let raw: Raw = serde_json::from_slice(&serde_json::to_vec(&raw).unwrap()).unwrap();
        for seconds in [0, 1, 60, 300, 1799] {
            let time = Timestamp::from_second(now().as_second() + seconds).unwrap();
            assert!(!telemetry_due(&raw, time));
            assert!(!landings_due(&raw, time));
        }
        assert!(landings_due(&raw, Timestamp::from_second(now().as_second() + 1800).unwrap()));
        assert!(!telemetry_due(&raw, Timestamp::from_second(now().as_second() + 86399).unwrap()));
        assert!(telemetry_due(&raw, Timestamp::from_second(now().as_second() + 86400).unwrap()));
    }

    #[test]
    fn failures_cool_down_even_when_no_successful_data_exists() {
        let mut raw = fixture();
        raw.supplemental_cache.telemetry_attempted_at = Some(now());
        raw.supplemental_cache.landings_attempted_at = Some(now());
        raw.errors.push(SourceError {
            source: "probeinfo".into(),
            message: "503 unavailable".into(),
            unset: false,
            retry_at: None,
        });
        raw.errors.push(SourceError {
            source: "treeherder-landings".into(),
            message: "429 throttled".into(),
            unset: false,
            retry_at: None,
        });
        assert!(!telemetry_due(&raw, Timestamp::from_second(now().as_second() + 299).unwrap()));
        assert!(telemetry_due(&raw, Timestamp::from_second(now().as_second() + 300).unwrap()));
        assert!(!landings_due(&raw, Timestamp::from_second(now().as_second() + 299).unwrap()));
        assert!(landings_due(&raw, Timestamp::from_second(now().as_second() + 300).unwrap()));
        assert!(!telemetry_due(&raw, Timestamp::from_second(now().as_second() - 60).unwrap()));
    }

    #[test]
    fn cached_refreshes_and_focus_checks_do_not_need_a_network_runtime() {
        let cfg = Config {
            email: Some("me@mozilla.com".into()),
            bugzilla_api_key: None,
            jira: None,
            phab: None,
            whiteboard: Some("[test]".into()),
            drift_days: 30,
            telemetry: true,
            landing_checks: true,
        };
        let mut raw = fixture();
        raw.telemetry = Some(Telemetry {
            fetched_at: now(),
            metrics: Vec::new(),
        });
        raw.supplemental_cache.landings_attempted_at = Some(now());
        let http = Http::new().unwrap();
        for seconds in [0, 1, 60, 300, 1799] {
            let now = Timestamp::from_second(now().as_second() + seconds).unwrap();
            futures::executor::block_on(refresh(&http, &cfg, &mut raw, &[RevId(999)], |_| {}, now));
        }
        assert!(http.take_calls().is_empty());
    }

    #[test]
    fn restore_retains_age_but_never_reuses_another_identity() {
        let mut old = fixture();
        old.telemetry = Some(Telemetry {
            fetched_at: now(),
            metrics: Vec::new(),
        });
        let mut next = fixture();
        restore(&mut next, Some(&old));
        assert_eq!(next.telemetry.unwrap().fetched_at, now());
        let mut next = fixture();
        next.me.email = "another@example.org".into();
        restore(&mut next, Some(&old));
        assert!(next.telemetry.is_none());
    }

    #[test]
    fn failed_scan_keeps_old_evidence_without_making_it_fresh() {
        let mut raw = fixture();
        let old_time = Timestamp::from_second(now().as_second() - 3600).unwrap();
        let repo = crate::landing::parse_scan(
            include_bytes!("../fixtures/treeherder/recorded-autoland-push.json"),
            "autoland",
            &[RevId(338940)],
            old_time,
        )
        .unwrap();
        let commit = repo.matches[0].commit.clone();
        raw.landings = Some(Landings {
            revisions: vec![RevId(338940)],
            repositories: vec![repo],
        });
        let repo = RepositoryLandings {
            retry_at: None,
            repository: "autoland".into(),
            checked_at: now(),
            push_limit: 100,
            scanned_pushes: 0,
            oldest_push: None,
            error: Some("429 throttled".into()),
            matches: Vec::new(),
            backouts: Vec::new(),
        };
        apply_landings(
            &mut raw,
            Landings {
                revisions: vec![RevId(338940)],
                repositories: vec![repo],
            },
        );
        let repo = &raw.landings.as_ref().unwrap().repositories[0];
        assert_eq!(repo.checked_at, old_time);
        assert_eq!(repo.scanned_pushes, 1);
        assert_eq!(repo.matches[0].commit, commit);
        assert!(repo.error.is_some());
    }
}
