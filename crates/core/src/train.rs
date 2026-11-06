use jiff::civil::Date;

use jiff::Timestamp;

use crate::raw::{Bug, Raw, Trains};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Train {
    Autoland,
    Nightly(Date),
    Beta(Date),
    Released,
}

impl Train {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Autoland => "autoland",
            Self::Nightly(_) => "nightly",
            Self::Beta(_) => "beta",
            Self::Released => "released",
        }
    }
}

pub fn fixed_in(b: &Bug) -> Option<u32> {
    b.release_flags
        .iter()
        .filter(|(_, v)| matches!(v.as_str(), "fixed" | "verified"))
        .filter_map(|(k, _)| k.strip_prefix("cf_status_firefox")?.parse().ok())
        .min()
}

pub fn trusted(raw: &Raw, now: Timestamp) -> bool {
    !raw.errors.iter().any(|e| e.source == "product-details")
        && !raw.stale.iter().any(|e| e.source == "product-details")
        && (0..=48 * 60 * 60).contains(&now.as_second().saturating_sub(raw.fetched_at.as_second()))
}

pub fn current(t: &Trains, today: Date) -> bool {
    today < t.next_merge && today < t.next_release && t.release < t.beta && t.beta < t.nightly
}

pub fn position(version: u32, t: &Trains, today: Date) -> Option<Train> {
    if !current(t, today) {
        return None;
    }
    match version {
        v if v == t.nightly => Some(Train::Nightly(t.next_merge)),
        v if v == t.beta => Some(Train::Beta(t.next_release)),
        v if v <= t.release => Some(Train::Released),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LandsIn {
    pub version: u32,
    pub if_by: Option<Date>,
    pub after: Option<u32>,
}

pub fn lands_in(t: &Trains, today: Date) -> Option<LandsIn> {
    if !current(t, today) {
        return None;
    }
    Some(LandsIn {
        version: t.nightly,
        if_by: t.next_merge.checked_sub(jiff::Span::new().days(1)).ok(),
        after: t.nightly.checked_add(1),
    })
}

pub fn on_autoland(raw: &Raw, bug: &Bug) -> Option<Timestamp> {
    let me = raw.me.phid.as_deref()?;
    if crate::bugs::is_closed(bug) {
        return None;
    }
    let repositories = &raw.landings.as_ref()?.repositories;
    let mine = |rev| raw.revisions.iter().any(|r| r.id == rev && r.author == me);
    let in_repo = |name: &'static str| {
        repositories
            .iter()
            .filter(move |r| r.repository == name)
            .flat_map(|r| r.matches.iter())
            .filter(|m| crate::backout::on_bug(raw, bug.id, m.revision))
    };
    if in_repo("mozilla-central").next().is_some() {
        return None;
    }
    in_repo("autoland").filter(|m| mine(m.revision)).map(|m| m.pushed_at).max()
}

pub fn of_landed(raw: &Raw, bug: &Bug, today: Date) -> Option<Train> {
    if on_autoland(raw, bug).is_some() {
        return Some(Train::Autoland);
    }
    position(fixed_in(bug)?, raw.trains.as_ref()?, today)
}

pub fn day(d: Date) -> String {
    d.strftime("%b %-d").to_string()
}
