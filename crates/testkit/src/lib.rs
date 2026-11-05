use std::sync::OnceLock;

use proptest::prelude::*;
use serde_json::{Number, Value};

pub const NASTY: &[&str] = &[
    "",
    " ",
    "\n",
    "\t\t",
    "\r\n",
    "\u{0}",
    "\u{1b}[31mred\u{1b}[0m",
    "\u{7}\u{8}",
    "e\u{301}\u{301}\u{301}",
    "👩‍👩‍👧‍👦",
    "中文字符的宽度很重要",
    "\u{202e}desrever",
    "\u{200b}\u{200d}\u{feff}",
    "a\u{0}b",
    "D",
    "D0",
    "D99999999999999999999",
    "bug 0",
    "-1",
    "0",
    "4294967296",
    "NaN",
    "null",
    "1970-01-01T00:00:00Z",
    "9999-12-31T23:59:59Z",
    "-009999-01-01T00:00:00Z",
    "2026-02-30",
    "PROJ-",
    "-0",
    "[[[[",
    "%s%n",
    "      leading and trailing      ",
];

#[derive(Clone, Debug)]
pub enum Edit {
    Same(usize),
    Text(String),
    Null,
    Retype(usize),
    Remove,
    Duplicate,
    Empty,
}

#[derive(Clone, Debug)]
pub struct Mutation {
    pub at: usize,
    pub edit: Edit,
}

pub fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => prop::sample::select(NASTY).prop_map(str::to_owned),
        1 => "(?s).{0,40}",
        1 => prop::collection::vec(prop::sample::select(NASTY), 1..4).prop_map(|parts| parts.concat()),
        1 => (1usize..3000).prop_map(|n| "wide界".repeat(n / 5 + 1)),
    ]
}

pub fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        6 => any::<usize>().prop_map(Edit::Same),
        3 => text().prop_map(Edit::Text),
        1 => Just(Edit::Null),
        1 => any::<usize>().prop_map(Edit::Retype),
        2 => Just(Edit::Remove),
        2 => Just(Edit::Duplicate),
        1 => Just(Edit::Empty),
    ]
}

pub fn mutations(max: usize) -> impl Strategy<Value = Vec<Mutation>> {
    prop::collection::vec((any::<usize>(), edit()).prop_map(|(at, edit)| Mutation { at, edit }), 0..=max)
}

pub fn mutate(value: &mut Value, mutations: &[Mutation]) {
    for m in mutations {
        let mut at = m.at % count(value);
        edit_at(value, &mut at, &m.edit);
    }
}

pub fn mutated(value: &Value, mutations: &[Mutation]) -> Value {
    let mut value = value.clone();
    mutate(&mut value, mutations);
    value
}

pub fn fixture(path: &str) -> Value {
    let full = format!("{}/../../{path}", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&full).unwrap_or_else(|e| panic!("{full}: {e}"));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{full}: {e}"))
}

pub fn count(v: &Value) -> usize {
    1 + match v {
        Value::Array(items) => items.iter().map(count).sum(),
        Value::Object(map) => map.values().map(count).sum(),
        _ => 0,
    }
}

fn edit_at(v: &mut Value, at: &mut usize, edit: &Edit) -> bool {
    if *at == 0 {
        replace(v, edit);
        return true;
    }
    *at -= 1;
    match v {
        Value::Array(items) => {
            for i in 0..items.len() {
                if *at == 0 {
                    match edit {
                        Edit::Remove => {
                            items.remove(i);
                            return true;
                        }
                        Edit::Duplicate => {
                            let copy = items[i].clone();
                            items.insert(i, copy);
                            return true;
                        }
                        _ => {}
                    }
                }
                if edit_at(&mut items[i], at, edit) {
                    return true;
                }
            }
        }
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if *at == 0 && matches!(edit, Edit::Remove) {
                    map.remove(&key);
                    return true;
                }
                if let Some(child) = map.get_mut(&key)
                    && edit_at(child, at, edit)
                {
                    return true;
                }
            }
        }
        _ => {}
    }
    false
}

fn replace(v: &mut Value, edit: &Edit) {
    *v = match edit {
        Edit::Same(i) => same_kind(v, *i),
        Edit::Text(s) => Value::String(s.clone()),
        Edit::Null | Edit::Remove => Value::Null,
        Edit::Retype(i) => other_kind(*i),
        Edit::Duplicate => match v {
            Value::String(s) => Value::String(s.repeat(3)),
            Value::Array(items) => Value::Array(items.iter().chain(items.iter()).cloned().collect()),
            _ => v.clone(),
        },
        Edit::Empty => match v {
            Value::Array(_) => Value::Array(Vec::new()),
            Value::Object(_) => Value::Object(serde_json::Map::new()),
            Value::String(_) => Value::String(String::new()),
            Value::Number(_) => Value::from(0),
            _ => Value::Null,
        },
    };
}

fn number(i: usize) -> Value {
    match i % 12 {
        0 => Value::from(0),
        1 => Value::from(-1),
        2 => Value::from(1),
        3 => Value::from(u32::MAX),
        4 => Value::from(u64::from(u32::MAX) + 1),
        5 => Value::from(i64::MAX),
        6 => Value::from(i64::MIN),
        7 => Value::from(u64::MAX),
        8 => Number::from_f64(0.5).map_or(Value::Null, Value::Number),
        9 => Number::from_f64(-0.0).map_or(Value::Null, Value::Number),
        10 => Number::from_f64(1e300).map_or(Value::Null, Value::Number),
        _ => Value::from(i32::MAX),
    }
}

fn same_kind(v: &Value, i: usize) -> Value {
    match v {
        Value::String(_) => Value::String(NASTY[i % NASTY.len()].to_owned()),
        Value::Number(_) => number(i),
        Value::Bool(b) => Value::Bool(!b),
        Value::Array(items) if !items.is_empty() => match i % 3 {
            0 => Value::Array(items.iter().rev().cloned().collect()),
            1 => Value::Array(vec![items[0].clone()]),
            _ => Value::Array(std::iter::repeat_n(items[0].clone(), 8).collect()),
        },
        _ => v.clone(),
    }
}

fn other_kind(i: usize) -> Value {
    match i % 5 {
        0 => Value::Bool(true),
        1 => number(i / 5),
        2 => Value::String(NASTY[(i / 5) % NASTY.len()].to_owned()),
        3 => Value::Array(Vec::new()),
        _ => Value::Object(serde_json::Map::new()),
    }
}

pub fn config(cases: u32) -> ProptestConfig {
    let default = ProptestConfig::default();
    ProptestConfig {
        cases: if std::env::var_os("PROPTEST_CASES").is_some() {
            default.cases
        } else {
            cases
        },
        failure_persistence: Some(Box::new(prop::test_runner::FileFailurePersistence::WithSource("regressions"))),
        ..default
    }
}

pub const CACHES: &[&str] = &[
    "crates/store/fixtures/raw.json",
    "crates/store/fixtures/busy.json",
    "crates/store/fixtures/bugzilla_only.json",
];

pub const WHITEBOARD: &str = "[aiplatform]";

pub fn caches() -> &'static [Value] {
    static LOADED: OnceLock<Vec<Value>> = OnceLock::new();
    LOADED.get_or_init(|| CACHES.iter().map(|p| fixture(p)).collect())
}

pub fn cache() -> impl Strategy<Value = &'static Value> {
    (0..CACHES.len()).prop_map(|i| &caches()[i])
}

pub const FAILURE_CLAIMS: &[&str] = &["couldn't be read", "wasn't read"];

pub fn screen_problems(screen: &str, off: &[&str], failed: bool) -> Vec<String> {
    let mut problems = Vec::new();
    if !failed {
        for claim in FAILURE_CLAIMS.iter().filter(|c| screen.contains(*c)) {
            problems.push(format!("says {claim:?} though nothing failed"));
        }
    }
    for paragraph in paragraphs(screen) {
        for source in off {
            let named = paragraph.replace(&format!("auth set {source}"), "");
            if named.contains(source) && !named.contains("set up") {
                problems.push(format!("names {source}, which isn't set up: {}", paragraph.trim()));
            }
        }
    }
    problems
}

fn paragraphs(screen: &str) -> Vec<String> {
    let mut done = Vec::new();
    let mut open: Vec<String> = Vec::new();
    for line in screen.lines() {
        let lower = line.to_lowercase();
        if !lower.contains('│') {
            done.append(&mut open);
            done.push(lower);
            continue;
        }
        let cells: Vec<&str> = lower.split('│').map(str::trim).collect();
        open.resize(open.len().max(cells.len()), String::new());
        for (i, cell) in cells.iter().enumerate() {
            let ended = open[i].is_empty() || open[i].ends_with(['.', '…', ':', '?', '!']);
            if cell.is_empty() || ended {
                done.push(std::mem::take(&mut open[i]));
            }
            if !open[i].is_empty() {
                open[i].push(' ');
            }
            open[i].push_str(cell);
        }
    }
    done.append(&mut open);
    done.retain(|p| !p.is_empty());
    done
}
