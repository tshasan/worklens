use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use worklens_core::rules;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Down,
    Up,
    PageDown,
    PageUp,
    Top,
    Bottom,
    NextSection,
    PrevSection,
    Section(usize),
    NextPane,
    PrevPane,
    Menu,
    Open,
    Undo,
    Filter,
    ToggleHidden,
    Refresh,
    Setup,
    Log,
    Help,
    Back,
    Quit,
    Prefix(Group),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Next,
    Go,
    Copy,
    Item,
}

impl Group {
    pub const PREFIXED: [Self; 2] = [Self::Go, Self::Copy];

    pub const fn prefix(self) -> Option<char> {
        match self {
            Self::Go => Some('g'),
            Self::Copy => Some('y'),
            Self::Next | Self::Item => None,
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::Next => "Next step",
            Self::Go => "Go",
            Self::Copy => "Copy",
            Self::Item => "Item",
        }
    }

    pub fn of_prefix(c: char) -> Option<Self> {
        Self::PREFIXED.into_iter().find(|g| g.prefix() == Some(c))
    }
}

#[derive(Clone, Copy)]
enum K {
    Char(char),
    Code(KeyCode),
    Ctrl(char),
    Twice(char),
    Digit,
}

pub struct Binding {
    keys: &'static [K],
    pub label: &'static str,
    pub help: &'static str,
    pub hint: Option<&'static str>,
    pub rank: u8,
    action: Action,
}

use K::{Char, Code, Ctrl};

pub const BINDINGS: &[Binding] = &[
    Binding {
        keys: &[Code(KeyCode::Down), Char('j')],
        label: "↑ ↓",
        help: "Move",
        hint: Some("↑↓ move"),
        rank: 1,
        action: Action::Down,
    },
    Binding {
        keys: &[Code(KeyCode::Up), Char('k')],
        label: "",
        help: "",
        hint: None,
        rank: 0,
        action: Action::Up,
    },
    Binding {
        keys: &[Code(KeyCode::Right), Char('l')],
        label: "← →",
        help: "Switch section",
        hint: Some("←→ section"),
        rank: 3,
        action: Action::NextSection,
    },
    Binding {
        keys: &[Code(KeyCode::Left), Char('h')],
        label: "",
        help: "",
        hint: None,
        rank: 0,
        action: Action::PrevSection,
    },
    Binding {
        keys: &[K::Digit],
        label: "",
        help: "",
        hint: None,
        rank: 0,
        action: Action::Section(0),
    },
    Binding {
        keys: &[Code(KeyCode::Tab)],
        label: "Tab",
        help: "Switch detail tab",
        hint: Some("Tab detail"),
        rank: 9,
        action: Action::NextPane,
    },
    Binding {
        keys: &[Code(KeyCode::BackTab)],
        label: "",
        help: "",
        hint: None,
        rank: 0,
        action: Action::PrevPane,
    },
    Binding {
        keys: &[Code(KeyCode::PageDown), Ctrl('d')],
        label: "PgUp PgDn",
        help: "Scroll the detail",
        hint: None,
        rank: 0,
        action: Action::PageDown,
    },
    Binding {
        keys: &[Code(KeyCode::PageUp), Ctrl('u')],
        label: "",
        help: "",
        hint: None,
        rank: 0,
        action: Action::PageUp,
    },
    Binding {
        keys: &[Code(KeyCode::Home), K::Twice('g')],
        label: "",
        help: "",
        hint: None,
        rank: 0,
        action: Action::Top,
    },
    Binding {
        keys: &[Code(KeyCode::End), Char('G')],
        label: "",
        help: "",
        hint: None,
        rank: 0,
        action: Action::Bottom,
    },
    Binding {
        keys: &[Code(KeyCode::Enter)],
        label: "Enter",
        help: "Open the next step",
        hint: Some("Enter open"),
        rank: 4,
        action: Action::Open,
    },
    Binding {
        keys: &[Char('o')],
        label: "",
        help: "",
        hint: None,
        rank: 0,
        action: Action::Open,
    },
    Binding {
        keys: &[Char('g')],
        label: "",
        help: "",
        hint: Some("g go"),
        rank: 6,
        action: Action::Prefix(Group::Go),
    },
    Binding {
        keys: &[Char('y')],
        label: "",
        help: "",
        hint: Some("y copy"),
        rank: 7,
        action: Action::Prefix(Group::Copy),
    },
    Binding {
        keys: &[Char(' ')],
        label: "Space",
        help: "Every key for the item",
        hint: Some("Space actions"),
        rank: 5,
        action: Action::Menu,
    },
    Binding {
        keys: &[Char('/')],
        label: "/",
        help: "Filter",
        hint: Some("/ filter"),
        rank: 8,
        action: Action::Filter,
    },
    Binding {
        keys: &[Char('Z')],
        label: "Z",
        help: "Show done and snoozed",
        hint: Some("Z hidden"),
        rank: 14,
        action: Action::ToggleHidden,
    },
    Binding {
        keys: &[Char('u')],
        label: "",
        help: "",
        hint: None,
        rank: 0,
        action: Action::Undo,
    },
    Binding {
        keys: &[Char('r')],
        label: "r",
        help: "Refresh",
        hint: Some("r refresh"),
        rank: 12,
        action: Action::Refresh,
    },
    Binding {
        keys: &[Char('s')],
        label: "s",
        help: "Sources and settings",
        hint: Some("s sources"),
        rank: 13,
        action: Action::Setup,
    },
    Binding {
        keys: &[Char('@')],
        label: "@",
        help: "Request log",
        hint: None,
        rank: 0,
        action: Action::Log,
    },
    Binding {
        keys: &[Char('?')],
        label: "?",
        help: "Help",
        hint: Some("? help"),
        rank: 2,
        action: Action::Help,
    },
    Binding {
        keys: &[Code(KeyCode::Esc)],
        label: "",
        help: "",
        hint: None,
        rank: 0,
        action: Action::Back,
    },
    Binding {
        keys: &[Char('q')],
        label: "q",
        help: "Quit",
        hint: None,
        rank: 0,
        action: Action::Quit,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Bugzilla,
    Jira,
    Phabricator,
    Lando,
    Try,
    Landing,
    Link,
    Id,
    Brief,
    ReviewPrompt,
    DriftFix,
    Done,
    Snooze,
    Agent,
    Checkout,
}

pub struct ItemKey {
    pub verb: Verb,
    pub group: Group,
    pub key: char,
    pub help: &'static str,
}

const fn item(verb: Verb, group: Group, key: char, help: &'static str) -> ItemKey {
    ItemKey { verb, group, key, help }
}

pub const ITEM_KEYS: &[ItemKey] = &[
    item(Verb::Bugzilla, Group::Go, 'b', "Bugzilla"),
    item(Verb::Jira, Group::Go, 'j', "Jira"),
    item(Verb::Phabricator, Group::Go, 'p', "Phabricator"),
    item(Verb::Lando, Group::Go, 'l', "Lando"),
    item(Verb::Try, Group::Go, 't', "Try push"),
    item(Verb::Landing, Group::Go, 'a', "Landing push"),
    item(Verb::Link, Group::Go, '1', "Other links"),
    item(Verb::Id, Group::Copy, 'y', "ID"),
    item(Verb::Brief, Group::Copy, 'b', "Agent brief"),
    item(Verb::ReviewPrompt, Group::Copy, 'r', "Review prompt"),
    item(Verb::DriftFix, Group::Copy, 'f', "Drift fix"),
    item(Verb::Done, Group::Item, 'd', "Done; u undoes"),
    item(Verb::Snooze, Group::Item, 'z', "Snooze; u undoes"),
    item(Verb::Agent, Group::Item, 'c', "Coding agent"),
    item(Verb::Checkout, Group::Item, 'C', "Check out"),
];

pub const LINK_KEYS: [char; 9] = ['1', '2', '3', '4', '5', '6', '7', '8', '9'];

pub const fn item_key(verb: Verb) -> &'static ItemKey {
    &ITEM_KEYS[verb as usize]
}

pub fn chord(group: Group, key: char) -> String {
    group.prefix().map_or_else(|| key.to_string(), |p| format!("{p}{key}"))
}

pub fn chord_of(verb: Verb) -> String {
    let k = item_key(verb);
    chord(k.group, k.key)
}

pub fn item_letters() -> impl Iterator<Item = char> {
    ITEM_KEYS.iter().filter(|k| k.group == Group::Item).map(|k| k.key)
}

pub fn built_in() -> String {
    RESERVED
        .iter()
        .copied()
        .filter(|c| *c != ' ')
        .chain(item_letters())
        .chain('0'..='9')
        .collect()
}

pub const MARK_GRID: &[(&str, &str)] = &[
    ("✓", "try passed"),
    ("✗", "try failed"),
    ("○", "try running"),
    ("~", "old try pass"),
    ("⚑", "drift"),
    ("•", "new activity"),
    ("!", "critical"),
    ("*", "important"),
];

pub const LEGEND_LEN: usize = rules::TAGS.len() + MARK_GRID.len();

pub fn legend(i: usize) -> Option<(&'static str, &'static str)> {
    match rules::TAGS.get(i) {
        Some(f) => Some((f.tag, f.gloss)),
        None => MARK_GRID.get(i - rules::TAGS.len()).copied(),
    }
}

pub const RESERVED: &[char] = &['o', 'q', 'r', 's', 'u', '?', '/', 'Z', 'j', 'k', 'h', 'l', 'g', 'y', 'G', '@', ' '];

pub const ITEM_HINTS: &[(&str, &str, u8)] = &[("d", "done", 10), ("z", "snooze", 11)];

pub fn action_for(key: KeyEvent) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    BINDINGS.iter().find_map(|b| {
        b.keys.iter().find_map(|k| match (*k, key.code) {
            (Char(c), KeyCode::Char(got)) if !ctrl && c == got => Some(b.action),
            (Ctrl(c), KeyCode::Char(got)) if ctrl && c == got => Some(b.action),
            (Code(c), got) if c == got => Some(b.action),
            (K::Digit, KeyCode::Char(d @ '1'..='9')) if !ctrl => Some(Action::Section(d as usize - '1' as usize)),
            (K::Digit, KeyCode::Char('0')) if !ctrl => Some(Action::Section(9)),
            _ => None,
        })
    })
}

pub const SEP: &str = " · ";

pub fn list_hints() -> String {
    let hints: Vec<String> = footer(usize::MAX).iter().map(|(k, l)| format!("{k} {l}")).collect();
    hints.join(SEP)
}

pub fn vim_keys() -> String {
    let keys: Vec<String> = BINDINGS
        .iter()
        .filter(|b| b.keys.iter().any(|k| matches!(k, Code(_))))
        .flat_map(|b| b.keys.iter())
        .filter_map(|k| match k {
            Char(c) => Some(c.to_string()),
            Ctrl(c) => Some(format!("^{c}")),
            K::Twice(c) => Some(format!("{c}{c}")),
            _ => None,
        })
        .collect();
    keys.join(" ")
}

pub fn footer(width: usize) -> Vec<(&'static str, &'static str)> {
    let mut all: Vec<(u8, usize, &str, &str)> = BINDINGS
        .iter()
        .filter_map(|b| b.hint.map(|h| (b.rank, h)))
        .map(|(rank, h)| {
            let (k, l) = h.split_once(' ').unwrap_or((h, ""));
            (rank, k, l)
        })
        .chain(ITEM_HINTS.iter().map(|(k, l, rank)| (*rank, *k, *l)))
        .enumerate()
        .map(|(n, (rank, k, l))| (rank, n, k, l))
        .collect();
    all.sort_by_key(|(rank, n, _, _)| (*rank, *n));
    let mut used = 0;
    let mut kept: Vec<(usize, &str, &str)> = Vec::new();
    for (_, n, k, l) in all {
        let cells = k.chars().count() + 1 + l.chars().count() + if kept.is_empty() { 0 } else { SEP.chars().count() };
        if used + cells > width {
            break;
        }
        used += cells;
        kept.push((n, k, l));
    }
    kept.sort_by_key(|(n, _, _)| *n);
    kept.into_iter().map(|(_, k, l)| (k, l)).collect()
}

pub const MENU_HINTS: &[(&str, &str)] = &[("↑↓", "choose"), ("Enter or key", "run"), ("Esc", "back")];
pub const SNOOZE_HINTS: &[(&str, &str)] = &[("Enter", "save"), ("Esc", "cancel")];
pub const FILTER_HINTS: &[(&str, &str)] = &[("type", "to filter"), ("Enter", "keep"), ("Esc", "clear")];
pub const HELP_HINTS: &[(&str, &str)] = &[("Esc", "close"), ("q", "quit")];
pub const SETUP_HINTS: &[(&str, &str)] = &[
    ("PgUp PgDn", "scroll"),
    ("Tab", "settings"),
    ("r", "retry"),
    ("Esc", "back"),
    ("q", "quit"),
];
pub const SETTINGS_HINTS: &[(&str, &str)] = &[("Tab", "sources"), ("Esc", "back")];
pub const EDIT_HINTS: &[(&str, &str)] = &[("Enter", "save"), ("Esc", "cancel")];
pub const LOG_HINTS: &[(&str, &str)] = &[("↑↓", "scroll"), ("Esc", "back")];
pub const DETAIL_SCROLL_HINTS: &[(&str, &str)] = &[("PgUp PgDn", "scroll")];
pub const HELP_SCROLL_HINTS: &[(&str, &str)] = &[("↑↓", "scroll")];
pub const MENU_SCROLL_HINTS: &[(&str, &str)] = &[("PgUp PgDn", "read")];
pub const NO_CACHE_HINTS: &[(&str, &str)] = &[("r", "fetch"), ("s", "sources"), ("?", "help"), ("q", "quit")];

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    fn press(code: KeyCode) -> Option<Action> {
        action_for(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn keys_map_to_actions() {
        assert_eq!(press(KeyCode::Char('j')), Some(Action::Down));
        assert_eq!(press(KeyCode::Char('3')), Some(Action::Section(2)));
        assert_eq!(press(KeyCode::Char('9')), Some(Action::Section(8)));
        assert_eq!(press(KeyCode::Char('0')), Some(Action::Section(9)));
        assert_eq!(press(KeyCode::Char('g')), Some(Action::Prefix(Group::Go)));
        assert_eq!(press(KeyCode::Char('y')), Some(Action::Prefix(Group::Copy)));
        assert_eq!(press(KeyCode::Home), Some(Action::Top));
        assert_eq!(press(KeyCode::Char('G')), Some(Action::Bottom));
        for retired in ['B', 'J', 'P', 'L', 'T', 'b', 'R', 'F'] {
            assert_eq!(press(KeyCode::Char(retired)), None, "{retired} is retired");
        }
        assert_eq!(
            action_for(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Some(Action::PageDown)
        );
    }

    #[test]
    fn the_item_table_is_indexed_by_verb() {
        for (n, k) in ITEM_KEYS.iter().enumerate() {
            assert_eq!(k.verb as usize, n, "{:?} is out of place", k.verb);
        }
        assert_eq!(chord_of(Verb::Bugzilla), "gb");
        assert_eq!(chord_of(Verb::Landing), "ga");
        assert_eq!(chord_of(Verb::Id), "yy");
        assert_eq!(chord_of(Verb::Brief), "yb");
        assert_eq!(chord_of(Verb::ReviewPrompt), "yr");
        assert_eq!(chord_of(Verb::DriftFix), "yf");
        assert_eq!(chord_of(Verb::Done), "d");
        assert_eq!(chord_of(Verb::Checkout), "C");
    }

    #[test]
    fn no_two_item_keys_share_a_chord_and_prefixes_are_reserved() {
        let chords: Vec<String> = ITEM_KEYS.iter().map(|k| chord(k.group, k.key)).collect();
        for (n, c) in chords.iter().enumerate() {
            assert!(!chords[n + 1..].contains(c), "{c} twice");
        }
        for g in Group::PREFIXED {
            assert!(RESERVED.contains(&g.prefix().unwrap()), "{g:?}");
        }
        for c in item_letters() {
            assert!(!RESERVED.contains(&c) && press(KeyCode::Char(c)).is_none(), "{c} is taken");
        }
        let taken = built_in();
        for c in ['g', 'y', 'd', 'z', 'c', 'C', 'q', '1', '0'] {
            assert!(taken.contains(c), "{c} missing from {taken}");
        }
        assert!(!taken.contains('B') && !taken.contains(' '), "{taken}");
    }

    #[test]
    fn every_help_line_fits_its_column_at_80() {
        for b in BINDINGS {
            assert!(b.help.width() <= 27, "{} cells: {}", b.help.width(), b.help);
        }
        for k in ITEM_KEYS {
            assert!(k.help.width() <= 16, "{} cells: {}", k.help.width(), k.help);
        }
        for (tag, d) in (0..LEGEND_LEN).filter_map(legend) {
            assert!(tag.width() <= 3 && d.width() <= 14, "{tag} {d}");
        }
    }

    #[test]
    fn every_key_in_help_is_one_key() {
        for b in BINDINGS.iter().filter(|b| !b.label.is_empty()) {
            assert!(!b.help.contains(" / "), "one action per row: {}", b.help);
        }
        for k in ITEM_KEYS {
            assert_eq!(chord(k.group, k.key).chars().count(), 1 + usize::from(k.group.prefix().is_some()));
        }
    }

    #[test]
    fn vim_keys_come_from_the_table() {
        assert_eq!(vim_keys(), "j k l h ^d ^u gg G");
    }

    #[test]
    fn the_footer_comes_from_the_table() {
        assert_eq!(
            list_hints(),
            "↑↓ move · ←→ section · Tab detail · Enter open · g go · y copy · Space actions · / filter · Z hidden · r refresh · s sources · ? help · d done · z snooze"
        );
        assert_eq!(
            footer(78).iter().map(|(k, l)| format!("{k} {l}")).collect::<Vec<_>>().join(SEP),
            "↑↓ move · ←→ section · Enter open · g go · y copy · Space actions · ? help"
        );
    }
}
