use std::collections::{BTreeMap, HashMap};

use indexmap::IndexMap;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::ids::{BugId, JiraKey, RevId};

pub const RAW_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Raw {
    pub version: u32,
    pub fetched_at: Timestamp,
    pub me: Me,
    pub bugs: Vec<Bug>,
    pub issues: IndexMap<JiraKey, Issue>,
    pub revisions: Vec<Revision>,
    pub to_review: Vec<RevId>,
    pub bug_revisions: HashMap<BugId, Vec<RevId>>,
    pub last_comments: HashMap<BugId, Comment>,
    #[serde(default)]
    pub calls: Vec<Call>,
    #[serde(default)]
    pub try_pushes: Option<HashMap<RevId, TryPush>>,
    #[serde(default)]
    pub users: HashMap<String, String>,
    #[serde(default)]
    pub threads: HashMap<BugId, Thread>,
    pub errors: Vec<SourceError>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stale: Vec<Stale>,
    #[serde(default)]
    pub trains: Option<Trains>,
    #[serde(default)]
    pub uplift: HashMap<BugId, Uplift>,
    #[serde(default)]
    pub crashes: HashMap<BugId, Crashes>,
    #[serde(default)]
    pub group_review: HashMap<RevId, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<Telemetry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landings: Option<Landings>,
    #[serde(default)]
    pub supplemental_cache: SupplementalCache,
    #[serde(default)]
    pub roadmap: Roadmap,
    #[serde(default)]
    pub dependencies: Dependencies,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub away: HashMap<String, Timestamp>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Dependencies {
    #[serde(default)]
    pub bugs: HashMap<BugId, Bug>,
    #[serde(default)]
    pub parents: HashMap<RevId, Vec<String>>,
    #[serde(default)]
    pub revisions: HashMap<String, Revision>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceState {
    #[serde(rename = "ok")]
    Read,
    Off,
    Failed,
}

impl Raw {
    pub fn failures(&self) -> impl Iterator<Item = &SourceError> {
        self.errors.iter().filter(|e| !e.unset)
    }

    pub fn states(&self) -> [(&'static str, SourceState); 3] {
        ["bugzilla", "jira", "phabricator"].map(|source| (source, self.state(source)))
    }

    pub fn state(&self, source: &str) -> SourceState {
        let mut errors = self.errors.iter().filter(|e| e.source == source);
        match errors.next() {
            None => SourceState::Read,
            Some(first) if first.unset && errors.all(|e| e.unset) => SourceState::Off,
            Some(_) => SourceState::Failed,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Roadmap {
    pub parents: IndexMap<JiraKey, Issue>,
    pub ideas: IndexMap<JiraKey, Idea>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Idea {
    pub key: JiraKey,
    pub summary: String,
    pub status: String,
    pub status_category: StatusCategory,
    pub assignee: Option<String>,
    pub updated: Timestamp,
    #[serde(default)]
    pub weekly_status: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub quarter: Option<String>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub team: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupplementalCache {
    pub telemetry_attempted_at: Option<Timestamp>,
    pub landings_attempted_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Landings {
    pub revisions: Vec<RevId>,
    pub repositories: Vec<RepositoryLandings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryLandings {
    #[serde(default)]
    pub retry_at: Option<Timestamp>,
    pub repository: String,
    pub checked_at: Timestamp,
    pub push_limit: usize,
    pub scanned_pushes: usize,
    pub oldest_push: Option<Timestamp>,
    pub error: Option<String>,
    pub matches: Vec<Landing>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backouts: Vec<Backout>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backout {
    #[serde(default)]
    pub targets: Vec<String>,
    pub bugs: Vec<BugId>,
    pub cause: Option<String>,
    pub commit: String,
    pub push_revision: String,
    pub url: String,
    pub pushed_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Landing {
    pub revision: RevId,
    pub commit: String,
    pub push_revision: String,
    pub url: String,
    pub pushed_at: Timestamp,
    pub health: Option<PushHealth>,
    pub health_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PushHealth {
    pub builds: TryResult,
    pub lint: TryResult,
    pub tests: TryResult,
    pub failed: u32,
    pub running: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Telemetry {
    pub fetched_at: Timestamp,
    pub metrics: Vec<Metric>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metric {
    pub name: String,
    pub expires: String,
    pub disabled: bool,
    pub notification_emails: Vec<String>,
    pub bugs: Vec<BugId>,
    pub data_reviews: Vec<String>,
    pub source_url: Option<String>,
    pub definition_last_seen: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Crashes {
    pub count: u32,
    pub since: jiff::civil::Date,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Uplift {
    pub patch: bool,
    pub approvals: Vec<(String, String)>,
    #[serde(default)]
    pub flags: Vec<AttachmentFlag>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AttachmentFlag {
    pub attachment: Option<u32>,
    pub name: String,
    pub status: String,
}

impl Uplift {
    pub fn approval(&self, channel: &str) -> Option<&str> {
        self.approvals.iter().find(|(c, _)| c == channel).map(|(_, s)| s.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trains {
    pub nightly: u32,
    pub beta: u32,
    pub release: u32,
    pub esr: Vec<u32>,
    pub next_merge: jiff::civil::Date,
    pub next_release: jiff::civil::Date,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Me {
    pub email: String,
    pub jira_email: Option<String>,
    pub phid: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Bug {
    pub id: BugId,
    pub summary: String,
    pub status: String,
    pub resolution: String,
    pub assigned_to: String,
    pub severity: String,
    pub priority: String,
    pub keywords: Vec<String>,
    pub whiteboard: String,
    pub see_also: Vec<String>,
    pub last_change_time: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creation_time: Option<Timestamp>,
    #[serde(default)]
    pub flags: Vec<Flag>,
    #[serde(default)]
    pub depends_on: Vec<BugId>,
    #[serde(default)]
    pub regressed_by: Vec<BugId>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cf_crash_signature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cf_last_resolved: Option<Timestamp>,
    pub product: String,
    pub component: String,
    #[serde(default)]
    pub via: Vec<Via>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub release_flags: BTreeMap<String, String>,
}

impl Bug {
    pub fn status_for(&self, version: &str) -> Option<&str> {
        self.release_flags.get(&format!("cf_status_firefox{version}")).map(String::as_str)
    }

    pub fn tracking_for(&self, version: &str) -> Option<&str> {
        self.release_flags.get(&format!("cf_tracking_firefox{version}")).map(String::as_str)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    Assigned,
    Needinfo,
    Team,
    Mirror,
    Revision,
    Landed,
    Regression,
    Asked,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Flag {
    pub name: String,
    pub status: String,
    pub requestee: Option<String>,
    pub setter: String,
    pub creation_date: Timestamp,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Comment {
    pub author: String,
    pub time: Timestamp,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Thread {
    pub total: u32,
    pub comments: Vec<Comment>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub key: JiraKey,
    pub summary: String,
    pub status: String,
    pub status_category: StatusCategory,
    pub assignee: Option<String>,
    #[serde(default)]
    pub parent: Option<JiraKey>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    pub updated: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<Timestamp>,
    pub points: Option<f32>,
    pub sprint: Option<Sprint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ideas: Vec<JiraKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusCategory {
    New,
    Indeterminate,
    Done,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sprint {
    pub name: String,
    pub state: String,
    #[serde(default)]
    pub end_date: Option<Timestamp>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Revision {
    pub id: RevId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phid: Option<String>,
    pub title: String,
    pub status: RevStatus,
    pub author: String,
    pub bug: Option<BugId>,
    pub modified: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<Timestamp>,
    pub reviewers: Vec<Reviewer>,
    #[serde(default, rename = "diffPHID")]
    pub diff_phid: Option<String>,
    #[serde(default)]
    pub diff_created: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projects: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RevStatus {
    NeedsReview,
    NeedsRevision,
    Accepted,
    ChangesPlanned,
    Draft,
    Published,
    Abandoned,
    #[serde(other)]
    Other,
}

impl RevStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NeedsReview => "needs-review",
            Self::NeedsRevision => "needs-revision",
            Self::Accepted => "accepted",
            Self::ChangesPlanned => "changes-planned",
            Self::Draft => "draft",
            Self::Published => "published",
            Self::Abandoned => "abandoned",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Reviewer {
    pub phid: String,
    pub status: ReviewerStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewerStatus {
    Added,
    Blocking,
    Accepted,
    AcceptedOlder,
    Rejected,
    RejectedOlder,
    Commented,
    Resigned,
    #[serde(other)]
    Other,
}

impl ReviewerStatus {
    pub const fn owes_review(self) -> bool {
        matches!(
            self,
            Self::Added | Self::Blocking | Self::Commented | Self::Rejected | Self::RejectedOlder
        )
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Blocking => "blocking",
            Self::Accepted => "accepted",
            Self::AcceptedOlder => "accepted-older",
            Self::Rejected => "rejected",
            Self::RejectedOlder => "rejected-older",
            Self::Commented => "commented",
            Self::Resigned => "resigned",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TryPush {
    pub revision: String,
    pub pushed_at: Timestamp,
    pub url: String,
    pub builds: TryResult,
    pub lint: TryResult,
    pub tests: TryResult,
    pub failed: u32,
    pub running: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TryResult {
    Pass,
    Fail,
    Running,
    None,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Call {
    pub at: Timestamp,
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    pub ms: u32,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default, rename = "waitMs")]
    pub wait_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stale {
    pub source: String,
    pub since: Timestamp,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SourceError {
    pub source: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unset: bool,
    #[serde(default, rename = "retryAt", skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<Timestamp>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jira_offsets_without_a_colon_parse() {
        let jira: Timestamp = serde_json::from_str(r#""2026-11-02T19:30:47.363-0500""#).unwrap();
        let utc: Timestamp = serde_json::from_str(r#""2026-11-03T00:30:47.363Z""#).unwrap();
        assert_eq!(jira, utc);
    }

    #[test]
    fn unknown_statuses_dont_fail_the_cache() {
        let s: RevStatus = serde_json::from_str(r#""some-new-status""#).unwrap();
        assert_eq!(s, RevStatus::Other);
    }

    #[test]
    fn map_keys_are_ids() {
        let m: HashMap<BugId, Vec<RevId>> = serde_json::from_str(r#"{"2088250": [338974]}"#).unwrap();
        assert_eq!(m[&BugId(2_088_250)], [RevId(338_974)]);
    }
}
