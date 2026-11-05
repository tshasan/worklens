use worklens_core::raw::SourceState;
use worklens_core::{CoreConfig, Raw, Section, Snapshot, build_snapshot};

fn snapshot(fixture: &str) -> Snapshot {
    let path = format!("{}/../store/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"));
    let raw: Raw = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    build_snapshot(raw, &CoreConfig::default(), "2026-11-03T20:00:00Z".parse().unwrap())
}

#[test]
fn a_source_that_is_not_set_up_is_off_not_failed() {
    let s = snapshot("bugzilla_only.json");
    assert_eq!(
        s.raw.states(),
        [
            ("bugzilla", SourceState::Read),
            ("jira", SourceState::Off),
            ("phabricator", SourceState::Off),
        ]
    );
    assert!(s.raw.failures().next().is_none());
    for section in [Section::Land, Section::Sprint, Section::Drift] {
        let why = s.not_checked(section).unwrap();
        assert!(why.contains("isn't set up"), "{section:?}: {why}");
    }
}

#[test]
fn a_real_failure_beside_the_unset_note_is_still_a_failure() {
    let mut s = snapshot("bugzilla_only.json");
    s.raw.errors.push(worklens_core::raw::SourceError {
        source: "jira".into(),
        message: "timed out".into(),
        unset: false,
    });
    assert_eq!(s.raw.state("jira"), SourceState::Failed);
    assert!(s.not_checked(Section::Sprint).unwrap().contains("couldn't be read"));
}

#[test]
fn looking_up_an_item_from_an_unset_source_says_which_source() {
    let s = snapshot("bugzilla_only.json");
    for reference in ["D123", "d9", "https://phabricator.services.mozilla.com/D123"] {
        assert!(
            s.unread_source(reference).unwrap().starts_with("Phabricator isn't set up"),
            "{reference}"
        );
    }
    assert!(s.unread_source("AIPLAT-123").unwrap().starts_with("Jira isn't set up"));
    for reference in ["1995008", "D", "Dx", "-1", "AI-", "é-1", "D١٢", ""] {
        assert_eq!(s.unread_source(reference), None, "{reference}");
    }
    assert_eq!(snapshot("raw.json").unread_source("D123"), None);
}
