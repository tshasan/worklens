use std::collections::{HashMap, HashSet};

use worklens_core::Raw;
use worklens_core::raw::{Dependencies, SourceError};

use crate::bugzilla::Bugzilla;
use crate::fetch::Config;
use crate::http::Http;
use crate::phab::Phab;

pub(crate) async fn refresh(http: &Http, cfg: &Config, raw: &mut Raw) {
    raw.errors
        .retain(|e| !matches!(e.source.as_str(), "bugzilla-dependencies" | "phabricator-dependencies"));
    let mut dependencies = Dependencies::default();
    let present: HashSet<_> = raw.bugs.iter().map(|b| b.id).collect();
    let mut ids: Vec<_> = raw
        .bugs
        .iter()
        .flat_map(|b| b.depends_on.iter().copied())
        .filter(|id| !present.contains(id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let release_fields = crate::bugzilla::release_fields(raw.trains.as_ref());
    let bz = Bugzilla {
        http,
        api_key: cfg.bugzilla_api_key.as_deref(),
        release_fields: &release_fields,
    };
    let bugs = async { if ids.is_empty() { Ok(Vec::new()) } else { bz.by_ids(&ids).await } };
    let phab = async {
        let Some(config) = &cfg.phab else {
            return Ok((HashMap::new(), Vec::new(), HashMap::new()));
        };
        let p = Phab {
            http,
            host: &config.host,
            token: &config.token,
        };
        let phids: Vec<_> = raw.revisions.iter().filter_map(|r| r.phid.clone()).collect();
        let parents = p.parents(&phids).await?;
        let mut wanted: Vec<_> = parents.values().flatten().cloned().collect();
        wanted.sort();
        wanted.dedup();
        let revisions = p.by_phids(&wanted).await?;
        let mut names: Vec<_> = revisions
            .iter()
            .flat_map(|r| {
                std::iter::once(r.author.as_str())
                    .chain(r.reviewers.iter().map(|v| v.phid.as_str()))
                    .chain(r.projects.iter().flatten().map(String::as_str))
            })
            .collect();
        names.sort_unstable();
        names.dedup();
        let names = p.names(&names).await?;
        Ok::<_, crate::http::Error>((parents, revisions, names))
    };
    let (bugs, phab) = futures::join!(bugs, phab);
    match bugs {
        Ok(bugs) => dependencies.bugs.extend(bugs.into_iter().map(|b| (b.id, b))),
        Err(e) => raw.errors.push(SourceError {
            source: "bugzilla-dependencies".into(),
            message: e.summary(),
            unset: false,
            retry_at: e.retry_at(),
        }),
    }
    match phab {
        Ok((parents, revisions, names)) => {
            for r in &raw.revisions {
                if let Some(parents) = r.phid.as_ref().and_then(|p| parents.get(p)) {
                    dependencies.parents.insert(r.id, parents.clone());
                }
            }
            dependencies
                .revisions
                .extend(revisions.into_iter().filter_map(|r| Some((r.phid.clone()?, r))));
            raw.users.extend(names);
        }
        Err(e) => raw.errors.push(SourceError {
            source: "phabricator-dependencies".into(),
            message: e.summary(),
            unset: false,
            retry_at: e.retry_at(),
        }),
    }
    raw.dependencies = dependencies;
}
