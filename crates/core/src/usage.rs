use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::present;
use crate::section::Section;
use crate::snapshot::{Entry, Snapshot};

pub const DEFAULT_DAYS: i64 = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Start,
    Quit,
    Tab,
    Open,
    Copy,
    Checkout,
    Agent,
    Done,
    Snooze,
    Restore,
    Custom,
}

impl Kind {
    pub const ACTIONS: [Self; 8] = [
        Self::Open,
        Self::Copy,
        Self::Checkout,
        Self::Agent,
        Self::Done,
        Self::Snooze,
        Self::Restore,
        Self::Custom,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Quit => "quit",
            Self::Tab => "tab",
            Self::Open => "open",
            Self::Copy => "copy",
            Self::Checkout => "checkout",
            Self::Agent => "agent",
            Self::Done => "done",
            Self::Snooze => "snooze",
            Self::Restore => "restore",
            Self::Custom => "custom",
        }
    }

    const fn is_action(self) -> bool {
        !matches!(self, Self::Start | Self::Quit | Self::Tab)
    }

    const fn looks_at_item(self) -> bool {
        matches!(self, Self::Open | Self::Copy | Self::Checkout | Self::Agent | Self::Custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    Key,
    Menu,
    Enter,
}

impl Via {
    pub const ALL: [Self; 3] = [Self::Enter, Self::Key, Self::Menu];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Key => "key or chord",
            Self::Menu => "actions menu",
            Self::Enter => "Enter",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub at: Timestamp,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<Via>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_days: Option<i64>,
}

impl Event {
    pub const fn new(at: Timestamp, kind: Kind) -> Self {
        Self {
            at,
            kind,
            via: None,
            key: None,
            tab: None,
            section: None,
            tag: None,
            id: None,
            age_days: None,
        }
    }

    pub fn on_item(at: Timestamp, kind: Kind, s: &Snapshot, e: &Entry) -> Self {
        Self {
            section: e.placement.map(|p| p.section.id().to_owned()),
            tag: e.placement.map(|p| p.reason.tag().to_owned()),
            id: Some(s.id(e).to_string()),
            age_days: present::age_since(s, e).map(|since| (at.as_second() - since.as_second()).max(0) / 86_400),
            ..Self::new(at, kind)
        }
    }
}

pub fn parse(text: &str) -> Vec<Event> {
    text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Count {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub days: i64,
    pub sessions: usize,
    pub sessions_with_open: usize,
    pub median_seconds_to_first_open: Option<i64>,
    pub actions: Vec<Count>,
    pub via: Vec<Count>,
    pub keys: Vec<Count>,
    pub sections: Vec<Count>,
    pub tabs: Vec<Count>,
    pub done: usize,
    pub done_unopened: Vec<Count>,
}

fn bump(counts: &mut Vec<Count>, name: &str) {
    match counts.iter_mut().find(|c| c.name == name) {
        Some(c) => c.count += 1,
        None => counts.push(Count {
            name: name.to_owned(),
            count: 1,
        }),
    }
}

fn zeroed(names: impl Iterator<Item = &'static str>) -> Vec<Count> {
    names
        .map(|name| Count {
            name: name.to_owned(),
            count: 0,
        })
        .collect()
}

fn most_first(mut counts: Vec<Count>) -> Vec<Count> {
    counts.sort_by_key(|c| std::cmp::Reverse(c.count));
    counts
}

pub fn summarize(events: &[Event], now: Timestamp, days: i64) -> Summary {
    let from = now.as_second() - days.max(0) * 86_400;
    let mut sessions = 0;
    let mut sessions_with_open = 0;
    let mut started: Option<Timestamp> = None;
    let mut opened = true;
    let mut waits: Vec<i64> = Vec::new();
    let mut actions = zeroed(Kind::ACTIONS.iter().map(|k| k.name()));
    let mut via = zeroed(Via::ALL.iter().map(|v| v.name()));
    let mut keys = Vec::new();
    let mut sections = zeroed(Section::ALL.iter().map(|s| s.title()));
    let mut tabs = Vec::new();
    let mut done = 0;
    let mut done_unopened = Vec::new();
    let mut looked_at: Vec<&str> = Vec::new();
    for e in events {
        let recent = e.at.as_second() >= from;
        if e.kind.looks_at_item()
            && let Some(id) = e.id.as_deref()
        {
            looked_at.push(id);
        }
        if !recent {
            continue;
        }
        if e.kind == Kind::Start {
            sessions += 1;
            started = Some(e.at);
            opened = false;
        }
        if e.kind == Kind::Open && !opened {
            opened = true;
            sessions_with_open += 1;
            waits.extend(started.map(|s| e.at.as_second() - s.as_second()));
        }
        if e.kind == Kind::Tab
            && let Some(tab) = e.tab.as_deref()
        {
            bump(&mut tabs, tab);
        }
        if !e.kind.is_action() {
            continue;
        }
        bump(&mut actions, e.kind.name());
        if let Some(v) = e.via {
            bump(&mut via, v.name());
        }
        if let Some(k) = e.key.as_deref() {
            bump(&mut keys, k);
        }
        let title = e.section.as_deref().and_then(|id| Section::ALL.iter().find(|s| s.id() == id));
        if let Some(section) = title {
            bump(&mut sections, section.title());
        }
        if e.kind == Kind::Done {
            done += 1;
            if !e.id.as_deref().is_some_and(|id| looked_at.contains(&id)) {
                bump(&mut done_unopened, e.tag.as_deref().unwrap_or("untagged"));
            }
        }
    }
    waits.sort_unstable();
    Summary {
        days,
        sessions,
        sessions_with_open,
        median_seconds_to_first_open: waits.get(waits.len() / 2).copied(),
        actions: most_first(actions),
        via: most_first(via),
        keys: most_first(keys),
        sections: most_first(sections),
        tabs: most_first(tabs),
        done,
        done_unopened: most_first(done_unopened),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + second).unwrap()
    }

    fn item(second: i64, kind: Kind, id: &str, tag: &str) -> Event {
        Event {
            id: Some(id.to_owned()),
            tag: Some(tag.to_owned()),
            section: Some("waiting".to_owned()),
            via: Some(Via::Key),
            ..Event::new(at(second), kind)
        }
    }

    fn count(counts: &[Count], name: &str) -> Option<usize> {
        counts.iter().find(|c| c.name == name).map(|c| c.count)
    }

    #[test]
    fn sessions_count_starts_and_time_the_first_open() {
        let events = [
            Event::new(at(0), Kind::Start),
            item(10, Kind::Open, "1", "ni?"),
            item(50, Kind::Open, "2", "r?"),
            Event::new(at(100), Kind::Start),
            Event::new(at(110), Kind::Quit),
            Event::new(at(200), Kind::Start),
            item(230, Kind::Open, "3", "r?"),
            Event::new(at(300), Kind::Start),
            item(390, Kind::Open, "3", "r?"),
            Event::new(at(400), Kind::Start),
            Event {
                tab: Some("sprint".into()),
                ..Event::new(at(410), Kind::Tab)
            },
            item(460, Kind::Open, "4", "r?"),
        ];
        let s = summarize(&events, at(500), 14);
        assert_eq!((s.sessions, s.sessions_with_open), (5, 4));
        assert_eq!(s.median_seconds_to_first_open, Some(60), "waits of 10, 30, 60 and 90 seconds");
        assert_eq!(count(&s.actions, "open"), Some(5));
        assert_eq!(
            s.actions.iter().map(|c| c.count).sum::<usize>(),
            5,
            "starts, quits and tabs aren't actions"
        );
        assert_eq!(
            s.tabs,
            [Count {
                name: "sprint".into(),
                count: 1
            }]
        );
    }

    #[test]
    fn done_without_a_look_counts_as_noise_by_tag() {
        let events = [
            item(0, Kind::Open, "1", "ni?"),
            item(10, Kind::Done, "1", "ni?"),
            item(20, Kind::Done, "2", "r?#"),
            item(30, Kind::Done, "3", "r?#"),
            item(40, Kind::Open, "4", "bl"),
        ];
        let s = summarize(&events, at(100), 14);
        assert_eq!(s.done, 3);
        assert_eq!(
            s.done_unopened,
            [Count {
                name: "r?#".into(),
                count: 2
            }]
        );
    }

    #[test]
    fn old_events_are_left_out_but_still_count_as_a_look() {
        let day = 86_400;
        let events = [
            Event::new(at(0), Kind::Start),
            item(5, Kind::Open, "1", "ni?"),
            Event::new(at(20 * day), Kind::Start),
            item(20 * day + 5, Kind::Done, "1", "ni?"),
        ];
        let s = summarize(&events, at(20 * day + 10), 14);
        assert_eq!((s.sessions, s.sessions_with_open), (1, 0));
        assert_eq!(count(&s.actions, "open"), Some(0));
        assert_eq!(s.done, 1);
        assert!(s.done_unopened.is_empty(), "opened three weeks ago: {:?}", s.done_unopened);
    }

    #[test]
    fn unused_sections_and_routes_are_listed_at_zero() {
        let events = [item(0, Kind::Open, "1", "ni?")];
        let s = summarize(&events, at(10), 14);
        assert_eq!(count(&s.sections, Section::Waiting.title()), Some(1));
        assert_eq!(count(&s.sections, Section::Ideas.title()), Some(0));
        assert_eq!(count(&s.via, Via::Menu.name()), Some(0));
        assert_eq!(s.sections[0].name, Section::Waiting.title(), "most used first");
    }

    #[test]
    fn an_item_event_carries_its_section_tag_id_and_age() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/raw.json");
        let raw: crate::Raw = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let now: Timestamp = "2026-11-03T22:00:00Z".parse().unwrap();
        let s = crate::build_snapshot(raw, &crate::CoreConfig::default(), now);
        let e = s.entries.iter().find(|e| s.id(e).to_string() == "1995008").unwrap();
        let event = Event::on_item(now, Kind::Open, &s, e);
        assert_eq!(event.id.as_deref(), Some("1995008"));
        assert_eq!(event.section.as_deref(), Some("waiting"));
        assert_eq!(event.tag.as_deref(), Some("ni?"));
        assert_eq!(
            event.age_days,
            Some(4),
            "needinfo set 2026-10-30T10:00Z, four and a half days before"
        );
        assert_eq!((event.at, event.kind), (now, Kind::Open));
    }

    #[test]
    fn a_damaged_line_is_skipped_and_events_round_trip() {
        let event = item(0, Kind::Copy, "D1", "r?");
        let line = serde_json::to_string(&event).unwrap();
        let text = format!("{line}\nnot json\n{{\"at\":\"2026-11-24T00:00:00Z\",\"kind\":\"later\"}}\n{line}\n");
        assert_eq!(parse(&text), [event.clone(), event]);
        assert!(!line.contains("age_days"), "empty fields stay out of the file: {line}");
    }

    #[test]
    fn a_key_is_the_whole_chord_and_old_single_letter_lines_still_parse() {
        let old = r#"{"at":"2026-11-01T00:00:00Z","kind":"done","via":"key","key":"d"}"#;
        let chord = r#"{"at":"2026-11-01T00:00:01Z","kind":"open","via":"key","key":"gb"}"#;
        let events = parse(&format!("{old}\n{chord}\n"));
        let keys: Vec<Option<&str>> = events.iter().map(|e| e.key.as_deref()).collect();
        assert_eq!(keys, [Some("d"), Some("gb")]);
        let s = summarize(&events, "2026-11-02T00:00:00Z".parse().unwrap(), 14);
        assert_eq!(
            (count(&s.keys, "gb"), count(&s.keys, "d"), count(&s.keys, "g")),
            (Some(1), Some(1), None)
        );
        let line = serde_json::to_string(&events[1]).unwrap();
        assert!(line.contains(r#""key":"gb""#), "{line}");
    }
}
