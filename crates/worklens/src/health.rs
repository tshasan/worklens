use jiff::Timestamp;
use serde::Serialize;
use worklens_core::Raw;

use crate::config::Config;
use crate::secrets::Service;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceHealth {
    pub name: &'static str,
    pub state: &'static str,
    pub requests: usize,
    pub failed_requests: usize,
    pub slowest_ms: u32,
    pub error: Option<String>,
    pub stale_since: Option<Timestamp>,
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limited_until: Option<Timestamp>,
}

struct Source {
    name: &'static str,
    hosts: &'static [&'static str],
    errors: &'static [&'static str],
    key: Option<Service>,
}

const SOURCES: [Source; 7] = [
    Source {
        name: "Bugzilla",
        hosts: &["bugzilla.mozilla.org"],
        errors: &["bugzilla", "bugzilla-dependencies"],
        key: Some(Service::Bugzilla),
    },
    Source {
        name: "Phabricator",
        hosts: &["phabricator.services.mozilla.com"],
        errors: &[
            "phabricator",
            "phabricator-dependencies",
            "phabricator-availability",
            "phabricator-diffs",
        ],
        key: Some(Service::Phabricator),
    },
    Source {
        name: "Jira",
        hosts: &["atlassian.net", "api.atlassian.com"],
        errors: &["jira", "jira-ideas"],
        key: Some(Service::Jira),
    },
    Source {
        name: "Treeherder",
        hosts: &["treeherder.mozilla.org"],
        errors: &["treeherder", "treeherder-landings"],
        key: None,
    },
    Source {
        name: "Release trains",
        hosts: &["product-details.mozilla.org", "whattrainisitnow.com"],
        errors: &["product-details"],
        key: None,
    },
    Source {
        name: "Crash stats",
        hosts: &["crash-stats.mozilla.org"],
        errors: &["crash-stats"],
        key: None,
    },
    Source {
        name: "Glean",
        hosts: &["probeinfo.telemetry.mozilla.org"],
        errors: &["probeinfo"],
        key: None,
    },
];

fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', ':']).next().unwrap_or_default()
}

fn host_matches(host: &str, pattern: &str) -> bool {
    host == pattern || host.strip_suffix(pattern).is_some_and(|prefix| prefix.ends_with('.'))
}

pub fn from_cache(raw: &Raw, cfg: &Config) -> Vec<SourceHealth> {
    SOURCES
        .iter()
        .map(|src| {
            let calls: Vec<_> = raw
                .calls
                .iter()
                .filter(|c| src.hosts.iter().any(|h| host_matches(host(&c.url), h)))
                .collect();
            let failed_requests = calls.iter().filter(|c| c.status.is_none_or(|s| s >= 400)).count();
            let failure = raw.failures().find(|e| src.errors.contains(&e.source.as_str()));
            let error = failure.map(|e| e.message.clone());
            let rate_limited_until = raw
                .failures()
                .filter(|e| src.errors.contains(&e.source.as_str()))
                .filter_map(|e| e.retry_at)
                .max();
            let stale_since = raw.stale.iter().find(|s| src.errors.contains(&s.source.as_str())).map(|s| s.since);
            let credential = src.key.and_then(|service| cfg.credentials.iter().find(|c| c.service == service));
            let key = credential.and_then(|c| c.origin.as_ref()).map(ToString::to_string);
            let has_key = src.key.is_none() || key.is_some() || src.key == Some(Service::Bugzilla);
            let state = match (&error, stale_since, calls.is_empty()) {
                (Some(_), Some(_), _) => "stale",
                (Some(_), None, _) => "failed",
                (None, _, true) if !has_key => "off",
                (None, _, true) => "unused",
                (None, _, false) => "ok",
            };
            SourceHealth {
                name: src.name,
                state,
                requests: calls.len(),
                failed_requests,
                slowest_ms: calls.iter().map(|c| c.ms).max().unwrap_or(0),
                error,
                stale_since,
                key,
                rate_limited_until,
            }
        })
        .collect()
}

pub fn clock(t: Timestamp) -> String {
    t.to_zoned(jiff::tz::TimeZone::system()).strftime("%a %H:%M").to_string()
}

pub fn describe(h: &SourceHealth) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(e) = &h.error {
        parts.push(e.clone());
    }
    if let Some(since) = h.stale_since {
        parts.push(format!("showing data from {}", clock(since)));
    }
    match (h.requests, h.failed_requests) {
        (0, _) if h.state == "off" => parts.push("not set up".into()),
        (0, _) => parts.push("no requests".into()),
        (n, 0) => parts.push(format!("{n} {}, slowest {} ms", plural(n, "request"), h.slowest_ms)),
        (n, f) => parts.push(format!("{n} {}, {f} failed", plural(n, "request"))),
    }
    if let Some(key) = &h.key {
        parts.push(format!("key: {key}"));
    }
    parts.join(" · ")
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 { word.to_owned() } else { format!("{word}s") }
}

pub fn fix(h: &SourceHealth) -> Option<String> {
    if let Some(until) = h.rate_limited_until.filter(|_| matches!(h.state, "stale" | "failed")) {
        return Some(format!("the service is rate-limiting; refresh after {}", clock(until)));
    }
    match h.state {
        "stale" | "failed" => Some(match h.error.as_deref() {
            Some(e) if e.starts_with("401") || e.starts_with("403") => {
                format!("the key was refused; `worklens auth set {}`", h.name.to_lowercase())
            }
            Some(e) if e.starts_with("429") => "the service is rate-limiting; worklens retries on the next refresh".into(),
            _ => "worklens keeps the last good data and retries on the next refresh (r)".into(),
        }),
        "off" => Some(format!("`worklens auth set {}`, or `worklens setup`", h.name.to_lowercase())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_match_whole_labels_only() {
        assert_eq!(host("https://bugzilla.mozilla.org/rest/bug?id=1"), "bugzilla.mozilla.org");
        assert_eq!(host("https://example.com:8443/x"), "example.com");
        assert!(host_matches("mozilla-hub.atlassian.net", "atlassian.net"));
        assert!(host_matches("api.atlassian.com", "api.atlassian.com"));
        assert!(!host_matches("evilatlassian.net", "atlassian.net"));
    }

    #[test]
    fn a_stale_source_says_since_when_and_what_to_do() {
        let h = SourceHealth {
            name: "Jira",
            state: "stale",
            requests: 3,
            failed_requests: 1,
            slowest_ms: 900,
            error: Some("503 for https://api.atlassian.com/…".into()),
            stale_since: Some("2026-11-03T22:02:00Z".parse().unwrap()),
            key: Some("macOS Keychain".into()),
            rate_limited_until: None,
        };
        let line = describe(&h);
        assert!(line.starts_with("503"), "{line}");
        assert!(line.contains("showing data from"), "{line}");
        assert!(line.contains("3 requests, 1 failed"), "{line}");
        assert!(line.contains("key: macOS Keychain"), "{line}");
        assert!(fix(&h).unwrap().contains("keeps the last good data"));
    }

    #[test]
    fn a_refused_key_points_at_auth_set() {
        let h = SourceHealth {
            name: "Phabricator",
            state: "failed",
            requests: 1,
            failed_requests: 1,
            slowest_ms: 100,
            error: Some("401 for https://phabricator.services.mozilla.com/api/".into()),
            stale_since: None,
            key: None,
            rate_limited_until: None,
        };
        assert_eq!(fix(&h).unwrap(), "the key was refused; `worklens auth set phabricator`");
    }
}
