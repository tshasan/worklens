use std::borrow::Cow;

use crate::marks::{BugSig, FactSig, JiraSig, Mark, NeedinfoSig, ReviewSig, RevisionSig, Signature};
use crate::section::Section;
use crate::snapshot::{Entry, Snapshot};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Failed {
    pub bugzilla: bool,
    pub phabricator: bool,
    pub jira: bool,
    pub treeherder: bool,
    pub landings: bool,
    pub bug_dependencies: bool,
    pub revision_dependencies: bool,
}

impl Failed {
    pub fn of(s: &Snapshot) -> Self {
        let failed = |source: &str| s.raw.errors.iter().any(|e| e.source == source);
        Self {
            bugzilla: failed("bugzilla"),
            phabricator: failed("phabricator"),
            jira: failed("jira"),
            treeherder: failed("treeherder"),
            landings: failed("treeherder-landings"),
            bug_dependencies: failed("bugzilla") || failed("bugzilla-dependencies"),
            revision_dependencies: failed("phabricator") || failed("phabricator-dependencies"),
        }
    }
}

pub fn since_seen(s: &Snapshot, e: &Entry, seen: &Mark, new: &Signature) -> Vec<String> {
    let mut out = match Signature::parse(&seen.sig) {
        Some(old) => changes(&old, new, Failed::of(s)),
        None => Vec::new(),
    };
    if let Some(c) = s.last_comment(e).filter(|c| c.author != s.me() && c.time > seen.at) {
        out.push(format!("new comment from {}", c.author));
    }
    out
}

pub fn changes(old: &Signature, new: &Signature, failed: Failed) -> Vec<String> {
    let mut out = Vec::new();
    if !failed.bugzilla
        && !failed.phabricator
        && !failed.jira
        && !failed.treeherder
        && !failed.bug_dependencies
        && !failed.revision_dependencies
    {
        section(old.section.as_deref(), new.section.as_deref(), &mut out);
    }
    if !failed.bugzilla {
        needinfo(old.needinfo.as_ref(), new.needinfo.as_ref(), &mut out);
    }
    if !failed.phabricator {
        review(old.review.as_ref(), new.review.as_ref(), &mut out);
        revisions(&old.revisions, &new.revisions, failed.treeherder, &mut out);
    }
    if !failed.jira {
        jira(old.jira.as_ref(), new.jira.as_ref(), &mut out);
    }
    if !failed.bugzilla {
        bug(old.bug.as_ref(), new.bug.as_ref(), &mut out);
    }
    if !failed.bugzilla && !failed.phabricator && !failed.jira {
        drift(&old.drift, &new.drift, &mut out);
    }
    if let (Some(old), Some(new)) = (&old.facts, &new.facts) {
        for fact in new {
            let unavailable = match fact.source.as_str() {
                "bugzilla" => failed.bugzilla,
                "phabricator" => failed.phabricator,
                "treeherder" => failed.treeherder,
                "treeherder-landings" => failed.landings,
                "bugzilla-dependencies" => failed.bug_dependencies,
                "phabricator-dependencies" => failed.revision_dependencies,
                _ => false,
            };
            if unavailable {
                continue;
            }
            match old.iter().find(|v| v.source == fact.source && v.label == fact.label) {
                Some(was) if was.value != fact.value => out.push(fact_change(fact, Some(was))),
                None if !fact.value.is_empty() => out.push(fact_change(fact, None)),
                _ => {}
            }
        }
    }
    out
}

fn fact_change(fact: &FactSig, old: Option<&FactSig>) -> String {
    if let Some(id) = fact.label.strip_suffix(" diff") {
        return if old.is_some() {
            format!("{id}: new diff")
        } else {
            format!("{id}: diff observed")
        };
    }
    if fact.label.ends_with("parent relationships") || fact.label.ends_with("project tags") {
        let what = if old.is_some() { "changed" } else { "observed" };
        return format!("{} {what}; check Phabricator", fact.label);
    }
    match old {
        Some(was) => format!("{}: {} → {}", fact.label, value(&was.value), value(&fact.value)),
        None => format!("{} observed: {}", fact.label, value(&fact.value)),
    }
}

fn value(text: &str) -> &str {
    if text.is_empty() { "none" } else { text }
}

fn text(v: Option<&str>) -> &str {
    v.unwrap_or("nobody")
}

fn section(old: Option<&str>, new: Option<&str>, out: &mut Vec<String>) {
    let find = |id: &str| Section::ALL.into_iter().find(|s| s.id() == id);
    if old == new || old.is_some_and(|id| find(id).is_none()) {
        return;
    }
    let title = |v: Option<&str>| v.and_then(find).map_or("no section", Section::title);
    out.push(format!("moved: {} → {}", title(old), title(new)));
}

fn needinfo(old: Option<&NeedinfoSig>, new: Option<&NeedinfoSig>, out: &mut Vec<String>) {
    match (old, new) {
        (None, Some(n)) => out.push(format!("new needinfo from {}", n.setter)),
        (Some(o), None) => out.push(format!("needinfo from {} answered or cleared", o.setter)),
        (Some(o), Some(n)) if o != n => out.push(format!("new needinfo from {}", n.setter)),
        _ => {}
    }
}

fn review(old: Option<&ReviewSig>, new: Option<&ReviewSig>, out: &mut Vec<String>) {
    match (old, new) {
        (None, Some(n)) => out.push(format!("review asked: D{}", n.id)),
        (Some(o), None) => out.push(format!("D{} no longer waits on your review", o.id)),
        (Some(o), Some(n)) if o.status != n.status => {
            out.push(format!("D{}: {} → {}", n.id, o.status, n.status));
        }
        _ => {}
    }
}

fn revisions(old: &[RevisionSig], new: &[RevisionSig], failed_try: bool, out: &mut Vec<String>) {
    for n in new {
        let id = n.id;
        let Some(o) = old.iter().find(|r| r.id == id) else {
            out.push(format!("D{id} linked"));
            continue;
        };
        if o.status != n.status {
            out.push(format!("D{id}: {} → {}", o.status, n.status));
        }
        reviewers(id, &o.reviewers, &n.reviewers, out);
        if !failed_try && o.try_state != n.try_state {
            let old_try = o.try_state.as_deref().unwrap_or("none");
            let new_try = n.try_state.as_deref().unwrap_or("none");
            out.push(format!("D{id} try: {old_try} → {new_try}"));
        }
    }
    for o in old {
        if !new.iter().any(|n| n.id == o.id) {
            out.push(format!("D{} no longer linked", o.id));
        }
    }
}

fn reviewers(id: u32, old: &[String], new: &[String], out: &mut Vec<String>) {
    let split = |list: &[String]| -> Vec<(String, String)> {
        list.iter()
            .filter_map(|x| x.rsplit_once(':').map(|(n, st)| (n.to_owned(), st.to_owned())))
            .collect()
    };
    let (old, new) = (split(old), split(new));
    for (name, status) in &new {
        match old.iter().find(|(n, _)| n == name) {
            Some((_, was)) if was != status => out.push(format!("D{id}: {name} {was} → {status}")),
            Some(_) => {}
            None => out.push(format!("D{id}: {name} added as reviewer ({status})")),
        }
    }
    for (name, _) in &old {
        if !new.iter().any(|(n, _)| n == name) {
            out.push(format!("D{id}: {name} removed or resigned"));
        }
    }
}

fn jira(old: Option<&JiraSig>, new: Option<&JiraSig>, out: &mut Vec<String>) {
    match (old, new) {
        (None, Some(n)) => out.push(format!("Jira {} linked", n.key)),
        (Some(o), None) => out.push(format!("Jira {} no longer linked", o.key)),
        (Some(o), Some(n)) => {
            let key = &n.key;
            if o.key != n.key {
                out.push(format!("Jira key: {} → {key}", o.key));
            }
            if o.status != n.status {
                out.push(format!("{key}: {} → {}", o.status, n.status));
            }
            if o.assignee != n.assignee {
                out.push(format!(
                    "{key} assignee: {} → {}",
                    text(o.assignee.as_deref()),
                    text(n.assignee.as_deref())
                ));
            }
        }
        (None, None) => {}
    }
}

fn bug(old: Option<&BugSig>, new: Option<&BugSig>, out: &mut Vec<String>) {
    let (Some(old), Some(new)) = (old, new) else {
        return;
    };
    if old.state != new.state {
        out.push(format!("bug: {} → {}", old.state, new.state));
    }
    if old.assignee != new.assignee {
        out.push(format!(
            "bug assignee: {} → {}",
            text(old.assignee.as_deref()),
            text(new.assignee.as_deref())
        ));
    }
}

fn drift(old: &[Cow<str>], new: &[Cow<str>], out: &mut Vec<String>) {
    for n in new.iter().filter(|n| !old.contains(n)) {
        out.push(format!("new drift: {n}"));
    }
    for o in old.iter().filter(|o| !new.contains(o)) {
        out.push(format!("drift resolved: {o}"));
    }
}
