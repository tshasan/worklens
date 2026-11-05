use jiff::Timestamp;
use serde::Serialize;

use crate::ids::RevIdx;
use crate::raw::{Bug, RevStatus};
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

fn fixed_in(b: &Bug) -> Option<u32> {
    b.release_flags
        .iter()
        .filter(|(_, v)| matches!(v.as_str(), "fixed" | "verified"))
        .filter_map(|(k, _)| k.strip_prefix("cf_status_firefox")?.parse().ok())
        .min()
}

pub fn ships(s: &Snapshot, e: &Entry) -> Option<String> {
    let version = fixed_in(s.bug(e.bug?))?;
    let t = s.raw.trains.as_ref()?;
    let day = |d: jiff::civil::Date| d.strftime("%b %-d").to_string();
    if version <= t.release {
        Some(format!("Firefox {version} has the fix and is released (Bugzilla status)"))
    } else if version == t.beta {
        Some(format!(
            "Fixed in Beta {version}; Firefox {version} releases {} (Bugzilla status)",
            day(t.next_release)
        ))
    } else if version == t.nightly {
        Some(format!(
            "Fixed in Nightly {version}; it becomes Beta on {} (Bugzilla status)",
            day(t.next_merge)
        ))
    } else {
        None
    }
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
            let id = s.id(e);
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
