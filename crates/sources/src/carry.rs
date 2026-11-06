use worklens_core::Raw;
use worklens_core::raw::Stale;

const CARRIED: [&str; 7] = [
    "bugzilla",
    "jira",
    "jira-ideas",
    "phabricator",
    "treeherder",
    "product-details",
    "crash-stats",
];

pub(crate) fn carry_forward(raw: &mut Raw, previous: Option<Raw>) {
    let Some(mut prev) = previous.filter(|p| p.me.email.eq_ignore_ascii_case(&raw.me.email)) else {
        return;
    };
    for source in CARRIED {
        if !raw.failures().any(|e| e.source == source) {
            continue;
        }
        let since = prev.stale.iter().find(|s| s.source == source).map_or(prev.fetched_at, |s| s.since);
        let carried = match source {
            "bugzilla" => bugzilla(raw, &mut prev),
            "jira" => jira(raw, &mut prev) | roadmap(raw, &mut prev),
            "jira-ideas" => roadmap(raw, &mut prev),
            "phabricator" => phabricator(raw, &mut prev),
            "treeherder" => treeherder(raw, &mut prev),
            "product-details" => {
                let missing = raw.trains.is_none() && prev.trains.is_some();
                if missing {
                    raw.trains = prev.trains.take();
                }
                missing
            }
            "crash-stats" => {
                let before = raw.crashes.len();
                for (id, c) in prev.crashes.drain() {
                    raw.crashes.entry(id).or_insert(c);
                }
                raw.crashes.len() > before
            }
            _ => false,
        };
        if carried {
            raw.stale.push(Stale {
                source: source.to_owned(),
                since,
            });
        }
    }
}

fn bugzilla(raw: &mut Raw, prev: &mut Raw) -> bool {
    let before = raw.bugs.len();
    let have: std::collections::HashSet<_> = raw.bugs.iter().map(|b| b.id).collect();
    raw.bugs.extend(prev.bugs.drain(..).filter(|b| !have.contains(&b.id)));
    for (id, c) in prev.last_comments.drain() {
        raw.last_comments.entry(id).or_insert(c);
    }
    for (id, t) in prev.threads.drain() {
        raw.threads.entry(id).or_insert(t);
    }
    for (id, u) in prev.uplift.drain() {
        raw.uplift.entry(id).or_insert(u);
    }
    raw.bugs.len() > before
}

fn jira(raw: &mut Raw, prev: &mut Raw) -> bool {
    let before = raw.issues.len();
    for (key, issue) in prev.issues.drain(..) {
        raw.issues.entry(key).or_insert(issue);
    }
    if raw.me.jira_email.is_none() {
        raw.me.jira_email = prev.me.jira_email.take();
    }
    raw.issues.len() > before
}

fn roadmap(raw: &mut Raw, prev: &mut Raw) -> bool {
    let before = raw.roadmap.parents.len() + raw.roadmap.ideas.len();
    for (key, issue) in prev.roadmap.parents.drain(..) {
        raw.roadmap.parents.entry(key).or_insert(issue);
    }
    for (key, idea) in prev.roadmap.ideas.drain(..) {
        raw.roadmap.ideas.entry(key).or_insert(idea);
    }
    raw.roadmap.parents.len() + raw.roadmap.ideas.len() > before
}

fn phabricator(raw: &mut Raw, prev: &mut Raw) -> bool {
    let before = raw.revisions.len() + raw.to_review.len();
    let have: std::collections::HashSet<_> = raw.revisions.iter().map(|r| r.id).collect();
    raw.revisions.extend(prev.revisions.drain(..).filter(|r| !have.contains(&r.id)));
    for id in prev.to_review.drain(..) {
        if !raw.to_review.contains(&id) {
            raw.to_review.push(id);
        }
    }
    for (bug, revs) in prev.bug_revisions.drain() {
        raw.bug_revisions.entry(bug).or_insert(revs);
    }
    for (rev, group) in prev.group_review.drain() {
        raw.group_review.entry(rev).or_insert(group);
    }
    for (phid, name) in prev.users.drain() {
        raw.users.entry(phid).or_insert(name);
    }
    if raw.me.phid.is_none() {
        raw.me.phid = prev.me.phid.take();
    }
    raw.revisions.len() + raw.to_review.len() > before
}

fn treeherder(raw: &mut Raw, prev: &mut Raw) -> bool {
    let Some(old) = prev.try_pushes.take() else { return false };
    let pushes = raw.try_pushes.get_or_insert_with(Default::default);
    let before = pushes.len();
    for (rev, push) in old {
        pushes.entry(rev).or_insert(push);
    }
    pushes.len() > before
}

#[cfg(test)]
mod tests {
    use super::*;
    use worklens_core::raw::SourceError;

    fn fixture() -> Raw {
        serde_json::from_str(include_str!("../../store/fixtures/raw.json")).unwrap()
    }

    fn failed(raw: &mut Raw, source: &str) {
        raw.errors.push(SourceError {
            source: source.into(),
            message: "503 Service Unavailable".into(),
            unset: false,
            retry_at: None,
        });
    }

    #[test]
    fn a_failed_jira_keeps_last_times_tickets_and_says_since_when() {
        let prev = fixture();
        let tickets = prev.issues.len();
        assert!(tickets > 0, "the fixture has Jira tickets");
        let since = prev.fetched_at;
        let mut now = fixture();
        now.issues.clear();
        failed(&mut now, "jira");
        carry_forward(&mut now, Some(prev));
        assert_eq!(now.issues.len(), tickets);
        assert_eq!(now.stale.len(), 1);
        assert_eq!(now.stale[0].source, "jira");
        assert_eq!(now.stale[0].since, since);
    }

    #[test]
    fn sources_that_answered_are_left_alone() {
        let prev = fixture();
        let mut now = fixture();
        now.bugs.truncate(1);
        failed(&mut now, "jira");
        carry_forward(&mut now, Some(prev));
        assert_eq!(now.bugs.len(), 1);
        assert!(now.stale.iter().all(|s| s.source != "bugzilla"));
    }

    #[test]
    fn this_fetchs_records_win_over_older_ones() {
        let prev = fixture();
        let mut now = fixture();
        let id = now.bugs[0].id;
        now.bugs[0].summary = "renamed today".into();
        failed(&mut now, "bugzilla");
        carry_forward(&mut now, Some(prev));
        let bugs: Vec<_> = now.bugs.iter().filter(|b| b.id == id).collect();
        assert_eq!(bugs.len(), 1, "merged by id, not duplicated");
        assert_eq!(bugs[0].summary, "renamed today");
    }

    #[test]
    fn stale_since_is_the_first_failure_not_the_last() {
        let mut prev = fixture();
        let first: jiff::Timestamp = "2026-11-01T09:00:00Z".parse().unwrap();
        prev.stale.push(Stale {
            source: "jira".into(),
            since: first,
        });
        let mut now = fixture();
        now.issues.clear();
        failed(&mut now, "jira");
        carry_forward(&mut now, Some(prev));
        assert_eq!(now.stale[0].since, first);
    }

    #[test]
    fn another_persons_cache_is_never_used() {
        let mut prev = fixture();
        prev.me.email = "someone-else@mozilla.com".into();
        let mut now = fixture();
        now.issues.clear();
        failed(&mut now, "jira");
        carry_forward(&mut now, Some(prev));
        assert!(now.issues.is_empty());
        assert!(now.stale.is_empty());
    }
}
