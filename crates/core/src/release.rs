use crate::evidence::decides;
use crate::present::Signal;
use crate::raw::RevStatus;
use crate::{Entry, Snapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub text: &'static str,
    pub signal: Signal,
}

const fn cell(text: &'static str, signal: Signal) -> Cell {
    Cell { text, signal }
}

#[derive(Debug)]
pub struct Channel {
    pub name: String,
    pub reported: Cell,
    pub uplift: Cell,
    pub landing: Cell,
}

pub fn shown(s: &Snapshot, e: &Entry) -> bool {
    let landed = e.revisions.iter().chain(&e.review).any(|r| {
        let rev = s.rev(*r);
        rev.status == RevStatus::Published
            || s.raw.landings.as_ref().is_some_and(|report| {
                report
                    .repositories
                    .iter()
                    .any(|repo| repo.matches.iter().any(|m| m.revision == rev.id))
            })
    });
    let listed = e.placement.is_some_and(|p| matches!(p.reason, crate::rules::Reason::Landed(_)));
    e.bug.is_some() && s.raw.trains.is_some() && (decides(e) || landed || listed)
}

pub fn matrix(s: &Snapshot, e: &Entry) -> Option<Vec<Channel>> {
    let bug = s.bug(e.bug?);
    let trains = s.raw.trains.as_ref()?;
    let mut channels = vec![
        (format!("Nightly {}", trains.nightly), trains.nightly.to_string(), None),
        (format!("Beta {}", trains.beta), trains.beta.to_string(), Some("beta".to_owned())),
        (
            format!("Release {}", trains.release),
            trains.release.to_string(),
            Some("release".to_owned()),
        ),
    ];
    channels.extend(
        trains
            .esr
            .iter()
            .map(|v| (format!("ESR {v}"), format!("_esr{v}"), Some(format!("esr{v}")))),
    );
    let nightly_landing = landing(s, e);
    Some(
        channels
            .into_iter()
            .map(|(name, version, approval)| Channel {
                reported: reported(bug.status_for(&version)),
                uplift: match approval {
                    None => cell("—", Signal::Unknown),
                    Some(channel) => uplift(s, bug.id, &channel),
                },
                landing: if version == trains.nightly.to_string() {
                    nightly_landing
                } else {
                    cell("not checked", Signal::Unknown)
                },
                name,
            })
            .collect(),
    )
}

fn reported(status: Option<&str>) -> Cell {
    match status {
        None => cell("unknown", Signal::Unknown),
        Some("---") => cell("not set", Signal::Unknown),
        Some("fixed") => cell("fixed", Signal::Good),
        Some("verified") => cell("verified", Signal::Good),
        Some("affected") => cell("affected", Signal::Pending),
        Some("fix-optional") => cell("fix-optional", Signal::Known),
        Some("unaffected") => cell("unaffected", Signal::Known),
        Some("wontfix") => cell("wontfix", Signal::Known),
        Some("disabled") => cell("disabled", Signal::Known),
        Some(_) => cell("other", Signal::Known),
    }
}

fn uplift(s: &Snapshot, bug: crate::BugId, channel: &str) -> Cell {
    let Some(uplift) = s.raw.uplift.get(&bug) else {
        return cell("unknown", Signal::Unknown);
    };
    let flags: Vec<&str> = uplift
        .approvals
        .iter()
        .filter(|(c, _)| c == channel)
        .map(|(_, status)| status.as_str())
        .collect();
    if flags.contains(&"-") {
        cell("denied", Signal::Bad)
    } else if flags.contains(&"?") {
        cell("requested", Signal::Pending)
    } else if flags.contains(&"+") {
        cell("approved", Signal::Good)
    } else {
        cell("no flag", Signal::Unknown)
    }
}

fn landing(s: &Snapshot, e: &Entry) -> Cell {
    let Some(report) = &s.raw.landings else {
        return cell("not checked", Signal::Unknown);
    };
    let revs: Vec<_> = e.revisions.iter().chain(&e.review).map(|r| s.rev(*r)).collect();
    if !revs.iter().any(|rev| report.revisions.contains(&rev.id)) {
        return cell("not checked", Signal::Unknown);
    }
    let mut failed = false;
    let mut seen = None;
    for repo in &report.repositories {
        let found = repo.matches.iter().find_map(|m| {
            let rev = revs.iter().find(|rev| rev.id == m.revision)?;
            Some(rev.diff_created.is_some_and(|d| d > m.pushed_at))
        });
        match (found, repo.repository.as_str()) {
            (Some(older), "mozilla-central") => seen = Some(("central", older)),
            (Some(older), "autoland") if seen.is_none() => seen = Some(("autoland", older)),
            (None, _) if repo.error.is_some() => failed = true,
            _ => {}
        }
    }
    match seen {
        Some((_, true)) => cell("older diff", Signal::Pending),
        Some((repo, false)) => cell(repo, Signal::Good),
        None if failed => cell("lookup failed", Signal::Pending),
        None => cell("not seen", Signal::Unknown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::Raw;
    use crate::{CoreConfig, build_snapshot};

    fn now() -> jiff::Timestamp {
        "2026-11-03T20:00:00Z".parse().unwrap()
    }

    fn raw() -> Raw {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/raw.json");
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn channels(raw: Raw) -> Vec<Channel> {
        let s = build_snapshot(raw, &CoreConfig::default(), now());
        let e = s.entries.iter().find(|e| e.bug.is_some_and(|b| s.bug(b).id.0 == 1995008)).unwrap();
        matrix(&s, e).unwrap()
    }

    fn landings(error: Option<&str>, matched: bool, pushed_at: &str) -> crate::raw::Landings {
        let matches = if matched {
            serde_json::json!([{"revision":321987,"commit":"a".repeat(40),"pushRevision":"b".repeat(40),
                "url":"https://treeherder.mozilla.org/jobs?repo=autoland","pushedAt":pushed_at,
                "health":null,"healthError":null}])
        } else {
            serde_json::json!([])
        };
        serde_json::from_value(serde_json::json!({
            "revisions":[321987],"repositories":[{
                "repository":"autoland","checkedAt":"2026-11-03T19:00:00Z","pushLimit":100,
                "scannedPushes":40,"oldestPush":null,"error":error,"matches":matches}]
        }))
        .unwrap()
    }

    #[test]
    fn every_channel_has_a_column_newest_first() {
        let names: Vec<String> = channels(raw()).into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["Nightly 161", "Beta 160", "Release 159", "ESR 153", "ESR 115"]);
    }

    #[test]
    fn fixed_on_nightly_with_a_requested_beta_uplift_and_an_unaffected_esr() {
        let mut raw = raw();
        let bug = raw.bugs.iter_mut().find(|b| b.id.0 == 1995008).unwrap();
        bug.release_flags.insert("cf_status_firefox161".into(), "fixed".into());
        bug.release_flags.insert("cf_status_firefox160".into(), "affected".into());
        bug.release_flags.insert("cf_status_firefox159".into(), "---".into());
        bug.release_flags.insert("cf_status_firefox_esr153".into(), "unaffected".into());
        raw.uplift.insert(
            bug.id,
            crate::raw::Uplift {
                patch: true,
                approvals: vec![
                    ("beta".into(), "+".into()),
                    ("beta".into(), "?".into()),
                    ("esr115".into(), "-".into()),
                ],
                flags: Vec::new(),
            },
        );
        let c = channels(raw);
        let reported: Vec<&str> = c.iter().map(|c| c.reported.text).collect();
        assert_eq!(reported, ["fixed", "affected", "not set", "unaffected", "unknown"]);
        let uplift: Vec<&str> = c.iter().map(|c| c.uplift.text).collect();
        assert_eq!(uplift, ["—", "requested", "no flag", "no flag", "denied"]);
        assert_eq!(c[1].uplift.signal, Signal::Pending);
    }

    #[test]
    fn without_attachments_read_an_uplift_is_unknown_not_absent() {
        let c = channels(raw());
        assert_eq!(c[1].uplift, cell("unknown", Signal::Unknown));
    }

    #[test]
    fn landing_says_which_repository_and_never_claims_other_channels() {
        let c = channels(raw());
        assert_eq!(c[0].landing.text, "not checked", "no landing report at all");

        let mut r = raw();
        r.landings = Some(landings(None, true, "2026-11-03T10:00:00Z"));
        let c = channels(r);
        assert_eq!(c[0].landing, cell("autoland", Signal::Good));
        assert!(
            c[1..].iter().all(|c| c.landing.text == "not checked"),
            "only Nightly's repos are scanned"
        );
    }

    #[test]
    fn a_missed_scan_a_failed_scan_and_an_older_diff_read_differently() {
        let mut r = raw();
        r.landings = Some(landings(None, false, "2026-11-03T10:00:00Z"));
        assert_eq!(channels(r)[0].landing.text, "not seen");

        let mut r = raw();
        r.landings = Some(landings(Some("HTTP 503"), false, "2026-11-03T10:00:00Z"));
        assert_eq!(channels(r)[0].landing.text, "lookup failed");

        let mut r = raw();
        r.landings = Some(landings(None, true, "2026-10-20T10:00:00Z"));
        assert_eq!(channels(r)[0].landing, cell("older diff", Signal::Pending));
    }

    #[test]
    fn shown_for_landing_and_landed_items_only() {
        let s = build_snapshot(raw(), &CoreConfig::default(), now());
        let e = s.entries.iter().find(|e| e.bug.is_some_and(|b| s.bug(b).id.0 == 1995008)).unwrap();
        assert!(!shown(&s, e), "a needinfo doesn't need the matrix");

        let mut r = raw();
        r.landings = Some(landings(None, true, "2026-11-03T10:00:00Z"));
        let s = build_snapshot(r, &CoreConfig::default(), now());
        let e = s.entries.iter().find(|e| e.bug.is_some_and(|b| s.bug(b).id.0 == 1995008)).unwrap();
        assert!(shown(&s, e), "a landed item does");
    }
}
