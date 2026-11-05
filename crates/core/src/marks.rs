use std::borrow::Cow;
use std::collections::HashMap;

use jiff::civil::{Date, Weekday};
use jiff::{Timestamp, ToSpan, Zoned};
use serde::{Deserialize, Serialize, Serializer};

use crate::bugs::{State, assignee};
use crate::key::mark_key;
use crate::snapshot::{Entry, Snapshot, TryState};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Marks {
    pub version: u32,
    pub last_look: Option<Timestamp>,
    pub items: HashMap<String, ItemMarks>,
}

impl Default for Marks {
    fn default() -> Self {
        Self {
            version: 1,
            last_look: None,
            items: HashMap::new(),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemMarks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done: Option<Mark>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snooze: Option<Snooze>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<Mark>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    pub at: Timestamp,
    pub sig: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snooze {
    pub at: Timestamp,
    pub sig: String,
    pub until: Option<Timestamp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hidden {
    Done,
    Snoozed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkState {
    pub hidden: Option<Hidden>,
    pub snoozed_until: Option<Timestamp>,
    pub is_new: bool,
}

impl MarkState {
    pub const NONE: Self = Self {
        hidden: None,
        snoozed_until: None,
        is_new: false,
    };
}

#[derive(Debug, Default, PartialEq, Deserialize)]
pub struct Signature<'a> {
    pub section: Option<Cow<'a, str>>,
    pub needinfo: Option<NeedinfoSig<'a>>,
    pub review: Option<ReviewSig<'a>>,
    pub revisions: Vec<RevisionSig<'a>>,
    pub jira: Option<JiraSig<'a>>,
    pub bug: Option<BugSig<'a>>,
    pub drift: Vec<Cow<'a, str>>,
    #[serde(default)]
    pub facts: Option<Vec<FactSig>>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct FactSig {
    pub source: String,
    pub label: String,
    pub value: String,
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct NeedinfoSig<'a> {
    pub setter: Cow<'a, str>,
    pub created: Timestamp,
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct ReviewSig<'a> {
    pub id: u32,
    pub status: Cow<'a, str>,
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct RevisionSig<'a> {
    pub id: u32,
    pub status: Cow<'a, str>,
    pub reviewers: Vec<String>,
    pub try_state: Option<Cow<'a, str>>,
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct JiraSig<'a> {
    pub key: Cow<'a, str>,
    pub status: Cow<'a, str>,
    pub assignee: Option<Cow<'a, str>>,
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct BugSig<'a> {
    pub state: Cow<'a, str>,
    pub assignee: Option<Cow<'a, str>>,
}

impl Serialize for Signature<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let Self {
            section,
            needinfo,
            review,
            revisions,
            jira,
            bug,
            drift,
            facts,
        } = self;
        if let Some(facts) = facts {
            (section, needinfo, review, revisions, jira, bug, drift, facts).serialize(s)
        } else {
            (section, needinfo, review, revisions, jira, bug, drift).serialize(s)
        }
    }
}

impl Serialize for NeedinfoSig<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        (&self.setter, self.created).serialize(s)
    }
}

impl Serialize for ReviewSig<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        (self.id, &self.status).serialize(s)
    }
}

impl Serialize for RevisionSig<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        (self.id, &self.status, &self.reviewers, &self.try_state).serialize(s)
    }
}

impl Serialize for JiraSig<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        (&self.key, &self.status, &self.assignee).serialize(s)
    }
}

impl Serialize for BugSig<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        (&self.state, &self.assignee).serialize(s)
    }
}

impl<'a> Signature<'a> {
    pub fn of(s: &'a Snapshot, e: &'a Entry) -> Self {
        let revisions = e
            .revisions
            .iter()
            .map(|&r| {
                let rev = s.rev(r);
                let mut reviewers: Vec<String> = crate::present::reviewers(rev)
                    .map(|x| format!("{}:{}", s.name(&x.phid), x.status.as_str()))
                    .collect();
                reviewers.sort_unstable();
                let try_state = match s.try_state(r) {
                    TryState::Push { push, .. } if push.failed > 0 => Some("fail"),
                    TryState::Push { push, .. } if push.running > 0 => Some("running"),
                    TryState::Push { .. } => Some("pass"),
                    TryState::Unknown | TryState::NoPush => None,
                };
                RevisionSig {
                    id: rev.id.0,
                    status: Cow::Borrowed(rev.status.as_str()),
                    reviewers,
                    try_state: try_state.map(Cow::Borrowed),
                }
            })
            .collect();
        Self {
            section: e.placement.map(|p| Cow::Borrowed(p.section.id())),
            needinfo: s.needinfo(e).map(|f| NeedinfoSig {
                setter: Cow::Borrowed(f.setter.as_str()),
                created: f.creation_date,
            }),
            review: e.review.map(|r| ReviewSig {
                id: s.rev(r).id.0,
                status: Cow::Borrowed(s.rev(r).status.as_str()),
            }),
            revisions,
            jira: e.issue.map(|i| {
                let issue = s.issue(i);
                JiraSig {
                    key: Cow::Borrowed(issue.key.0.as_str()),
                    status: Cow::Borrowed(issue.status.as_str()),
                    assignee: issue.assignee.as_deref().map(Cow::Borrowed),
                }
            }),
            bug: e.bug.map(|b| BugSig {
                state: Cow::Owned(State(s.bug(b)).to_string()),
                assignee: assignee(s.bug(b)).map(Cow::Borrowed),
            }),
            drift: e.drift.iter().map(|d| Cow::Borrowed(d.message.as_str())).collect(),
            facts: Some(facts(s, e)),
        }
    }

    pub fn parse(sig: &str) -> Option<Signature<'static>> {
        serde_json::from_str(sig).ok()
    }

    pub fn to_sig(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

pub fn signature(s: &Snapshot, e: &Entry) -> String {
    Signature::of(s, e).to_sig()
}

fn commented_since(s: &Snapshot, e: &Entry, since: Timestamp) -> bool {
    s.last_comment(e).is_some_and(|c| c.author != s.me() && c.time > since)
}

fn changed_since(s: &Snapshot, e: &Entry, sig: &str, at: Timestamp, now_sig: &str) -> bool {
    let changed = if now_sig == sig {
        false
    } else {
        match (Signature::parse(sig), Signature::parse(now_sig)) {
            (Some(old), Some(new)) => !crate::changes::changes(&old, &new, crate::changes::Failed::of(s)).is_empty(),
            _ => true,
        }
    };
    changed || commented_since(s, e, at)
}

fn facts(s: &Snapshot, e: &Entry) -> Vec<FactSig> {
    let mut out = Vec::new();
    let mut add = |source: &str, label: String, value: String| {
        out.push(FactSig {
            source: source.into(),
            label,
            value,
        });
    };
    for idx in e.revisions.iter().chain(&e.review) {
        let rev = s.rev(*idx);
        if let Some(diff) = &rev.diff_phid {
            add("phabricator", format!("{} diff", rev.id), diff.clone());
        }
        if let Some(parents) = s.raw.dependencies.parents.get(&rev.id) {
            let mut parents = parents.clone();
            parents.sort();
            parents.dedup();
            add(
                "phabricator-dependencies",
                format!("{} parent relationships", rev.id),
                parents.join(", "),
            );
        }
        if let Some(projects) = &rev.projects {
            let mut projects = projects.clone();
            projects.sort();
            projects.dedup();
            add(
                "phabricator",
                format!("{} project tags", rev.id),
                projects.iter().map(|p| s.name(p)).collect::<Vec<_>>().join(", "),
            );
        }
        if let TryState::Push { push, .. } = s.try_state(*idx) {
            add("treeherder", format!("{} try push", rev.id), push.revision.clone());
            add(
                "treeherder",
                format!("{} try jobs", rev.id),
                job_state(push.builds, push.lint, push.tests, push.failed, push.running),
            );
        }
        for repo in s.raw.landings.iter().flat_map(|l| &l.repositories).filter(|r| r.error.is_none()) {
            if let Some(landing) = repo.matches.iter().find(|m| m.revision == rev.id) {
                add(
                    "treeherder-landings",
                    format!("{} {} landing", rev.id, repo.repository),
                    landing.commit.clone(),
                );
                if let Some(health) = landing.health.filter(|_| landing.health_error.is_none()) {
                    add(
                        "treeherder-landings",
                        format!("{} {} jobs", rev.id, repo.repository),
                        job_state(health.builds, health.lint, health.tests, health.failed, health.running),
                    );
                }
            }
        }
    }
    if let Some(bug) = e.bug.map(|b| s.bug(b)) {
        let mut ids = bug.depends_on.clone();
        ids.sort_unstable();
        ids.dedup();
        add(
            "bugzilla",
            format!("bug {} dependencies", bug.id),
            ids.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "),
        );
        for (flag, value) in &bug.release_flags {
            add("bugzilla", format!("bug {} {flag}", bug.id), value.clone());
        }
        if let Some(up) = s.raw.uplift.get(&bug.id) {
            let mut approvals = up.approvals.clone();
            approvals.sort();
            approvals.dedup();
            add(
                "bugzilla",
                format!("bug {} uplift approvals", bug.id),
                approvals
                    .iter()
                    .map(|(channel, status)| format!("{channel}{status}"))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
    }
    for dependency in crate::dependencies::of(s, e).into_iter().filter(|d| d.blocked.is_some()) {
        add(
            &format!("{}-dependencies", dependency.source),
            format!("dependency {}", dependency.id),
            dependency.status,
        );
    }
    out.sort_by(|a, b| (&a.source, &a.label).cmp(&(&b.source, &b.label)));
    out.dedup_by(|a, b| a.source == b.source && a.label == b.label);
    out
}

fn job_state(
    builds: crate::raw::TryResult,
    lint: crate::raw::TryResult,
    tests: crate::raw::TryResult,
    failed: u32,
    running: u32,
) -> String {
    format!("builds {builds:?}, lint {lint:?}, tests {tests:?}; {failed} failed, {running} running")
}

fn state_and_changes(s: &Snapshot, e: &Entry, marks: &Marks, now: Timestamp) -> (MarkState, Vec<String>) {
    let Some(m) = marks.items.get(&mark_key(s, e)) else {
        let state = MarkState {
            is_new: marks.last_look.is_some(),
            ..MarkState::NONE
        };
        return (state, Vec::new());
    };
    let now_sig = Signature::of(s, e);
    let sig = now_sig.to_sig();
    let done = m.done.as_ref().is_some_and(|d| !changed_since(s, e, &d.sig, d.at, &sig));
    let snooze = m
        .snooze
        .as_ref()
        .filter(|z| !changed_since(s, e, &z.sig, z.at, &sig) && z.until.is_none_or(|u| u > now));
    let seen = m.seen.as_ref().filter(|seen| changed_since(s, e, &seen.sig, seen.at, &sig));
    let state = MarkState {
        hidden: if done {
            Some(Hidden::Done)
        } else {
            snooze.map(|_| Hidden::Snoozed)
        },
        snoozed_until: snooze.and_then(|z| z.until),
        is_new: seen.is_some(),
    };
    let changes = seen.map_or_else(Vec::new, |seen| crate::changes::since_seen(s, e, seen, &now_sig));
    (state, changes)
}

pub fn apply(s: &mut Snapshot, marks: &Marks, now: Timestamp) {
    let states: Vec<(MarkState, Vec<String>)> = s.entries.iter().map(|e| state_and_changes(s, e, marks, now)).collect();
    for (e, (state, changes)) in s.entries.iter_mut().zip(states) {
        e.marks = state;
        e.changes = changes;
    }
}

pub fn upgrade(marks: &mut Marks, s: &Snapshot) -> bool {
    let mut changed = false;
    for e in &s.entries {
        let Some(item) = marks.items.get_mut(&mark_key(s, e)) else {
            continue;
        };
        for sig in item
            .seen
            .iter_mut()
            .map(|m| &mut m.sig)
            .chain(item.done.iter_mut().map(|m| &mut m.sig))
            .chain(item.snooze.iter_mut().map(|m| &mut m.sig))
        {
            if let Some(mut old) = Signature::parse(sig)
                && old.facts.is_none()
            {
                old.facts = Some(facts(s, e));
                *sig = old.to_sig();
                changed = true;
            }
        }
    }
    changed
}

fn item<'a>(marks: &'a mut Marks, s: &Snapshot, e: &Entry) -> &'a mut ItemMarks {
    marks.items.entry(mark_key(s, e)).or_default()
}

fn stamp(s: &Snapshot, e: &Entry, now: Timestamp) -> Mark {
    Mark {
        at: now,
        sig: signature(s, e),
    }
}

pub fn mark_done(marks: &mut Marks, s: &Snapshot, e: &Entry, now: Timestamp) {
    let mark = stamp(s, e, now);
    let m = item(marks, s, e);
    m.done = Some(mark);
    m.snooze = None;
}

pub fn mark_snoozed(marks: &mut Marks, s: &Snapshot, e: &Entry, until: Option<Timestamp>, now: Timestamp) {
    let Mark { at, sig } = stamp(s, e, now);
    let m = item(marks, s, e);
    m.snooze = Some(Snooze { at, sig, until });
    m.done = None;
}

pub fn restore(marks: &mut Marks, s: &Snapshot, e: &Entry) {
    let m = item(marks, s, e);
    m.done = None;
    m.snooze = None;
}

pub fn mark_seen(marks: &mut Marks, s: &Snapshot, e: &Entry, now: Timestamp) {
    let mark = stamp(s, e, now);
    let m = item(marks, s, e);
    if m.seen.as_ref().is_none_or(|seen| seen.sig != mark.sig) {
        m.seen = Some(mark);
    }
}

pub fn mark_all_seen(marks: &mut Marks, s: &Snapshot, now: Timestamp) {
    for e in &s.entries {
        if marks.items.get(&mark_key(s, e)).is_none_or(|m| m.seen.is_none()) {
            mark_seen(marks, s, e, now);
        }
    }
}

const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

pub fn parse_when(text: &str, now: &Zoned) -> Option<Timestamp> {
    let t = text.trim().to_ascii_lowercase();
    let today = now.date();
    let at9 = |d: Date| d.at(9, 0, 0, 0).to_zoned(now.time_zone().clone()).ok().map(|z| z.timestamp());
    if t == "next week" {
        return parse_when("mon", now);
    }
    if t == "tomorrow" {
        return at9(today.checked_add(1.day()).ok()?);
    }
    if let Some(unit) = t.chars().last().filter(|c| *c == 'd' || *c == 'w') {
        let n = t[..t.len() - 1].trim_end();
        if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) {
            let days = n.parse::<i64>().ok()?.checked_mul(if unit == 'w' { 7 } else { 1 })?;
            return at9(today.checked_add(jiff::Span::new().try_days(days).ok()?).ok()?);
        }
    }
    if t.len() <= 9
        && let Some(day) = DAYS.iter().position(|d| t.starts_with(d))
    {
        let target = Weekday::from_sunday_zero_offset(i8::try_from(day).ok()?).ok()?;
        let ahead = (target.to_sunday_zero_offset() - today.weekday().to_sunday_zero_offset() + 7) % 7;
        return at9(today.checked_add(i64::from(if ahead == 0 { 7 } else { ahead }).days()).ok()?);
    }
    if let Ok(date) = t.parse::<Date>() {
        return at9(date);
    }
    let (a, b) = t.split_once(char::is_whitespace)?;
    let b = b.trim_start();
    let (month, day) = if a.bytes().all(|c| c.is_ascii_digit()) { (b, a) } else { (a, b) };
    if month.len() < 3 || !month.bytes().all(|c| c.is_ascii_lowercase()) || day.is_empty() || day.len() > 2 {
        return None;
    }
    let m = MONTHS.iter().position(|x| month.starts_with(x))?;
    let day: i8 = day.parse().ok()?;
    let month = i8::try_from(m + 1).ok()?;
    let mut date = Date::new(today.year(), month, day).ok()?;
    if date.at(0, 0, 0, 0).to_zoned(now.time_zone().clone()).ok()?.timestamp() < now.timestamp() {
        date = Date::new(today.year() + 1, month, day).ok()?;
    }
    at9(date)
}
