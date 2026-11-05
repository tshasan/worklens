use serde::Serialize;

use crate::bugs::{State, assignee, bug_url, is_closed, rev_url};
use crate::raw::{Raw, RevStatus};
use crate::{Entry, Snapshot};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependency {
    pub id: String,
    pub title: Option<String>,
    pub status: String,
    pub owner: Option<String>,
    pub url: String,
    pub source: &'static str,
    pub blocked: Option<bool>,
}

pub fn failed(raw: &Raw, source: &str) -> bool {
    raw.errors
        .iter()
        .any(|e| e.source == source || e.source == format!("{source}-dependencies"))
}

pub fn of(s: &Snapshot, e: &Entry) -> Vec<Dependency> {
    let mut out = Vec::new();
    if let Some(bug) = e.bug.map(|b| s.bug(b)) {
        let mut ids = bug.depends_on.clone();
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            let found = s.raw.bugs.iter().find(|b| b.id == id).or_else(|| s.raw.dependencies.bugs.get(&id));
            let known = found.filter(|_| !failed(&s.raw, "bugzilla"));
            out.push(Dependency {
                id: format!("bug {id}"),
                title: known.map(|b| b.summary.clone()),
                status: known.map_or_else(|| "unknown; not fetched or not visible".into(), |b| State(b).to_string()),
                owner: known.and_then(assignee).map(str::to_owned),
                url: bug_url(id),
                source: "bugzilla",
                blocked: known.map(|b| !is_closed(b)),
            });
        }
    }
    for idx in e.revisions.iter().chain(&e.review) {
        let rev = s.rev(*idx);
        let Some(parents) = s.raw.dependencies.parents.get(&rev.id) else {
            out.push(Dependency {
                id: format!("{} stack", rev.id),
                title: None,
                status: "unknown; parent relationships not fetched".into(),
                owner: None,
                url: rev_url(rev.id),
                source: "phabricator",
                blocked: None,
            });
            continue;
        };
        for phid in parents {
            let found = s
                .raw
                .dependencies
                .revisions
                .get(phid)
                .or_else(|| s.raw.revisions.iter().find(|r| r.phid.as_ref() == Some(phid)));
            let known = found.filter(|_| !failed(&s.raw, "phabricator"));
            let id = found.map_or_else(|| format!("{} parent", rev.id), |r| r.id.to_string());
            let url = found.map_or_else(|| rev_url(rev.id), |r| rev_url(r.id));
            if out.iter().any(|d| d.url == url && d.id == id) {
                continue;
            }
            out.push(Dependency {
                id,
                title: known.map(|r| r.title.clone()),
                status: known.map_or_else(
                    || "unknown; not fetched or not visible".into(),
                    |r| {
                        if r.status == RevStatus::Accepted && crate::guidance::classification(&s.raw, r) {
                            "accepted; data classification required".into()
                        } else if r.status == RevStatus::Accepted && crate::guidance::pending_review(r) {
                            "accepted; blocking review outstanding".into()
                        } else {
                            r.status.as_str().into()
                        }
                    },
                ),
                owner: known.map(|r| s.name(&r.author).to_owned()),
                url,
                source: "phabricator",
                blocked: known.and_then(|r| match r.status {
                    RevStatus::Published => Some(false),
                    RevStatus::Accepted => Some(crate::guidance::classification(&s.raw, r) || crate::guidance::pending_review(r)),
                    RevStatus::Other => None,
                    _ => Some(true),
                }),
            });
        }
    }
    out
}
