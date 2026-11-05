use std::collections::{BTreeMap, HashSet};

use jiff::Timestamp;
use serde::Deserialize;
use worklens_core::BugId;
use worklens_core::raw::{Metric, Telemetry};

use crate::http::{Http, Result, parse};

pub const METRICS_URL: &str = "https://probeinfo.telemetry.mozilla.org/glean/firefox-desktop/metrics";

pub async fn fetch(http: &Http, email: &str, bugs: &[BugId]) -> Result<Telemetry> {
    let bytes = http.send(http.client.get(METRICS_URL)).await?;
    Ok(Telemetry {
        fetched_at: Timestamp::now(),
        metrics: parse_metrics(&bytes, email, bugs)?,
    })
}

#[derive(Deserialize)]
struct Probe {
    #[serde(rename = "in-source", default)]
    in_source: bool,
    history: Vec<Definition>,
}

#[derive(Deserialize)]
struct Definition {
    dates: Dates,
    expires: serde_json::Value,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    notification_emails: Vec<String>,
    #[serde(default)]
    bugs: Vec<String>,
    #[serde(default)]
    data_reviews: Vec<String>,
    source_url: Option<String>,
}

#[derive(Deserialize)]
struct Dates {
    last: String,
}

fn bug_id(url: &str) -> Option<BugId> {
    url.strip_prefix("https://bugzilla.mozilla.org/show_bug.cgi?id=")?
        .split(['&', '#'])
        .next()?
        .parse()
        .ok()
        .map(BugId)
}

pub fn parse_metrics(bytes: &[u8], email: &str, bugs: &[BugId]) -> Result<Vec<Metric>> {
    let probes: BTreeMap<String, Probe> = parse("Glean definitions", bytes)?;
    let bugs: HashSet<BugId> = bugs.iter().copied().collect();
    let mut metrics = Vec::new();
    for (name, probe) in probes {
        if !probe.in_source {
            continue;
        }
        let Some(def) = probe.history.into_iter().max_by(|a, b| a.dates.last.cmp(&b.dates.last)) else {
            continue;
        };
        let linked: Vec<BugId> = def.bugs.iter().filter_map(|url| bug_id(url)).collect();
        if !def.notification_emails.iter().any(|e| e.eq_ignore_ascii_case(email)) && !linked.iter().any(|id| bugs.contains(id)) {
            continue;
        }
        let expires = match def.expires {
            serde_json::Value::String(s) => s,
            serde_json::Value::Number(n) => n.to_string(),
            _ => "unknown".into(),
        };
        metrics.push(Metric {
            name,
            expires,
            disabled: def.disabled,
            notification_emails: def.notification_emails,
            bugs: linked,
            data_reviews: def.data_reviews,
            source_url: def.source_url,
            definition_last_seen: def.dates.last,
        });
    }
    Ok(metrics)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_definition_controls_contact_expiry_and_links() {
        let bytes = include_bytes!("../fixtures/glean.json");
        let metrics = parse_metrics(bytes, "ME@mozilla.com", &[BugId(42)]).unwrap();
        assert_eq!(
            metrics.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            ["example.linked", "example.owned"]
        );
        assert_eq!(metrics[0].bugs, [BugId(42)]);
        assert_eq!(metrics[1].expires, "163");
        assert_eq!(metrics[1].definition_last_seen, "2026-11-02 00:00:00");
        assert!(parse_metrics(bytes, "other@mozilla.com", &[]).unwrap().is_empty());
    }
}
