use jiff::Timestamp;
use serde::Serialize;

use crate::ids::RevIdx;
use crate::raw::RevStatus;
use crate::section::Section;
use crate::snapshot::{Entry, Snapshot, TryState};

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Event {
    pub at: Timestamp,
    pub source: &'static str,
    pub text: String,
}

const SAME_MOMENT_SECONDS: i64 = 60;

fn apart(a: Timestamp, b: Timestamp) -> bool {
    (a.as_second() - b.as_second()).abs() > SAME_MOMENT_SECONDS
}

pub fn events(s: &Snapshot, e: &Entry) -> Vec<Event> {
    let mut out = Vec::new();
    let mut push = |at: Timestamp, source: &'static str, text: String| out.push(Event { at, source, text });
    if let Some(i) = e.issue.map(|i| s.issue(i))
        && let Some(at) = i.created
    {
        push(at, "Jira", format!("{} created", i.key));
    }
    if let Some(b) = e.bug.map(|b| s.bug(b)) {
        if let Some(at) = b.creation_time {
            push(at, "Bugzilla", format!("Bug {} filed", b.id));
        }
        for f in b.flags.iter().filter(|f| f.name == "needinfo" && f.status == "?") {
            let to = match f.requestee.as_deref() {
                Some(r) if r.eq_ignore_ascii_case(s.me()) => "you",
                Some(r) => r,
                None => "anyone",
            };
            push(f.creation_date, "Bugzilla", format!("needinfo to {to} from {}", f.setter));
        }
        if let (Some(at), false) = (b.cf_last_resolved, b.resolution.is_empty()) {
            push(at, "Bugzilla", format!("Resolved {}", b.resolution));
        }
    }
    let mut revs: Vec<RevIdx> = Vec::new();
    for r in e.review.iter().chain(&e.revisions) {
        if !revs.contains(r) {
            revs.push(*r);
        }
    }
    for r in revs {
        let rev = s.rev(r);
        let id = rev.id;
        let mut times = Vec::new();
        if let Some(at) = rev.created {
            let by = if s.is_mine(rev) {
                String::new()
            } else {
                format!(" by {}", s.raw.users.get(&rev.author).map_or("someone else", String::as_str))
            };
            push(at, "Phabricator", format!("{id} posted{by}"));
            times.push(at);
        }
        if let Some(at) = rev.diff_created.filter(|d| times.iter().all(|t| apart(*t, *d))) {
            push(at, "Phabricator", format!("{id} new diff"));
            times.push(at);
        }
        if let TryState::Push { push: p, .. } = s.try_state(r) {
            push(p.pushed_at, "Treeherder", format!("{id} pushed to try"));
        }
        for repo in s.raw.landings.iter().flat_map(|l| &l.repositories) {
            for m in repo.matches.iter().filter(|m| m.revision == id) {
                push(m.pushed_at, "Treeherder", format!("{id} landed on {}", repo.repository));
            }
        }
        let outcome = !matches!(rev.status, RevStatus::NeedsReview | RevStatus::Draft | RevStatus::Other);
        if outcome && times.iter().all(|t| apart(*t, rev.modified)) {
            push(
                rev.modified,
                "Phabricator",
                format!("{id} last changed, now {}", rev.status.as_str()),
            );
        }
    }
    out.sort_by_key(|ev| ev.at);
    out
}

pub fn ships(s: &Snapshot, e: &Entry) -> Option<String> {
    use crate::train::{Train, day, position};
    let version = crate::train::fixed_in(s.bug(e.bug?))?;
    let today = s.now.to_zoned(jiff::tz::TimeZone::UTC).date();
    let train = s
        .raw
        .trains
        .as_ref()
        .filter(|_| crate::train::trusted(&s.raw, s.now))
        .and_then(|t| position(version, t, today));
    let expected = match train {
        Some(Train::Released) => "expected in a released train".to_owned(),
        Some(Train::Beta(d)) => format!("expected in Beta; scheduled release {}", day(d)),
        Some(Train::Nightly(d)) => format!("expected in Nightly; scheduled Beta merge {}", day(d)),
        _ => "train unknown: calendar missing, stale or version unmatched".to_owned(),
    };
    Some(format!(
        "Bugzilla marks Firefox {version} fixed; {expected}; build inclusion unverified"
    ))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Since {
    pub changed: usize,
    pub new: usize,
    pub highlights: Vec<String>,
}

pub fn since(s: &Snapshot) -> Since {
    let mut out = Since::default();
    let mut counted = Vec::new();
    for section in Section::ALL.into_iter().filter(|x| *x != Section::Team) {
        for i in s.section(section, false) {
            let e = s.entry(i);
            if !e.marks.is_new || e.scope != crate::snapshot::Scope::Mine || counted.contains(&i) {
                continue;
            }
            counted.push(i);
            let id = match e.id {
                crate::snapshot::ItemId::Bug(bug) => format!("Bug {bug}"),
                _ => s.id(e).to_string(),
            };
            match e.changes.first() {
                Some(change) => {
                    out.changed += 1;
                    out.highlights.push(format!("{id} {change}"));
                }
                None => {
                    out.new += 1;
                    out.highlights.push(format!("{id} new in {}", section.title()));
                }
            }
        }
    }
    out
}
