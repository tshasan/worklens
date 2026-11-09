use std::collections::{HashMap, HashSet};
use std::fmt::{self, Write as _};

use jiff::Timestamp;

use crate::bugs::{HumanDays, State, days_between, is_closed, is_dead, jira_key_of};
use crate::drift::{Drift, DriftInput, JiraStatuses, Level, drift_of};
use crate::ideas::{Work, idea_of};
use crate::ids::{BugId, BugIdx, EntryIdx, IdeaIdx, IssueIdx, LocalIdx, RevId, RevIdx};
use crate::marks::MarkState;
use crate::raw::{
    Bug, Comment, Flag, Idea, Issue, Raw, RevStatus, ReviewerStatus, Revision, SourceState, StatusCategory, TryPush, TryResult, Via,
};
use crate::rules::{Ctx, FIND_ANOTHER_REVIEWER, NEW_COMMENT_DAYS, NextStep, PING_REVIEWER, Placement, Reason, STALE_DAYS, place};
use crate::section::Section;

#[derive(Clone)]
pub struct CoreConfig {
    pub whiteboard: Option<String>,
    pub sprint_budget: f32,
    pub jira_statuses: JiraStatuses,
    pub jbi_syncs: Vec<crate::drift::Synced>,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            whiteboard: None,
            sprint_budget: 10.0,
            jira_statuses: JiraStatuses::default(),
            jbi_syncs: Vec::new(),
        }
    }
}

impl CoreConfig {
    pub fn whiteboard(&self) -> Option<&str> {
        self.whiteboard.as_deref().filter(|w| !w.is_empty())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemId {
    Bug(BugId),
    Jira(IssueIdx),
    Rev(RevId),
    Idea(IdeaIdx),
    Branch(LocalIdx),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Mine,
    Team,
}

#[derive(Debug)]
pub struct Entry {
    pub id: ItemId,
    pub scope: Scope,
    pub bug: Option<BugIdx>,
    pub issue: Option<IssueIdx>,
    pub idea: Option<IdeaIdx>,
    pub revisions: Vec<RevIdx>,
    pub review: Option<RevIdx>,
    pub needinfo: Option<u32>,
    pub drift: Vec<Drift>,
    pub placement: Option<Placement>,
    pub marks: MarkState,
    pub changes: Vec<String>,
    pub stack: Option<crate::ids::StackIdx>,
    pub local: Vec<LocalIdx>,
}

impl Entry {
    const fn new(id: ItemId) -> Self {
        Self {
            id,
            scope: Scope::Mine,
            bug: None,
            issue: None,
            idea: None,
            revisions: Vec::new(),
            review: None,
            needinfo: None,
            drift: Vec::new(),
            placement: None,
            marks: MarkState::NONE,
            changes: Vec::new(),
            stack: None,
            local: Vec::new(),
        }
    }

    pub fn has_error_drift(&self) -> bool {
        self.drift.iter().any(|d| d.level == Level::Error)
    }
}

#[derive(Debug)]
pub struct SprintSummary {
    pub first: IssueIdx,
    pub budget: f32,
    pub committed: f32,
    pub done: f32,
    pub unestimated: Vec<IssueIdx>,
    pub tickets: Vec<IssueIdx>,
}

#[derive(Clone, Copy, Debug)]
pub enum TryState<'a> {
    Unknown,
    NoPush,
    Push { push: &'a TryPush, outdated: bool },
}

pub struct Snapshot {
    pub raw: Raw,
    pub now: Timestamp,
    pub entries: Vec<Entry>,
    sections: [Vec<EntryIdx>; Section::COUNT],
    pub stacks: Vec<crate::stack::Stack>,
    pub sprint: Option<SprintSummary>,
    pub syncs: Vec<crate::local::Sync>,
    has_whiteboard: bool,
}

pub fn build_snapshot(raw: Raw, cfg: &CoreConfig, now: Timestamp) -> Snapshot {
    let me = &raw.me;
    let phid = me.phid.as_deref();
    let to_review: HashSet<RevId> = raw.to_review.iter().copied().chain(raw.group_review.keys().copied()).collect();

    let mut by_bug: HashMap<BugId, Vec<RevIdx>> = HashMap::with_capacity(raw.bug_revisions.len());
    let rev_idx: HashMap<RevId, RevIdx> = raw.revisions.iter().enumerate().map(|(i, r)| (r.id, RevIdx::new(i))).collect();
    for (i, r) in raw.revisions.iter().enumerate() {
        if let Some(bug) = r.bug {
            by_bug.entry(bug).or_default().push(RevIdx::new(i));
        }
    }
    for (bug, ids) in &raw.bug_revisions {
        let list = by_bug.entry(*bug).or_default();
        for idx in ids.iter().filter_map(|id| rev_idx.get(id).copied()) {
            if !list.contains(&idx) {
                list.push(idx);
            }
        }
    }
    let revs_of_bug = |id: BugId| -> Vec<RevIdx> {
        by_bug.get(&id).map_or_else(Vec::new, |list| {
            list.iter()
                .copied()
                .filter(|r| !to_review.contains(&raw.revisions[r.get()].id))
                .collect()
        })
    };
    let issue_at = |key: &str| raw.issues.get_index_of(key).map(IssueIdx::new);
    let idea_at = |i: Option<IssueIdx>| i.and_then(|i| raw.issues.get_index(i.get())).and_then(|(_, x)| idea_of(&raw, x));

    let with_jira = raw.state("jira") == SourceState::Read;
    let active_sprint = raw.issues.values().find_map(|i| i.sprint.as_ref()).map(|sp| sp.name.as_str());
    let mut entries: Vec<Entry> = Vec::with_capacity(raw.bugs.len() + raw.revisions.len());
    let mut claimed: HashSet<&str> = HashSet::with_capacity(raw.issues.len());
    let mut statuses: Vec<RevStatus> = Vec::new();

    for (b, bug) in raw.bugs.iter().enumerate() {
        if bug.via.iter().all(|v| *v == Via::Team) {
            continue;
        }
        let key = jira_key_of(bug);
        let issue = key.and_then(issue_at);
        if let Some((_, x)) = issue.and_then(|i| raw.issues.get_index(i.get())) {
            claimed.insert(&x.key.0);
        }
        let mut e = Entry::new(ItemId::Bug(bug.id));
        e.bug = Some(BugIdx::new(b));
        e.issue = issue;
        e.idea = idea_at(issue);
        e.needinfo = bug
            .flags
            .iter()
            .position(|f| f.name == "needinfo" && f.status == "?" && f.requestee.as_deref() == Some(me.email.as_str()))
            .map(|p| u32::try_from(p).unwrap_or(u32::MAX));
        e.review = by_bug
            .get(&bug.id)
            .and_then(|list| list.iter().copied().find(|r| to_review.contains(&raw.revisions[r.get()].id)));
        e.revisions = revs_of_bug(bug.id);
        if with_jira && bug.assigned_to == me.email {
            statuses.clear();
            statuses.extend(e.revisions.iter().map(|r| raw.revisions[r.get()].status));
            e.drift = drift_of(&DriftInput {
                bug,
                key,
                issue: issue.and_then(|i| raw.issues.get_index(i.get())).map(|(_, x)| x),
                revs: &statuses,
                mirrored: cfg.whiteboard().is_some_and(|w| bug.whiteboard.contains(w)),
                me,
                statuses: &cfg.jira_statuses,
                active_sprint,
                syncs: &cfg.jbi_syncs,
            });
        }
        entries.push(e);
    }

    if let Some(jira_email) = me.jira_email.as_deref() {
        for (i, (_, issue)) in raw.issues.iter().enumerate() {
            let idx = IssueIdx::new(i);
            if issue.assignee.as_deref() == Some(jira_email) && claimed.insert(&issue.key.0) {
                let mut e = Entry::new(ItemId::Jira(idx));
                e.issue = Some(idx);
                e.idea = idea_at(Some(idx));
                entries.push(e);
            }
        }
    }

    for (i, r) in raw.revisions.iter().enumerate() {
        if r.bug.is_some() {
            continue;
        }
        let mut e = Entry::new(ItemId::Rev(r.id));
        if to_review.contains(&r.id) {
            e.review = Some(RevIdx::new(i));
        } else if phid == Some(r.author.as_str()) {
            e.revisions.push(RevIdx::new(i));
        } else {
            continue;
        }
        entries.push(e);
    }

    let syncs = crate::local::syncs(&raw);
    link_local(&raw, &syncs, &mut entries, now);

    let ctx = Ctx { raw: &raw, now };
    for e in &mut entries {
        e.placement = Some(place(&ctx, e));
    }
    entries.retain(|e| {
        let only = |via: Via| e.bug.is_some_and(|b| raw.bugs[b.get()].via.iter().all(|v| *v == via));
        let kept = e.placement.is_some_and(|p| p.reason.facets().kept_when_landed);
        let unsent = e.placement.is_some_and(|p| matches!(p.reason, Reason::NotUploaded(_)));
        (!only(Via::Landed) || kept || !e.drift.is_empty()) && (!only(Via::Local) || unsent || !e.drift.is_empty())
    });
    let today = now.to_zoned(jiff::tz::TimeZone::UTC).date();
    if with_jira {
        for e in &mut entries {
            if let Some(d) = parent_drift(&raw, e) {
                e.drift.push(d);
            }
        }
        let mut ideas: Vec<IdeaIdx> = entries.iter().filter_map(|e| e.idea).collect();
        let mut seen = HashSet::new();
        ideas.retain(|i| seen.insert(*i));
        for i in ideas {
            let work: Vec<Work> = entries
                .iter()
                .filter(|e| e.idea == Some(i))
                .filter_map(|e| work_of_entry(&raw, e))
                .collect();
            let mut e = Entry::new(ItemId::Idea(i));
            e.idea = Some(i);
            if let Some((_, idea)) = raw.roadmap.ideas.get_index(i.get()) {
                e.drift = crate::ideas::drift_of(idea, &work, Scope::Mine, today);
            }
            e.placement = Some(place(&ctx, &e));
            entries.push(e);
        }
    }
    let mine = entries.len();

    if with_jira && cfg.whiteboard().is_some() {
        let mut team_work: Vec<(IdeaIdx, Work)> = Vec::new();
        for (b, bug) in raw.bugs.iter().enumerate() {
            if !bug.via.contains(&Via::Team) {
                continue;
            }
            let key = jira_key_of(bug);
            let issue = key.and_then(issue_at);
            if let Some(i) = idea_at(issue) {
                team_work.push((
                    i,
                    Work {
                        label: format!("bug {}", bug.id),
                        status: State(bug).to_string(),
                        open: !is_closed(bug),
                    },
                ));
            }
            let revisions = if is_dead(bug) { Vec::new() } else { revs_of_bug(bug.id) };
            statuses.clear();
            statuses.extend(revisions.iter().map(|r| raw.revisions[r.get()].status));
            let drift = drift_of(&DriftInput {
                bug,
                key,
                issue: issue.and_then(|i| raw.issues.get_index(i.get())).map(|(_, x)| x),
                revs: &statuses,
                mirrored: true,
                me,
                statuses: &cfg.jira_statuses,
                active_sprint: None,
                syncs: &cfg.jbi_syncs,
            });
            if drift.is_empty() {
                continue;
            }
            let mut e = Entry::new(ItemId::Bug(bug.id));
            e.scope = Scope::Team;
            e.bug = Some(BugIdx::new(b));
            e.issue = issue;
            e.revisions = revisions;
            e.drift = drift;
            entries.push(e);
        }
        let mut ideas: Vec<IdeaIdx> = team_work.iter().map(|(i, _)| *i).collect();
        let mut seen = HashSet::new();
        ideas.retain(|i| seen.insert(*i));
        for i in ideas {
            let work: Vec<Work> = team_work.iter().filter(|(x, _)| *x == i).map(|(_, w)| w.clone()).collect();
            let Some((_, idea)) = raw.roadmap.ideas.get_index(i.get()) else {
                continue;
            };
            let drift = crate::ideas::drift_of(idea, &work, Scope::Team, today);
            if drift.is_empty() {
                continue;
            }
            let mut e = Entry::new(ItemId::Idea(i));
            e.scope = Scope::Team;
            e.idea = Some(i);
            e.drift = drift;
            entries.push(e);
        }
    }

    let sprint = sprint_of(&raw, cfg);
    let stacks = crate::stack::find(&raw, &entries[..mine]);
    crate::stack::assign(&stacks, &mut entries);
    let mut sections = sections_of(&raw, &entries, mine, sprint.as_ref(), &syncs);
    for list in &mut sections {
        crate::stack::group(list, &entries, &stacks);
    }
    Snapshot {
        raw,
        now,
        entries,
        sections,
        stacks,
        sprint,
        syncs,
        has_whiteboard: cfg.whiteboard().is_some(),
    }
}

fn link_local(raw: &Raw, syncs: &[crate::local::Sync], entries: &mut Vec<Entry>, now: Timestamp) {
    use crate::local::Sync;
    let Some(local) = raw.local.as_ref() else { return };
    let mut branches: Vec<usize> = Vec::new();
    for (i, c) in local.commits.iter().enumerate() {
        let (idx, sync) = (LocalIdx::new(i), syncs[i]);
        if !sync.is_live() {
            continue;
        }
        let by_rev = crate::local::revision_of(local, c).and_then(|r| {
            entries
                .iter()
                .position(|e| e.scope == Scope::Mine && e.revisions.iter().chain(&e.review).any(|x| raw.revisions[x.get()].id == r))
        });
        let by_bug = || {
            c.bug
                .and_then(|b| entries.iter().position(|e| e.scope == Scope::Mine && e.id == ItemId::Bug(b)))
        };
        match by_rev.or_else(by_bug) {
            Some(at) => entries[at].local.push(idx),
            None => match branches
                .iter()
                .find(|n| matches!(entries[**n].id, ItemId::Branch(f) if local.commits[f.get()].place == c.place))
            {
                Some(at) => entries[*at].local.push(idx),
                None => {
                    let mut e = Entry::new(ItemId::Branch(idx));
                    e.local.push(idx);
                    branches.push(entries.len());
                    entries.push(e);
                }
            },
        }
    }
    for e in entries.iter_mut().filter(|e| !e.local.is_empty()) {
        e.local.sort_by_key(|i| std::cmp::Reverse(local.commits[i.get()].committed));
        if let ItemId::Branch(_) = e.id {
            e.id = ItemId::Branch(e.local[0]);
        }
        for i in &e.local {
            e.drift.extend(crate::local::drift_of(raw, *i, syncs[i.get()], now));
        }
        let unsent: Vec<LocalIdx> = e.local.iter().copied().filter(|i| syncs[i.get()] == Sync::NotUploaded).collect();
        let has_revision = !e.revisions.is_empty() || e.review.is_some();
        if has_revision {
            e.drift.extend(crate::local::unsent_drift(raw, &unsent, now));
        }
    }
}

fn parent_drift(raw: &Raw, e: &Entry) -> Option<Drift> {
    let (_, issue) = raw.issues.get_index(e.issue?.get())?;
    let parent = crate::ideas::parent_of(raw, issue).filter(|p| p.status_category == StatusCategory::Done)?;
    let work = work_of_entry(raw, e).filter(|w| w.open)?;
    let kind = parent.kind.as_deref().unwrap_or("parent").to_lowercase();
    Some(Drift {
        level: Level::Warn,
        message: format!("{}'s {kind} {} is {}", issue.key, parent.key, parent.status),
        cause: format!("{} is still {}", work.label, work.status),
        fix: None,
    })
}

fn work_of_entry(raw: &Raw, e: &Entry) -> Option<Work> {
    if let Some(bug) = e.bug.map(|b| &raw.bugs[b.get()]) {
        return Some(Work {
            label: format!("bug {}", bug.id),
            status: State(bug).to_string(),
            open: !is_closed(bug),
        });
    }
    let (_, issue) = raw.issues.get_index(e.issue?.get())?;
    Some(Work {
        label: issue.key.0.clone(),
        status: issue.status.clone(),
        open: issue.status_category != StatusCategory::Done,
    })
}

fn sprint_of(raw: &Raw, cfg: &CoreConfig) -> Option<SprintSummary> {
    let jira_email = raw.me.jira_email.as_deref()?;
    let mut tickets = Vec::new();
    let (mut committed, mut done) = (0.0, 0.0);
    let mut unestimated = Vec::new();
    let mut counted: HashSet<&str> = HashSet::new();
    for (i, (_, issue)) in raw.issues.iter().enumerate() {
        if issue.sprint.is_none() || issue.assignee.as_deref() != Some(jira_email) || !counted.insert(&issue.key.0) {
            continue;
        }
        let idx = IssueIdx::new(i);
        tickets.push(idx);
        let points = issue.points.unwrap_or(0.0);
        committed += points;
        if issue.status_category == StatusCategory::Done {
            done += points;
        }
        if issue.points.is_none() {
            unestimated.push(idx);
        }
    }
    Some(SprintSummary {
        first: *tickets.first()?,
        budget: cfg.sprint_budget,
        committed,
        done,
        unestimated,
        tickets,
    })
}

fn sections_of(
    raw: &Raw,
    entries: &[Entry],
    mine: usize,
    sprint: Option<&SprintSummary>,
    syncs: &[crate::local::Sync],
) -> [Vec<EntryIdx>; Section::COUNT] {
    let mut sections: [Vec<EntryIdx>; Section::COUNT] = Default::default();
    let since = |i: &EntryIdx| entries[i.get()].placement.and_then(|p| p.since);
    let issue_key = |i: IssueIdx| raw.issues.get_index(i.get()).map(|(_, x)| x.key.0.as_str());
    let sprint_keys: HashSet<&str> = sprint
        .map(|s| s.tickets.iter().filter_map(|i| issue_key(*i)).collect())
        .unwrap_or_default();
    for (i, e) in entries[..mine].iter().enumerate() {
        let idx = EntryIdx::new(i);
        if let Some(p) = e.placement.filter(|p| p.section != Section::Sprint) {
            sections[p.section.index()].push(idx);
        }
        if e.issue.and_then(issue_key).is_some_and(|k| sprint_keys.contains(k)) {
            sections[Section::Sprint.index()].push(idx);
        }
        if !e.drift.is_empty() {
            sections[Section::Drift.index()].push(idx);
        }
        let local_placed = e.placement.is_some_and(|p| p.section == Section::Local);
        if !local_placed && e.local.iter().any(|l| syncs.get(l.get()) == Some(&crate::local::Sync::NotUploaded)) {
            sections[Section::Local.index()].push(idx);
        }
    }
    for s in [Section::Waiting, Section::Land, Section::Returned, Section::Others, Section::Other] {
        let facet = |i: &EntryIdx| entries[i.get()].placement.map(|p| p.reason.facets());
        let first = |i: &EntryIdx| facet(i).is_some_and(|f| f.sorts_first);
        let last = |i: &EntryIdx| facet(i).is_some_and(|f| f.sorts_last);
        sections[s.index()].sort_by_key(|i| (!first(i), entries[i.get()].placement.map(|p| p.tier), last(i), since(i)));
    }
    sections[Section::Sprint.index()].sort_by_key(since);
    sections[Section::Landed.index()].sort_by_key(|i| std::cmp::Reverse(since(i)));
    let newest_local = |i: &EntryIdx| {
        let commits = raw.local.as_ref().map(|l| &l.commits);
        entries[i.get()]
            .local
            .iter()
            .filter_map(|l| commits?.get(l.get()))
            .map(|c| c.committed)
            .max()
    };
    sections[Section::Local.index()].sort_by_key(|i| std::cmp::Reverse(newest_local(i)));
    let priority = |i: &EntryIdx| {
        let e = &entries[i.get()];
        let text = match (e.bug, e.issue) {
            (Some(b), _) => Some(raw.bugs[b.get()].priority.as_str()),
            (None, Some(x)) => raw.issues.get_index(x.get()).and_then(|(_, x)| x.priority.as_deref()),
            (None, None) => None,
        };
        crate::rules::priority_rank(text)
    };
    sections[Section::Backlog.index()].sort_by_key(|i| (priority(i), since(i).is_none(), since(i)));
    sections[Section::Ideas.index()].sort_by_key(|i| entries[i.get()].drift.is_empty());
    sections[Section::Drift.index()].sort_by_key(|i| (!entries[i.get()].has_error_drift(), since(i)));
    let mut team: Vec<EntryIdx> = (mine..entries.len()).map(EntryIdx::new).collect();
    team.sort_by_key(|i| {
        let e = &entries[i.get()];
        (!e.has_error_drift(), e.bug.map(|b| raw.bugs[b.get()].id))
    });
    sections[Section::Team.index()] = team;
    sections
}

impl Snapshot {
    pub fn not_checked(&self, section: Section) -> Option<&'static str> {
        match section {
            Section::Land | Section::Returned | Section::Others => match self.raw.state("phabricator") {
                SourceState::Off => Some("Off: Phabricator isn't set up. `worklens auth set phabricator` turns it on."),
                SourceState::Failed => Some("Not checked: Phabricator couldn't be read."),
                SourceState::Read => None,
            },
            Section::Local => match &self.raw.local {
                None => Some("Off: point the agent directory in `worklens setup` at your Firefox git checkout to turn this on."),
                Some(l) if l.error.is_some() => Some("Not checked: git couldn't read your checkout; `worklens doctor` says why."),
                Some(_) => None,
            },
            Section::Drift if self.raw.local.is_some() && !self.sections[Section::Drift.index()].is_empty() => None,
            Section::Sprint | Section::Ideas | Section::Drift | Section::Team => match self.raw.state("jira") {
                SourceState::Off => Some("Off: Jira isn't set up. `worklens auth set jira` turns it on."),
                SourceState::Failed => Some("Not checked: Jira couldn't be read."),
                SourceState::Read if section == Section::Team && !self.has_whiteboard => {
                    Some("Off: set your team's whiteboard tag in `worklens setup` to turn this on.")
                }
                SourceState::Read => None,
            },
            _ => None,
        }
    }

    pub fn off(&self, section: Section) -> bool {
        let source = match section {
            Section::Local => return self.raw.local.is_none(),
            Section::Drift if self.raw.local.is_some() && !self.sections[Section::Drift.index()].is_empty() => return false,
            Section::Land | Section::Returned | Section::Others => "phabricator",
            Section::Sprint | Section::Ideas | Section::Drift | Section::Team => "jira",
            _ => return false,
        };
        self.raw.state(source) == SourceState::Off
    }

    pub fn empty_text(&self, section: Section) -> &'static str {
        self.not_checked(section).unwrap_or(section.empty())
    }

    pub fn me(&self) -> &str {
        &self.raw.me.email
    }

    pub fn stack(&self, k: crate::ids::StackIdx) -> &crate::stack::Stack {
        &self.stacks[k.get()]
    }

    pub fn entry(&self, i: EntryIdx) -> &Entry {
        &self.entries[i.get()]
    }

    pub fn bug(&self, i: BugIdx) -> &Bug {
        &self.raw.bugs[i.get()]
    }

    pub fn rev(&self, i: RevIdx) -> &Revision {
        &self.raw.revisions[i.get()]
    }

    pub fn issue(&self, i: IssueIdx) -> &Issue {
        self.raw
            .issues
            .get_index(i.get())
            .map(|(_, issue)| issue)
            .expect("IssueIdx from this snapshot")
    }

    pub fn idea(&self, i: IdeaIdx) -> &Idea {
        self.raw
            .roadmap
            .ideas
            .get_index(i.get())
            .map(|(_, idea)| idea)
            .expect("IdeaIdx from this snapshot")
    }

    pub fn work_of(&self, i: IdeaIdx) -> impl Iterator<Item = EntryIdx> + '_ {
        self.entries
            .iter()
            .enumerate()
            .filter(move |(_, e)| e.idea == Some(i) && e.scope == Scope::Mine && !matches!(e.id, ItemId::Idea(_)))
            .map(|(n, _)| EntryIdx::new(n))
    }

    pub fn work(&self, e: &Entry) -> Option<Work> {
        work_of_entry(&self.raw, e)
    }

    pub fn parent(&self, e: &Entry) -> Option<&Issue> {
        crate::ideas::parent_of(&self.raw, self.issue(e.issue?))
    }

    pub fn section(&self, s: Section, hidden: bool) -> impl Iterator<Item = EntryIdx> + '_ {
        self.sections[s.index()]
            .iter()
            .copied()
            .filter(move |i| hidden || self.entries[i.get()].marks.hidden.is_none())
    }

    pub fn id(&self, e: &Entry) -> EntryId<'_> {
        EntryId { snap: self, id: e.id }
    }

    pub fn title<'a>(&'a self, e: &'a Entry) -> &'a str {
        match e.id {
            ItemId::Bug(_) => e.bug.map_or("", |b| self.bug(b).summary.as_str()),
            ItemId::Jira(i) => &self.issue(i).summary,
            ItemId::Idea(i) => &self.idea(i).summary,
            ItemId::Rev(_) => e
                .review
                .or_else(|| e.revisions.first().copied())
                .map_or("", |r| self.rev(r).title.as_str()),
            ItemId::Branch(i) => crate::local::commit(&self.raw, i).map_or("", |c| c.subject.as_str()),
        }
    }

    pub fn needinfo(&self, e: &Entry) -> Option<&Flag> {
        self.bug(e.bug?).flags.get(e.needinfo? as usize)
    }

    pub fn needinfo_comment(&self, e: &Entry) -> Option<&Comment> {
        let flag = self.needinfo(e)?;
        let thread = self.raw.threads.get(&self.bug(e.bug?).id)?;
        thread
            .comments
            .iter()
            .find(|c| c.author == flag.setter && (c.time.as_second() - flag.creation_date.as_second()).abs() <= 120)
    }

    pub fn last_comment(&self, e: &Entry) -> Option<&Comment> {
        self.raw.last_comments.get(&self.bug(e.bug?).id)
    }

    pub fn is_mine(&self, rev: &Revision) -> bool {
        self.raw.me.phid.as_deref() == Some(rev.author.as_str())
    }

    pub fn name<'a>(&'a self, phid: &'a str) -> &'a str {
        self.raw.users.get(phid).map_or(phid, String::as_str)
    }

    pub fn try_state(&self, r: RevIdx) -> TryState<'_> {
        let rev = self.rev(r);
        let Some(pushes) = self.raw.try_pushes.as_ref() else {
            return TryState::Unknown;
        };
        if !self.is_mine(rev) || self.raw.errors.iter().any(|e| e.source == "treeherder") {
            return TryState::Unknown;
        }
        pushes.get(&rev.id).map_or(TryState::NoPush, |push| TryState::Push {
            push,
            outdated: rev.diff_created.is_some_and(|d| push.pushed_at < d),
        })
    }

    pub fn reason<'a>(&'a self, e: &'a Entry) -> ReasonText<'a> {
        ReasonText { snap: self, entry: e }
    }

    pub fn tracked_for(&self, e: &Entry) -> Vec<&str> {
        let Some(b) = e.bug.map(|b| self.bug(b)) else { return Vec::new() };
        b.release_flags
            .iter()
            .filter(|(_, v)| crate::rules::is_tracked(v))
            .filter_map(|(k, _)| k.strip_prefix("cf_tracking_firefox"))
            .map(|v| v.trim_start_matches('_'))
            .collect()
    }

    pub fn merge(&self) -> Option<(u32, i64)> {
        let t = self.raw.trains.as_ref()?;
        let today = self.now.to_zoned(jiff::tz::TimeZone::UTC).date();
        let days = today.until(t.next_merge).ok()?.get_days();
        Some((t.nightly, i64::from(days)))
    }

    pub fn lands_in(&self, e: &Entry) -> Option<crate::train::LandsIn> {
        let accepted = e.placement.is_some_and(|p| matches!(p.reason, Reason::Accepted(_)));
        let t = self
            .raw
            .trains
            .as_ref()
            .filter(|_| accepted && crate::train::trusted(&self.raw, self.now))?;
        crate::train::lands_in(t, self.now.to_zoned(jiff::tz::TimeZone::UTC).date())
    }

    pub fn revision_of(&self, e: &Entry) -> Option<RevIdx> {
        e.review
            .or_else(|| crate::present::live_revisions(self, e).next())
            .or_else(|| e.revisions.first().copied())
    }

    pub fn next_step(&self, e: &Entry) -> Option<NextStep> {
        let p = e.placement?;
        if let Reason::InReview(r) = p.reason
            && !crate::present::away(self, self.rev(r)).is_empty()
        {
            return Some(NextStep::AddReviewer(self.rev(r).id));
        }
        if let Some(step) = p.reason.next_step(&self.raw, p.stale, !e.drift.is_empty()) {
            return Some(step);
        }
        if self
            .last_comment(e)
            .is_some_and(|c| c.author != self.me() && days_between(Some(c.time), self.now) < NEW_COMMENT_DAYS)
        {
            return Some(NextStep::ReadComment);
        }
        let waiting = e
            .revisions
            .iter()
            .map(|r| self.rev(*r))
            .filter(|r| self.is_mine(r) && r.status == RevStatus::NeedsReview);
        for r in waiting {
            match crate::bugs::business_days_between(r.modified, self.now) {
                n if n >= FIND_ANOTHER_REVIEWER => return Some(NextStep::FindAnotherReviewer(r.id)),
                n if n >= PING_REVIEWER => return Some(NextStep::PingReviewer(r.id)),
                _ => {}
            }
        }
        let issue = e.issue.map(|i| self.issue(i));
        if let Some(issue) = issue.filter(|i| i.sprint.is_some() && i.status_category != StatusCategory::Done) {
            let updated = e.bug.map_or(issue.updated, |b| self.bug(b).last_change_time);
            if days_between(Some(updated), self.now) > STALE_DAYS {
                return Some(NextStep::SprintUpdate);
            }
        }
        e.has_error_drift().then_some(NextStep::FixDrift)
    }

    pub fn tag<'a>(&'a self, e: &'a Entry) -> Tag<'a> {
        Tag { snap: self, entry: e }
    }

    pub fn find(&self, reference: &str) -> Option<EntryIdx> {
        let mut r = reference_in(reference);
        if r.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("bug")) {
            r = r[3..].trim_start();
        }
        let mut id = String::new();
        let by_id = self.entries.iter().position(|e| {
            id.clear();
            let _ = write!(id, "{}", self.id(e));
            id.eq_ignore_ascii_case(r)
        });
        let names = |rev: RevIdx| {
            let text = self.rev(rev).id.to_string();
            text.eq_ignore_ascii_case(r)
        };
        by_id
            .or_else(|| {
                self.entries.iter().position(|e| {
                    e.issue.is_some_and(|i| self.issue(i).key.0.eq_ignore_ascii_case(r))
                        || e.review.is_some_and(names)
                        || e.revisions.iter().copied().any(names)
                })
            })
            .map(EntryIdx::new)
    }

    pub fn unread_source(&self, reference: &str) -> Option<&'static str> {
        let r = reference_in(reference).as_bytes();
        let digits = |b: &[u8]| !b.is_empty() && b.iter().all(u8::is_ascii_digit);
        let revision = r.first().is_some_and(|b| b.eq_ignore_ascii_case(&b'D')) && digits(&r[1..]);
        let ticket = r
            .iter()
            .position(|&b| b == b'-')
            .is_some_and(|dash| dash > 0 && r[..dash].iter().all(u8::is_ascii_alphabetic) && digits(&r[dash + 1..]));
        match (revision, ticket) {
            (true, _) => match self.raw.state("phabricator") {
                SourceState::Off => Some("Phabricator isn't set up. `worklens auth set phabricator` turns it on."),
                SourceState::Failed => Some("Phabricator couldn't be read; `worklens doctor` says why."),
                SourceState::Read => None,
            },
            (_, true) => match self.raw.state("jira") {
                SourceState::Off => Some("Jira isn't set up. `worklens auth set jira` turns it on."),
                SourceState::Failed => Some("Jira couldn't be read; `worklens doctor` says why."),
                SourceState::Read => None,
            },
            _ => None,
        }
    }
}

fn reference_in(text: &str) -> &str {
    let text = text.trim().trim_start_matches('<').trim_end_matches('>');
    if !text.contains(['/', '?']) {
        return text;
    }
    let (path, query) = text.split_once('?').unwrap_or((text, ""));
    let query = query.split('#').next().unwrap_or_default();
    let param = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("id=").or_else(|| pair.strip_prefix("selectedIssue=")));
    param.unwrap_or_else(|| {
        let path = path.split('#').next().unwrap_or_default();
        path.rsplit('/').find(|s| !s.is_empty()).unwrap_or_default()
    })
}

pub struct EntryId<'a> {
    snap: &'a Snapshot,
    id: ItemId,
}

impl fmt::Display for EntryId<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.id {
            ItemId::Bug(b) => write!(f, "{b}"),
            ItemId::Jira(i) => f.write_str(&self.snap.issue(i).key.0),
            ItemId::Rev(r) => write!(f, "{r}"),
            ItemId::Idea(i) => f.write_str(&self.snap.idea(i).key.0),
            ItemId::Branch(i) => f.write_str(crate::local::commit(&self.snap.raw, i).map_or("", |c| c.place.as_str())),
        }
    }
}

pub struct ReasonText<'a> {
    snap: &'a Snapshot,
    entry: &'a Entry,
}

impl fmt::Display for ReasonText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (s, e) = (self.snap, self.entry);
        let Some(p) = e.placement else { return Ok(()) };
        let days = HumanDays(days_between(p.since, s.now));
        let ago = if days.0 < 1.0 { "today".to_owned() } else { format!("{days} ago") };
        let review_event = |r| {
            if s.rev(r).diff_created.is_some() {
                "latest diff posted"
            } else {
                "revision last modified"
            }
        };
        match p.reason {
            Reason::BackedOut(b) => {
                let Some(found) = crate::backout::of(&s.raw, b) else {
                    return f.write_str("backed out");
                };
                let gap = crate::backout::Gap(found.backout.pushed_at.as_second() - found.landing.pushed_at.as_second());
                let cause = found.backout.cause.as_deref().unwrap_or("cause not stated");
                write!(
                    f,
                    "{} backed out of {} {gap} after landing: {cause}",
                    found.landing.revision, found.repository
                )
            }
            Reason::Needinfo => {
                let from = s.needinfo(e).map_or("someone", |flag| flag.setter.as_str());
                write!(f, "needinfo from {from}, {days}")
            }
            Reason::ReviewRequested(r) => {
                let rev = s.rev(r);
                write!(f, "review requested on {}", rev.id)?;
                if let Some(author) = s.raw.users.get(&rev.author) {
                    write!(f, " by {author}")?;
                }
                write!(f, "; {} {ago}", review_event(r))
            }
            Reason::Accepted(r) => {
                write!(f, "{} accepted by ", s.rev(r).id)?;
                write_reviewers(f, s, r, |x| x == ReviewerStatus::Accepted, "a reviewer")?;
                write!(f, "; revision last modified {ago}")?;
                match s.try_state(r) {
                    TryState::Unknown => Ok(()),
                    state => write!(f, "; {}", TryText(state)),
                }
            }
            Reason::LandingBlocked(r) => match crate::guidance::landing_issue(&s.raw, r) {
                Some(issue) => write!(f, "{issue}"),
                None => write!(f, "{} landing needs attention", s.rev(r).id),
            },
            Reason::ChangesRequested(r) => {
                write!(f, "changes requested on {} by ", s.rev(r).id)?;
                write_reviewers(f, s, r, |x| x == ReviewerStatus::Rejected, "a reviewer")?;
                write!(f, "; revision last modified {ago}")
            }
            Reason::InReview(r) if !crate::present::away(s, s.rev(r)).is_empty() => {
                write!(f, "{} waiting on ", s.rev(r).id)?;
                for (n, (who, until)) in crate::present::away(s, s.rev(r)).into_iter().enumerate() {
                    if n > 0 {
                        f.write_str("; ")?;
                    }
                    let date = until.to_zoned(jiff::tz::TimeZone::UTC).date();
                    write!(f, "{}, away until {}", s.name(who), crate::train::day(date))?;
                }
                Ok(())
            }
            Reason::InReview(r) => {
                write!(f, "{} waiting on ", s.rev(r).id)?;
                write_reviewers(
                    f,
                    s,
                    r,
                    |x| matches!(x, ReviewerStatus::Added | ReviewerStatus::Blocking),
                    "reviewers",
                )?;
                write!(f, "; {} {ago}", review_event(r))
            }
            Reason::Sprint(i) => {
                let issue = s.issue(i);
                let sprint = issue.sprint.as_ref().map_or("", |x| x.name.as_str());
                write!(f, "{} is {} in {sprint}", issue.key, issue.status)
            }
            Reason::UpliftApproved(c) => write!(f, "uplift to {c} approved, not landed there yet, {days}"),
            Reason::RegressionOf(b) => write!(f, "regression from your fix in bug {b}, {days}"),
            Reason::GroupReview(r) => {
                let rev = s.rev(r);
                let group = s.raw.group_review.get(&rev.id).map_or("a group", |g| s.name(g));
                write!(f, "review requested of {group} on {}", rev.id)?;
                if let Some(author) = s.raw.users.get(&rev.author) {
                    write!(f, " by {author}")?;
                }
                write!(f, "; {} {ago}", review_event(r))
            }
            Reason::StillCrashing => {
                let c = e.bug.and_then(|b| s.raw.crashes.get(&s.bug(b).id));
                let (n, since) = c.map_or((0, String::new()), |c| (c.count, c.since.strftime("%b %d").to_string()));
                write!(f, "still crashing after your fix: {n} Nightly crashes in builds since {since}")
            }
            Reason::Asked(at) => {
                let who = e
                    .bug
                    .and_then(|b| s.bug(b).flags.get(at as usize))
                    .and_then(|fl| fl.requestee.as_deref())
                    .unwrap_or("someone");
                if p.stale {
                    write!(f, "you asked {who}, no answer for {days}")
                } else {
                    write!(f, "you asked {who}, {days}")
                }
            }
            Reason::UpliftRequested(c) => write!(f, "uplift to {c} requested, waiting on release management, {days}"),
            Reason::Idea(i) => {
                let idea = s.idea(i);
                f.write_str(&idea.status)?;
                if let Some(w) = &idea.weekly_status {
                    write!(f, ", {w}")?;
                }
                if let Some(t) = &idea.target {
                    write!(f, ", target {t}")?;
                }
                match s.work_of(i).count() {
                    1 => f.write_str("; 1 of your items rolls up to it"),
                    n => write!(f, "; {n} of your items roll up to it"),
                }
            }
            Reason::Landed(_) => {
                let Some(bug) = e.bug.map(|b| s.bug(b)) else { return Ok(()) };
                match crate::train::fixed_in(bug) {
                    Some(v) => write!(f, "Bugzilla marks Firefox {v} fixed; build inclusion unverified"),
                    None if crate::train::on_autoland(&s.raw, bug).is_some() => {
                        f.write_str("Landing observed on autoland; Nightly inclusion unverified")
                    }
                    None => f.write_str("Resolved FIXED; no Firefox status flag set; build inclusion unverified"),
                }
            }
            Reason::Backlog(_) => match p.since {
                Some(_) => write!(f, "Filed {} ago, not started", HumanDays(days_between(p.since, s.now))),
                None => f.write_str("Assigned to you, not started"),
            },
            Reason::NotUploaded(newest) => {
                let unsent: Vec<&crate::raw::LocalCommit> = e
                    .local
                    .iter()
                    .filter(|i| s.syncs.get(i.get()) == Some(&crate::local::Sync::NotUploaded))
                    .filter_map(|i| crate::local::commit(&s.raw, *i))
                    .collect();
                let place = crate::local::commit(&s.raw, newest).map_or("", |c| c.place.as_str());
                match unsent.len() {
                    1 => write!(f, "1 commit on {place} not on Phabricator, made {ago}")?,
                    n => write!(f, "{n} commits on {place} not on Phabricator, newest {ago}")?,
                }
                if unsent.iter().any(|c| crate::local::is_wip(&c.subject)) {
                    f.write_str(", marked WIP")?;
                }
                Ok(())
            }
            Reason::Other => match (e.bug, e.issue) {
                (Some(b), _) => write!(f, "bug is {}", State(s.bug(b))),
                (None, Some(i)) => write!(f, "{} is {}", s.issue(i).key, s.issue(i).status),
                (None, None) => f.write_str("open revision"),
            },
        }
    }
}

fn write_reviewers(f: &mut fmt::Formatter<'_>, s: &Snapshot, r: RevIdx, wanted: fn(ReviewerStatus) -> bool, fallback: &str) -> fmt::Result {
    let matching = || crate::present::reviewers(s.rev(r)).filter(|x| wanted(x.status));
    let count = matching().count();
    if count == 0 {
        return f.write_str(fallback);
    }
    if count > 2 {
        let blocking = matching().filter(|x| x.status == ReviewerStatus::Blocking).count();
        write!(f, "{count} reviewers")?;
        return if blocking > 0 { write!(f, ", {blocking} blocking") } else { Ok(()) };
    }
    for (n, x) in matching().enumerate() {
        if n > 0 {
            f.write_str(", ")?;
        }
        f.write_str(s.name(&x.phid))?;
        if x.status == ReviewerStatus::Blocking {
            f.write_str(" (blocking)")?;
        }
    }
    Ok(())
}

pub struct TryText<'a>(pub TryState<'a>);

impl fmt::Display for TryText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let TryState::Push { push, outdated } = self.0 else {
            return f.write_str("no try push");
        };
        let jobs = |n: u32| if n == 1 { "job" } else { "jobs" };
        if push.failed > 0 {
            f.write_str("try failed: ")?;
            let failing = [("builds", push.builds), ("lint", push.lint), ("tests", push.tests)];
            for (n, (what, _)) in failing.iter().filter(|(_, r)| *r == TryResult::Fail).enumerate() {
                if n > 0 {
                    f.write_str(", ")?;
                }
                f.write_str(what)?;
            }
            write!(f, " ({} {})", push.failed, jobs(push.failed))?;
        } else if push.running > 0 {
            write!(f, "try running ({} {} left)", push.running, jobs(push.running))?;
        } else {
            f.write_str("try passed")?;
        }
        if outdated {
            f.write_str(" (before the latest diff)")
        } else {
            Ok(())
        }
    }
}

pub struct Tag<'a> {
    snap: &'a Snapshot,
    entry: &'a Entry,
}

impl fmt::Display for Tag<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(p) = self.entry.placement else { return Ok(()) };
        match p.reason {
            Reason::Sprint(i) => match self.snap.issue(i).points {
                Some(points) => write!(f, "{points}p"),
                None => f.write_str("?p"),
            },
            reason => f.write_str(reason.tag()),
        }
    }
}
