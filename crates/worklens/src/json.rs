use std::collections::BTreeMap;

use jiff::Timestamp;
use serde::Serialize;
use serde_json::{Value, json};
use worklens_core::bugs::{State, assignee, bug_url, jira_url, rev_url};
use worklens_core::drift::Drift;
use worklens_core::marks::Hidden;
use worklens_core::raw::{StatusCategory, TryPush};
use worklens_core::snapshot::{ItemId, Scope, TryState, TryText};
use worklens_core::{Entry, IssueIdx, RevIdx, Snapshot, present};

pub const SCHEMA: &str = "worklens/2";

const EXCERPT_CHARS: usize = 600;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item<'a> {
    id: String,
    title: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    section: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    since: Option<Timestamp>,
    stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tier: Option<&'static str>,
    tag: String,
    action: Option<String>,
    action_url: Option<String>,
    bug: Option<Bug<'a>>,
    jira: Option<Jira<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<Parent<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idea: Option<IdeaJson<'a>>,
    revisions: Vec<Rev<'a>>,
    review: Option<Rev<'a>>,
    needinfo: Option<Needinfo<'a>>,
    last_comment: Option<LastComment<'a>>,
    drift: Vec<DriftJson<'a>>,
    tracked: Vec<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<&'a str>,
    hidden: Option<&'static str>,
    snoozed_until: Option<Timestamp>,
    is_new: bool,
    changes: &'a [String],
    workflow: worklens_core::evidence::Workflow,
    dependencies: Vec<worklens_core::dependencies::Dependency>,
    timeline: Vec<worklens_core::timeline::Event>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ships: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Bug<'a> {
    id: u32,
    url: String,
    state: String,
    status: &'a str,
    resolution: Option<&'a str>,
    severity: &'a str,
    priority: &'a str,
    keywords: &'a [String],
    product: &'a str,
    component: &'a str,
    assignee: Option<&'a str>,
    last_change: Timestamp,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Jira<'a> {
    key: &'a str,
    url: String,
    summary: &'a str,
    status: &'a str,
    status_category: StatusCategory,
    assignee: Option<&'a str>,
    points: Option<f64>,
    sprint: Option<SprintRef<'a>>,
    updated: Timestamp,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Parent<'a> {
    key: &'a str,
    url: String,
    #[serde(rename = "type")]
    kind: Option<&'a str>,
    summary: &'a str,
    status: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IdeaJson<'a> {
    key: &'a str,
    url: String,
    summary: &'a str,
    status: &'a str,
    status_category: StatusCategory,
    owner: Option<&'a str>,
    weekly_status: Option<&'a str>,
    target: Option<&'a str>,
    quarter: Option<&'a str>,
    theme: Option<&'a str>,
    priority: Option<&'a str>,
    team: Option<&'a str>,
    updated: Timestamp,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    work: Vec<String>,
}

fn idea<'a>(s: &'a Snapshot, e: &Entry) -> Option<IdeaJson<'a>> {
    let i = e.idea?;
    let x = s.idea(i);
    let work = if matches!(e.id, ItemId::Idea(_)) {
        s.work_of(i).map(|w| s.id(s.entry(w)).to_string()).collect()
    } else {
        Vec::new()
    };
    Some(IdeaJson {
        key: &x.key.0,
        url: jira_url(&x.key.0),
        summary: &x.summary,
        status: &x.status,
        status_category: x.status_category,
        owner: x.assignee.as_deref(),
        weekly_status: x.weekly_status.as_deref(),
        target: x.target.as_deref(),
        quarter: x.quarter.as_deref(),
        theme: x.theme.as_deref(),
        priority: x.priority.as_deref(),
        team: x.team.as_deref(),
        updated: x.updated,
        work,
    })
}

fn parent<'a>(s: &'a Snapshot, e: &Entry) -> Option<Parent<'a>> {
    s.parent(e).map(|p| Parent {
        key: &p.key.0,
        url: jira_url(&p.key.0),
        kind: p.kind.as_deref(),
        summary: &p.summary,
        status: &p.status,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SprintRef<'a> {
    name: &'a str,
    end_date: Option<Timestamp>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Rev<'a> {
    id: u32,
    title: &'a str,
    status: &'static str,
    mine: bool,
    modified: Timestamp,
    url: String,
    author: Option<&'a str>,
    reviewers: Vec<Reviewer<'a>>,
    #[serde(rename = "try", skip_serializing_if = "Option::is_none")]
    try_push: Option<Option<TryJson<'a>>>,
}

#[derive(Serialize)]
struct Reviewer<'a> {
    name: &'a str,
    status: &'static str,
}

#[derive(Serialize)]
struct TryJson<'a> {
    #[serde(flatten)]
    push: &'a TryPush,
    outdated: bool,
    summary: String,
}

#[derive(Serialize)]
struct Needinfo<'a> {
    from: &'a str,
    since: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    comment: Option<&'a str>,
}

#[derive(Serialize)]
struct LastComment<'a> {
    author: &'a str,
    time: Timestamp,
    excerpt: &'a str,
}

#[derive(Serialize)]
struct DriftJson<'a> {
    level: &'static str,
    message: &'a str,
    cause: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    fix: Option<Fix<'a>>,
}

#[derive(Serialize)]
struct Fix<'a> {
    #[serde(rename = "where")]
    side: &'static str,
    field: &'static str,
    value: &'a str,
}

fn first_chars(text: &str, n: usize) -> &str {
    text.char_indices().nth(n).map_or(text, |(at, _)| &text[..at])
}

fn drift(d: &Drift) -> DriftJson<'_> {
    DriftJson {
        level: d.level.as_str(),
        message: &d.message,
        cause: &d.cause,
        fix: d.fix.as_ref().map(|f| Fix {
            side: f.side.as_str(),
            field: f.field,
            value: &f.value,
        }),
    }
}

fn try_json(state: TryState<'_>) -> Option<Option<TryJson<'_>>> {
    match state {
        TryState::Unknown => None,
        TryState::NoPush => Some(None),
        TryState::Push { push, outdated } => Some(Some(TryJson {
            push,
            outdated,
            summary: TryText(state).to_string(),
        })),
    }
}

fn rev(s: &Snapshot, r: RevIdx) -> Rev<'_> {
    let x = s.rev(r);
    Rev {
        id: x.id.0,
        title: &x.title,
        status: x.status.as_str(),
        mine: s.is_mine(x),
        modified: x.modified,
        url: rev_url(x.id),
        author: s.raw.users.get(&x.author).map(String::as_str),
        reviewers: present::reviewers(x)
            .map(|v| Reviewer {
                name: s.name(&v.phid),
                status: v.status.as_str(),
            })
            .collect(),
        try_push: try_json(s.try_state(r)),
    }
}

fn jira(s: &Snapshot, i: IssueIdx) -> Jira<'_> {
    let x = s.issue(i);
    Jira {
        key: &x.key.0,
        url: jira_url(&x.key.0),
        summary: &x.summary,
        status: &x.status,
        status_category: x.status_category,
        assignee: x.assignee.as_deref(),
        points: x.points.map(f64::from),
        sprint: x.sprint.as_ref().map(|sp| SprintRef {
            name: &sp.name,
            end_date: sp.end_date,
        }),
        updated: x.updated,
    }
}

pub fn item<'a>(s: &'a Snapshot, e: &'a Entry) -> Item<'a> {
    let p = e.placement;
    Item {
        id: s.id(e).to_string(),
        title: s.title(e),
        section: p.map(|p| p.section.id()),
        reason: p.map(|_| s.reason(e).to_string()),
        since: p.and_then(|p| p.since),
        stale: p.is_some_and(|p| p.stale),
        tier: p.map(|p| p.tier.as_str()),
        tag: s.tag(e).to_string(),
        action: s.next_step(e).map(|n| n.to_string()),
        action_url: s.next_step(e).and_then(|step| worklens_core::guidance::next_url(&s.raw, step)),
        bug: e.bug.map(|b| {
            let x = s.bug(b);
            Bug {
                id: x.id.0,
                url: bug_url(x.id),
                state: State(x).to_string(),
                status: &x.status,
                resolution: (!x.resolution.is_empty()).then_some(x.resolution.as_str()),
                severity: &x.severity,
                priority: &x.priority,
                keywords: &x.keywords,
                product: &x.product,
                component: &x.component,
                assignee: assignee(x),
                last_change: x.last_change_time,
            }
        }),
        jira: e.issue.map(|i| jira(s, i)),
        parent: parent(s, e),
        idea: idea(s, e),
        revisions: e.revisions.iter().map(|r| rev(s, *r)).collect(),
        review: e.review.map(|r| rev(s, r)),
        needinfo: s.needinfo(e).map(|f| Needinfo {
            from: &f.setter,
            since: f.creation_date,
            comment: s.needinfo_comment(e).map(|c| first_chars(&c.text, EXCERPT_CHARS)),
        }),
        last_comment: s.last_comment(e).map(|c| LastComment {
            author: &c.author,
            time: c.time,
            excerpt: first_chars(&c.text, EXCERPT_CHARS),
        }),
        drift: e.drift.iter().map(drift).collect(),
        tracked: s.tracked_for(e),
        owner: (e.scope == Scope::Team).then(|| present::owner(s, e)).flatten(),
        hidden: e.marks.hidden.map(|h| match h {
            Hidden::Done => "done",
            Hidden::Snoozed => "snoozed",
        }),
        snoozed_until: e.marks.snoozed_until,
        is_new: e.marks.is_new,
        changes: &e.changes,
        workflow: worklens_core::evidence::workflow(s, e),
        dependencies: worklens_core::dependencies::of(s, e),
        timeline: worklens_core::timeline::events(s, e),
        ships: worklens_core::timeline::ships(s, e),
    }
}

pub fn compact(s: &Snapshot, e: &Entry) -> Value {
    if e.scope == Scope::Team {
        return compact_drift(s, e);
    }
    let mine = || e.revisions.iter().copied().filter(|r| s.is_mine(s.rev(*r)));
    let tries: BTreeMap<String, Value> = mine()
        .filter_map(|r| {
            let state = s.try_state(r);
            let value = match state {
                TryState::Unknown => return None,
                TryState::NoPush => Value::Null,
                TryState::Push { push, .. } => json!({ "status": TryText(state).to_string(), "url": push.url }),
            };
            Some((s.rev(r).id.to_string(), value))
        })
        .collect();
    let reviewers: BTreeMap<String, Vec<String>> = mine()
        .map(|r| s.rev(r))
        .filter(|r| !r.reviewers.is_empty())
        .map(|r| {
            let names = present::reviewers(r);
            (
                r.id.to_string(),
                names.map(|v| format!("{} {}", s.name(&v.phid), v.status.as_str())).collect(),
            )
        })
        .collect();
    let p = e.placement;
    json!({
        "id": s.id(e).to_string(),
        "title": s.title(e),
        "section": p.map(|p| p.section.id()),
        "workflow": worklens_core::evidence::workflow(s, e),
        "dependencies": worklens_core::dependencies::of(s, e),
        "reason": s.reason(e).to_string(),
        "since": p.and_then(|p| p.since),
        "stale": p.is_some_and(|p| p.stale),
        "action": s.next_step(e).map(|n| n.to_string()),
        "actionUrl": s.next_step(e).and_then(|step| worklens_core::guidance::next_url(&s.raw, step)),
        "bug": e.bug.map(|b| {
            let x = s.bug(b);
            json!({ "url": bug_url(x.id), "state": State(x).to_string(), "severity": x.severity, "priority": x.priority })
        }),
        "jira": e.issue.map(|i| {
            let x = s.issue(i);
            json!({ "key": x.key.0, "status": x.status, "points": x.points, "url": jira_url(&x.key.0) })
        }),
        "parent": s.parent(e).map(|p| format!("{} {}", p.key, p.status)),
        "idea": e.idea.map(|i| {
            let x = s.idea(i);
            json!({ "key": x.key.0, "status": x.status, "weeklyStatus": x.weekly_status, "target": x.target, "url": jira_url(&x.key.0) })
        }),
        "revisions": e.revisions.iter().map(|r| format!("{} {}", s.rev(*r).id, s.rev(*r).status.as_str())).collect::<Vec<_>>(),
        "try": tries,
        "reviewers": reviewers,
        "review": e.review.map(|r| s.rev(r).id.to_string()),
        "needinfo": s.needinfo(e).map(|f| json!({ "from": f.setter, "since": f.creation_date })),
        "drift": e.drift.iter().map(|d| d.message.as_str()).collect::<Vec<_>>(),
        "tracked": s.tracked_for(e),
        "isNew": e.marks.is_new,
        "changes": e.changes,
        "timeline": worklens_core::timeline::events(s, e),
        "ships": worklens_core::timeline::ships(s, e),
    })
}

pub fn compact_drift(s: &Snapshot, e: &Entry) -> Value {
    json!({
        "id": s.id(e).to_string(),
        "title": s.title(e),
        "owner": present::owner(s, e),
        "bug": e.bug.map(|b| bug_url(s.bug(b).id)),
        "jira": e.issue.map(|i| format!("{} {}", s.issue(i).key, s.issue(i).status)),
        "drift": e.drift.iter().map(drift).collect::<Vec<_>>(),
        "workflow": worklens_core::evidence::workflow(s, e),
    })
}

pub fn sprint(s: &Snapshot) -> Value {
    let Some(sp) = &s.sprint else { return Value::Null };
    let keys = |xs: &[IssueIdx]| xs.iter().map(|i| s.issue(*i).key.0.as_str()).collect::<Vec<_>>();
    let first = s.issue(sp.first).sprint.as_ref();
    json!({
        "name": first.map(|x| x.name.as_str()),
        "endDate": first.and_then(|x| x.end_date),
        "budget": sp.budget,
        "committed": sp.committed,
        "done": sp.done,
        "unestimated": keys(&sp.unestimated),
        "keys": keys(&sp.tickets),
    })
}

pub fn status(s: &Snapshot, now: Timestamp) -> serde_json::Map<String, Value> {
    let age = (now.as_millisecond() - s.raw.fetched_at.as_millisecond()).max(0) / 60_000;
    let mut map = serde_json::Map::new();
    map.insert("fetchedAt".into(), json!(s.raw.fetched_at));
    map.insert("ageMinutes".into(), json!(age));
    map.insert("partial".into(), json!(s.raw.failures().next().is_some()));
    map.insert("errors".into(), serde_json::to_value(&s.raw.errors).unwrap_or_default());
    map.insert("stale".into(), serde_json::to_value(&s.raw.stale).unwrap_or_default());
    map
}

pub enum Field<'a> {
    Value(Value),
    Items(Vec<Item<'a>>),
    Sections(Vec<SectionItems<'a>>),
}

#[derive(Serialize)]
pub struct SectionItems<'a> {
    pub id: &'static str,
    pub title: &'static str,
    pub items: Vec<Item<'a>>,
}

pub enum Doc<'a> {
    Object(Vec<(String, Field<'a>)>),
    Other(Value),
}

impl<'a> Doc<'a> {
    pub fn object(fields: impl IntoIterator<Item = (&'static str, Field<'a>)>) -> Self {
        Doc::Object(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
    }
}

impl From<Value> for Doc<'_> {
    fn from(value: Value) -> Self {
        match value {
            Value::Object(map) => Doc::Object(map.into_iter().map(|(k, v)| (k, Field::Value(v))).collect()),
            other => Doc::Other(other),
        }
    }
}

impl Serialize for Field<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Field::Value(v) => v.serialize(serializer),
            Field::Items(items) => items.serialize(serializer),
            Field::Sections(sections) => sections.serialize(serializer),
        }
    }
}

impl Serialize for Doc<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap as _;
        match self {
            Doc::Object(fields) => {
                let mut map = serializer.serialize_map(Some(fields.len()))?;
                for (k, v) in fields {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
            Doc::Other(v) => v.serialize(serializer),
        }
    }
}

pub fn stamp<'a>(doc: Doc<'a>, status: &serde_json::Map<String, Value>) -> Doc<'a> {
    match doc {
        Doc::Object(mut fields) => {
            for (key, v) in status {
                if !fields.iter().any(|(k, _)| k == key) {
                    fields.push((key.clone(), Field::Value(v.clone())));
                }
            }
            Doc::Object(fields)
        }
        other => other,
    }
}
