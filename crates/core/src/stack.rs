use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::ids::{EntryIdx, RevId, StackIdx};
use crate::raw::{Raw, RevStatus};
use crate::snapshot::{Entry, Snapshot};

#[derive(Debug, PartialEq, Eq)]
pub struct Stack {
    pub revisions: Vec<RevId>,
    pub entries: Vec<EntryIdx>,
}

struct Known {
    id: RevId,
    status: RevStatus,
}

fn open(status: RevStatus) -> bool {
    !matches!(status, RevStatus::Published | RevStatus::Abandoned)
}

fn known(raw: &Raw) -> HashMap<&str, Known> {
    let mut out = HashMap::new();
    for r in raw.dependencies.revisions.values().chain(&raw.revisions) {
        if let Some(phid) = r.phid.as_deref() {
            out.insert(
                phid,
                Known {
                    id: r.id,
                    status: r.status,
                },
            );
        }
    }
    out
}

fn status_of(raw: &Raw, id: RevId) -> Option<RevStatus> {
    raw.revisions
        .iter()
        .chain(raw.dependencies.revisions.values())
        .find(|r| r.id == id)
        .map(|r| r.status)
}

fn chain(raw: &Raw, known: &HashMap<&str, Known>, top: RevId) -> Vec<RevId> {
    let mut out = vec![top];
    let mut at = top;
    while let Some(parent) = raw.dependencies.parents.get(&at).and_then(|p| p.first()) {
        let Some(k) = known.get(parent.as_str()).filter(|k| open(k.status)) else {
            break;
        };
        if out.contains(&k.id) {
            break;
        }
        out.push(k.id);
        at = k.id;
    }
    out.reverse();
    out
}

fn revision_of(raw: &Raw, e: &Entry) -> Option<RevId> {
    if let Some(r) = e.review {
        return Some(raw.revisions[r.get()].id);
    }
    let me = raw.me.phid.as_deref()?;
    e.revisions
        .iter()
        .map(|r| &raw.revisions[r.get()])
        .find(|r| r.author == me && open(r.status))
        .map(|r| r.id)
}

#[derive(Default)]
struct Group {
    revisions: Vec<RevId>,
    members: Vec<(usize, EntryIdx)>,
}

pub fn find(raw: &Raw, entries: &[Entry]) -> Vec<Stack> {
    let known = known(raw);
    let mut groups: Vec<Group> = Vec::new();
    let mut slot: HashMap<RevId, usize> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        let Some(top) = revision_of(raw, e) else { continue };
        let revisions = chain(raw, &known, top);
        let n = *slot.entry(revisions[0]).or_insert_with(|| {
            groups.push(Group::default());
            groups.len() - 1
        });
        let group = &mut groups[n];
        group.members.push((revisions.len(), EntryIdx::new(i)));
        if revisions.len() > group.revisions.len() {
            group.revisions = revisions;
        }
    }
    groups
        .into_iter()
        .filter(|g| g.members.len() > 1)
        .map(|mut g| {
            g.members.sort();
            Stack {
                revisions: g.revisions,
                entries: g.members.into_iter().map(|(_, e)| e).collect(),
            }
        })
        .collect()
}

pub fn assign(stacks: &[Stack], entries: &mut [Entry]) {
    for (k, stack) in stacks.iter().enumerate() {
        for e in &stack.entries {
            entries[e.get()].stack = Some(StackIdx::new(k));
        }
    }
}

pub fn group(list: &mut Vec<EntryIdx>, entries: &[Entry], stacks: &[Stack]) {
    let here: HashSet<EntryIdx> = list.iter().copied().collect();
    let mut placed: HashSet<EntryIdx> = HashSet::with_capacity(list.len());
    let mut out = Vec::with_capacity(list.len());
    for i in list.iter() {
        if placed.contains(i) {
            continue;
        }
        match entries[i.get()].stack {
            Some(k) => {
                for m in stacks[k.get()].entries.iter().filter(|m| here.contains(m)) {
                    placed.insert(*m);
                    out.push(*m);
                }
            }
            None => out.push(*i),
        }
    }
    *list = out;
}

pub fn header(s: &Snapshot, k: StackIdx) -> String {
    let stack = s.stack(k);
    let bottom = stack.revisions[0];
    let status = status_of(&s.raw, bottom).map_or("status unknown", RevStatus::as_str);
    format!("Stack of {} · bottom {bottom} {status}", stack.revisions.len())
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub position: usize,
    pub of: usize,
    pub bottom: String,
    pub bottom_status: Option<&'static str>,
    pub revisions: Vec<String>,
}

pub fn place(s: &Snapshot, e: &Entry) -> Option<Place> {
    let stack = s.stack(e.stack?);
    let mine = revision_of(&s.raw, e)?;
    let at = stack.revisions.iter().position(|r| *r == mine)?;
    let bottom = stack.revisions[0];
    Some(Place {
        position: at + 1,
        of: stack.revisions.len(),
        bottom: bottom.to_string(),
        bottom_status: status_of(&s.raw, bottom).map(RevStatus::as_str),
        revisions: stack.revisions.iter().map(ToString::to_string).collect(),
    })
}

pub fn position(s: &Snapshot, e: &Entry) -> Option<String> {
    let p = place(s, e)?;
    Some(if p.position == 1 {
        format!("Stack: bottom of {}; it lands first", p.of)
    } else {
        format!("Stack: {} of {}", p.position, p.of)
    })
}
