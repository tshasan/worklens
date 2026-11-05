use std::collections::HashMap;

use futures::{StreamExt, TryStreamExt, stream};
use jiff::{Timestamp, ToSpan};
use reqwest::RequestBuilder;
use serde::{Deserialize, Serialize};
use worklens_core::raw::{Bug, Comment};
use worklens_core::{BugId, RevId};

use crate::MAX_IN_FLIGHT;
use crate::http::{Http, Result, parse};

pub const BZ: &str = "https://bugzilla.mozilla.org";
const FIELDS: &str = "id,summary,status,resolution,assigned_to,severity,priority,keywords,whiteboard,see_also,last_change_time,creation_time,flags,depends_on,regressed_by,cf_crash_signature,cf_last_resolved,product,component";

pub struct Bugzilla<'a> {
    pub http: &'a Http,
    pub api_key: Option<&'a str>,
    pub release_fields: &'a str,
}

pub fn release_fields(trains: Option<&worklens_core::raw::Trains>) -> String {
    let Some(t) = trains else { return String::new() };
    let mut out = String::new();
    for v in [t.nightly, t.beta, t.release] {
        out.push_str(&format!(",cf_status_firefox{v},cf_tracking_firefox{v}"));
    }
    for v in &t.esr {
        out.push_str(&format!(",cf_status_firefox_esr{v},cf_tracking_firefox_esr{v}"));
    }
    out
}

impl Bugzilla<'_> {
    fn get(&self, path: &str) -> RequestBuilder {
        let req = self.http.client.get(format!("{BZ}{path}"));
        match self.api_key {
            Some(key) => req.header("X-BUGZILLA-API-KEY", key),
            None => req,
        }
    }

    async fn search<P: Serialize + ?Sized>(&self, params: &P) -> Result<Vec<Bug>> {
        let fields = format!("{FIELDS}{}", self.release_fields);
        let req = self
            .get("/rest/bug")
            .query(params)
            .query(&[("include_fields", fields.as_str()), ("limit", "0")]);
        parse_bugs(&self.http.send(req).await?)
    }

    pub async fn assigned_to(&self, email: &str) -> Result<Vec<Bug>> {
        self.search(&[("assigned_to", email), ("resolution", "---")]).await
    }

    pub async fn needinfo_on(&self, email: &str) -> Result<Vec<Bug>> {
        self.search(&[
            ("f1", "requestees.login_name"),
            ("o1", "equals"),
            ("v1", email),
            ("f2", "flagtypes.name"),
            ("o2", "equals"),
            ("v2", "needinfo?"),
        ])
        .await
    }

    pub async fn team(&self, whiteboard: &str, days: i64) -> Result<Vec<Bug>> {
        let since = (Timestamp::now() - (days * 24).hours()).strftime("%Y-%m-%d").to_string();
        let wb = [("status_whiteboard", whiteboard), ("status_whiteboard_type", "allwordssubstr")];
        let open_params = [wb[0], wb[1], ("resolution", "---")];
        let recent_params = [
            wb[0],
            wb[1],
            ("bug_status", "RESOLVED"),
            ("bug_status", "VERIFIED"),
            ("chfield", "bug_status"),
            ("chfieldfrom", &since),
        ];
        let (mut open, recent) = futures::try_join!(self.search(&open_params), self.search(&recent_params))?;
        open.extend(recent);
        Ok(open)
    }

    pub async fn fixed_by(&self, email: &str, days: i64) -> Result<Vec<Bug>> {
        let since = (Timestamp::now() - (days * 24).hours()).strftime("%Y-%m-%d").to_string();
        self.search(&[
            ("assigned_to", email),
            ("resolution", "FIXED"),
            ("chfield", "resolution"),
            ("chfieldfrom", &since),
        ])
        .await
    }

    pub async fn needinfo_from(&self, email: &str) -> Result<Vec<Bug>> {
        self.search(&[
            ("f1", "setters.login_name"),
            ("o1", "equals"),
            ("v1", email),
            ("f2", "flagtypes.name"),
            ("o2", "equals"),
            ("v2", "needinfo?"),
        ])
        .await
    }

    pub async fn regressions_of(&self, ids: &[BugId]) -> Result<Vec<Bug>> {
        let mut out = Vec::new();
        for part in ids.chunks(100) {
            let list: Vec<String> = part.iter().map(ToString::to_string).collect();
            let list = list.join(",");
            out.extend(
                self.search(&[
                    ("f1", "regressed_by"),
                    ("o1", "anyexact"),
                    ("v1", list.as_str()),
                    ("resolution", "---"),
                ])
                .await?,
            );
        }
        Ok(out)
    }

    pub async fn uplift(&self, ids: &[BugId]) -> Result<HashMap<BugId, worklens_core::raw::Uplift>> {
        let mut out = HashMap::with_capacity(ids.len());
        for part in ids.chunks(100) {
            let Some((first, rest)) = part.split_first() else { continue };
            let mut params: Vec<(&str, String)> = rest.iter().map(|id| ("ids", id.to_string())).collect();
            params.push(("include_fields", "id,content_type,file_name,is_obsolete,flags".into()));
            let req = self.get(&format!("/rest/bug/{first}/attachment")).query(&params);
            out.extend(parse_uplift(&self.http.send(req).await?)?);
        }
        Ok(out)
    }

    pub async fn mirroring(&self, keys: &[&str]) -> Result<Vec<Bug>> {
        let mut out = Vec::new();
        for part in keys.chunks(50) {
            let mut params: Vec<(String, String)> = Vec::with_capacity(part.len() * 3 + 3);
            params.push(("f1".into(), "OP".into()));
            params.push(("j1".into(), "OR".into()));
            for (i, key) in part.iter().enumerate() {
                let n = i + 2;
                params.push((format!("f{n}"), "see_also".into()));
                params.push((format!("o{n}"), "substring".into()));
                params.push((format!("v{n}"), format!("/browse/{key}")));
            }
            params.push((format!("f{}", part.len() + 2), "CP".into()));
            out.extend(self.search(&params).await?);
        }
        Ok(out)
    }

    pub async fn by_ids(&self, ids: &[BugId]) -> Result<Vec<Bug>> {
        let mut out = Vec::with_capacity(ids.len());
        for part in ids.chunks(200) {
            let list: Vec<String> = part.iter().map(ToString::to_string).collect();
            out.extend(self.search(&[("id", list.join(","))]).await?);
        }
        Ok(out)
    }

    pub async fn comments(&self, ids: &[BugId]) -> Result<HashMap<BugId, Vec<Comment>>> {
        let pages: Vec<Vec<(BugId, Vec<Comment>)>> = stream::iter(ids.chunks(10))
            .map(|part| async move {
                let (first, rest) = part.split_first().expect("chunks are never empty");
                let more: Vec<(&str, String)> = rest.iter().map(|id| ("ids", id.to_string())).collect();
                let req = self.get(&format!("/rest/bug/{first}/comment")).query(&more);
                parse_comments(&self.http.send(req).await?)
            })
            .buffer_unordered(MAX_IN_FLIGHT)
            .try_collect()
            .await?;
        Ok(pages.into_iter().flatten().collect())
    }

    pub async fn revision_attachments(&self, ids: &[BugId]) -> Result<HashMap<BugId, Vec<RevId>>> {
        let mut out = HashMap::new();
        for part in ids.chunks(100) {
            let Some((first, rest)) = part.split_first() else { continue };
            let mut params: Vec<(&str, String)> = rest.iter().map(|id| ("ids", id.to_string())).collect();
            params.push(("include_fields", "content_type,file_name,is_obsolete,creator".into()));
            let req = self.get(&format!("/rest/bug/{first}/attachment")).query(&params);
            out.extend(parse_attachments(&self.http.send(req).await?)?);
        }
        Ok(out)
    }
}

pub fn parse_bugs(bytes: &[u8]) -> Result<Vec<Bug>> {
    #[derive(Deserialize)]
    struct Page {
        bugs: Vec<BzBug>,
    }
    #[derive(Deserialize)]
    struct BzBug {
        #[serde(flatten)]
        bug: Bug,
        #[serde(flatten)]
        rest: HashMap<String, serde_json::Value>,
    }
    let page: Page = parse("Bugzilla bugs", bytes)?;
    Ok(page
        .bugs
        .into_iter()
        .map(|BzBug { mut bug, rest }| {
            bug.release_flags = rest
                .into_iter()
                .filter(|(k, _)| k.starts_with("cf_status_firefox") || k.starts_with("cf_tracking_firefox"))
                .filter_map(|(k, v)| Some((k, v.as_str()?.to_owned())))
                .collect();
            bug
        })
        .collect())
}

pub fn parse_uplift(bytes: &[u8]) -> Result<Vec<(BugId, worklens_core::raw::Uplift)>> {
    #[derive(Deserialize)]
    struct Page {
        bugs: HashMap<BugId, Vec<Attachment>>,
    }
    #[derive(Deserialize)]
    struct Attachment {
        id: Option<u32>,
        content_type: String,
        is_obsolete: serde_json::Value,
        #[serde(default)]
        flags: Vec<Flag>,
    }
    #[derive(Deserialize)]
    struct Flag {
        name: String,
        status: String,
    }
    let page: Page = parse("Bugzilla attachments", bytes)?;
    Ok(page
        .bugs
        .into_iter()
        .map(|(id, atts)| {
            let live: Vec<&Attachment> = atts
                .iter()
                .filter(|a| a.content_type == "text/x-phabricator-request" && a.is_obsolete != true && a.is_obsolete != 1)
                .collect();
            let approvals = live
                .iter()
                .flat_map(|a| &a.flags)
                .filter_map(|f| Some((f.name.strip_prefix("approval-mozilla-")?.to_owned(), f.status.clone())))
                .collect();
            let flags = atts
                .iter()
                .filter(|a| a.is_obsolete != true && a.is_obsolete != 1)
                .flat_map(|a| {
                    a.flags
                        .iter()
                        .filter(|f| !f.name.starts_with("approval-mozilla-"))
                        .map(|f| worklens_core::raw::AttachmentFlag {
                            attachment: a.id,
                            name: f.name.clone(),
                            status: f.status.clone(),
                        })
                })
                .collect();
            (
                id,
                worklens_core::raw::Uplift {
                    patch: !live.is_empty(),
                    approvals,
                    flags,
                },
            )
        })
        .collect())
}

pub fn parse_comments(bytes: &[u8]) -> Result<Vec<(BugId, Vec<Comment>)>> {
    #[derive(Deserialize)]
    struct Page {
        bugs: HashMap<BugId, Thread>,
    }
    #[derive(Deserialize)]
    struct Thread {
        comments: Vec<BzComment>,
    }
    #[derive(Deserialize)]
    struct BzComment {
        creator: String,
        creation_time: Timestamp,
        text: String,
    }
    let page: Page = parse("Bugzilla comments", bytes)?;
    Ok(page
        .bugs
        .into_iter()
        .map(|(id, t)| {
            (
                id,
                t.comments
                    .into_iter()
                    .map(|c| Comment {
                        author: c.creator,
                        time: c.creation_time,
                        text: c.text,
                    })
                    .collect(),
            )
        })
        .collect())
}

pub fn parse_attachments(bytes: &[u8]) -> Result<Vec<(BugId, Vec<RevId>)>> {
    #[derive(Deserialize)]
    struct Page {
        bugs: HashMap<BugId, Vec<Attachment>>,
    }
    #[derive(Deserialize)]
    struct Attachment {
        content_type: String,
        file_name: String,
        is_obsolete: serde_json::Value,
    }
    let page: Page = parse("Bugzilla attachments", bytes)?;
    Ok(page
        .bugs
        .into_iter()
        .filter_map(|(id, atts)| {
            let revs: Vec<RevId> = atts
                .iter()
                .filter(|a| a.content_type == "text/x-phabricator-request" && a.is_obsolete != true && a.is_obsolete != 1)
                .filter_map(|a| d_number(&a.file_name))
                .collect();
            (!revs.is_empty()).then_some((id, revs))
        })
        .collect())
}

fn d_number(text: &str) -> Option<RevId> {
    text.match_indices('D').find_map(|(at, _)| {
        let digits: &str = &text[at + 1..];
        let end = digits.bytes().take_while(u8::is_ascii_digit).count();
        digits[..end].parse().ok().map(RevId)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/bugzilla");

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("{FIXTURES}/{name}")).unwrap()
    }

    #[test]
    fn bugs_parse_with_no_via_and_extra_fields() {
        let bugs = parse_bugs(&fixture("bugs.json")).unwrap();
        assert_eq!(bugs.len(), 2);
        assert_eq!(bugs[0].id, BugId(2_086_888));
        assert!(bugs[0].via.is_empty());
        assert_eq!(
            bugs[0].creation_time.map(|t| t.to_string()).as_deref(),
            Some("2026-10-28T12:00:00Z")
        );
        assert_eq!(bugs[1].creation_time, None, "a bug without creation_time still parses");
        assert_eq!(bugs[0].flags[0].requestee.as_deref(), Some("me@mozilla.com"));
    }

    #[test]
    fn comments_are_keyed_by_bug() {
        let threads: HashMap<_, _> = parse_comments(&fixture("comments.json")).unwrap().into_iter().collect();
        assert_eq!(threads[&BugId(2_086_888)].len(), 2);
        assert_eq!(threads[&BugId(2_086_888)][1].author, "reviewer@example.org");
    }

    #[test]
    fn only_live_phabricator_attachments_count() {
        let atts: HashMap<_, _> = parse_attachments(&fixture("attachments.json")).unwrap().into_iter().collect();
        assert_eq!(
            atts[&BugId(2_057_035)],
            [RevId(335_404)],
            "obsolete (true and 1) and non-Phabricator attachments are skipped"
        );
        assert!(!atts.contains_key(&BugId(1)), "a bug with none is left out");
    }

    #[test]
    fn release_flags_are_kept_and_other_extra_fields_dropped() {
        let bugs = parse_bugs(br#"{"bugs":[{"id":1,"summary":"s","status":"RESOLVED","resolution":"FIXED","assigned_to":"me","severity":"S3","priority":"P3","keywords":[],"whiteboard":"","see_also":[],"last_change_time":"2026-11-01T00:00:00Z","product":"Core","component":"ML","cf_status_firefox161":"fixed","cf_status_firefox_esr153":"affected","cf_tracking_firefox160":"+","assigned_to_detail":{"email":"me"}}]}"#).unwrap();
        assert_eq!(bugs[0].status_for("161"), Some("fixed"));
        assert_eq!(bugs[0].status_for("_esr153"), Some("affected"));
        assert_eq!(bugs[0].tracking_for("160"), Some("+"));
        assert_eq!(bugs[0].release_flags.len(), 3);
    }

    #[test]
    fn approval_flags_come_from_live_patches() {
        let up: HashMap<_, _> = parse_uplift(br#"{"bugs":{"5":[{"content_type":"text/x-phabricator-request","file_name":"phabricator-D1-url.txt","is_obsolete":0,"flags":[{"name":"approval-mozilla-beta","status":"?"}]},{"content_type":"text/x-phabricator-request","file_name":"phabricator-D2-url.txt","is_obsolete":1,"flags":[{"name":"approval-mozilla-release","status":"+"}]}],"6":[]}}"#).unwrap().into_iter().collect();
        assert!(up[&BugId(5)].patch);
        assert_eq!(up[&BugId(5)].approval("beta"), Some("?"));
        assert_eq!(up[&BugId(5)].approval("release"), None, "flags on obsolete patches don't count");
        assert!(!up[&BugId(6)].patch);
    }

    #[test]
    fn d_numbers_in_file_names() {
        assert_eq!(d_number("phabricator-D335404-url.txt"), Some(RevId(335_404)));
        assert_eq!(d_number("Data.txt"), None);
    }

    #[test]
    fn specialist_flags_keep_the_attachment_target_and_exclude_obsolete() {
        let up: HashMap<_, _> = parse_uplift(br#"{"bugs":{"5":[{"id":123,"content_type":"text/plain","is_obsolete":0,"flags":[{"name":"sec-approval","status":"?"}]},{"id":124,"content_type":"text/plain","is_obsolete":true,"flags":[{"name":"sec-approval","status":"+"}]}]}}"#).unwrap().into_iter().collect();
        assert_eq!(up[&BugId(5)].flags.len(), 1);
        assert_eq!(up[&BugId(5)].flags[0].attachment, Some(123));
        assert_eq!(up[&BugId(5)].flags[0].status, "?");
    }
}
