use crate::bugs::{State, assignee, is_closed, is_dead, jira_url};
use crate::raw::{Bug, Issue, Me, RevStatus, StatusCategory};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Synced {
    Summary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Error,
    Warn,
}

impl Level {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Jira,
    Bugzilla,
}

impl Side {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Jira => "jira",
            Self::Bugzilla => "bugzilla",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Fix {
    pub side: Side,
    pub field: &'static str,
    pub value: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Drift {
    pub level: Level,
    pub message: String,
    pub cause: String,
    pub fix: Option<Fix>,
}

pub struct DriftInput<'a> {
    pub bug: &'a Bug,
    pub key: Option<&'a str>,
    pub issue: Option<&'a Issue>,
    pub revs: &'a [RevStatus],
    pub mirrored: bool,
    pub me: &'a Me,
    pub statuses: &'a JiraStatuses,
    pub active_sprint: Option<&'a str>,
    pub syncs: &'a [Synced],
}

fn same_title(bug: &str, jira: &str) -> bool {
    let jira = jira.trim();
    bug.trim() == jira || (!jira.is_empty() && bug.starts_with(jira) && bug[jira.len()..].starts_with(char::is_whitespace))
}

fn same_email(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn same_assignee(bz: Option<&str>, jira: Option<&str>, me: &Me) -> bool {
    match (bz, jira) {
        (None, None) => true,
        (Some(bz), Some(jira)) => {
            same_email(bz, jira) || (same_email(bz, &me.email) && me.jira_email.as_deref().is_some_and(|mine| same_email(jira, mine)))
        }
        _ => false,
    }
}

#[derive(Clone, Debug)]
pub struct JiraStatuses {
    pub done: Vec<String>,
    pub review: Vec<String>,
    pub in_progress: Vec<String>,
    pub blocked: Vec<String>,
}

impl Default for JiraStatuses {
    fn default() -> Self {
        let names = |xs: &[&str]| xs.iter().map(|x| (*x).to_owned()).collect();
        Self {
            done: names(&["Closed", "READY TO RELEASE"]),
            review: names(&["ENG REVIEW"]),
            in_progress: names(&["In Progress"]),
            blocked: names(&["Blocked"]),
        }
    }
}

fn names_contain(names: &[String], status: &str) -> bool {
    names.iter().any(|n| n.eq_ignore_ascii_case(status))
}

impl JiraStatuses {
    fn knows(&self, status: &str) -> bool {
        [&self.done, &self.review, &self.in_progress, &self.blocked]
            .iter()
            .any(|names| names_contain(names, status))
    }
}

struct Expected<'a> {
    names: &'a [String],
    category: StatusCategory,
    why: &'static str,
}

impl Expected<'_> {
    fn suggestion(&self) -> &str {
        self.names.first().map_or(
            match self.category {
                StatusCategory::New => "To Do",
                StatusCategory::Indeterminate => "In Progress",
                StatusCategory::Done => "Done",
            },
            String::as_str,
        )
    }
}

fn expected_from_revisions<'a>(revs: &[RevStatus], statuses: &'a JiraStatuses) -> Option<Expected<'a>> {
    let live = || revs.iter().copied().filter(|s| *s != RevStatus::Abandoned);
    live().next()?;
    Some(if live().all(|s| s == RevStatus::Published) {
        Expected {
            names: &statuses.done,
            category: StatusCategory::Done,
            why: "all revisions landed",
        }
    } else if live().any(|s| matches!(s, RevStatus::NeedsReview | RevStatus::Accepted | RevStatus::NeedsRevision)) {
        Expected {
            names: &statuses.review,
            category: StatusCategory::Indeterminate,
            why: "a revision is in review",
        }
    } else {
        Expected {
            names: &statuses.in_progress,
            category: StatusCategory::Indeterminate,
            why: "revisions are WIP",
        }
    })
}

fn closed_cause(bug: &Bug) -> String {
    if bug.resolution == "INACTIVE" || bug.resolution == "MOVED" {
        format!("JBI has no status for {}, so it never closes the ticket", bug.resolution)
    } else {
        let what = if bug.resolution.is_empty() { &bug.status } else { &bug.resolution };
        format!("JBI maps {what} to Closed, so the sync failed or the ticket was reopened in Jira (JBI is one-way)")
    }
}

fn bugzilla_category(bug: &Bug) -> StatusCategory {
    if is_closed(bug) {
        StatusCategory::Done
    } else if bug.status == "ASSIGNED" || bug.status == "REOPENED" {
        StatusCategory::Indeterminate
    } else {
        StatusCategory::New
    }
}

fn jira_status(value: &str) -> Option<Fix> {
    Some(Fix {
        side: Side::Jira,
        field: "status",
        value: value.to_owned(),
    })
}

pub fn drift_of(input: &DriftInput<'_>) -> Vec<Drift> {
    let DriftInput {
        bug,
        key,
        issue,
        revs,
        mirrored,
        me,
        statuses,
        active_sprint,
        syncs,
    } = *input;
    let closed = statuses.done.first().map_or("Closed", String::as_str);
    let mut out = Vec::new();
    let mut push = |level, message: String, cause: String, fix: Option<Fix>| {
        out.push(Drift {
            level,
            message,
            cause,
            fix,
        })
    };

    if is_dead(bug) {
        if let Some(issue) = issue.filter(|i| i.status_category != StatusCategory::Done) {
            push(
                Level::Error,
                format!("bug is {} but {} is still {}", bug.resolution, issue.key, issue.status),
                closed_cause(bug),
                jira_status(closed),
            );
        }
        return out;
    }
    let Some(key) = key else {
        if mirrored {
            push(
                Level::Error,
                "no Jira link in See Also".to_owned(),
                "JBI makes the ticket when the whiteboard tag is added; it rejects private bugs, and a failed sync waits in its dead-letter queue"
                    .to_owned(),
                None,
            );
        }
        return out;
    };
    let Some(issue) = issue else {
        push(
            Level::Error,
            format!("{key} not found (moved, deleted, or not visible)"),
            "the ticket was deleted, or moved to a project you can't see".to_owned(),
            None,
        );
        return out;
    };
    if issue.key.0 != key {
        push(
            Level::Warn,
            format!("See Also points at old key {key}; it's now {}", issue.key),
            "the ticket moved to another project; JBI doesn't update See Also".to_owned(),
            Some(Fix {
                side: Side::Bugzilla,
                field: "See Also",
                value: jira_url(&issue.key.0),
            }),
        );
    }

    let bz = bugzilla_category(bug);
    let jira_done = issue.status_category == StatusCategory::Done;
    let expected = expected_from_revisions(revs, statuses);
    if bz == StatusCategory::Done && !jira_done {
        push(
            Level::Error,
            format!("bug is {} but Jira is still {}", State(bug), issue.status),
            closed_cause(bug),
            jira_status(closed),
        );
    } else if bz != StatusCategory::Done && jira_done {
        push(
            Level::Error,
            format!("Jira is {} but bug is still {}", issue.status, State(bug)),
            "closed in Jira; JBI is one-way, so the bug isn't updated".to_owned(),
            None,
        );
    } else if let Some(e) = expected.as_ref().filter(|e| {
        let status = issue.status.as_str();
        let disagrees = if statuses.knows(status) {
            !names_contain(e.names, status)
        } else {
            issue.status_category != e.category
        };
        bz != StatusCategory::Done && !names_contain(&statuses.blocked, status) && disagrees
    }) {
        push(
            Level::Error,
            format!("Jira is {} but {}; expected {}", issue.status, e.why, e.suggestion()),
            "JBI sets status from the bug only, not from Phabricator; review and landing statuses are moved by hand".to_owned(),
            jira_status(e.suggestion()),
        );
    } else if expected.is_none() && bz == StatusCategory::New && issue.status_category == StatusCategory::Indeterminate {
        push(
            Level::Warn,
            format!("Jira is {} but the bug is {} with no revisions", issue.status, bug.status),
            format!(
                "JBI doesn't sync {0}: a bug reset to {0} leaves the ticket where it was",
                bug.status
            ),
            None,
        );
    }

    let open_revision = revs.iter().any(|r| {
        matches!(
            r,
            RevStatus::NeedsReview | RevStatus::Accepted | RevStatus::NeedsRevision | RevStatus::ChangesPlanned
        )
    });
    if let Some(sprint) = active_sprint
        && issue.sprint.is_none()
        && !jira_done
        && open_revision
    {
        push(
            Level::Warn,
            format!("{} is in no sprint, but the bug has an open revision", issue.key),
            "JBI creates and updates the ticket but never plans it; sprints are set by hand in Jira".to_owned(),
            Some(Fix {
                side: Side::Jira,
                field: "sprint",
                value: sprint.to_owned(),
            }),
        );
    }

    if syncs.contains(&Synced::Summary) && !same_title(&bug.summary, &issue.summary) {
        push(
            Level::Warn,
            "title: Jira's differs from the bug's".to_owned(),
            "JBI copies the title only when it changes in Bugzilla, so an edit in Jira stays until then".to_owned(),
            Some(Fix {
                side: Side::Jira,
                field: "summary",
                value: bug.summary.clone(),
            }),
        );
    }

    let bz_who = assignee(bug);
    let jira_who = issue.assignee.as_deref();
    if !same_assignee(bz_who, jira_who, me) {
        let message = format!(
            "assignee: Bugzilla {}, Jira {}",
            bz_who.unwrap_or("nobody"),
            jira_who.unwrap_or("unassigned")
        );
        let fix = bz_who.map(|who| Fix {
            side: Side::Jira,
            field: "assignee",
            value: who.to_owned(),
        });
        let (level, cause) = match (bz_who, jira_who) {
            (_, None) => (
                Level::Warn,
                "JBI leaves the Jira assignee empty when the Bugzilla email has no Jira account",
            ),
            (None, _) => (Level::Warn, "assigned in Jira; JBI is one-way, so Bugzilla isn't updated"),
            _ => (
                Level::Error,
                "JBI only re-sends the assignee when it changes in Bugzilla, so one side was changed by hand",
            ),
        };
        push(level, message, cause.to_owned(), fix);
    }
    out
}
