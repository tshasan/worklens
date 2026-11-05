use std::collections::{HashMap, HashSet};
use std::future::Future;

use futures::{StreamExt, stream};
use indexmap::IndexMap;
use jiff::Timestamp;
use worklens_core::bugs::jira_key_of;
use worklens_core::raw::{Bug, Comment, Issue, Me, Revision, SourceError, Thread, TryPush, Via};
use worklens_core::{BugId, JiraKey, RAW_VERSION, Raw, RevId};

use crate::MAX_IN_FLIGHT;
use crate::bugzilla::Bugzilla;
use crate::http::{Error, Http, Result};
use crate::jira::{Jira, JiraConfig};
use crate::phab::Phab;
use crate::treeherder::{Treeherder, push_url};

#[derive(Clone)]
pub struct PhabConfig {
    pub host: String,
    pub token: String,
}

#[derive(Clone)]
pub struct Config {
    pub email: Option<String>,
    pub bugzilla_api_key: Option<String>,
    pub jira: Option<JiraConfig>,
    pub phab: Option<PhabConfig>,
    pub whiteboard: Option<String>,
    pub drift_days: i64,
    pub telemetry: bool,
    pub landing_checks: bool,
}

const MY_TICKETS: &str = "assignee = currentUser() AND (statusCategory != Done OR sprint in openSprints()) ORDER BY updated DESC";

const RECENT_COMMENTS: usize = worklens_core::brief::RECENT_COMMENTS;

const CRASH_DAYS: f64 = 10.0;

const LANDED_DAYS: i64 = 30;

fn soft<T>(errors: &mut Vec<SourceError>, source: &str, r: Result<T>, fallback: T) -> T {
    r.unwrap_or_else(|e| {
        errors.push(SourceError {
            source: source.to_owned(),
            message: e.summary(),
            unset: false,
        });
        fallback
    })
}

pub(crate) async fn maybe<T>(f: Option<impl Future<Output = Result<T>>>) -> Result<Option<T>> {
    match f {
        Some(f) => f.await.map(Some),
        None => Ok(None),
    }
}

fn tag(bugs: &mut IndexMap<BugId, Bug>, list: Vec<Bug>, why: Via) {
    for mut b in list {
        match bugs.get_mut(&b.id) {
            Some(existing) if !existing.via.contains(&why) => existing.via.push(why),
            Some(_) => {}
            None => {
                b.via = vec![why];
                bugs.insert(b.id, b);
            }
        }
    }
}

pub async fn fetch_raw(http: &Http, cfg: &Config, previous: Option<Raw>, log: impl Fn(&str)) -> Result<Raw> {
    if cfg.email.is_none() && cfg.jira.is_none() && cfg.phab.is_none() {
        return Err(identity_error());
    }
    let mut errors: Vec<SourceError> = Vec::new();
    let th = &Treeherder { http };
    let jira = match &cfg.jira {
        Some(j) => soft(&mut errors, "jira", Jira::connect(http, j).await.map(Some), None),
        None => {
            errors.push(SourceError {
                source: "jira".into(),
                message: "not set up, so Sprint, Ideas and Drift are off; `worklens auth set jira` turns them on".into(),
                unset: true,
            });
            None
        }
    };
    let phab = cfg.phab.as_ref().map(|p| Phab {
        http,
        host: &p.host,
        token: &p.token,
    });
    if phab.is_none() {
        errors.push(SourceError {
            source: "phabricator".into(),
            message: "not set up, so revisions are off; `worklens auth set phabricator` turns them on".into(),
            unset: true,
        });
    }

    log("Identifying you");
    let (jira_me, phab_me, trains) = futures::join!(
        maybe(jira.as_ref().map(Jira::me)),
        maybe(phab.as_ref().map(Phab::whoami)),
        crate::trains::trains(http)
    );
    let trains = soft(&mut errors, "product-details", trains.map(Some), None);
    let release_fields = crate::bugzilla::release_fields(trains.as_ref());
    let bz = Bugzilla {
        http,
        api_key: cfg.bugzilla_api_key.as_deref(),
        release_fields: &release_fields,
    };
    let jira_me = soft(&mut errors, "jira", jira_me, None);
    let phab_me = soft(&mut errors, "phabricator", phab_me, None);
    let email = cfg
        .email
        .clone()
        .or_else(|| jira_me.clone())
        .or_else(|| phab_me.as_ref().map(|m| m.primary_email.clone()))
        .ok_or_else(identity_error)?;
    let my_phid = phab_me.as_ref().map(|m| m.phid.as_str());

    log(match (jira.is_some(), phab.is_some()) {
        (true, true) => "Fetching bugs, tickets and revisions",
        (true, false) => "Fetching bugs and tickets",
        (false, true) => "Fetching bugs and revisions",
        (false, false) => "Fetching bugs",
    });
    let (assigned, needinfos, issues, my_revs, to_review, team, fixed, asked, groups) = futures::join!(
        bz.assigned_to(&email),
        bz.needinfo_on(&email),
        maybe(jira.as_ref().map(|j| j.search(MY_TICKETS))),
        maybe(phab.as_ref().zip(my_phid).map(|(p, id)| p.mine(id))),
        maybe(phab.as_ref().zip(my_phid).map(|(p, id)| p.to_review(id))),
        maybe(cfg.whiteboard.as_deref().map(|w| bz.team(w, cfg.drift_days))),
        bz.fixed_by(&email, LANDED_DAYS),
        bz.needinfo_from(&email),
        maybe(phab.as_ref().zip(my_phid).map(|(p, id)| p.group_reviews(id))),
    );
    let assigned = soft(&mut errors, "bugzilla", assigned, Vec::new());
    let needinfos = soft(&mut errors, "bugzilla", needinfos, Vec::new());
    let issues = soft(&mut errors, "jira", issues, None).unwrap_or_default();
    let my_revs = soft(&mut errors, "phabricator", my_revs, None).unwrap_or_default();
    let to_review = soft(&mut errors, "phabricator", to_review, None).unwrap_or_default();
    let team = soft(&mut errors, "bugzilla", team, None).unwrap_or_default();
    let fixed = soft(&mut errors, "bugzilla", fixed, Vec::new());
    let asked = soft(&mut errors, "bugzilla", asked, Vec::new());
    let direct: HashSet<RevId> = to_review.iter().map(|r| r.id).collect();
    let groups: Vec<(Revision, String)> = soft(&mut errors, "phabricator", groups, None)
        .unwrap_or_default()
        .into_iter()
        .filter(|(r, _)| !direct.contains(&r.id))
        .collect();
    let group_review: HashMap<RevId, String> = groups.iter().map(|(r, g)| (r.id, g.clone())).collect();
    let group_revs: Vec<Revision> = groups.into_iter().map(|(r, _)| r).collect();

    let my_ids: Vec<RevId> = my_revs.iter().map(|r| r.id).collect();
    let diff_phids: Vec<String> = my_revs
        .iter()
        .chain(&to_review)
        .chain(&group_revs)
        .filter_map(|r| r.diff_phid.clone())
        .collect();
    let try_status = async {
        let mut errors = Vec::new();
        let diff_refs: Vec<&str> = diff_phids.iter().map(String::as_str).collect();
        let (pushes, dates) = futures::join!(
            th.latest_pushes(&email, &my_ids),
            maybe(phab.as_ref().filter(|_| !diff_refs.is_empty()).map(|p| p.diff_dates(&diff_refs))),
        );
        let pushes = soft(&mut errors, "treeherder", pushes, HashMap::new());
        let dates = soft(&mut errors, "phabricator", dates, None).unwrap_or_default();
        let healths: Vec<(RevId, Result<Option<crate::treeherder::Health>>)> = stream::iter(&pushes)
            .map(|(rev, p)| async move { (*rev, th.health(&p.revision).await) })
            .buffer_unordered(MAX_IN_FLIGHT)
            .collect()
            .await;
        let mut try_pushes = HashMap::with_capacity(healths.len());
        for (rev, health) in healths {
            let (Some(push), Some(h)) = (pushes.get(&rev), soft(&mut errors, "treeherder", health, None)) else {
                continue;
            };
            try_pushes.insert(
                rev,
                TryPush {
                    revision: push.revision.clone(),
                    pushed_at: push.pushed_at,
                    url: push_url(&push.revision),
                    builds: h.builds,
                    lint: h.lint,
                    tests: h.tests,
                    failed: h.failed,
                    running: h.running,
                },
            );
        }
        (try_pushes, dates, errors)
    };

    let the_rest = async {
        let mut errors = Vec::new();
        let live_team: Vec<BugId> = team
            .iter()
            .filter(|b| b.status != "RESOLVED" && b.status != "VERIFIED")
            .map(|b| b.id)
            .collect();
        let mut bugs: IndexMap<BugId, Bug> = IndexMap::with_capacity(assigned.len() + needinfos.len() + team.len());
        tag(&mut bugs, assigned, Via::Assigned);
        tag(&mut bugs, needinfos, Via::Needinfo);
        tag(&mut bugs, team, Via::Team);
        let landed: Vec<BugId> = fixed.iter().map(|b| b.id).collect();
        tag(&mut bugs, fixed, Via::Landed);
        tag(&mut bugs, asked, Via::Asked);

        if !issues.is_empty() {
            log("Linking Jira tickets to bugs");
        }
        let linked: HashSet<&str> = bugs.values().filter_map(jira_key_of).collect();
        let unlinked: Vec<&str> = issues.iter().map(|i| i.key.0.as_str()).filter(|k| !linked.contains(k)).collect();
        let mirrors = if unlinked.is_empty() {
            Ok(Vec::new())
        } else {
            bz.mirroring(&unlinked).await
        };
        drop(linked);
        tag(&mut bugs, soft(&mut errors, "bugzilla", mirrors, Vec::new()), Via::Mirror);
        if !landed.is_empty() {
            let found = bz.regressions_of(&landed).await;
            tag(&mut bugs, soft(&mut errors, "bugzilla", found, Vec::new()), Via::Regression);
        }
        let mut rev_bugs: Vec<BugId> = my_revs
            .iter()
            .chain(&to_review)
            .chain(&group_revs)
            .filter_map(|r| r.bug)
            .filter(|id| !bugs.contains_key(id))
            .collect();
        rev_bugs.dedup();
        rev_bugs.sort_unstable();
        rev_bugs.dedup();
        if !rev_bugs.is_empty() {
            let found = bz.by_ids(&rev_bugs).await;
            tag(&mut bugs, soft(&mut errors, "bugzilla", found, Vec::new()), Via::Revision);
        }

        let mut issue_by_key: IndexMap<JiraKey, Issue> = issues.into_iter().map(|i| (JiraKey(i.key.0.clone()), i)).collect();
        let mut missing: Vec<&str> = bugs
            .values()
            .filter_map(jira_key_of)
            .filter(|k| !issue_by_key.contains_key(*k))
            .collect();
        let mut seen = HashSet::new();
        missing.retain(|k| seen.insert(*k));
        if let (Some(j), false) = (&jira, missing.is_empty()) {
            for (key, issue) in soft(&mut errors, "jira", j.by_keys(&missing).await, Vec::new()) {
                issue_by_key.insert(JiraKey(key), issue);
            }
        }

        log("Reading comments and team revisions");
        let mine: Vec<BugId> = bugs
            .values()
            .filter(|b| !b.via.iter().all(|v| *v == Via::Team))
            .map(|b| b.id)
            .collect();
        let mut attachment_scope = live_team.clone();
        attachment_scope.extend(&mine);
        attachment_scope.sort_unstable();
        attachment_scope.dedup();
        let (comments, team_atts, uplift, roadmap) = futures::join!(
            bz.comments(&mine),
            maybe(
                phab_me
                    .as_ref()
                    .filter(|_| !attachment_scope.is_empty())
                    .map(|_| bz.revision_attachments(&attachment_scope))
            ),
            maybe((!mine.is_empty()).then(|| bz.uplift(&mine))),
            maybe(jira.as_ref().map(|j| j.roadmap(&issue_by_key))),
        );
        let roadmap = soft(&mut errors, "jira-ideas", roadmap, None).unwrap_or_default();
        let comments = soft(&mut errors, "bugzilla", comments, HashMap::new());
        let uplift = soft(&mut errors, "bugzilla", uplift, None).unwrap_or_default();

        let now = Timestamp::now();
        let checks: Vec<(BugId, &str, jiff::civil::Date)> = bugs
            .values()
            .filter(|b| b.via.contains(&Via::Landed))
            .filter_map(|b| {
                let resolved = b.cf_last_resolved?;
                let recent = worklens_core::bugs::days_between(Some(resolved), now) <= CRASH_DAYS;
                let since = resolved.to_zoned(jiff::tz::TimeZone::UTC).date().tomorrow().ok()?;
                recent.then_some((b.id, since))
            })
            .flat_map(|(id, since)| {
                worklens_core::bugs::crash_signatures(&bugs[&id])
                    .into_iter()
                    .map(move |sig| (id, sig, since))
            })
            .collect();
        let counts: Vec<(BugId, jiff::civil::Date, Result<u32>)> = stream::iter(checks)
            .map(|(id, sig, since)| async move { (id, since, crate::crashstats::nightly_crashes(http, sig, since).await) })
            .buffer_unordered(MAX_IN_FLIGHT)
            .collect()
            .await;
        let mut crashes: HashMap<BugId, worklens_core::raw::Crashes> = HashMap::new();
        for (id, since, count) in counts {
            let count = soft(&mut errors, "crash-stats", count, 0);
            crashes.entry(id).or_insert(worklens_core::raw::Crashes { count: 0, since }).count += count;
        }
        let team_atts = soft(&mut errors, "bugzilla", team_atts, None).unwrap_or_default();

        let known: HashSet<RevId> = my_revs.iter().chain(&to_review).chain(&group_revs).map(|r| r.id).collect();
        let mut team_rev_ids: Vec<RevId> = team_atts.values().flatten().copied().filter(|id| !known.contains(id)).collect();
        team_rev_ids.sort_unstable();
        team_rev_ids.dedup();
        let team_revs = match &phab {
            Some(p) if !team_rev_ids.is_empty() => soft(&mut errors, "phabricator", p.by_ids(&team_rev_ids).await, Vec::new()),
            _ => Vec::new(),
        };
        let to_review_ids: Vec<RevId> = to_review.iter().map(|r| r.id).collect();
        let mut revisions: Vec<Revision> = Vec::with_capacity(my_revs.len() + to_review.len() + team_revs.len());
        revisions.extend(my_revs.iter().map(clone_revision));
        revisions.extend(to_review);
        revisions.extend(group_revs);
        revisions.extend(team_revs);

        let mut phids: Vec<&str> = revisions
            .iter()
            .flat_map(|r| {
                std::iter::once(r.author.as_str())
                    .chain(r.reviewers.iter().map(|x| x.phid.as_str()))
                    .chain(r.projects.iter().flatten().map(String::as_str))
            })
            .collect();
        let mut seen = HashSet::new();
        phids.retain(|p| seen.insert(*p));
        log("Checking try pushes and reviewers");
        let users = match &phab {
            Some(p) if !phids.is_empty() => soft(&mut errors, "phabricator", p.names(&phids).await, HashMap::new()),
            _ => HashMap::new(),
        };
        (
            bugs,
            issue_by_key,
            revisions,
            to_review_ids,
            team_atts,
            comments,
            users,
            uplift,
            crashes,
            roadmap,
            errors,
        )
    };

    let (
        (try_pushes, diff_dates, try_errors),
        (bugs, issues, mut revisions, to_review, bug_revisions, comments, users, uplift, crashes, roadmap, rest_errors),
    ) = futures::join!(try_status, the_rest);
    errors.extend(rest_errors);
    errors.extend(try_errors);
    for r in &mut revisions {
        if let Some(phid) = &r.diff_phid {
            r.diff_created = diff_dates.get(phid).copied().or(r.diff_created);
        }
    }

    let mut last_comments = HashMap::with_capacity(comments.len());
    let mut threads = HashMap::with_capacity(comments.len());
    for (id, mut cs) in comments {
        let Some(last) = cs.last() else { continue };
        last_comments.insert(
            id,
            Comment {
                author: last.author.clone(),
                time: last.time,
                text: last.text.clone(),
            },
        );
        let total = u32::try_from(cs.len()).unwrap_or(u32::MAX);
        if cs.len() > RECENT_COMMENTS + 1 {
            cs.drain(1..cs.len() - RECENT_COMMENTS);
        }
        threads.insert(id, Thread { total, comments: cs });
    }

    let mut raw = Raw {
        version: RAW_VERSION,
        fetched_at: Timestamp::now(),
        me: Me {
            email,
            jira_email: jira_me,
            phid: phab_me.map(|m| m.phid),
        },
        bugs: bugs.into_values().collect(),
        issues,
        revisions,
        to_review,
        bug_revisions,
        last_comments,
        calls: Vec::new(),
        try_pushes: Some(try_pushes),
        users,
        threads,
        errors,
        stale: Vec::new(),
        trains,
        uplift,
        crashes,
        group_review,
        telemetry: None,
        landings: None,
        supplemental_cache: Default::default(),
        roadmap,
        dependencies: Default::default(),
    };
    crate::supplemental::restore(&mut raw, previous.as_ref());
    crate::supplemental::refresh(http, cfg, &mut raw, &[], log, Timestamp::now()).await;
    crate::carry::carry_forward(&mut raw, previous);
    crate::dependencies::refresh(http, cfg, &mut raw).await;
    Ok(raw)
}

fn identity_error() -> Error {
    Error::Api("Can't tell who you are: run `worklens setup`, or set \"email\" in ~/.config/worklens.json.".into())
}

fn clone_revision(r: &Revision) -> Revision {
    Revision {
        id: r.id,
        phid: r.phid.clone(),
        title: r.title.clone(),
        status: r.status,
        author: r.author.clone(),
        bug: r.bug,
        modified: r.modified,
        created: r.created,
        reviewers: r
            .reviewers
            .iter()
            .map(|x| worklens_core::raw::Reviewer {
                phid: x.phid.clone(),
                status: x.status,
            })
            .collect(),
        diff_phid: r.diff_phid.clone(),
        diff_created: r.diff_created,
        projects: r.projects.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolvable_identity_fails_before_any_request_without_a_network_runtime() {
        let cfg = Config {
            email: None,
            bugzilla_api_key: None,
            jira: None,
            phab: None,
            whiteboard: Some("[test]".into()),
            drift_days: 30,
            telemetry: true,
            landing_checks: true,
        };
        let http = Http::new().unwrap();
        let result = futures::executor::block_on(fetch_raw(&http, &cfg, None, |_| {}));
        assert!(result.unwrap_err().to_string().contains("Can't tell who you are"));
        assert!(http.take_calls().is_empty());
    }
}
