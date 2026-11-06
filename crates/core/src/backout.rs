use crate::ids::{BugId, RevId};
use crate::raw::{Backout, Landing, Raw};

pub struct Found<'a> {
    pub repository: &'a str,
    pub backout: &'a Backout,
    pub landing: &'a Landing,
}

pub fn on_bug(raw: &Raw, bug: BugId, rev: RevId) -> bool {
    raw.revisions.iter().any(|r| r.id == rev && r.bug == Some(bug)) || raw.bug_revisions.get(&bug).is_some_and(|l| l.contains(&rev))
}

pub fn of(raw: &Raw, bug: BugId) -> Option<Found<'_>> {
    let me = raw.me.phid.as_deref()?;
    let repositories = &raw.landings.as_ref()?.repositories;
    repositories
        .iter()
        .flat_map(|repo| {
            repo.matches.iter().filter_map(move |landing| {
                if !on_bug(raw, bug, landing.revision) || !raw.revisions.iter().any(|r| r.id == landing.revision && r.author == me) {
                    return None;
                }
                let backout = repo
                    .backouts
                    .iter()
                    .filter(|b| affects(b, landing) && b.pushed_at >= landing.pushed_at)
                    .max_by_key(|b| b.pushed_at)?;
                if repo
                    .matches
                    .iter()
                    .any(|m| m.revision == landing.revision && m.pushed_at > backout.pushed_at)
                {
                    return None;
                }
                Some(Found {
                    repository: &repo.repository,
                    backout,
                    landing,
                })
            })
        })
        .max_by_key(|f| f.backout.pushed_at)
}

pub fn affects(backout: &Backout, landing: &Landing) -> bool {
    backout.targets.iter().any(|target| {
        (12..=40).contains(&target.len())
            && target.bytes().all(|b| b.is_ascii_hexdigit())
            && landing.commit.to_ascii_lowercase().starts_with(&target.to_ascii_lowercase())
    })
}

pub fn after<'a>(raw: &'a Raw, repository: &str, landing: &Landing) -> Option<&'a Backout> {
    raw.landings
        .as_ref()?
        .repositories
        .iter()
        .filter(|r| r.repository == repository)
        .flat_map(|r| &r.backouts)
        .filter(|b| affects(b, landing) && b.pushed_at >= landing.pushed_at)
        .min_by_key(|b| b.pushed_at)
}

pub struct Gap(pub i64);

impl std::fmt::Display for Gap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let minutes = self.0.max(0) / 60;
        match minutes {
            m if m < 60 => write!(f, "{m}m"),
            m if m < 48 * 60 => write!(f, "{}h", m / 60),
            m => write!(f, "{} days", m / (24 * 60)),
        }
    }
}
