use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BugId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RevId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JiraKey(pub String);

macro_rules! index_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u32);

        impl $name {
            pub fn new(i: usize) -> Self {
                Self(u32::try_from(i).expect("fewer than 4 billion items"))
            }

            pub const fn get(self) -> usize {
                self.0 as usize
            }
        }
    };
}

index_type!(BugIdx);
index_type!(RevIdx);
index_type!(IssueIdx);
index_type!(EntryIdx);
index_type!(IdeaIdx);

impl fmt::Display for BugId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for RevId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "D{}", self.0)
    }
}

impl std::borrow::Borrow<str> for JiraKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for JiraKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display() {
        assert_eq!(BugId(2_086_888).to_string(), "2086888");
        assert_eq!(RevId(339_120).to_string(), "D339120");
    }
}
