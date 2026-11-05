use jiff::Timestamp;

use crate::bugs::days_between;
use crate::ids::{IdeaIdx, IssueIdx, RevIdx};
use crate::raw::{Bug, Raw, RevStatus, Revision, StatusCategory};
use crate::section::Section;
use crate::snapshot::Entry;

pub const STALE_DAYS: f64 = 7.0;

pub const PING_REVIEWER: i64 = 3;
pub const FIND_ANOTHER_REVIEWER: i64 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Critical,
    Important,
    Other,
}

impl Tier {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Important => "important",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Needinfo,
    ReviewRequested(RevIdx),
    Accepted(RevIdx),
    LandingBlocked(RevIdx),
    ChangesRequested(RevIdx),
    InReview(RevIdx),
    Sprint(IssueIdx),
    UpliftApproved(Channel),
    NeedsUplift,
    UpliftRequested(Channel),
    RegressionOf(crate::ids::BugId),
    Asked(u32),
    StillCrashing,
    BetaStatusUnset(u32),
    GroupReview(RevIdx),
    Idea(IdeaIdx),
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facets {
    pub tag: &'static str,
    pub gloss: &'static str,
    pub kept_when_landed: bool,
    pub sorts_last: bool,
}

const fn facets(tag: &'static str, gloss: &'static str) -> Facets {
    Facets {
        tag,
        gloss,
        kept_when_landed: false,
        sorts_last: false,
    }
}

const fn kept_when_landed(tag: &'static str, gloss: &'static str) -> Facets {
    Facets {
        kept_when_landed: true,
        ..facets(tag, gloss)
    }
}

pub const NEEDINFO: Facets = facets("ni?", "asked of you");
pub const ASKED: Facets = facets("ni>", "you asked");
pub const REVIEW: Facets = facets("r?", "review");
pub const GROUP_REVIEW: Facets = Facets {
    sorts_last: true,
    ..facets("r?#", "group review")
};
pub const ACCEPTED: Facets = facets("r+", "land it");
pub const LANDING_BLOCKED: Facets = facets("chk", "check landing");
pub const CHANGES_REQUESTED: Facets = facets("r-", "needs changes");
pub const REGRESSION: Facets = facets("reg", "regression");
pub const STILL_CRASHING: Facets = kept_when_landed("cr", "crashing");
pub const NEEDS_UPLIFT: Facets = kept_when_landed("up", "needs uplift");
pub const UPLIFT_APPROVED: Facets = kept_when_landed("up+", "approved");
pub const UPLIFT_REQUESTED: Facets = kept_when_landed("up?", "asked");
pub const BETA_STATUS_UNSET: Facets = kept_when_landed("st?", "no Beta status");
pub const UNTAGGED: Facets = facets("", "");

pub const TAGS: &[Facets] = &[
    NEEDINFO,
    ASKED,
    REVIEW,
    GROUP_REVIEW,
    ACCEPTED,
    LANDING_BLOCKED,
    CHANGES_REQUESTED,
    REGRESSION,
    STILL_CRASHING,
    NEEDS_UPLIFT,
    UPLIFT_APPROVED,
    UPLIFT_REQUESTED,
    BETA_STATUS_UNSET,
];

impl Reason {
    pub const fn facets(self) -> Facets {
        match self {
            Self::Needinfo => NEEDINFO,
            Self::Asked(_) => ASKED,
            Self::ReviewRequested(_) | Self::InReview(_) => REVIEW,
            Self::GroupReview(_) => GROUP_REVIEW,
            Self::Accepted(_) => ACCEPTED,
            Self::LandingBlocked(_) => LANDING_BLOCKED,
            Self::ChangesRequested(_) => CHANGES_REQUESTED,
            Self::RegressionOf(_) => REGRESSION,
            Self::StillCrashing => STILL_CRASHING,
            Self::NeedsUplift => NEEDS_UPLIFT,
            Self::UpliftApproved(_) => UPLIFT_APPROVED,
            Self::UpliftRequested(_) => UPLIFT_REQUESTED,
            Self::BetaStatusUnset(_) => BETA_STATUS_UNSET,
            Self::Sprint(_) | Self::Idea(_) | Self::Other => UNTAGGED,
        }
    }

    pub const fn tag(self) -> &'static str {
        self.facets().tag
    }

    pub fn next_step(self, raw: &Raw, stale: bool, has_drift: bool) -> Option<NextStep> {
        let rev = |r: RevIdx| raw.revisions[r.get()].id;
        match self {
            Self::Needinfo => Some(NextStep::AnswerNeedinfo),
            Self::ReviewRequested(r) | Self::GroupReview(r) => Some(NextStep::Review(rev(r))),
            Self::Accepted(r) => Some(NextStep::Land(rev(r))),
            Self::LandingBlocked(r) => crate::guidance::landing_issue(raw, r).map(|i| i.next_step()),
            Self::ChangesRequested(r) => Some(NextStep::AddressFeedback(rev(r))),
            Self::UpliftApproved(c) => Some(NextStep::LandUplift(c)),
            Self::NeedsUplift => Some(NextStep::RequestUplift),
            Self::RegressionOf(b) => Some(NextStep::LookAtRegression(b)),
            Self::Asked(_) if stale => Some(NextStep::FollowUp),
            Self::StillCrashing => Some(NextStep::CheckCrashes),
            Self::BetaStatusUnset(v) => Some(NextStep::SetBetaStatus(v)),
            Self::Idea(_) if has_drift => Some(NextStep::TellIdeaOwner),
            Self::InReview(_) | Self::Sprint(_) | Self::Other | Self::UpliftRequested(_) | Self::Asked(_) | Self::Idea(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub section: Section,
    pub reason: Reason,
    pub since: Option<Timestamp>,
    pub tier: Tier,
    pub stale: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Beta(u32),
    Esr(u32),
}

impl Channel {
    pub fn all(t: &crate::raw::Trains) -> impl Iterator<Item = Self> + '_ {
        std::iter::once(Self::Beta(t.beta)).chain(t.esr.iter().map(|v| Self::Esr(*v)))
    }

    pub fn approval_name(self) -> String {
        match self {
            Self::Beta(_) => "beta".into(),
            Self::Esr(v) => format!("esr{v}"),
        }
    }

    pub fn status_name(self) -> String {
        match self {
            Self::Beta(v) => v.to_string(),
            Self::Esr(v) => format!("_esr{v}"),
        }
    }
}

impl std::fmt::Display for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Beta(v) => write!(f, "Beta {v}"),
            Self::Esr(v) => write!(f, "ESR {v}"),
        }
    }
}

pub fn is_tracked(flag: &str) -> bool {
    flag == "+" || flag == "blocking"
}

fn is_fixed(status: Option<&str>) -> bool {
    matches!(status, Some("fixed" | "verified"))
}

pub struct Ctx<'a> {
    pub raw: &'a Raw,
    pub now: Timestamp,
}

pub struct Hit {
    pub reason: Reason,
    pub since: Option<Timestamp>,
    pub stale: bool,
}

pub struct Rule {
    pub id: &'static str,
    pub section: Section,
    pub test: fn(&Ctx<'_>, &Entry) -> Option<Hit>,
}

#[rustfmt::skip]
pub const RULES: &[Rule] = &[
    Rule { id: "needinfo",         section: Section::Waiting,  test: needinfo },
    Rule { id: "review",           section: Section::Waiting,  test: review },
    Rule { id: "regression",       section: Section::Waiting,  test: regression },
    Rule { id: "stillCrashing",    section: Section::Waiting,  test: still_crashing },
    Rule { id: "landingBlocked",   section: Section::Waiting,  test: landing_blocked },
    Rule { id: "accepted",         section: Section::Land,     test: accepted },
    Rule { id: "upliftApproved",   section: Section::Land,     test: uplift_approved },
    Rule { id: "changesRequested", section: Section::Returned, test: changes_requested },
    Rule { id: "needsUplift",      section: Section::Uplift,   test: needs_uplift },
    Rule { id: "betaStatusUnset",  section: Section::Uplift,   test: beta_status_unset },
    Rule { id: "inReview",         section: Section::Others,   test: in_review },
    Rule { id: "upliftRequested",  section: Section::Others,   test: uplift_requested },
    Rule { id: "asked",            section: Section::Others,   test: asked },
    Rule { id: "sprint",           section: Section::Sprint,   test: sprint },
    Rule { id: "idea",             section: Section::Ideas,    test: idea },
];

pub fn place(ctx: &Ctx<'_>, e: &Entry) -> Placement {
    let tier = tier_of(ctx.raw, e);
    RULES
        .iter()
        .find_map(|rule| (rule.test)(ctx, e).map(|hit| (rule.section, hit)))
        .map_or_else(
            || Placement {
                section: Section::Other,
                reason: Reason::Other,
                since: other_since(ctx.raw, e),
                tier,
                stale: false,
            },
            |(section, hit)| Placement {
                section,
                reason: hit.reason,
                since: hit.since,
                tier,
                stale: hit.stale,
            },
        )
}

fn tier_of(raw: &Raw, e: &Entry) -> Tier {
    let Some(bug) = e.bug.map(|b| &raw.bugs[b.get()]) else {
        return Tier::Other;
    };
    let has = |k: &str| bug.keywords.iter().any(|x| x == k);
    let tracked = bug
        .release_flags
        .iter()
        .any(|(k, v)| k.starts_with("cf_tracking_firefox") && is_tracked(v));
    if bug.severity == "S1" || has("sec-critical") || tracked {
        Tier::Critical
    } else if bug.severity == "S2" || has("sec-high") {
        Tier::Important
    } else {
        Tier::Other
    }
}

fn needinfo(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let flag = &ctx.raw.bugs[e.bug?.get()].flags[e.needinfo? as usize];
    Some(Hit {
        reason: Reason::Needinfo,
        since: Some(flag.creation_date),
        stale: false,
    })
}

fn review(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let r = e.review?;
    let group = ctx.raw.group_review.contains_key(&ctx.raw.revisions[r.get()].id);
    Some(Hit {
        reason: if group {
            Reason::GroupReview(r)
        } else {
            Reason::ReviewRequested(r)
        },
        since: Some(review_since(&ctx.raw.revisions[r.get()])),
        stale: false,
    })
}

fn review_since(rev: &Revision) -> Timestamp {
    rev.diff_created.unwrap_or(rev.modified)
}

fn mine_with(ctx: &Ctx<'_>, e: &Entry, status: RevStatus) -> Option<RevIdx> {
    let phid = ctx.raw.me.phid.as_deref()?;
    e.revisions.iter().copied().find(|r| {
        let rev = &ctx.raw.revisions[r.get()];
        rev.author == phid && rev.status == status
    })
}

fn on_revision(ctx: &Ctx<'_>, r: RevIdx, reason: Reason, can_go_stale: bool) -> Hit {
    let modified = ctx.raw.revisions[r.get()].modified;
    let stale = can_go_stale && days_between(Some(modified), ctx.now) > STALE_DAYS;
    Hit {
        reason,
        since: Some(modified),
        stale,
    }
}

fn accepted(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let r = mine_with(ctx, e, RevStatus::Accepted)?;
    Some(on_revision(ctx, r, Reason::Accepted(r), true))
}

fn landing_blocked(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let r = mine_with(ctx, e, RevStatus::Accepted)?;
    crate::guidance::landing_issue(ctx.raw, r)?;
    Some(on_revision(ctx, r, Reason::LandingBlocked(r), false))
}

fn changes_requested(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let r = mine_with(ctx, e, RevStatus::NeedsRevision)?;
    Some(on_revision(ctx, r, Reason::ChangesRequested(r), false))
}

fn in_review(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let r = mine_with(ctx, e, RevStatus::NeedsReview)?;
    let since = review_since(&ctx.raw.revisions[r.get()]);
    let stale = crate::bugs::business_days_between(since, ctx.now) >= FIND_ANOTHER_REVIEWER;
    Some(Hit {
        reason: Reason::InReview(r),
        since: Some(since),
        stale,
    })
}

fn sprint(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let i = e.issue?;
    let (_, issue) = ctx.raw.issues.get_index(i.get())?;
    let mine = issue.assignee.is_some() && issue.assignee == ctx.raw.me.jira_email;
    (mine && issue.sprint.is_some() && issue.status_category != StatusCategory::Done).then_some(Hit {
        reason: Reason::Sprint(i),
        since: Some(issue.updated),
        stale: false,
    })
}

fn idea(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let crate::snapshot::ItemId::Idea(i) = e.id else { return None };
    let (_, idea) = ctx.raw.roadmap.ideas.get_index(i.get())?;
    Some(Hit {
        reason: Reason::Idea(i),
        since: Some(idea.updated),
        stale: false,
    })
}

fn uplift_parts<'a>(ctx: &Ctx<'a>, e: &Entry) -> Option<(&'a Bug, &'a crate::raw::Uplift, &'a crate::raw::Trains)> {
    let bug = &ctx.raw.bugs[e.bug?.get()];
    Some((bug, ctx.raw.uplift.get(&bug.id)?, ctx.raw.trains.as_ref()?))
}

fn regression(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let bug = &ctx.raw.bugs[e.bug?.get()];
    if crate::bugs::is_closed(bug) {
        return None;
    }
    let mine = |id: &crate::ids::BugId| ctx.raw.bugs.iter().any(|b| b.id == *id && b.via.contains(&crate::raw::Via::Landed));
    let regressor = bug.regressed_by.iter().find(|id| mine(id))?;
    Some(Hit {
        reason: Reason::RegressionOf(*regressor),
        since: Some(bug.last_change_time),
        stale: false,
    })
}

pub const CRASHES_AFTER_FIX: u32 = 5;

fn still_crashing(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let bug = &ctx.raw.bugs[e.bug?.get()];
    let crashes = ctx.raw.crashes.get(&bug.id).filter(|c| c.count >= CRASHES_AFTER_FIX)?;
    let since = crashes.since.to_zoned(jiff::tz::TimeZone::UTC).ok()?.timestamp();
    Some(Hit {
        reason: Reason::StillCrashing,
        since: Some(since),
        stale: false,
    })
}

fn asked(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let bug = &ctx.raw.bugs[e.bug?.get()];
    let me = ctx.raw.me.email.as_str();
    let (at, flag) = bug
        .flags
        .iter()
        .enumerate()
        .find(|(_, f)| f.name == "needinfo" && f.status == "?" && f.setter == me && f.requestee.as_deref().is_some_and(|r| r != me))?;
    let stale = days_between(Some(flag.creation_date), ctx.now) > STALE_DAYS;
    Some(Hit {
        reason: Reason::Asked(u32::try_from(at).ok()?),
        since: Some(flag.creation_date),
        stale,
    })
}

fn uplift_approved(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let (bug, up, trains) = uplift_parts(ctx, e)?;
    let channel =
        Channel::all(trains).find(|c| up.approval(&c.approval_name()) == Some("+") && !is_fixed(bug.status_for(&c.status_name())))?;
    Some(Hit {
        reason: Reason::UpliftApproved(channel),
        since: Some(bug.last_change_time),
        stale: false,
    })
}

fn needs_uplift(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let (bug, up, trains) = uplift_parts(ctx, e)?;
    let fixed = is_fixed(bug.status_for(&trains.nightly.to_string()));
    let skipped = bug.whiteboard.contains("[checkin-needed-beta]") || assignee_asked(bug) || has_open_regression(ctx.raw, bug);
    let affected =
        Channel::all(trains).any(|c| bug.status_for(&c.status_name()) == Some("affected") && up.approval(&c.approval_name()).is_none());
    (fixed && up.patch && !skipped && affected).then_some(Hit {
        reason: Reason::NeedsUplift,
        since: Some(bug.last_change_time),
        stale: false,
    })
}

fn assignee_asked(bug: &Bug) -> bool {
    bug.flags
        .iter()
        .any(|f| f.name == "needinfo" && f.status == "?" && f.requestee.as_deref() == Some(bug.assigned_to.as_str()))
}

fn has_open_regression(raw: &Raw, bug: &Bug) -> bool {
    raw.bugs
        .iter()
        .any(|b| b.regressed_by.contains(&bug.id) && !crate::bugs::is_closed(b))
}

fn beta_status_unset(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let (bug, up, trains) = uplift_parts(ctx, e)?;
    let fixed = is_fixed(bug.status_for(&trains.nightly.to_string()));
    let unset = matches!(bug.status_for(&trains.beta.to_string()), Some("---") | None);
    (fixed && up.patch && unset).then_some(Hit {
        reason: Reason::BetaStatusUnset(trains.beta),
        since: Some(bug.last_change_time),
        stale: false,
    })
}

fn uplift_requested(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let (bug, up, trains) = uplift_parts(ctx, e)?;
    let channel = Channel::all(trains).find(|c| up.approval(&c.approval_name()) == Some("?"))?;
    Some(Hit {
        reason: Reason::UpliftRequested(channel),
        since: Some(bug.last_change_time),
        stale: false,
    })
}

fn other_since(raw: &Raw, e: &Entry) -> Option<Timestamp> {
    match (e.bug, e.issue) {
        (Some(b), _) => Some(raw.bugs[b.get()].last_change_time),
        (None, Some(i)) => raw.issues.get_index(i.get()).map(|(_, issue)| issue.updated),
        (None, None) => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NextStep {
    AnswerNeedinfo,
    Review(crate::ids::RevId),
    Land(crate::ids::RevId),
    ResolveClassification(crate::ids::RevId),
    CheckReview(crate::ids::RevId),
    InspectTry(crate::ids::RevId),
    CheckDependency(crate::ids::RevId),
    CheckStack(crate::ids::RevId),
    AddressFeedback(crate::ids::RevId),
    ReadComment,
    PingReviewer(crate::ids::RevId),
    FindAnotherReviewer(crate::ids::RevId),
    LookAtRegression(crate::ids::BugId),
    FollowUp,
    CheckCrashes,
    SetBetaStatus(u32),
    LandUplift(Channel),
    RequestUplift,
    SprintUpdate,
    FixDrift,
    TellIdeaOwner,
}

pub const NEW_COMMENT_DAYS: f64 = 14.0;

impl std::fmt::Display for NextStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AnswerNeedinfo => f.write_str("Answer the needinfo"),
            Self::Review(r) => write!(f, "Review {r}"),
            Self::Land(r) => write!(f, "Land {r}"),
            Self::ResolveClassification(r) => write!(f, "Resolve data classification on {r} in Phabricator"),
            Self::CheckReview(r) => write!(f, "Check outstanding reviews on {r} in Phabricator"),
            Self::InspectTry(r) => write!(f, "Inspect {r}'s try push in Treeherder"),
            Self::CheckDependency(r) => write!(f, "Inspect dependency {r} in Phabricator"),
            Self::CheckStack(r) => write!(f, "Check {r}'s parent revisions in Phabricator"),
            Self::AddressFeedback(r) => write!(f, "Address feedback on {r}"),
            Self::ReadComment => f.write_str("Read the new comment"),
            Self::PingReviewer(r) => write!(f, "Ping the reviewer on {r}"),
            Self::FindAnotherReviewer(r) => write!(f, "Find another reviewer for {r}, or ping again"),
            Self::LookAtRegression(b) => write!(f, "Look at the regression from your fix in bug {b}"),
            Self::FollowUp => f.write_str("Follow up on your question, or ask someone else"),
            Self::CheckCrashes => f.write_str("Check whether the fix worked: look at the crashes on crash-stats"),
            Self::SetBetaStatus(v) => write!(f, "Set status-firefox{v} to affected or unaffected"),
            Self::LandUplift(c) => write!(f, "Land the uplift to {c}"),
            Self::RequestUplift => f.write_str("Request an uplift in Lando, or set the status flags if it isn't needed"),
            Self::SprintUpdate => f.write_str("Post a status update or move it out of the sprint"),
            Self::FixDrift => f.write_str("Fix the Jira/Bugzilla mismatch"),
            Self::TellIdeaOwner => f.write_str("Tell the idea's owner where your work stands"),
        }
    }
}
