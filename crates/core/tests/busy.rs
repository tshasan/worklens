use std::collections::BTreeSet;

use jiff::Timestamp;
use worklens_core::rules::{Reason, TAGS};
use worklens_core::snapshot::Scope;
use worklens_core::{CoreConfig, Raw, Section, Snapshot, build_snapshot};

fn busy() -> Snapshot {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/busy.json");
    let raw: Raw = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let cfg = CoreConfig {
        whiteboard: Some("[aiplatform]".into()),
        ..CoreConfig::default()
    };
    build_snapshot(raw, &cfg, "2026-11-03T19:00:00Z".parse::<Timestamp>().unwrap())
}

#[test]
fn the_busy_fixture_fills_every_section() {
    let s = busy();
    for section in Section::ALL {
        assert!(s.section(section, false).next().is_some(), "{section:?} is empty");
    }
}

#[test]
fn the_busy_fixture_fires_every_rule() {
    let s = busy();
    let placed: Vec<Reason> = s.entries.iter().filter_map(|e| e.placement).map(|p| p.reason).collect();
    let tags: BTreeSet<&str> = placed.iter().map(|r| r.tag()).collect();
    for facets in TAGS {
        assert!(tags.contains(facets.tag), "no entry tagged {:?}", facets.tag);
    }
    assert!(placed.iter().any(|r| matches!(r, Reason::Sprint(_))));
    assert!(placed.iter().any(|r| matches!(r, Reason::Idea(_))));
    assert!(placed.iter().any(|r| matches!(r, Reason::Other)));
    assert!(s.entries.iter().filter_map(|e| e.placement).any(|p| p.stale), "something is stale");
}

#[test]
fn the_busy_fixture_has_drift_of_both_levels_and_team_work() {
    let s = busy();
    assert!(s.entries.iter().any(|e| e.has_error_drift()));
    assert!(s.entries.iter().any(|e| !e.drift.is_empty() && !e.has_error_drift()));
    assert!(s.entries.iter().any(|e| e.scope == Scope::Team && e.bug.is_some()));
    assert!(s.entries.iter().any(|e| e.scope == Scope::Team && e.idea.is_some()));
    assert!(s.entries.len() > 80, "busy means busy: {}", s.entries.len());
}
