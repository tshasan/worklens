use jiff::Timestamp;
use serde::Serialize;

use crate::bugs::{bug_url, rev_url};
use crate::raw::{Metric, RevStatus, ReviewerStatus};
use crate::section::Section;
use crate::{Entry, Snapshot};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub label: String,
    pub state: &'static str,
    pub summary: String,
    pub source: &'static str,
    pub url: String,
    pub observed_at: Timestamp,
}

const NO_APPROVAL: &str = "No approval flag observed; applicability not established";

impl Evidence {
    pub fn is_known(&self) -> bool {
        !matches!(self.state, "unknown" | "unconfirmed" | "not_observed") && self.summary != NO_APPROVAL
    }
    pub fn is_reviewer(&self) -> bool {
        self.source == "phabricator" && self.label.split_once(' ').is_some_and(|(_, rest)| rest.starts_with("review: "))
    }
    pub fn is_uplift(&self) -> bool {
        self.source == "bugzilla" && self.label.ends_with(" uplift")
    }
}

pub fn decides(e: &Entry) -> bool {
    matches!(e.placement.map(|p| p.section), Some(Section::Land))
        || matches!(e.placement.map(|p| p.reason), Some(crate::rules::Reason::LandingBlocked(_)))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub approvals: Vec<Evidence>,
    pub landings: Vec<Evidence>,
    pub channels: Vec<Evidence>,
    pub telemetry: Vec<Evidence>,
    pub limitations: Vec<String>,
    #[serde(skip)]
    pub unknown: Vec<String>,
}

pub fn workflow(s: &Snapshot, e: &Entry) -> Workflow {
    let mut approvals = Vec::new();
    let mut channels = Vec::new();
    let mut telemetry = Vec::new();
    let mut limitations = Vec::new();
    let mut unknown = Vec::new();
    for idx in e.revisions.iter().chain(&e.review) {
        let rev = s.rev(*idx);
        let mut add = |label: String, state, summary| {
            approvals.push(Evidence {
                label: format!("{} {label}", rev.id),
                state,
                summary,
                source: "phabricator",
                url: rev_url(rev.id),
                observed_at: s.raw.fetched_at,
            })
        };
        let tags: Vec<&str> = rev.projects.iter().flatten().map(|p| s.name(p).trim_start_matches('#')).collect();
        let (state, text) = if rev.projects.is_none() {
            ("unknown", "Project tags not fetched; data classification unknown")
        } else if tags.contains(&"needs-data-classification") {
            ("required", "Data classification required; this tag prevents landing in Lando")
        } else if tags.contains(&"data-classification-high") {
            ("high", "High data classification; inspect data-steward review below and upstream")
        } else if tags.contains(&"data-classification-low") {
            ("low", "Low data classification recorded; not a blanket approval to land")
        } else if tags.contains(&"data-classification-unnecessary") {
            ("unnecessary", "Data classification marked unnecessary")
        } else {
            ("unknown", "No recognized data-classification tag; applicability not established")
        };
        add("data review".into(), state, text.into());
        for reviewer in &rev.reviewers {
            let state = match reviewer.status {
                ReviewerStatus::Accepted => "accepted",
                ReviewerStatus::Rejected | ReviewerStatus::RejectedOlder => "changes_requested",
                ReviewerStatus::Resigned => "resigned",
                ReviewerStatus::AcceptedOlder => "older_diff",
                ReviewerStatus::Added | ReviewerStatus::Blocking | ReviewerStatus::Commented => "pending",
                ReviewerStatus::Other => "unknown",
            };
            add(
                format!("review: {}", s.name(&reviewer.phid)),
                state,
                reviewer.status.as_str().into(),
            );
        }
        add(
            "landing".into(),
            if rev.status == RevStatus::Published {
                "reported_closed"
            } else {
                "unconfirmed"
            },
            if s.raw.landings.as_ref().is_some_and(|report| {
                report
                    .repositories
                    .iter()
                    .any(|repo| repo.matches.iter().any(|m| m.revision == rev.id))
            }) {
                format!("Phabricator: {}; see separate integration-push observations", rev.status.as_str())
            } else if rev.status == RevStatus::Published {
                "Phabricator reports closed; repository commit and released build not independently verified".into()
            } else {
                format!("Phabricator: {}; acceptance alone is not landing evidence", rev.status.as_str())
            },
        );
    }
    if let Some(bug) = e.bug.map(|i| s.bug(i)) {
        for flag in bug.flags.iter().filter(|f| f.name != "needinfo") {
            approvals.push(Evidence {
                label: flag.name.clone(),
                state: "observed",
                summary: format!("Bug flag: {}{}; applicability must be checked upstream", flag.name, flag.status),
                source: "bugzilla",
                url: bug_url(bug.id),
                observed_at: s.raw.fetched_at,
            });
        }
        if let Some(uplift) = s.raw.uplift.get(&bug.id) {
            for flag in &uplift.flags {
                approvals.push(Evidence {
                    label: flag.name.clone(),
                    state: "observed",
                    summary: format!("Attachment flag: {}{}", flag.name, flag.status),
                    source: "bugzilla",
                    url: flag.attachment.map_or_else(
                        || bug_url(bug.id),
                        |id| format!("https://bugzilla.mozilla.org/attachment.cgi?id={id}&action=edit"),
                    ),
                    observed_at: s.raw.fetched_at,
                });
            }
        }
        if let Some(trains) = &s.raw.trains {
            let versions = [
                ("Nightly", trains.nightly.to_string(), None),
                ("Beta", trains.beta.to_string(), Some("beta".to_owned())),
                ("Release", trains.release.to_string(), None),
            ]
            .into_iter()
            .chain(trains.esr.iter().map(|v| ("ESR", format!("_esr{v}"), Some(format!("esr{v}")))));
            for (channel, version, approval_channel) in versions {
                let status = bug.status_for(&version).unwrap_or("unknown");
                let state = match status {
                    "fixed" | "verified" => "reported_fixed",
                    "affected" | "fix-optional" => "affected",
                    "unaffected" => "unaffected",
                    "wontfix" => "wontfix",
                    _ => "unknown",
                };
                let summary = match state {
                    "reported_fixed" => format!("Bugzilla reports {status}; this flag alone does not verify repository commit/build"),
                    "unknown" => format!("Bugzilla status {status}; fix presence unknown"),
                    _ => format!("Bugzilla reports {status}"),
                };
                channels.push(Evidence {
                    label: format!("{channel} {}", version.trim_start_matches("_esr")),
                    state,
                    summary,
                    source: "bugzilla",
                    url: bug_url(bug.id),
                    observed_at: s.raw.fetched_at,
                });
                if let Some(channel) = approval_channel
                    && let Some(uplift) = s.raw.uplift.get(&bug.id)
                {
                    let flags: Vec<&str> = uplift
                        .approvals
                        .iter()
                        .filter(|(c, _)| *c == channel)
                        .map(|(_, status)| status.as_str())
                        .collect();
                    let summary = if flags.is_empty() {
                        NO_APPROVAL.into()
                    } else {
                        format!("Attachment approval flags: {}; approval does not confirm landing", flags.join(", "))
                    };
                    approvals.push(Evidence {
                        label: format!("{channel} uplift"),
                        state: "observed",
                        summary,
                        source: "bugzilla",
                        url: bug_url(bug.id),
                        observed_at: s.raw.fetched_at,
                    });
                }
            }
        } else {
            limitations.push("Release trains unavailable; channel versions and fix presence unknown".into());
            unknown.push("trains".to_owned());
        }
        if let Some(report) = &s.raw.telemetry {
            telemetry.extend(
                report
                    .metrics
                    .iter()
                    .filter(|m| m.bugs.contains(&bug.id))
                    .map(|m| metric(s, m, report.fetched_at)),
            );
        }
    }
    if s.raw.telemetry.is_none() {
        limitations.push("Glean definitions not fetched, disabled, or unavailable; telemetry coverage unknown".into());
        unknown.push("telemetry".to_owned());
    }
    for error in &s.raw.errors {
        if matches!(error.source.as_str(), "bugzilla" | "phabricator" | "product-details" | "probeinfo") {
            limitations.push(format!("{}: {}; evidence may be incomplete", error.source, error.message));
            unknown.push(format!(
                "{} ({})",
                error.source,
                if error.unset { "not set up" } else { "fetch failed" }
            ));
        }
    }
    Workflow {
        approvals,
        landings: landing_evidence(s, e),
        channels,
        telemetry,
        limitations,
        unknown,
    }
}

fn landing_evidence(s: &Snapshot, e: &Entry) -> Vec<Evidence> {
    use crate::raw::TryResult;
    let Some(report) = &s.raw.landings else { return Vec::new() };
    let mut out = Vec::new();
    for idx in e.revisions.iter().chain(&e.review) {
        let rev = s.rev(*idx);
        if !report.revisions.contains(&rev.id) {
            continue;
        }
        for repo in &report.repositories {
            let mut state = "not_observed";
            let mut url = format!("https://treeherder.mozilla.org/jobs?repo={}", repo.repository);
            let summary = if let Some(error) = &repo.error {
                state = "lookup_failed";
                let cached = repo
                    .matches
                    .iter()
                    .find(|m| m.revision == rev.id)
                    .map_or_else(String::new, |landing| {
                        format!(
                            "; last known commit {} observed {}; cached checks are stale",
                            landing.commit, repo.checked_at
                        )
                    });
                format!("{} lookup failed: {error}; landing unknown{cached}", repo.repository)
            } else if let Some(landing) = repo.matches.iter().find(|m| m.revision == rev.id) {
                url.clone_from(&landing.url);
                let health = match landing.health {
                    Some(h) if h.failed > 0 => {
                        state = "observed_landing_failed";
                        format!("push has {} failing jobs (not proof this patch caused them)", h.failed)
                    }
                    Some(h) if h.running > 0 => {
                        state = "observed_landing_running";
                        format!("push has {} pending/running jobs", h.running)
                    }
                    Some(h) if [h.builds, h.lint, h.tests].iter().all(|r| *r == TryResult::Pass) => {
                        state = "observed_landing_passed";
                        "observed build/lint/test checks passed".into()
                    }
                    _ => {
                        state = "observed_landing";
                        landing
                            .health_error
                            .clone()
                            .unwrap_or_else(|| "push checks incomplete or unavailable".into())
                    }
                };
                let version = if rev.diff_created.is_some_and(|date| date > landing.pushed_at) {
                    state = "older_diff_landing";
                    "predates latest diff; latest diff landing unconfirmed; "
                } else {
                    ""
                };
                let backout = match crate::backout::after(&s.raw, &repo.repository, landing) {
                    Some(b) => format!("backed out in {} push {} at {}", repo.repository, b.push_revision, b.pushed_at),
                    None => {
                        "no attributed later backout in the scanned pushes; bug-number candidates and unscanned pushes remain unverified"
                            .to_owned()
                    }
                };
                format!(
                    "Commit {} observed in {} push {} at {}; {version}{health}; {backout}; shipped builds unverified",
                    landing.commit, repo.repository, landing.push_revision, landing.pushed_at
                )
            } else {
                format!(
                    "No exact trailer match in {} recently returned {} pushes (limit {}{}); not proof it never landed",
                    repo.repository,
                    repo.scanned_pushes,
                    repo.push_limit,
                    repo.oldest_push.map_or_else(String::new, |date| format!(", oldest {date}"))
                )
            };
            out.push(Evidence {
                label: format!("{} {}", rev.id, repo.repository),
                state,
                summary,
                source: "treeherder-landings",
                url,
                observed_at: repo.checked_at,
            });
        }
    }
    out
}

pub fn metric(s: &Snapshot, m: &Metric, observed_at: Timestamp) -> Evidence {
    let (state, summary) = if m.disabled {
        ("disabled", "Definition is disabled".to_owned())
    } else if m.expires == "never" {
        ("never", "No version expiry in definition".to_owned())
    } else if m.expires == "expired" {
        ("expired", "Definition explicitly expired".to_owned())
    } else if let Ok(version) = m.expires.parse::<u32>() {
        let state = match s.raw.trains.as_ref() {
            Some(t) if t.nightly >= version => "expired_on_nightly",
            Some(_) => "version_expiry",
            None => "unknown",
        };
        let summary = match state {
            "expired_on_nightly" => format!("Expired on current Nightly (expiry {version}); older channels may still collect"),
            "version_expiry" => format!("Expires starting in Firefox {version}"),
            _ => format!("Expiry {version}; current train unavailable"),
        };
        (state, summary)
    } else {
        ("unknown", format!("Unrecognized expiry: {}", m.expires))
    };
    Evidence {
        label: m.name.clone(),
        state,
        summary: format!("{summary}; definition last seen {}", m.definition_last_seen),
        source: "probeinfo",
        url: m
            .source_url
            .clone()
            .unwrap_or_else(|| "https://dictionary.telemetry.mozilla.org/apps/firefox_desktop".into()),
        observed_at,
    }
}

pub fn telemetry(s: &Snapshot) -> Vec<Evidence> {
    s.raw
        .telemetry
        .as_ref()
        .map_or_else(Vec::new, |r| r.metrics.iter().map(|m| metric(s, m, r.fetched_at)).collect())
}

fn all(w: &Workflow) -> impl Iterator<Item = &Evidence> {
    w.approvals.iter().chain(&w.channels).chain(&w.landings).chain(&w.telemetry)
}

fn line(v: &Evidence) -> String {
    format!("{}: {} [{}; observed {}] {}", v.label, v.summary, v.source, v.observed_at, v.url)
}

fn data_reviews(s: &Snapshot, e: &Entry) -> Vec<String> {
    let mut lines = Vec::new();
    if let (Some(bug), Some(report)) = (e.bug.map(|b| s.bug(b)), &s.raw.telemetry) {
        for metric in report.metrics.iter().filter(|m| m.bugs.contains(&bug.id)) {
            for url in &metric.data_reviews {
                lines.push(format!("{} data-review reference (not approval proof): {url}", metric.name));
            }
        }
    }
    lines
}

pub fn lines(s: &Snapshot, e: &Entry) -> Vec<String> {
    let w = workflow(s, e);
    let mut lines: Vec<String> = all(&w).map(line).collect();
    lines.extend(w.limitations);
    lines.extend(data_reviews(s, e));
    lines
}

pub fn show_lines(s: &Snapshot, e: &Entry) -> Vec<String> {
    let w = workflow(s, e);
    let every = decides(e);
    let (shown, quiet): (Vec<&Evidence>, Vec<&Evidence>) = all(&w).partition(|v| every || v.is_known());
    let mut lines: Vec<String> = shown.iter().map(|v| format!("{}: {}", v.label, v.summary)).collect();
    if !quiet.is_empty() {
        let labels: Vec<&str> = quiet.iter().map(|v| v.label.as_str()).collect();
        lines.push(format!("Nothing seen yet for {}", labels.join(", ")));
    }
    lines.extend(data_reviews(s, e));
    lines.extend(w.limitations);
    lines
}

pub fn brief_lines(s: &Snapshot, e: &Entry, every: bool) -> Vec<String> {
    let w = workflow(s, e);
    let mut lines: Vec<String> = all(&w).filter(|v| every || v.is_known()).map(line).collect();
    lines.extend(data_reviews(s, e));
    if lines.is_empty() && !every {
        return lines;
    }
    if !w.unknown.is_empty() {
        lines.push(format!("Unknown: {}", w.unknown.join(", ")));
    }
    lines
}
