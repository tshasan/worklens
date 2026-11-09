use std::collections::HashMap;

use jiff::Timestamp;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use worklens_core::raw::{RevStatus, Reviewer, ReviewerStatus, Revision};
use worklens_core::{BugId, RevId};

use crate::http::{Error, Http, Result, parse};

pub struct Phab<'a> {
    pub http: &'a Http,
    pub host: &'a str,
    pub token: &'a str,
}

#[derive(Deserialize)]
pub struct WhoAmI {
    pub phid: String,
    #[serde(rename = "primaryEmail")]
    pub primary_email: String,
}

type Params = Vec<(String, String)>;

fn list(name: &str, xs: &[impl ToString]) -> Params {
    xs.iter()
        .enumerate()
        .map(|(i, x)| (format!("constraints[{name}][{i}]"), x.to_string()))
        .collect()
}

const OPEN: [&str; 5] = ["needs-review", "needs-revision", "accepted", "changes-planned", "draft"];

impl Phab<'_> {
    async fn call<T: DeserializeOwned>(&self, method: &'static str, params: &[(String, String)]) -> Result<T> {
        let mut form: Vec<(&str, &str)> = Vec::with_capacity(params.len() + 1);
        form.push(("api.token", self.token));
        form.extend(params.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        let req = self.http.client.post(format!("{}{method}", self.host)).form(&form);
        conduit_result(method, &self.http.send(req).await?)
    }

    async fn search(&self, mut constraints: Params) -> Result<Vec<Revision>> {
        #[derive(Deserialize)]
        struct Page {
            data: Vec<PhabRevision>,
            cursor: Cursor,
        }
        #[derive(Deserialize)]
        struct Cursor {
            after: Option<String>,
        }
        constraints.push(("attachments[reviewers]".into(), "1".into()));
        constraints.push(("attachments[projects]".into(), "1".into()));
        let mut out = Vec::new();
        loop {
            let page: Page = self.call("differential.revision.search", &constraints).await?;
            out.extend(page.data.into_iter().map(normalize));
            let Some(after) = page.cursor.after else { return Ok(out) };
            constraints.retain(|(k, _)| k != "after");
            constraints.push(("after".into(), after));
        }
    }

    pub async fn whoami(&self) -> Result<WhoAmI> {
        self.call("user.whoami", &[]).await
    }

    pub async fn mine(&self, phid: &str) -> Result<Vec<Revision>> {
        let mut c = list("authorPHIDs", &[phid]);
        c.extend(list("statuses", &OPEN));
        self.search(c).await
    }

    pub async fn to_review(&self, phid: &str) -> Result<Vec<Revision>> {
        let mut c = list("reviewerPHIDs", &[phid]);
        c.extend(list("statuses", &["needs-review"]));
        let mut revs = self.search(c).await?;
        revs.retain(|r| r.author != phid && r.reviewers.iter().any(|x| x.phid == phid && x.status.owes_review()));
        Ok(revs)
    }

    pub async fn group_reviews(&self, phid: &str) -> Result<Vec<(Revision, String)>> {
        #[derive(Deserialize)]
        struct Page {
            data: Vec<Project>,
        }
        #[derive(Deserialize)]
        struct Project {
            phid: String,
        }
        let projects: Page = self.call("project.search", &list("members", &[phid])).await?;
        let groups: Vec<String> = projects.data.into_iter().map(|p| p.phid).collect();
        if groups.is_empty() {
            return Ok(Vec::new());
        }
        let mut c = list("reviewerPHIDs", &groups);
        c.extend(list("statuses", &["needs-review"]));
        let revs = self.search(c).await?;
        Ok(revs
            .into_iter()
            .filter(|r| r.author != phid && !r.reviewers.iter().any(|x| x.phid == phid))
            .filter_map(|r| {
                let group = r
                    .reviewers
                    .iter()
                    .find(|x| groups.contains(&x.phid) && matches!(x.status, ReviewerStatus::Added | ReviewerStatus::Blocking))?
                    .phid
                    .clone();
                Some((r, group))
            })
            .collect())
    }

    pub async fn diff_dates(&self, phids: &[&str]) -> Result<HashMap<String, Timestamp>> {
        #[derive(Deserialize)]
        struct Page {
            data: Vec<Diff>,
        }
        #[derive(Deserialize)]
        struct Diff {
            phid: String,
            fields: DiffFields,
        }
        #[derive(Deserialize)]
        struct DiffFields {
            #[serde(rename = "dateCreated")]
            date_created: i64,
        }
        let mut out = HashMap::with_capacity(phids.len());
        for part in phids.chunks(100) {
            let page: Page = self.call("differential.diff.search", &list("phids", part)).await?;
            for d in page.data {
                if let Ok(t) = Timestamp::from_second(d.fields.date_created) {
                    out.insert(d.phid, t);
                }
            }
        }
        Ok(out)
    }

    pub async fn diffs(&self, phids: &[&str]) -> Result<HashMap<String, (u32, Timestamp)>> {
        let mut out = HashMap::with_capacity(phids.len());
        for part in phids.chunks(100) {
            let bytes: DiffPage = self.call("differential.diff.search", &list("phids", part)).await?;
            out.extend(diff_ids(bytes));
        }
        Ok(out)
    }

    pub async fn raw_diff(&self, id: u32) -> Result<String> {
        self.call("differential.getrawdiff", &[("diffID".to_owned(), id.to_string())]).await
    }

    pub async fn names(&self, phids: &[&str]) -> Result<HashMap<String, String>> {
        let mut out = HashMap::with_capacity(phids.len());
        for part in phids.chunks(100) {
            let params: Params = part
                .iter()
                .enumerate()
                .map(|(i, p)| (format!("phids[{i}]"), (*p).to_owned()))
                .collect();
            let res: HashMap<String, PhidInfo> = self.call("phid.query", &params).await?;
            out.extend(res.into_iter().map(|(phid, info)| (phid, info.display_name())));
        }
        Ok(out)
    }

    pub async fn availability(&self, phids: &[&str]) -> Result<HashMap<String, Timestamp>> {
        let mut out = HashMap::new();
        for part in phids.chunks(100) {
            let mut params = list("phids", part);
            params.push(("attachments[availability]".into(), "1".into()));
            let page: UserPage = self.call("user.search", &params).await?;
            out.extend(away_of(page));
        }
        Ok(out)
    }

    pub async fn by_ids(&self, ids: &[RevId]) -> Result<Vec<Revision>> {
        let mut out = Vec::with_capacity(ids.len());
        for part in ids.chunks(100) {
            let numbers: Vec<u32> = part.iter().map(|r| r.0).collect();
            out.extend(self.search(list("ids", &numbers)).await?);
        }
        Ok(out)
    }

    pub async fn by_phids(&self, phids: &[String]) -> Result<Vec<Revision>> {
        let mut out = Vec::new();
        for part in phids.chunks(100) {
            out.extend(self.search(list("phids", part)).await?);
        }
        Ok(out)
    }

    pub async fn parents(&self, phids: &[String]) -> Result<HashMap<String, Vec<String>>> {
        let mut out: HashMap<String, Vec<String>> = phids.iter().map(|p| (p.clone(), Vec::new())).collect();
        for part in phids.chunks(100) {
            let mut params: Params = part
                .iter()
                .enumerate()
                .map(|(i, p)| (format!("sourcePHIDs[{i}]"), p.clone()))
                .collect();
            params.push(("types[0]".into(), "revision.parent".into()));
            loop {
                let page: EdgePage = self.call("edge.search", &params).await?;
                add_parents(&mut out, page.data);
                let Some(after) = page.cursor.after else { break };
                params.retain(|(k, _)| k != "after");
                params.push(("after".into(), after));
            }
        }
        Ok(out)
    }
}

#[derive(Deserialize)]
pub struct DiffPage {
    data: Vec<DiffRow>,
}

#[derive(Deserialize)]
struct DiffRow {
    id: u32,
    phid: String,
    fields: DiffRowFields,
}

#[derive(Deserialize)]
struct DiffRowFields {
    #[serde(rename = "dateCreated", default)]
    date_created: Option<i64>,
}

fn diff_ids(page: DiffPage) -> impl Iterator<Item = (String, (u32, Timestamp))> {
    page.data
        .into_iter()
        .filter_map(|d| Some((d.phid, (d.id, Timestamp::from_second(d.fields.date_created?).ok()?))))
}

pub fn parse_diffs(bytes: &[u8]) -> Result<HashMap<String, (u32, Timestamp)>> {
    Ok(diff_ids(conduit_result("differential.diff.search", bytes)?).collect())
}

pub fn parse_raw_diff(bytes: &[u8]) -> Result<String> {
    conduit_result("differential.getrawdiff", bytes)
}

#[derive(Deserialize)]
pub struct UserPage {
    data: Vec<User>,
}

#[derive(Deserialize)]
struct User {
    phid: String,
    #[serde(default)]
    attachments: UserAttachments,
}

#[derive(Deserialize, Default)]
struct UserAttachments {
    availability: Option<Availability>,
}

#[derive(Deserialize)]
struct Availability {
    value: Option<String>,
    until: Option<i64>,
}

fn away_of(page: UserPage) -> impl Iterator<Item = (String, Timestamp)> {
    page.data.into_iter().filter_map(|u| {
        let a = u.attachments.availability?;
        let until = Timestamp::from_second(a.until?).ok()?;
        (a.value.as_deref() == Some("away")).then_some((u.phid, until))
    })
}

pub fn parse_availability(bytes: &[u8]) -> Result<HashMap<String, Timestamp>> {
    let page: UserPage = conduit_result("user.search", bytes)?;
    Ok(away_of(page).collect())
}

#[derive(Deserialize)]
struct EdgePage {
    data: Vec<Edge>,
    cursor: EdgeCursor,
}

#[derive(Deserialize)]
struct EdgeCursor {
    after: Option<String>,
}

#[derive(Deserialize)]
struct Edge {
    #[serde(rename = "sourcePHID")]
    source: String,
    #[serde(rename = "destinationPHID")]
    destination: String,
    #[serde(rename = "edgeType")]
    kind: String,
}

fn add_parents(out: &mut HashMap<String, Vec<String>>, edges: Vec<Edge>) {
    for edge in edges {
        if edge.kind == "revision.parent"
            && let Some(parents) = out.get_mut(&edge.source)
            && !parents.contains(&edge.destination)
        {
            parents.push(edge.destination);
            parents.sort();
        }
    }
}

pub fn parse_parents(bytes: &[u8], sources: &[String]) -> Result<HashMap<String, Vec<String>>> {
    let page: EdgePage = conduit_result("edge.search", bytes)?;
    let mut out = sources.iter().map(|p| (p.clone(), Vec::new())).collect();
    add_parents(&mut out, page.data);
    Ok(out)
}

#[derive(Deserialize)]
pub struct PhidInfo {
    name: String,
    #[serde(rename = "type")]
    kind: String,
}

impl PhidInfo {
    fn display_name(self) -> String {
        if self.kind == "PROJ" && !self.name.starts_with('#') {
            format!("#{}", self.name)
        } else {
            self.name
        }
    }
}

pub fn conduit_result<T: DeserializeOwned>(method: &'static str, bytes: &[u8]) -> Result<T> {
    #[derive(Deserialize)]
    struct Response<T> {
        result: Option<T>,
        error_code: Option<String>,
        error_info: Option<String>,
    }
    let res: Response<T> = parse(method, bytes)?;
    match (res.error_code, res.result) {
        (None, Some(result)) => Ok(result),
        (code, _) => Err(Error::Api(format!(
            "{method}: {}",
            res.error_info.or(code).unwrap_or_else(|| "no result".into())
        ))),
    }
}

#[derive(Deserialize)]
pub struct PhabRevision {
    id: u32,
    #[serde(default)]
    phid: Option<String>,
    fields: Fields,
    #[serde(default)]
    attachments: Attachments,
}

#[derive(Deserialize)]
struct Fields {
    title: String,
    #[serde(rename = "isDraft", default)]
    is_draft: bool,
    status: Status,
    #[serde(rename = "authorPHID")]
    author_phid: String,
    #[serde(rename = "bugzilla.bug-id")]
    bug: Option<String>,
    #[serde(rename = "dateModified")]
    date_modified: i64,
    #[serde(rename = "dateCreated", default)]
    date_created: Option<i64>,
    #[serde(rename = "diffPHID")]
    diff_phid: Option<String>,
}

#[derive(Deserialize)]
struct Status {
    value: RevStatus,
}

#[derive(Deserialize, Default)]
struct Attachments {
    reviewers: Option<ReviewerList>,
    projects: Option<ProjectList>,
}

#[derive(Deserialize)]
struct ProjectList {
    #[serde(rename = "projectPHIDs")]
    project_phids: Vec<String>,
}

#[derive(Deserialize)]
struct ReviewerList {
    reviewers: Vec<PhabReviewer>,
}

#[derive(Deserialize)]
struct PhabReviewer {
    #[serde(rename = "reviewerPHID")]
    phid: String,
    status: ReviewerStatus,
}

fn normalize(d: PhabRevision) -> Revision {
    let f = d.fields;
    Revision {
        id: RevId(d.id),
        phid: d.phid,
        title: f.title,
        status: if f.is_draft { RevStatus::Draft } else { f.status.value },
        author: f.author_phid,
        bug: f.bug.and_then(|b| b.parse().ok()).filter(|n| *n > 0).map(BugId),
        modified: Timestamp::from_second(f.date_modified).unwrap_or_default(),
        created: f.date_created.and_then(|t| Timestamp::from_second(t).ok()),
        reviewers: d.attachments.reviewers.map_or_else(Vec::new, |r| {
            r.reviewers
                .into_iter()
                .map(|x| Reviewer {
                    phid: x.phid,
                    status: x.status,
                })
                .collect()
        }),
        diff_phid: f.diff_phid,
        diff_created: None,
        projects: d.attachments.projects.map(|p| p.project_phids),
    }
}

pub fn parse_revisions(bytes: &[u8]) -> Result<Vec<Revision>> {
    #[derive(Deserialize)]
    struct Page {
        data: Vec<PhabRevision>,
    }
    Ok(conduit_result::<Page>("differential.revision.search", bytes)?
        .data
        .into_iter()
        .map(normalize)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{}/fixtures/phabricator/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    #[test]
    fn recorded_availability_and_source_derived_away_states_parse() {
        let bytes = include_bytes!("../fixtures/phabricator/availability-recorded.json");
        assert!(parse_availability(bytes).unwrap().is_empty());
        let mut response: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        response["result"]["data"][0]["attachments"]["availability"]["value"] = serde_json::json!("away");
        response["result"]["data"][0]["attachments"]["availability"]["until"] = serde_json::json!(1795003200i64);
        let parsed = parse_availability(&serde_json::to_vec(&response).unwrap()).unwrap();
        assert_eq!(parsed["PHID-USER-fixture"].as_second(), 1795003200);
        response["result"]["data"][0]["attachments"]["availability"]["value"] = serde_json::json!("busy");
        assert!(parse_availability(&serde_json::to_vec(&response).unwrap()).unwrap().is_empty());
        assert!(parse_availability(br#"{"error_code":"ERR-INVALID-AUTH","error_info":"failed","result":null}"#).is_err());
    }

    #[test]
    fn revisions_normalize() {
        let revs = parse_revisions(&fixture("revision.search.json")).unwrap();
        assert_eq!(revs.len(), 2);
        let r = &revs[0];
        assert_eq!(r.phid.as_deref(), Some("PHID-DREV-1"));
        assert_eq!(
            (r.id, r.status, r.bug),
            (RevId(336_313), RevStatus::NeedsReview, Some(BugId(2_074_080)))
        );
        assert_eq!(r.modified.to_string(), "2025-11-02T00:00:00Z");
        assert_eq!(r.created.map(|t| t.to_string()).as_deref(), Some("2025-11-01T00:00:00Z"));
        assert_eq!(revs[1].created, None, "a revision without dateCreated still parses");
        assert_eq!(r.reviewers[0].status, ReviewerStatus::Blocking);
        assert_eq!(revs[1].status, RevStatus::Draft, "isDraft wins over the status");
        assert_eq!(revs[1].bug, None, "an empty bug id is no bug");
    }

    #[test]
    fn parent_edges_are_directional_and_missing_parents_are_not_invented() {
        let bytes = include_bytes!("../fixtures/phabricator/edge.search.json");
        let sources = vec!["PHID-DREV-1".into(), "PHID-DREV-2".into()];
        let parents = parse_parents(bytes, &sources).unwrap();
        assert_eq!(parents["PHID-DREV-1"], ["PHID-DREV-parent"]);
        assert!(parents["PHID-DREV-2"].is_empty());
        assert_eq!(parents.len(), 2);
    }

    #[test]
    fn diff_ids_and_dates_parse_and_undated_diffs_are_dropped() {
        let diffs = parse_diffs(&fixture("diff.search.json")).unwrap();
        assert_eq!(diffs.len(), 1);
        let (id, at) = diffs["PHID-DIFF-latest"];
        assert_eq!((id, at.as_second()), (1_500_001, 1_793_995_200));
        assert!(parse_diffs(br#"{"result":null,"error_code":"ERR-CONDUIT-CORE","error_info":"no"}"#).is_err());
    }

    #[test]
    fn raw_diffs_are_conduit_strings() {
        let text = parse_raw_diff(&fixture("getrawdiff.json")).unwrap();
        assert!(text.starts_with("diff --git a/toolkit/components/ml/a.txt"));
        assert!(parse_raw_diff(br#"{"result":{"not":"a string"},"error_code":null,"error_info":null}"#).is_err());
    }

    #[test]
    fn conduit_errors_are_errors() {
        let err = conduit_result::<serde_json::Value>(
            "user.whoami",
            br#"{"result":null,"error_code":"ERR-INVALID-AUTH","error_info":"API token is invalid."}"#,
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "user.whoami: API token is invalid.");
    }

    #[test]
    fn project_attachments_distinguish_missing_from_empty() {
        let mut page: serde_json::Value = serde_json::from_slice(&fixture("revision.search.json")).unwrap();
        page["result"]["data"][0]["attachments"]["projects"] = serde_json::json!({"projectPHIDs":["PHID-PROJ-data"]});
        page["result"]["data"][1]["attachments"]["projects"] = serde_json::json!({"projectPHIDs":[]});
        let revisions = parse_revisions(&serde_json::to_vec(&page).unwrap()).unwrap();
        assert_eq!(revisions[0].projects.as_ref().unwrap(), &["PHID-PROJ-data"]);
        assert!(revisions[1].projects.as_ref().unwrap().is_empty());
        assert!(parse_revisions(&fixture("revision.search.json")).unwrap()[0].projects.is_none());
    }

    #[test]
    fn projects_are_shown_as_groups() {
        let res: HashMap<String, PhidInfo> = conduit_result("phid.query", &fixture("phid.query.json")).unwrap();
        let names: HashMap<_, _> = res.into_iter().map(|(k, v)| (k, v.display_name())).collect();
        assert_eq!(names["PHID-PROJ-1"], "#ai-platform-reviewers");
        assert_eq!(names["PHID-USER-1"], "quillfox");
    }
}
