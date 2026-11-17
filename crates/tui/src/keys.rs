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
}

#[derive(Clone, Copy)]
enum K {
    Char(char),
    Code(KeyCode),
    Ctrl(char),
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
        rank: 7,
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
        keys: &[Code(KeyCode::Home), Char('g')],
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
        keys: &[Char(' ')],
        label: "Space",
        help: "Actions for the item",
        hint: Some("Space actions"),
        rank: 5,
        action: Action::Menu,
    },
    Binding {
        keys: &[Char('/')],
        label: "/",
        help: "Filter",
        hint: Some("/ filter"),
        rank: 6,
        action: Action::Filter,
    },
    Binding {
        keys: &[Char('Z')],
        label: "Z",
        help: "Show done and snoozed",
        hint: Some("Z hidden"),
        rank: 12,
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
        rank: 10,
        action: Action::Refresh,
    },
    Binding {
        keys: &[Char('s')],
        label: "s",
        help: "Sources and settings",
        hint: Some("s sources"),
        rank: 11,
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

pub const ITEM_KEYS: &[(&str, &str)] = &[
    ("B", "Open in Bugzilla"),
    ("J", "Open in Jira"),
    ("P", "Open in Phabricator"),
    ("L", "Open in Lando"),
    ("T", "Open in Treeherder"),
    ("b", "Copy a brief for an agent"),
    ("y", "Copy the ID"),
    ("c", "Start your coding agent"),
    ("C", "Check out the revision"),
    ("R", "Copy a review prompt"),
    ("F", "Copy the drift fix"),
    ("d", "Done until new activity (u undoes)"),
    ("z", "Snooze until a date (u undoes)"),
];

pub const ITEM_LETTERS: &[char] = &['B', 'J', 'P', 'L', 'T', 'b', 'y', 'c', 'C', 'R', 'F', 'd', 'z'];

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

pub const RESERVED: &[char] = &['o', 'q', 'r', 's', 'u', '?', '/', 'Z', 'j', 'k', 'h', 'l', 'g', 'G', '@', ' '];

pub const ITEM_HINTS: &[(&str, &str, u8)] = &[("d", "done", 8), ("z", "snooze", 9)];

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
        assert_eq!(press(KeyCode::Char('B')), None, "item actions aren't list keys");
        assert_eq!(
            action_for(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Some(Action::PageDown)
        );
    }

    #[test]
    fn every_help_line_fits_its_column_at_80() {
        for b in BINDINGS {
            assert!(b.help.width() <= 27, "{} cells: {}", b.help.width(), b.help);
        }
        for (_, d) in ITEM_KEYS {
            assert!(d.width() <= 35, "{} cells: {d}", d.width());
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
        for (k, _) in ITEM_KEYS {
            assert_eq!(k.chars().count(), 1, "one letter per row: {k}");
        }
    }

    #[test]
    fn help_describes_every_built_in_item_letter() {
        let described: Vec<char> = ITEM_KEYS.iter().flat_map(|(k, _)| k.chars()).collect();
        for c in ITEM_LETTERS {
            assert!(described.contains(c), "{c} is an item action but not in help");
        }
    }

    #[test]
    fn vim_keys_come_from_the_table() {
        assert_eq!(vim_keys(), "j k l h ^d ^u g G");
    }

    #[test]
    fn the_footer_comes_from_the_table() {
        assert_eq!(
            list_hints(),
            "↑↓ move · ←→ section · Tab detail · Enter open · Space actions · / filter · Z hidden · r refresh · s sources · ? help · d done · z snooze"
        );
        assert_eq!(
            footer(80).iter().map(|(k, l)| format!("{k} {l}")).collect::<Vec<_>>().join(SEP),
            "↑↓ move · ←→ section · Enter open · Space actions · / filter · ? help"
        );
    }
}
