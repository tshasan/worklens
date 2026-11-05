use std::fmt;

use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::bugs::{assignee, bug_url, jira_url, lando_url, rev_url};
use crate::drift::{Fix, Side};
use crate::ids::{RevId, RevIdx};
use crate::raw::{RevStatus, Reviewer, ReviewerStatus, Revision, TryResult};
use crate::snapshot::{Entry, Snapshot, TryState};

pub fn age_since(s: &Snapshot, e: &Entry) -> Option<Timestamp> {
    e.placement
        .and_then(|p| p.since)
        .or_else(|| e.bug.map(|b| s.bug(b).last_change_time))
        .or_else(|| e.issue.map(|i| s.issue(i).updated))
        .or_else(|| e.idea.map(|i| s.idea(i).updated))
}

pub fn live_revisions<'a>(s: &'a Snapshot, e: &'a Entry) -> impl Iterator<Item = RevIdx> + 'a {
    e.revisions.iter().copied().filter(|r| {
        let rev = s.rev(*r);
        s.is_mine(rev) && !matches!(rev.status, RevStatus::Published | RevStatus::Abandoned)
    })
}

pub fn try_revision(s: &Snapshot, e: &Entry) -> Option<RevIdx> {
    live_revisions(s, e).find(|r| !matches!(s.try_state(*r), TryState::Unknown))
}

pub fn owner<'a>(s: &'a Snapshot, e: &Entry) -> Option<&'a str> {
    if let crate::snapshot::ItemId::Idea(i) = e.id {
        return s.idea(i).assignee.as_deref();
    }
    e.bug
        .and_then(|b| assignee(s.bug(b)))
        .or_else(|| e.issue.and_then(|i| s.issue(i).assignee.as_deref()))
}

pub fn reviewers(rev: &Revision) -> impl Iterator<Item = &Reviewer> {
    rev.reviewers.iter().filter(|v| v.status != ReviewerStatus::Resigned)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Known,
    Unknown,
    Good,
    Pending,
    Bad,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    pub signal: Signal,
}

fn segment(text: impl Into<String>, signal: Signal) -> Segment {
    Segment { text: text.into(), signal }
}

fn reviewer_name(s: &Snapshot, phid: &str) -> String {
    let name = s.name(phid);
    if s.raw.me.phid.as_deref() == Some(phid) {
        "you".to_owned()
    } else if phid.starts_with("PHID-PROJ-") && !name.starts_with('#') {
        format!("{name} (group)")
    } else {
        name.to_owned()
    }
}

pub fn review_segments(s: &Snapshot, rev: &Revision) -> Vec<Segment> {
    use ReviewerStatus as R;
    struct Group {
        states: &'static [R],
        before: &'static str,
        after: &'static str,
        signal: Signal,
    }
    const fn group(states: &'static [R], before: &'static str, after: &'static str, signal: Signal) -> Group {
        Group {
            states,
            before,
            after,
            signal,
        }
    }
    const GROUPS: [Group; 8] = [
        group(&[R::Accepted], "", " accepted", Signal::Good),
        group(&[R::Rejected], "", " requested changes", Signal::Bad),
        group(&[R::AcceptedOlder], "", " accepted an older diff", Signal::Pending),
        group(&[R::RejectedOlder], "", " requested changes on an older diff", Signal::Pending),
        group(&[R::Blocking, R::Added], "waiting on ", "", Signal::Known),
        group(&[R::Commented], "", " commented", Signal::Known),
        group(&[R::Resigned], "", " resigned", Signal::Unknown),
        group(&[R::Other], "", ": review state not recognized", Signal::Unknown),
    ];
    let mut out = Vec::new();
    for g in &GROUPS {
        let names: Vec<String> = rev
            .reviewers
            .iter()
            .filter(|r| g.states.contains(&r.status))
            .map(|r| {
                let name = reviewer_name(s, &r.phid);
                if r.status == R::Blocking {
                    format!("{name} (blocking)")
                } else {
                    name
                }
            })
            .collect();
        if !names.is_empty() {
            out.push(segment(format!("{}{}{}", g.before, names.join(", "), g.after), g.signal));
        }
    }
    if out.is_empty() {
        out.push(segment("no reviewers", Signal::Unknown));
    }
    out
}

pub fn try_segments(state: TryState<'_>) -> Vec<Segment> {
    let (push, outdated) = match state {
        TryState::Unknown => return Vec::new(),
        TryState::NoPush => return vec![segment("no try push", Signal::Unknown)],
        TryState::Push { push, outdated } => (push, outdated),
    };
    let mut out: Vec<Segment> = [("builds", push.builds), ("lint", push.lint), ("tests", push.tests)]
        .into_iter()
        .map(|(kind, result)| match result {
            TryResult::Pass => segment(format!("{kind} passed"), Signal::Good),
            TryResult::Fail => segment(format!("{kind} failed"), Signal::Bad),
            TryResult::Running => segment(format!("{kind} running"), Signal::Pending),
            TryResult::None => segment(format!("no {kind} jobs"), Signal::Unknown),
        })
        .collect();
    let jobs = |n: u32| if n == 1 { "job" } else { "jobs" };
    if push.failed > 0 {
        out.push(segment(format!("{} failing {}", push.failed, jobs(push.failed)), Signal::Bad));
    }
    if push.running > 0 {
        out.push(segment(format!("{} {} left", push.running, jobs(push.running)), Signal::Pending));
    }
    if outdated {
        out.push(segment("before the latest diff", Signal::Pending));
    }
    out
}

pub fn date(t: Timestamp, tz: &TimeZone) -> String {
    t.to_zoned(tz.clone()).strftime("%a %b %d %Y").to_string()
}

pub struct FixText<'a>(pub &'a Fix);

impl fmt::Display for FixText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let side = match self.0.side {
            Side::Jira => "Jira",
            Side::Bugzilla => "Bugzilla",
        };
        write!(f, "set {side} {} to {}", self.0.field, self.0.value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Bug,
    Jira,
    Parent,
    Idea,
    Revision,
    Lando,
    Try,
    Evidence,
}

#[derive(Debug)]
pub struct Link {
    pub kind: LinkKind,
    pub label: String,
    pub url: String,
    pub rev: Option<RevId>,
}

pub fn links(s: &Snapshot, e: &Entry) -> Vec<Link> {
    let mut out = Vec::new();
    let mut add = |kind, label: String, url: String, rev| {
        if !out.iter().any(|l: &Link| l.url == url) {
            out.push(Link { kind, label, url, rev });
        }
    };
    if let Some(b) = e.bug.map(|b| s.bug(b)) {
        add(LinkKind::Bug, "Bugzilla".into(), bug_url(b.id), None);
    }
    if let Some(i) = e.issue.map(|i| s.issue(i)) {
        add(LinkKind::Jira, "Jira".into(), jira_url(&i.key.0), None);
    }
    if let Some(p) = s.parent(e) {
        add(
            LinkKind::Parent,
            p.kind.clone().unwrap_or_else(|| "Parent".into()),
            jira_url(&p.key.0),
            None,
        );
    }
    if let Some(i) = e.idea.map(|i| s.idea(i)) {
        add(LinkKind::Idea, "Idea".into(), jira_url(&i.key.0), None);
    }
    for r in e.review.iter().chain(&e.revisions) {
        let id = s.rev(*r).id;
        add(LinkKind::Revision, id.to_string(), rev_url(id), Some(id));
    }
    for r in &e.revisions {
        let rev = s.rev(*r);
        if s.is_mine(rev) && rev.status == RevStatus::Accepted {
            add(LinkKind::Lando, format!("Lando {}", rev.id), lando_url(rev.id), Some(rev.id));
        }
        if let TryState::Push { push, .. } = s.try_state(*r) {
            add(LinkKind::Try, format!("try {}", rev.id), push.url.clone(), Some(rev.id));
        }
    }
    for dependency in crate::dependencies::of(s, e) {
        add(
            LinkKind::Evidence,
            format!("Dependency {} ({})", dependency.id, dependency.status),
            dependency.url,
            None,
        );
    }
    let workflow = crate::evidence::workflow(s, e);
    for evidence in workflow
        .approvals
        .iter()
        .chain(&workflow.channels)
        .chain(&workflow.landings)
        .chain(&workflow.telemetry)
    {
        add(LinkKind::Evidence, evidence.label.clone(), evidence.url.clone(), None);
    }
    out
}

pub fn ago(t: Timestamp, now: Timestamp) -> String {
    let m = ((now.as_millisecond() - t.as_millisecond()) as f64 / 60_000.0).round();
    if m < 1.0 {
        "just now".into()
    } else if m < 60.0 {
        format!("{m}m ago")
    } else if m < 1440.0 {
        format!("{}h ago", (m / 60.0).round())
    } else {
        format!("{}d ago", (m / 1440.0).round())
    }
}
