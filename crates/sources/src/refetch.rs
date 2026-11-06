use std::collections::HashMap;

use worklens_core::raw::{Comment, RevStatus, Thread, Via};
use worklens_core::{BugId, JiraKey, Raw, RevId};

use crate::bugzilla::Bugzilla;
use crate::fetch::{Config, maybe};
use crate::http::{Http, Result};
use crate::jira::Jira;
use crate::phab::Phab;

#[derive(Clone, Debug, Default)]
pub struct ItemRef {
    pub bug: Option<BugId>,
    pub revisions: Vec<RevId>,
    pub jira: Option<String>,
}

pub async fn refetch_items(http: &Http, cfg: &Config, mut raw: Raw, items: &[ItemRef]) -> Result<Raw> {
    http.restore_cooldowns(&raw);
    let release_fields = crate::bugzilla::release_fields(raw.trains.as_ref());
    let bz = Bugzilla {
        http,
        api_key: cfg.bugzilla_api_key.as_deref(),
        release_fields: &release_fields,
    };
    let phab = cfg.phab.as_ref().map(|p| Phab {
        http,
        host: &p.host,
        token: &p.token,
    });
    let jira = match &cfg.jira {
        Some(j) => Jira::connect(http, j).await.ok(),
        None => None,
    };
    let mut bug_ids: Vec<BugId> = items.iter().filter_map(|i| i.bug).collect();
    bug_ids.sort_unstable();
    bug_ids.dedup();
    let mut rev_ids: Vec<RevId> = items.iter().flat_map(|i| i.revisions.iter().copied()).collect();
    rev_ids.sort_unstable();
    rev_ids.dedup();
    let mut keys: Vec<&str> = items.iter().filter_map(|i| i.jira.as_deref()).collect();
    keys.sort_unstable();
    keys.dedup();

    let (bugs, comments, mut revs, issues) = futures::try_join!(
        maybe((!bug_ids.is_empty()).then(|| bz.by_ids(&bug_ids))),
        maybe((!bug_ids.is_empty()).then(|| bz.comments(&bug_ids))),
        maybe(phab.as_ref().filter(|_| !rev_ids.is_empty()).map(|p| p.by_ids(&rev_ids))),
        maybe(jira.as_ref().filter(|_| !keys.is_empty()).map(|j| j.by_keys(&keys))),
    )?;
    let my_phid = raw.me.phid.clone();

    if let Some(p) = &phab {
        let phids: Vec<&str> = revs
            .iter()
            .flatten()
            .flat_map(|r| r.projects.iter().flatten().map(String::as_str))
            .collect();
        let diff_phids: Vec<&str> = revs.iter().flatten().filter_map(|r| r.diff_phid.as_deref()).collect();
        let (names, dates) = futures::try_join!(p.names(&phids), p.diff_dates(&diff_phids))?;
        raw.users.extend(names);
        for rev in revs.iter_mut().flatten() {
            rev.diff_created = rev.diff_phid.as_ref().and_then(|phid| dates.get(phid)).copied();
        }
    }
    if !bug_ids.is_empty() {
        raw.uplift.extend(bz.uplift(&bug_ids).await?);
    }

    for mut r in revs.unwrap_or_default() {
        let asked = r.status == RevStatus::NeedsReview
            && Some(&r.author) != my_phid.as_ref()
            && r.reviewers
                .iter()
                .any(|x| Some(&x.phid) == my_phid.as_ref() && x.status.owes_review());
        raw.to_review.retain(|id| *id != r.id);
        if asked {
            raw.to_review.push(r.id);
        }
        match raw.revisions.iter_mut().find(|x| x.id == r.id) {
            Some(old) => {
                if r.diff_phid == old.diff_phid && r.diff_created.is_none() {
                    r.diff_created = old.diff_created;
                }
                *old = r;
            }
            None => raw.revisions.push(r),
        }
    }
    for (key, issue) in issues.unwrap_or_default() {
        raw.issues.insert(JiraKey(key), issue);
    }
    for mut b in bugs.unwrap_or_default() {
        if let Some(old) = raw.bugs.iter_mut().find(|x| x.id == b.id) {
            b.via = std::mem::take(&mut old.via);
            *old = b;
        }
    }
    apply_comments(&mut raw, comments.unwrap_or_default());

    let (email, revisions, to_review) = (&raw.me.email, &raw.revisions, &raw.to_review);
    raw.bugs.retain(|b| {
        if !bug_ids.contains(&b.id)
            || b.via
                .iter()
                .any(|v| matches!(v, Via::Assigned | Via::Team | Via::Mirror | Via::Landed | Via::Regression))
        {
            return true;
        }
        let needinfo = b
            .flags
            .iter()
            .any(|f| f.name == "needinfo" && f.status == "?" && f.requestee.as_ref() == Some(email));
        let asked = b
            .flags
            .iter()
            .any(|f| f.name == "needinfo" && f.status == "?" && &f.setter == email && f.requestee.as_ref() != Some(email));
        let review = revisions.iter().any(|r| r.bug == Some(b.id) && to_review.contains(&r.id));
        let authored = revisions.iter().any(|r| r.bug == Some(b.id) && Some(&r.author) == my_phid.as_ref());
        needinfo || asked || review || authored
    });
    crate::supplemental::refresh(http, cfg, &mut raw, &rev_ids, |_| {}, jiff::Timestamp::now()).await;
    crate::dependencies::refresh(http, cfg, &mut raw).await;
    raw.calls.extend(http.take_calls());
    Ok(raw)
}

fn apply_comments(raw: &mut Raw, comments: HashMap<BugId, Vec<Comment>>) {
    for (id, mut cs) in comments {
        let Some(last) = cs.last() else { continue };
        raw.last_comments.insert(
            id,
            Comment {
                author: last.author.clone(),
                time: last.time,
                text: last.text.clone(),
            },
        );
        let total = u32::try_from(cs.len()).unwrap_or(u32::MAX);
        let keep = worklens_core::brief::RECENT_COMMENTS;
        if cs.len() > keep + 1 {
            cs.drain(1..cs.len() - keep);
        }
        raw.threads.insert(id, Thread { total, comments: cs });
    }
}
