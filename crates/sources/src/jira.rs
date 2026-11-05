use std::collections::HashMap;

use futures::{StreamExt, stream};
use indexmap::IndexMap;
use jiff::Timestamp;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use worklens_core::JiraKey;
use worklens_core::raw::{Idea, Issue, Roadmap, Sprint, StatusCategory};

use crate::MAX_IN_FLIGHT;
use crate::http::{Http, Result, parse};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JiraAuth {
    Classic { email: String },
    Scoped { email: String },
    Bearer,
}

impl JiraAuth {
    pub fn name(&self) -> &'static str {
        match self {
            JiraAuth::Classic { .. } => "classic",
            JiraAuth::Scoped { .. } => "scoped",
            JiraAuth::Bearer => "bearer",
        }
    }
}

#[derive(Clone)]
pub struct JiraConfig {
    pub site: String,
    pub token: String,
    pub auth: JiraAuth,
    pub points_field: String,
    pub sprint_field: String,
}

pub struct Jira<'a> {
    http: &'a Http,
    cfg: &'a JiraConfig,
    base: String,
}

impl<'a> Jira<'a> {
    pub async fn connect(http: &'a Http, cfg: &'a JiraConfig) -> Result<Self> {
        #[derive(Deserialize)]
        struct Tenant {
            #[serde(rename = "cloudId")]
            cloud_id: String,
        }
        let site = cfg.site.trim_end_matches('/');
        let base = if let JiraAuth::Classic { .. } = cfg.auth {
            format!("{site}/rest/api/3")
        } else {
            let tenant: Tenant = parse(
                "Jira tenant_info",
                &http.send(http.client.get(format!("{site}/_edge/tenant_info"))).await?,
            )?;
            format!("https://api.atlassian.com/ex/jira/{}/rest/api/3", tenant.cloud_id)
        };
        Ok(Self { http, cfg, base })
    }

    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.cfg.auth {
            JiraAuth::Classic { email } | JiraAuth::Scoped { email } => req.basic_auth(email, Some(&self.cfg.token)),
            JiraAuth::Bearer => req.bearer_auth(&self.cfg.token),
        }
    }

    fn fields(&self) -> [&str; 11] {
        [
            "summary",
            "issuelinks",
            "status",
            "assignee",
            "parent",
            "issuetype",
            "priority",
            "updated",
            "created",
            &self.cfg.points_field,
            &self.cfg.sprint_field,
        ]
    }

    pub async fn me(&self) -> Result<String> {
        #[derive(Deserialize)]
        struct Me {
            #[serde(rename = "emailAddress")]
            email: String,
        }
        let req = self
            .authed(self.http.client.get(format!("{}/myself", self.base)))
            .header("Accept", "application/json");
        Ok(parse::<Me>("Jira myself", &self.http.send(req).await?)?.email)
    }

    pub async fn discover_fields(&self) -> Result<FieldIds> {
        let req = self
            .authed(self.http.client.get(format!("{}/field", self.base)))
            .header("Accept", "application/json");
        parse_fields(&self.http.send(req).await?)
    }

    pub async fn search(&self, jql: &str) -> Result<Vec<Issue>> {
        let issues: Vec<JiraIssue> = self.pages(jql, &self.fields()).await?;
        Ok(issues.into_iter().map(|i| normalize(i, self.cfg)).collect())
    }

    async fn pages<T: DeserializeOwned>(&self, jql: &str, fields: &[&str]) -> Result<Vec<T>> {
        #[derive(Deserialize)]
        struct Page<T> {
            issues: Vec<T>,
            #[serde(rename = "nextPageToken")]
            next: Option<String>,
        }
        let mut out = Vec::new();
        let mut next: Option<String> = None;
        loop {
            let mut body = json!({ "jql": jql, "fields": fields, "maxResults": 100 });
            if let Some(token) = &next {
                body["nextPageToken"] = Value::from(token.as_str());
            }
            let req = self
                .authed(self.http.client.post(format!("{}/search/jql", self.base)))
                .header("Accept", "application/json")
                .json(&body);
            let page: Page<T> = parse("Jira search", &self.http.send(req).await?)?;
            out.extend(page.issues);
            match page.next {
                Some(token) => next = Some(token),
                None => return Ok(out),
            }
        }
    }

    pub async fn by_keys(&self, keys: &[&str]) -> Result<Vec<(String, Issue)>> {
        let mut out: Vec<(String, Issue)> = Vec::with_capacity(keys.len());
        for part in keys.chunks(100) {
            for issue in self.search(&format!("key in ({})", part.join(","))).await? {
                out.push((issue.key.0.clone(), issue));
            }
        }
        let missing: Vec<&str> = keys.iter().copied().filter(|k| !out.iter().any(|(key, _)| key == k)).collect();
        let fields = self.fields().join(",");
        let mut found: Vec<(String, Issue)> = stream::iter(missing)
            .map(|key| {
                let req = self
                    .authed(self.http.client.get(format!("{}/issue/{key}", self.base)))
                    .query(&[("fields", &fields)]);
                async move {
                    let bytes = self.http.send(req).await.ok()?;
                    let issue = parse::<JiraIssue>("Jira issue", &bytes).ok()?;
                    Some((key.to_owned(), normalize(issue, self.cfg)))
                }
            })
            .buffer_unordered(MAX_IN_FLIGHT)
            .filter_map(std::future::ready)
            .collect()
            .await;
        found.sort_by_key(|(key, _)| keys.iter().position(|k| k == key));
        out.extend(found);
        Ok(out)
    }

    pub async fn roadmap(&self, issues: &IndexMap<JiraKey, Issue>) -> Result<Roadmap> {
        let mut out = Roadmap::default();
        let mut next: Vec<JiraKey> = Vec::new();
        for _ in 0..worklens_core::ideas::MAX_DEPTH {
            next.clear();
            for issue in issues.values().chain(out.parents.values()) {
                if let Some(p) = issue.parent.as_ref()
                    && !issues.contains_key(p)
                    && !out.parents.contains_key(p)
                    && !next.contains(p)
                {
                    next.push(p.clone());
                }
            }
            if next.is_empty() {
                break;
            }
            let keys: Vec<&str> = next.iter().map(|k| k.0.as_str()).collect();
            for (key, issue) in self.by_keys(&keys).await? {
                out.parents.insert(JiraKey(key), issue);
            }
        }
        let mut ideas: Vec<&str> = Vec::new();
        for key in issues.values().chain(out.parents.values()).flat_map(|i| &i.ideas) {
            if !ideas.contains(&key.0.as_str()) {
                ideas.push(&key.0);
            }
        }
        if ideas.is_empty() {
            return Ok(out);
        }
        let ids = self.discover_fields().await?.idea;
        let mut fields = vec!["summary", "status", "assignee", "updated", "project"];
        fields.extend(ids.all());
        for part in ideas.chunks(100) {
            let found: Vec<IdeaIssue> = self.pages(&format!("key in ({})", part.join(",")), &fields).await?;
            for idea in found.into_iter().filter_map(|i| normalize_idea(i, &ids)) {
                out.ideas.insert(idea.key.clone(), idea);
            }
        }
        out.ideas.sort_by_cached_key(|k, _| ideas.iter().position(|x| *x == k.0));
        Ok(out)
    }
}

#[derive(Deserialize)]
pub struct JiraIssue {
    key: String,
    fields: Fields,
}

#[derive(Deserialize)]
struct Fields {
    summary: String,
    status: Status,
    assignee: Option<Person>,
    parent: Option<Parent>,
    issuetype: Option<Named>,
    priority: Option<Named>,
    updated: Timestamp,
    #[serde(default)]
    created: Option<Timestamp>,
    #[serde(default)]
    issuelinks: Vec<IssueLink>,
    #[serde(flatten)]
    custom: HashMap<String, Value>,
}

const DELIVERY_LINK: &str = "Polaris issue link";

#[derive(Deserialize)]
struct IssueLink {
    #[serde(rename = "type")]
    kind: Named,
    #[serde(rename = "outwardIssue")]
    outward: Option<Parent>,
}

#[derive(Deserialize)]
pub struct IdeaIssue {
    key: String,
    fields: IdeaFields,
}

#[derive(Deserialize)]
struct IdeaFields {
    summary: String,
    status: Status,
    assignee: Option<Person>,
    updated: Timestamp,
    project: Option<Project>,
    #[serde(flatten)]
    custom: HashMap<String, Value>,
}

#[derive(Deserialize)]
struct Project {
    #[serde(rename = "projectTypeKey")]
    kind: String,
}

fn field_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => (!s.trim().is_empty()).then(|| s.trim().to_owned()),
        Value::Number(n) => Some(n.to_string()),
        Value::Array(xs) => {
            let parts: Vec<String> = xs.iter().filter_map(field_text).collect();
            (!parts.is_empty()).then(|| parts.join(", "))
        }
        Value::Object(m) => m.get("value").or_else(|| m.get("name")).and_then(field_text),
        _ => None,
    }
}

pub fn normalize_idea(i: IdeaIssue, ids: &IdeaFieldIds) -> Option<Idea> {
    let f = i.fields;
    if f.project.is_some_and(|p| p.kind != "product_discovery") {
        return None;
    }
    let pick = |names: &[String]| names.iter().find_map(|id| f.custom.get(id).and_then(field_text));
    Some(Idea {
        key: JiraKey(i.key),
        summary: f.summary,
        status: f.status.name,
        status_category: f.status.category.key,
        assignee: f.assignee.and_then(|a| a.email),
        updated: f.updated,
        weekly_status: pick(&ids.weekly_status),
        target: pick(&ids.target),
        quarter: pick(&ids.quarter),
        theme: pick(&ids.theme),
        priority: pick(&ids.priority),
        team: pick(&ids.team),
    })
}

#[derive(Deserialize)]
struct Status {
    name: String,
    #[serde(rename = "statusCategory")]
    category: Category,
}

#[derive(Deserialize)]
struct Category {
    key: StatusCategory,
}

#[derive(Deserialize)]
struct Person {
    #[serde(rename = "emailAddress")]
    email: Option<String>,
}

#[derive(Deserialize)]
struct Parent {
    key: String,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

pub fn normalize(i: JiraIssue, cfg: &JiraConfig) -> Issue {
    let mut f = i.fields;
    let points = f.custom.get(&cfg.points_field).and_then(Value::as_f64).map(|p| p as f32);
    let sprints: Vec<Sprint> = f
        .custom
        .remove(&cfg.sprint_field)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    Issue {
        key: JiraKey(i.key),
        summary: f.summary,
        status: f.status.name,
        status_category: f.status.category.key,
        assignee: f.assignee.and_then(|a| a.email),
        parent: f.parent.map(|p| JiraKey(p.key)),
        kind: f.issuetype.map(|t| t.name),
        priority: f.priority.map(|p| p.name),
        updated: f.updated,
        created: f.created,
        points,
        sprint: sprints.into_iter().find(|s| s.state == "active"),
        ideas: f
            .issuelinks
            .into_iter()
            .filter(|l| l.kind.name == DELIVERY_LINK)
            .filter_map(|l| l.outward.map(|o| JiraKey(o.key)))
            .collect(),
    }
}

pub fn parse_search(bytes: &[u8], cfg: &JiraConfig) -> Result<Vec<Issue>> {
    #[derive(Deserialize)]
    struct Page {
        issues: Vec<JiraIssue>,
    }
    Ok(parse::<Page>("Jira search", bytes)?
        .issues
        .into_iter()
        .map(|i| normalize(i, cfg))
        .collect())
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct FieldIds {
    pub story_points: Option<String>,
    pub sprint: Option<String>,
    pub idea: IdeaFieldIds,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct IdeaFieldIds {
    pub weekly_status: Vec<String>,
    pub target: Vec<String>,
    pub quarter: Vec<String>,
    pub theme: Vec<String>,
    pub priority: Vec<String>,
    pub team: Vec<String>,
}

impl IdeaFieldIds {
    fn all(&self) -> impl Iterator<Item = &str> {
        [
            &self.weekly_status,
            &self.target,
            &self.quarter,
            &self.theme,
            &self.priority,
            &self.team,
        ]
        .into_iter()
        .flatten()
        .map(String::as_str)
    }
}

pub fn parse_fields(bytes: &[u8]) -> Result<FieldIds> {
    #[derive(Deserialize)]
    struct Field {
        id: String,
        name: String,
        #[serde(default)]
        schema: Option<Schema>,
    }
    #[derive(Deserialize)]
    struct Schema {
        #[serde(default)]
        custom: Option<String>,
    }
    let fields: Vec<Field> = parse("Jira fields", bytes)?;
    let custom = |f: &Field| f.schema.as_ref().and_then(|s| s.custom.as_deref()).unwrap_or_default().to_owned();
    let named = |names: &[&str]| {
        names
            .iter()
            .find_map(|n| fields.iter().find(|f| f.name.eq_ignore_ascii_case(n)))
            .map(|f| f.id.clone())
    };
    let every = |name: &str| -> Vec<String> {
        fields
            .iter()
            .filter(|f| f.name.eq_ignore_ascii_case(name))
            .map(|f| f.id.clone())
            .collect()
    };
    Ok(FieldIds {
        idea: IdeaFieldIds {
            weekly_status: every("Weekly Status"),
            target: every("Full rollout on release"),
            quarter: every("Release Qtr"),
            theme: every("Strategic Theme"),
            priority: every("Priority Level"),
            team: every("Team"),
        },
        story_points: named(&["Story Points", "Story point estimate"]),
        sprint: fields
            .iter()
            .find(|f| custom(f) == "com.pyxis.greenhopper.jira:gh-sprint")
            .map(|f| f.id.clone())
            .or_else(|| named(&["Sprint"])),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_found_by_name_and_sprint_by_schema() {
        let body = br#"[
            {"id":"summary","name":"Summary"},
            {"id":"customfield_10016","name":"Story point estimate","schema":{"custom":"com.atlassian.jira.plugin.system.customfieldtypes:jsw-story-points"}},
            {"id":"customfield_10099","name":"Iteration","schema":{"custom":"com.pyxis.greenhopper.jira:gh-sprint"}}
        ]"#;
        assert_eq!(
            parse_fields(body).unwrap(),
            FieldIds {
                story_points: Some("customfield_10016".into()),
                sprint: Some("customfield_10099".into()),
                idea: IdeaFieldIds::default(),
            }
        );
        let both = br#"[{"id":"a","name":"Story point estimate"},{"id":"b","name":"Story Points"}]"#;
        assert_eq!(parse_fields(both).unwrap().story_points.as_deref(), Some("b"));
        assert_eq!(parse_fields(b"[]").unwrap(), FieldIds::default());
    }

    #[test]
    fn each_kind_of_token_goes_in_its_own_header() {
        let http = Http::new().unwrap();
        let mut cfg = cfg();
        cfg.token = "tok".into();
        let header = |cfg: &JiraConfig| {
            let jira = Jira {
                http: &http,
                cfg,
                base: String::new(),
            };
            let req = jira.authed(http.client.get("https://example.invalid/")).build().unwrap();
            req.headers()["authorization"].to_str().unwrap().to_owned()
        };
        assert_eq!(header(&cfg), "Bearer tok");
        cfg.auth = JiraAuth::Classic { email: "me@x".into() };
        assert_eq!(header(&cfg), "Basic bWVAeDp0b2s=");
    }

    fn cfg() -> JiraConfig {
        JiraConfig {
            site: "https://mozilla-hub.atlassian.net".into(),
            token: String::new(),
            auth: JiraAuth::Bearer,
            points_field: "customfield_10037".into(),
            sprint_field: "customfield_10020".into(),
        }
    }

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{}/fixtures/jira/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    #[test]
    fn an_epic_lists_the_ideas_it_implements() {
        let epics = parse_search(&fixture("epic.json"), &cfg()).unwrap();
        assert_eq!(epics[0].kind.as_deref(), Some("Epic"));
        assert_eq!(
            epics[0].ideas,
            [JiraKey("AIDI-79".into())],
            "only outward delivery links; not Relates, not what implements the epic"
        );
        let plain = parse_search(&fixture("search.json"), &cfg()).unwrap();
        assert!(plain[0].ideas.is_empty());
    }

    #[test]
    fn ideas_read_their_fields_by_name() {
        let fields = br#"[
            {"id":"customfield_20999","name":"Weekly Status"},
            {"id":"customfield_20295","name":"Weekly Status"},
            {"id":"customfield_20136","name":"Full rollout on release"},
            {"id":"customfield_18971","name":"Release Qtr"},
            {"id":"customfield_18970","name":"Strategic Theme"},
            {"id":"customfield_20066","name":"Priority Level"},
            {"id":"customfield_10001","name":"Team"}
        ]"#;
        let ids = parse_fields(fields).unwrap().idea;
        assert_eq!(ids.weekly_status, ["customfield_20999", "customfield_20295"]);
        #[derive(Deserialize)]
        struct Page {
            issues: Vec<IdeaIssue>,
        }
        let page: Page = serde_json::from_slice(&fixture("ideas.json")).unwrap();
        let ideas: Vec<Idea> = page.issues.into_iter().filter_map(|i| normalize_idea(i, &ids)).collect();
        assert_eq!(ideas.len(), 1, "a software-project theme is not an idea");
        let i = &ideas[0];
        assert_eq!((i.key.0.as_str(), i.status.as_str()), ("AIDI-79", "Developing"));
        assert_eq!(i.weekly_status.as_deref(), Some("Dev on track"), "the first field that has a value");
        assert_eq!(i.target.as_deref(), Some("Fx158.0.1 (Oct 20)"));
        assert_eq!(i.quarter.as_deref(), Some("Q4 | 2026"));
        assert_eq!(i.theme.as_deref(), Some("Example theme"));
        assert_eq!(i.priority.as_deref(), Some("P2"));
        assert_eq!(i.team.as_deref(), Some("Example team"));
        assert_eq!(i.assignee.as_deref(), Some("owner@mozilla.com"));
    }

    #[test]
    fn issues_normalize_with_custom_fields() {
        let bytes = std::fs::read(format!("{}/fixtures/jira/search.json", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let issues = parse_search(&bytes, &cfg()).unwrap();
        let i = &issues[0];
        assert_eq!(i.key.0, "AIPLAT-1416");
        assert_eq!((i.points, i.status_category), (Some(3.0), StatusCategory::Indeterminate));
        assert_eq!(
            i.sprint.as_ref().map(|s| s.name.as_str()),
            Some("162 Core Nov 2 - Nov 16"),
            "the active sprint, not the closed one"
        );
        assert_eq!(i.updated.to_string(), "2026-11-03T00:30:47.363Z");
        assert_eq!(i.created.map(|t| t.to_string()).as_deref(), Some("2026-10-20T14:00:00Z"));
        let unassigned = &issues[1];
        assert_eq!(
            (unassigned.assignee.as_deref(), unassigned.points, unassigned.sprint.is_none()),
            (None, None, true)
        );
    }
}
