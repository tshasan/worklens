use std::fmt;

use crate::bugs::rev_url;
use crate::raw::{Raw, RevStatus, ReviewerStatus, TryResult};
use crate::rules::NextStep;
use crate::{RevId, RevIdx};

#[derive(Clone, Copy, Debug)]
pub enum LandingIssue {
    Classification(RevId),
    Review(RevId),
    UnknownReview(RevId),
    Try(RevId, &'static str),
    Parent(RevId, RevId, RevStatus),
    UnknownParent(RevId),
}

impl LandingIssue {
    pub fn next_step(self) -> NextStep {
        match self {
            Self::Classification(r) => NextStep::ResolveClassification(r),
            Self::Review(r) | Self::UnknownReview(r) => NextStep::CheckReview(r),
            Self::Try(r, _) => NextStep::InspectTry(r),
            Self::Parent(_, r, _) => NextStep::CheckDependency(r),
            Self::UnknownParent(r) => NextStep::CheckStack(r),
        }
    }
}

impl fmt::Display for LandingIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Classification(r) => write!(f, "{r} requires data classification before landing"),
            Self::Review(r) => write!(f, "{r} has an outstanding blocking review or requested changes"),
            Self::UnknownReview(r) => write!(f, "{r}'s review state could not be refreshed"),
            Self::Try(r, state) => write!(f, "{r} accepted; {state}"),
            Self::Parent(r, parent, status) => write!(f, "{r} depends on {parent}, which is {}", status.as_str()),
            Self::UnknownParent(r) => write!(f, "{r}'s parent readiness is unknown"),
        }
    }
}

pub(crate) fn classification(raw: &Raw, rev: &crate::raw::Revision) -> bool {
    rev.projects.iter().flatten().any(|p| {
        raw.users
            .get(p)
            .is_some_and(|n| n.trim_start_matches('#') == "needs-data-classification")
    })
}

pub(crate) fn pending_review(rev: &crate::raw::Revision) -> bool {
    rev.reviewers
        .iter()
        .any(|r| matches!(r.status, ReviewerStatus::Blocking | ReviewerStatus::Rejected))
}

pub fn landing_issue(raw: &Raw, idx: RevIdx) -> Option<LandingIssue> {
    let rev = &raw.revisions[idx.get()];
    if rev.status != RevStatus::Accepted {
        return None;
    }
    if raw.errors.iter().any(|e| e.source == "phabricator") {
        return Some(LandingIssue::UnknownReview(rev.id));
    }
    if classification(raw, rev) {
        return Some(LandingIssue::Classification(rev.id));
    }
    if pending_review(rev) {
        return Some(LandingIssue::Review(rev.id));
    }
    if raw.errors.iter().any(|e| e.source == "phabricator-dependencies") {
        return Some(LandingIssue::UnknownParent(rev.id));
    }
    if let Some(parents) = raw.dependencies.parents.get(&rev.id) {
        for phid in parents {
            let parent = raw
                .dependencies
                .revisions
                .get(phid)
                .or_else(|| raw.revisions.iter().find(|r| r.phid.as_ref() == Some(phid)));
            let Some(parent) = parent else {
                return Some(LandingIssue::UnknownParent(rev.id));
            };
            match parent.status {
                RevStatus::Published => {}
                RevStatus::Accepted => {
                    if classification(raw, parent) {
                        return Some(LandingIssue::Classification(parent.id));
                    }
                    if pending_review(parent) {
                        return Some(LandingIssue::Review(parent.id));
                    }
                }
                RevStatus::Other => return Some(LandingIssue::UnknownParent(rev.id)),
                status => return Some(LandingIssue::Parent(rev.id, parent.id, status)),
            }
        }
    }
    if !raw.errors.iter().any(|e| e.source == "treeherder")
        && let Some(push) = raw.try_pushes.as_ref().and_then(|p| p.get(&rev.id))
    {
        let state = if rev.diff_created.is_some_and(|d| push.pushed_at < d) {
            Some("try results belong to an older diff")
        } else if push.failed > 0 || [push.builds, push.lint, push.tests].contains(&TryResult::Fail) {
            Some("try has failing jobs; inspect before landing")
        } else if push.running > 0 || [push.builds, push.lint, push.tests].contains(&TryResult::Running) {
            Some("try jobs are still running")
        } else {
            None
        };
        if let Some(state) = state {
            return Some(LandingIssue::Try(rev.id, state));
        }
    }
    None
}

pub fn next_url(raw: &Raw, step: NextStep) -> Option<String> {
    match step {
        NextStep::ResolveClassification(r) | NextStep::CheckReview(r) | NextStep::CheckDependency(r) | NextStep::CheckStack(r) => {
            Some(rev_url(r))
        }
        NextStep::InspectTry(r) => raw.try_pushes.as_ref()?.get(&r).map(|p| p.url.clone()),
        NextStep::Land(r) => Some(crate::bugs::lando_url(r)),
        _ => None,
    }
}
