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
    BackedOut(crate::ids::BugId),
    Needinfo,
    ReviewRequested(RevIdx),
    Accepted(RevIdx),
    LandingBlocked(RevIdx),
    ChangesRequested(RevIdx),
    InReview(RevIdx),
    Sprint(IssueIdx),
    UpliftApproved(Channel),
    UpliftRequested(Channel),
    RegressionOf(crate::ids::BugId),
    Asked(u32),
    StillCrashing,
    GroupReview(RevIdx),
    Idea(IdeaIdx),
    Landed(crate::ids::BugId),
    Backlog(Unstarted),
    NotUploaded(crate::ids::LocalIdx),
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unstarted {
    Bug(crate::ids::BugId),
    Ticket(IssueIdx),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facets {
    pub tag: &'static str,
    pub gloss: &'static str,
    pub kept_when_landed: bool,
    pub sorts_first: bool,
    pub sorts_last: bool,
}

const fn facets(tag: &'static str, gloss: &'static str) -> Facets {
    Facets {
        tag,
        gloss,
        kept_when_landed: false,
        sorts_first: false,
        sorts_last: false,
    }
}

const fn kept_when_landed(tag: &'static str, gloss: &'static str) -> Facets {
    Facets {
        kept_when_landed: true,
        ..facets(tag, gloss)
    }
}

pub const BACKED_OUT: Facets = Facets {
    sorts_first: true,
    ..kept_when_landed("bo", "backed out")
};
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
pub const UPLIFT_APPROVED: Facets = kept_when_landed("up+", "approved");
pub const UPLIFT_REQUESTED: Facets = kept_when_landed("up?", "asked");
pub const LANDED: Facets = kept_when_landed("ld", "landed");
pub const BACKLOG: Facets = facets("bl", "not started");
pub const NOT_UPLOADED: Facets = facets("loc", "not uploaded");
pub const UNTAGGED: Facets = facets("", "");

pub const TAGS: &[Facets] = &[
    BACKED_OUT,
    NEEDINFO,
    ASKED,
    REVIEW,
    GROUP_REVIEW,
    ACCEPTED,
    LANDING_BLOCKED,
    CHANGES_REQUESTED,
    REGRESSION,
    STILL_CRASHING,
    UPLIFT_APPROVED,
    UPLIFT_REQUESTED,
    LANDED,
    BACKLOG,
    NOT_UPLOADED,
];

impl Reason {
    pub const fn facets(self) -> Facets {
        match self {
            Self::BackedOut(_) => BACKED_OUT,
            Self::Needinfo => NEEDINFO,
            Self::Asked(_) => ASKED,
            Self::ReviewRequested(_) | Self::InReview(_) => REVIEW,
            Self::GroupReview(_) => GROUP_REVIEW,
            Self::Accepted(_) => ACCEPTED,
            Self::LandingBlocked(_) => LANDING_BLOCKED,
            Self::ChangesRequested(_) => CHANGES_REQUESTED,
            Self::RegressionOf(_) => REGRESSION,
            Self::StillCrashing => STILL_CRASHING,
            Self::UpliftApproved(_) => UPLIFT_APPROVED,
            Self::UpliftRequested(_) => UPLIFT_REQUESTED,
            Self::Landed(_) => LANDED,
            Self::Backlog(_) => BACKLOG,
            Self::NotUploaded(_) => NOT_UPLOADED,
            Self::Sprint(_) | Self::Idea(_) | Self::Other => UNTAGGED,
        }
    }

    pub const fn tag(self) -> &'static str {
        self.facets().tag
    }

    pub fn next_step(self, raw: &Raw, stale: bool, has_drift: bool) -> Option<NextStep> {
        let rev = |r: RevIdx| raw.revisions[r.get()].id;
        match self {
            Self::BackedOut(b) => {
                crate::backout::of(raw, b)?;
                Some(NextStep::OpenBackout(b))
            }
            Self::Needinfo => Some(NextStep::AnswerNeedinfo),
            Self::ReviewRequested(r) | Self::GroupReview(r) => Some(NextStep::Review(rev(r))),
            Self::Accepted(r) => Some(NextStep::Land(rev(r))),
            Self::LandingBlocked(r) => crate::guidance::landing_issue(raw, r).map(|i| i.next_step()),
            Self::ChangesRequested(r) => Some(NextStep::AddressFeedback(rev(r))),
            Self::UpliftApproved(c) => Some(NextStep::LandUplift(c)),
            Self::RegressionOf(b) => Some(NextStep::LookAtRegression(b)),
            Self::Asked(_) if stale => Some(NextStep::FollowUp),
            Self::StillCrashing => Some(NextStep::CheckCrashes),
            Self::Idea(_) if has_drift => Some(NextStep::TellIdeaOwner),
            Self::Landed(b) | Self::Backlog(Unstarted::Bug(b)) => Some(NextStep::OpenBug(b)),
            Self::Backlog(Unstarted::Ticket(i)) => Some(NextStep::OpenTicket(i)),
            Self::NotUploaded(_) => Some(NextStep::Upload),
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
    Rule { id: "backedOut",        section: Section::Waiting,  test: backed_out },
    Rule { id: "needinfo",         section: Section::Waiting,  test: needinfo },
    Rule { id: "review",           section: Section::Waiting,  test: review },
    Rule { id: "regression",       section: Section::Waiting,  test: regression },
    Rule { id: "stillCrashing",    section: Section::Waiting,  test: still_crashing },
    Rule { id: "landingBlocked",   section: Section::Waiting,  test: landing_blocked },
    Rule { id: "accepted",         section: Section::Land,     test: accepted },
    Rule { id: "upliftApproved",   section: Section::Land,     test: uplift_approved },
    Rule { id: "changesRequested", section: Section::Returned, test: changes_requested },
    Rule { id: "inReview",         section: Section::Others,   test: in_review },
    Rule { id: "upliftRequested",  section: Section::Others,   test: uplift_requested },
    Rule { id: "asked",            section: Section::Others,   test: asked },
    Rule { id: "notUploaded",      section: Section::Local,    test: not_uploaded },
    Rule { id: "sprint",           section: Section::Sprint,   test: sprint },
    Rule { id: "idea",             section: Section::Ideas,    test: idea },
    Rule { id: "landed",           section: Section::Landed,   test: landed },
    Rule { id: "backlog",          section: Section::Backlog,  test: backlog },
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

fn backed_out(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let bug = ctx.raw.bugs[e.bug?.get()].id;
    let found = crate::backout::of(ctx.raw, bug)?;
    Some(Hit {
        reason: Reason::BackedOut(bug),
        since: Some(found.backout.pushed_at),
        stale: false,
    })
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
    let open = issue.sprint.as_ref().is_some_and(|s| s.state != "closed");
    (mine && open && issue.status_category != StatusCategory::Done).then_some(Hit {
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

pub const LANDED_DAYS: f64 = 30.0;

fn landed(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let bug = &ctx.raw.bugs[e.bug?.get()];
    let since = if let Some(at) = crate::train::on_autoland(ctx.raw, bug) {
        at
    } else {
        let fixed = bug.via.contains(&crate::raw::Via::Landed) && crate::bugs::is_closed(bug) && bug.resolution == "FIXED";
        let at = bug.cf_last_resolved.unwrap_or(bug.last_change_time);
        (fixed && days_between(Some(at), ctx.now) <= LANDED_DAYS).then_some(at)?
    };
    Some(Hit {
        reason: Reason::Landed(bug.id),
        since: Some(since),
        stale: false,
    })
}

fn backlog(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let raw = ctx.raw;
    let ticket = e.issue.and_then(|i| raw.issues.get_index(i.get()).map(|(_, x)| (i, x)));
    if let Some(b) = e.bug {
        let bug = &raw.bugs[b.get()];
        let started = !e.revisions.is_empty()
            || e.review.is_some()
            || !e.local.is_empty()
            || ticket.is_some_and(|(_, x)| x.status_category == StatusCategory::Indeterminate);
        return (bug.assigned_to == raw.me.email && !crate::bugs::is_closed(bug) && !started).then_some(Hit {
            reason: Reason::Backlog(Unstarted::Bug(bug.id)),
            since: bug.creation_time,
            stale: false,
        });
    }
    let (i, issue) = ticket?;
    let mine = issue.assignee.is_some() && issue.assignee == raw.me.jira_email;
    let in_open_sprint = issue.sprint.as_ref().is_some_and(|s| s.state != "closed");
    (mine && issue.status_category == StatusCategory::New && !in_open_sprint).then_some(Hit {
        reason: Reason::Backlog(Unstarted::Ticket(i)),
        since: issue.created,
        stale: false,
    })
}

fn not_uploaded(ctx: &Ctx<'_>, e: &Entry) -> Option<Hit> {
    let local = ctx.raw.local.as_ref()?;
    if e.bug.is_some_and(|b| crate::bugs::is_closed(&ctx.raw.bugs[b.get()])) {
        return None;
    }
    let phid = ctx.raw.me.phid.as_deref();
    let open_rev = e.revisions.iter().any(|r| {
        let rev = &ctx.raw.revisions[r.get()];
        Some(rev.author.as_str()) == phid && !matches!(rev.status, RevStatus::Published | RevStatus::Abandoned)
    });
    if open_rev {
        return None;
    }
    let newest = e
        .local
        .iter()
        .copied()
        .filter(|i| crate::local::sync(local, *i) == crate::local::Sync::NotUploaded)
        .max_by_key(|i| local.commits[i.get()].committed)?;
    Some(Hit {
        reason: Reason::NotUploaded(newest),
        since: Some(local.commits[newest.get()].committed),
        stale: false,
    })
}

pub fn priority_rank(priority: Option<&str>) -> u8 {
    match priority.map(str::trim) {
        Some("P1" | "Highest" | "Blocker") => 1,
        Some("P2" | "High" | "Critical") => 2,
        Some("P3" | "Medium") => 3,
        Some("P4" | "Low") => 4,
        Some("P5" | "Lowest" | "Trivial") => 5,
        _ => 9,
    }
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
    OpenBackout(crate::ids::BugId),
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
    LandUplift(Channel),
    SprintUpdate,
    FixDrift,
    TellIdeaOwner,
    OpenBug(crate::ids::BugId),
    OpenTicket(IssueIdx),
    AddReviewer(crate::ids::RevId),
    Upload,
}

pub const NEW_COMMENT_DAYS: f64 = 14.0;

impl std::fmt::Display for NextStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenBackout(b) => write!(f, "Open the backout of bug {b} in Treeherder"),
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
            Self::LandUplift(c) => write!(f, "Land the uplift to {c}"),
            Self::SprintUpdate => f.write_str("Post a status update or move it out of the sprint"),
            Self::FixDrift => f.write_str("Fix the Jira/Bugzilla mismatch"),
            Self::TellIdeaOwner => f.write_str("Tell the idea's owner where your work stands"),
            Self::OpenBug(b) => write!(f, "Open bug {b} in Bugzilla"),
            Self::AddReviewer(r) => write!(f, "Consider another reviewer for {r} in Phabricator"),
            Self::OpenTicket(_) => f.write_str("Open the ticket in Jira to start it or unassign yourself"),
            Self::Upload => f.write_str("Upload it with `moz-phab submit`, or delete the branch if it's dead"),
        }
    }
}
