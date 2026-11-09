use jiff::Timestamp;

use crate::bugs::{HumanDays, days_between};
use crate::drift::{Drift, Level};
use crate::ids::LocalIdx;
use crate::raw::{Local, LocalCommit, Raw, RevStatus, Upload};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sync {
    NotUploaded,
    Same,
    Differs,
    Unchecked,
    Closed,
    Superseded,
}

impl Sync {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotUploaded => "not_uploaded",
            Self::Same => "same",
            Self::Differs => "differs",
            Self::Unchecked => "unchecked",
            Self::Closed => "closed",
            Self::Superseded => "superseded",
        }
    }

    pub const fn is_live(self) -> bool {
        !matches!(self, Self::Closed | Self::Superseded)
    }
}

pub fn commit(raw: &Raw, i: LocalIdx) -> Option<&LocalCommit> {
    raw.local.as_ref()?.commits.get(i.get())
}

pub fn upload<'a>(raw: &'a Raw, c: &LocalCommit) -> Option<&'a Upload> {
    raw.local.as_ref()?.uploads.get(&c.revision?)
}

pub fn sync(local: &Local, i: LocalIdx) -> Sync {
    let Some(c) = local.commits.get(i.get()) else {
        return Sync::Unchecked;
    };
    let Some(r) = c.revision else {
        let newer = |o: &LocalCommit| (o.committed, o.sha.as_str()) > (c.committed, c.sha.as_str());
        let copy = |o: &LocalCommit| {
            let same = (!c.fingerprint.is_empty() && o.fingerprint == c.fingerprint)
                || (c.bug.is_some() && o.bug == c.bug && title_of(&o.subject) == title_of(&c.subject));
            let older_than_upload = c.bug.is_some() && o.bug == c.bug && o.revision.is_some() && newer(o);
            older_than_upload || (same && (o.revision.is_some() || newer(o)))
        };
        return if local.commits.iter().any(copy) {
            Sync::Superseded
        } else {
            Sync::NotUploaded
        };
    };
    let newer = |o: &LocalCommit| o.revision == Some(r) && (o.committed, o.sha.as_str()) > (c.committed, c.sha.as_str());
    if local.commits.iter().any(newer) {
        return Sync::Superseded;
    }
    let Some(up) = local.uploads.get(&r) else { return Sync::Unchecked };
    if matches!(up.status, RevStatus::Published | RevStatus::Abandoned) {
        return Sync::Closed;
    }
    match &up.fingerprint {
        None => Sync::Unchecked,
        Some(f) if *f == c.fingerprint => Sync::Same,
        Some(_) => Sync::Differs,
    }
}

pub fn syncs(raw: &Raw) -> Vec<Sync> {
    raw.local
        .as_ref()
        .map_or_else(Vec::new, |l| (0..l.commits.len()).map(|i| sync(l, LocalIdx::new(i))).collect())
}

pub fn is_wip(subject: &str) -> bool {
    let lower = subject.to_ascii_lowercase();
    lower.contains("do not land") || lower.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == "wip")
}

pub fn fixup_of<'a>(local: &'a Local, c: &LocalCommit) -> Option<&'a LocalCommit> {
    let rest = ["fixup! ", "squash! ", "amend! "].iter().find_map(|p| c.subject.strip_prefix(p))?;
    local.commits.iter().find(|o| o.subject == rest && o.revision.is_some())
}

pub fn revision_of(local: &Local, c: &LocalCommit) -> Option<crate::ids::RevId> {
    c.revision.or_else(|| fixup_of(local, c)?.revision)
}

pub fn short(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

pub fn title_of(subject: &str) -> &str {
    let s = subject.trim();
    let mut s = s.strip_prefix("WIP:").or_else(|| s.strip_prefix("WIP ")).unwrap_or(s).trim_start();
    while let Some((head, last)) = s.rsplit_once(' ')
        && is_reviewers(last)
    {
        s = head.trim_end();
    }
    s.trim_end_matches('.')
}

fn is_reviewers(word: &str) -> bool {
    let w = word.trim_end_matches('.');
    (w.starts_with("r?") || w.starts_with("r=")) && w.len() > 2
}

fn ago(at: Timestamp, now: Timestamp) -> String {
    let days = HumanDays(days_between(Some(at), now));
    if days.0 < 1.0 { "today".into() } else { format!("{days} ago") }
}

pub fn drift_of(raw: &Raw, i: LocalIdx, sync: Sync, now: Timestamp) -> Vec<Drift> {
    let mut out = Vec::new();
    let (Some(c), Some(r)) = (commit(raw, i), commit(raw, i).and_then(|c| c.revision)) else {
        return out;
    };
    let Some(up) = upload(raw, c) else { return out };
    if !sync.is_live() {
        return out;
    }
    let sha = short(&c.sha);
    if sync == Sync::Differs {
        let cause = match up.uploaded {
            Some(at) if at > c.committed => format!(
                "the diff was posted {}, after your commit on {}; `moz-phab patch {r}` gets it",
                ago(at, now),
                c.place
            ),
            Some(at) => format!(
                "your commit on {} is newer than the diff posted {}; `moz-phab submit` uploads it",
                c.place,
                ago(at, now)
            ),
            None => format!("on {}; `moz-phab submit` uploads it", c.place),
        };
        out.push(Drift {
            level: Level::Warn,
            message: format!("{r}'s latest diff differs from local commit {sha}"),
            cause,
            fix: None,
        });
    }
    if title_of(&c.subject) != title_of(&up.title) {
        out.push(Drift {
            level: Level::Warn,
            message: format!("{r}'s title differs from local commit {sha}"),
            cause: format!("Phabricator says \"{}\", {} says \"{}\"", up.title, c.place, title_of(&c.subject)),
            fix: None,
        });
    }
    if let (Some(here), Some(there)) = (c.bug, up.bug)
        && here != there
    {
        out.push(Drift {
            level: Level::Warn,
            message: format!("local commit {sha} says bug {here}, {r} is on bug {there}"),
            cause: format!("on {}", c.place),
            fix: None,
        });
    }
    out
}

pub fn unsent_drift(raw: &Raw, commits: &[LocalIdx], now: Timestamp) -> Vec<Drift> {
    let mut out = Vec::new();
    let Some(local) = raw.local.as_ref() else { return out };
    let mut rest: Vec<&LocalCommit> = Vec::new();
    for c in commits.iter().filter_map(|i| local.commits.get(i.get())) {
        match fixup_of(local, c).and_then(|t| t.revision) {
            Some(r) => out.push(Drift {
                level: Level::Warn,
                message: format!("fixup {} for {r} isn't squashed and uploaded", short(&c.sha)),
                cause: format!("on {}, committed {}", c.place, ago(c.committed, now)),
                fix: None,
            }),
            None => rest.push(c),
        }
    }
    if let Some(newest) = rest.iter().max_by_key(|c| c.committed) {
        let n = rest.len();
        let what = if n == 1 {
            "1 local commit isn't".to_owned()
        } else {
            format!("{n} local commits aren't")
        };
        out.push(Drift {
            level: Level::Warn,
            message: format!("{what} on Phabricator"),
            cause: format!(
                "newest on {}, committed {}: \"{}\"",
                newest.place,
                ago(newest.committed, now),
                newest.subject
            ),
            fix: None,
        });
    }
    out
}

pub fn state_text(raw: &Raw, i: LocalIdx, sync: Sync) -> String {
    let Some(local) = raw.local.as_ref() else { return String::new() };
    let Some(c) = local.commits.get(i.get()) else {
        return String::new();
    };
    let rev = revision_of(local, c);
    match (sync, rev) {
        (Sync::NotUploaded, Some(r)) => format!("fixup for {r}, not uploaded"),
        (Sync::NotUploaded, None) => "not on Phabricator".into(),
        (Sync::Same, Some(r)) => format!("matches {r}"),
        (Sync::Differs, Some(r)) => format!("differs from {r}'s latest diff"),
        (Sync::Closed, Some(r)) => format!("{r} is closed"),
        (Sync::Superseded, _) => "older copy".into(),
        (_, Some(r)) => format!("{r}, not compared"),
        (_, None) => "not compared".into(),
    }
}

pub fn line(raw: &Raw, i: LocalIdx, sync: Sync) -> Option<String> {
    let c = commit(raw, i)?;
    Some(format!("{} · {} · {}", short(&c.sha), c.place, state_text(raw, i, sync)))
}
