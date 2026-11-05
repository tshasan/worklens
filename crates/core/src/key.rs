use std::fmt;

use crate::ids::{BugId, EntryIdx, JiraKey, RevId};
use crate::marks::Hidden;
use crate::section::Section;
use crate::snapshot::{Entry, ItemId, Scope, Snapshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryKey {
    item: Item,
    scope: Scope,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Item {
    Bug(BugId),
    Jira(JiraKey),
    Rev(RevId),
    Idea(JiraKey),
}

impl EntryKey {
    pub fn of(s: &Snapshot, e: &Entry) -> Self {
        let item = match e.id {
            ItemId::Bug(b) => Item::Bug(b),
            ItemId::Jira(i) => Item::Jira(s.issue(i).key.clone()),
            ItemId::Rev(r) => Item::Rev(r),
            ItemId::Idea(i) => Item::Idea(s.idea(i).key.clone()),
        };
        Self { item, scope: e.scope }
    }

    pub fn is(&self, s: &Snapshot, e: &Entry) -> bool {
        self.scope == e.scope
            && match (&self.item, e.id) {
                (Item::Bug(a), ItemId::Bug(b)) => *a == b,
                (Item::Jira(a), ItemId::Jira(i)) => *a == s.issue(i).key,
                (Item::Rev(a), ItemId::Rev(b)) => *a == b,
                (Item::Idea(a), ItemId::Idea(i)) => *a == s.idea(i).key,
                _ => false,
            }
    }

    pub fn find(&self, s: &Snapshot) -> Option<EntryIdx> {
        s.entries.iter().position(|e| self.is(s, e)).map(EntryIdx::new)
    }
}

impl fmt::Display for EntryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.item {
            Item::Bug(b) => write!(f, "{b}"),
            Item::Jira(k) | Item::Idea(k) => f.write_str(&k.0),
            Item::Rev(r) => write!(f, "{r}"),
        }
    }
}

pub fn mark_key(s: &Snapshot, e: &Entry) -> String {
    s.id(e).to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Whereabouts {
    Hidden(Hidden),
    In(Section),
    Unlisted,
}

impl Entry {
    pub fn whereabouts(&self) -> Whereabouts {
        match (self.marks.hidden, self.placement) {
            (Some(h), _) => Whereabouts::Hidden(h),
            (None, Some(p)) => Whereabouts::In(p.section),
            (None, None) => Whereabouts::Unlisted,
        }
    }
}
