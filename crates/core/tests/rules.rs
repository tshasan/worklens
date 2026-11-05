use worklens_core::ids::{BugId, IdeaIdx, IssueIdx, RevIdx};
use worklens_core::rules::{Channel, Reason, TAGS};

fn every_reason() -> [Reason; 17] {
    let r = RevIdx(0);
    let all = [
        Reason::Needinfo,
        Reason::ReviewRequested(r),
        Reason::Accepted(r),
        Reason::LandingBlocked(r),
        Reason::ChangesRequested(r),
        Reason::InReview(r),
        Reason::Sprint(IssueIdx(0)),
        Reason::UpliftApproved(Channel::Beta(146)),
        Reason::NeedsUplift,
        Reason::UpliftRequested(Channel::Esr(140)),
        Reason::RegressionOf(BugId(1)),
        Reason::Asked(0),
        Reason::StillCrashing,
        Reason::BetaStatusUnset(146),
        Reason::GroupReview(r),
        Reason::Idea(IdeaIdx(0)),
        Reason::Other,
    ];
    for reason in all {
        match reason {
            Reason::Needinfo
            | Reason::ReviewRequested(_)
            | Reason::Accepted(_)
            | Reason::LandingBlocked(_)
            | Reason::ChangesRequested(_)
            | Reason::InReview(_)
            | Reason::Sprint(_)
            | Reason::UpliftApproved(_)
            | Reason::NeedsUplift
            | Reason::UpliftRequested(_)
            | Reason::RegressionOf(_)
            | Reason::Asked(_)
            | Reason::StillCrashing
            | Reason::BetaStatusUnset(_)
            | Reason::GroupReview(_)
            | Reason::Idea(_)
            | Reason::Other => {}
        }
    }
    all
}

#[test]
fn each_reason_has_its_tag() {
    let tags: Vec<&str> = every_reason().iter().map(|r| r.tag()).collect();
    assert_eq!(
        tags,
        [
            "ni?", "r?", "r+", "chk", "r-", "r?", "", "up+", "up", "up?", "reg", "ni>", "cr", "st?", "r?#", "", ""
        ]
    );
}

#[test]
fn reasons_share_a_tag_only_when_they_share_its_meaning() {
    for (i, a) in TAGS.iter().enumerate() {
        assert!(!a.tag.is_empty() && !a.gloss.is_empty(), "{a:?}");
        for b in &TAGS[i + 1..] {
            assert_ne!(a.tag, b.tag, "two legend rows use {}", a.tag);
        }
    }
    for reason in every_reason() {
        let f = reason.facets();
        assert!(
            f.tag.is_empty() || TAGS.contains(&f),
            "{reason:?} is tagged {} but not in the legend",
            f.tag
        );
    }
}

#[test]
fn only_uplift_and_crash_work_survives_on_bugs_i_only_landed() {
    let kept: Vec<&str> = every_reason()
        .iter()
        .filter(|r| r.facets().kept_when_landed)
        .map(|r| r.tag())
        .collect();
    assert_eq!(kept, ["up+", "up", "up?", "cr", "st?"]);
}
