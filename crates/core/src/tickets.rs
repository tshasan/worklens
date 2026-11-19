use std::collections::HashSet;

use crate::bugs::jira_keys_of;
use crate::ids::{IdeaIdx, IssueIdx};
use crate::raw::{Bug, Issue, Raw};

const CONTAINER_KINDS: [&str; 2] = ["epic", "initiative"];

pub struct Parents<'a>(HashSet<&'a str>);

impl<'a> Parents<'a> {
    pub fn of(raw: &'a Raw) -> Self {
        let mut keys: HashSet<&str> = raw
            .issues
            .values()
            .filter_map(|i| i.parent.as_ref())
            .map(|k| k.0.as_str())
            .collect();
        keys.extend(raw.roadmap.parents.keys().map(|k| k.0.as_str()));
        Self(keys)
    }
}

pub struct Tickets<'a> {
    pub key: Option<&'a str>,
    pub issue: Option<IssueIdx>,
    pub rollups: Vec<&'a str>,
    pub idea: Option<IdeaIdx>,
}

fn is_meta(summary: &str) -> bool {
    summary.trim_start().get(..6).is_some_and(|p| p.eq_ignore_ascii_case("[meta]"))
}

fn is_idea(raw: &Raw, key: &str, issue: Option<&Issue>) -> bool {
    raw.roadmap.ideas.contains_key(key)
        || issue
            .and_then(|i| i.kind.as_deref())
            .is_some_and(|k| k.eq_ignore_ascii_case("idea"))
}

fn is_container(parents: &Parents<'_>, key: &str, issue: Option<&Issue>) -> bool {
    parents.0.contains(key)
        || issue.is_some_and(|i| {
            let kind = i.kind.as_deref().unwrap_or_default();
            parents.0.contains(i.key.0.as_str()) || is_meta(&i.summary) || CONTAINER_KINDS.iter().any(|k| kind.eq_ignore_ascii_case(k))
        })
}

pub fn is_rollup(raw: &Raw, parents: &Parents<'_>, bug: &Bug, key: &str, issue: Option<&Issue>) -> bool {
    let meta_bug = bug.keywords.iter().any(|k| k == "meta") || is_meta(&bug.summary);
    is_idea(raw, key, issue) || (!meta_bug && is_container(parents, key, issue))
}

pub fn of<'a>(raw: &'a Raw, parents: &Parents<'_>, bug: &'a Bug) -> Tickets<'a> {
    let mut out = Tickets {
        key: None,
        issue: None,
        rollups: Vec::new(),
        idea: None,
    };
    let mut best = u8::MAX;
    for key in jira_keys_of(bug) {
        if out.key == Some(key) || out.rollups.contains(&key) {
            continue;
        }
        let at = raw.issues.get_index_of(key);
        let issue = at.and_then(|i| raw.issues.get_index(i)).map(|(_, x)| x);
        if is_rollup(raw, parents, bug, key, issue) {
            out.rollups.push(key);
            if out.idea.is_none() {
                out.idea = raw
                    .roadmap
                    .ideas
                    .get_index_of(key)
                    .map(IdeaIdx::new)
                    .or_else(|| issue.and_then(|x| crate::ideas::idea_of(raw, x)));
            }
            continue;
        }
        let rank = issue.map_or(4, |x| {
            let planned = x.sprint.as_ref().is_some_and(|s| s.state != "closed");
            let owned = crate::drift::same_assignee(crate::bugs::assignee(bug), x.assignee.as_deref(), &raw.me);
            match (planned, owned) {
                (true, true) => 0,
                (true, false) => 1,
                (false, true) => 2,
                (false, false) => 3,
            }
        });
        if rank < best {
            best = rank;
            out.key = Some(key);
            out.issue = at.map(IssueIdx::new);
        }
    }
    out
}
