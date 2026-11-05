use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use proptest::prelude::*;
use serde_json::Value;
use worklens_core::marks::{self, Marks, Signature};
use worklens_core::{CoreConfig, Raw, Section, Snapshot, brief, build_snapshot, changes, evidence, key, present, release, timeline};
use worklens_testkit::{Mutation, WHITEBOARD, cache, mutated, mutations, text};

fn cfg() -> CoreConfig {
    CoreConfig {
        whiteboard: Some(WHITEBOARD.into()),
        ..CoreConfig::default()
    }
}

fn fetched_at() -> Timestamp {
    "2026-11-03T19:00:00Z".parse().unwrap()
}

#[expect(clippy::disallowed_types, reason = "prop_oneof! builds its union with Arc")]
fn now() -> impl Strategy<Value = Timestamp> {
    prop_oneof![
        3 => (-400i64 * 86400..400 * 86400).prop_map(|s| fetched_at() + SignedDuration::from_secs(s)),
        1 => (Timestamp::MIN.as_second()..=Timestamp::MAX.as_second())
            .prop_map(|s| Timestamp::from_second(s).unwrap()),
    ]
}

fn raw(seed: &Value, ms: &[Mutation]) -> Option<Raw> {
    serde_json::from_value(mutated(seed, ms)).ok()
}

fn every_view(s: &Snapshot) {
    for section in Section::ALL {
        for hidden in [false, true] {
            for i in s.section(section, hidden) {
                let _ = s.entry(i);
            }
        }
    }
    for e in &s.entries {
        let _ = s.id(e).to_string();
        let _ = s.title(e);
        let _ = s.reason(e).to_string();
        let _ = s.tag(e).to_string();
        let _ = s.next_step(e);
        let _ = s.tracked_for(e);
        let _ = s.work(e);
        let _ = brief::render(s, e);
        let _ = brief::review_prompt(s, e);
        let _ = present::links(s, e);
        let _ = present::age_since(s, e);
        let _ = present::owner(s, e);
        let _ = timeline::events(s, e);
        let _ = timeline::ships(s, e);
        let _ = evidence::lines(s, e);
        let _ = evidence::brief_lines(s, e, true);
        let _ = release::matrix(s, e);
        let _ = present::date(s.now, &TimeZone::UTC);
        let k = key::mark_key(s, e);
        let _ = s.find(&k);
        let sig = marks::signature(s, e);
        if let Some(parsed) = Signature::parse(&sig) {
            let _ = changes::changes(&parsed, &parsed, changes::Failed::of(s));
        }
        for r in present::live_revisions(s, e) {
            let _ = present::review_segments(s, s.rev(r));
        }
    }
    let _ = timeline::since(s);
    let _ = evidence::telemetry(s);
    let _ = s.merge();
}

proptest! {
    #![proptest_config(worklens_testkit::config(256))]

    #[test]
    fn snapshots_never_panic_on_mutated_caches(seed in cache(), ms in mutations(16), now in now()) {
        if let Some(raw) = raw(seed, &ms) {
            let s = build_snapshot(raw, &cfg(), now);
            every_view(&s);
        }
    }

    #[test]
    fn every_section_entry_points_at_a_real_entry(seed in cache(), ms in mutations(16), now in now()) {
        if let Some(raw) = raw(seed, &ms) {
            let s = build_snapshot(raw, &cfg(), now);
            for section in Section::ALL {
                for i in s.section(section, true) {
                    prop_assert!(i.get() < s.entries.len());
                }
            }
        }
    }

    #[test]
    fn every_entry_shows_somewhere(seed in cache(), ms in mutations(16), now in now()) {
        if let Some(raw) = raw(seed, &ms) {
            let s = build_snapshot(raw, &cfg(), now);
            let mut shown = vec![false; s.entries.len()];
            for section in Section::ALL {
                for i in s.section(section, true) {
                    shown[i.get()] = true;
                }
            }
            for (i, e) in s.entries.iter().enumerate() {
                prop_assert!(shown[i], "{} is in no section", s.id(e));
            }
        }
    }

    #[test]
    fn building_twice_gives_the_same_sections(seed in cache(), ms in mutations(16), now in now()) {
        if let (Some(a), Some(b)) = (raw(seed, &ms), raw(seed, &ms)) {
            let a = build_snapshot(a, &cfg(), now);
            let b = build_snapshot(b, &cfg(), now);
            for section in Section::ALL {
                let left: Vec<String> = a.section(section, true).map(|i| a.id(a.entry(i)).to_string()).collect();
                let right: Vec<String> = b.section(section, true).map(|i| b.id(b.entry(i)).to_string()).collect();
                prop_assert_eq!(left, right, "{:?}", section);
            }
        }
    }

    #[test]
    fn marks_survive_mutation(seed in cache(), ms in mutations(8), now in now()) {
        let Some(raw) = raw(seed, &[]) else { return Ok(()) };
        let mut s = build_snapshot(raw, &cfg(), fetched_at());
        let mut m = Marks::default();
        marks::mark_all_seen(&mut m, &s, fetched_at());
        for (i, e) in s.entries.iter().enumerate() {
            match i % 3 {
                0 => marks::mark_done(&mut m, &s, e, fetched_at()),
                1 => marks::mark_snoozed(&mut m, &s, e, Some(now), fetched_at()),
                _ => {}
            }
        }
        let json = serde_json::to_value(&m).unwrap();
        if let Ok(m) = serde_json::from_value::<Marks>(mutated(&json, &ms)) {
            marks::apply(&mut s, &m, now);
            every_view(&s);
        }
    }

    #[test]
    fn snooze_text_never_panics(t in text(), now in now()) {
        let _ = marks::parse_when(&t, &now.to_zoned(TimeZone::UTC));
        let _ = Signature::parse(&t);
    }
}
