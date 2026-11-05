use std::path::PathBuf;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use worklens_core::CoreConfig;
use worklens_core::marks::Marks;
use worklens_core::raw::Raw;
use worklens_testkit::fixture;
use worklens_tui::{App, Msg, Options, update, view};

const ITEMS: u32 = 1500;
const BUDGET: Duration = Duration::from_millis(16);

fn now() -> Timestamp {
    "2026-11-03T20:00:00Z".parse().unwrap()
}

fn big() -> Raw {
    let mut v = fixture("crates/store/fixtures/raw.json");
    let bug = v["bugs"][0].clone();
    let rev = v["revisions"][0].clone();
    let mut bugs = Vec::new();
    let mut revs = Vec::new();
    let mut links = serde_json::Map::new();
    for i in 0..ITEMS {
        let id = 1_000_000 + i;
        let mut b = bug.clone();
        b["id"] = json!(id);
        b["summary"] = json!(format!("Item {i} with a title long enough to need truncating in the list pane"));
        if i % 3 == 0 {
            b["flags"] = json!([]);
        }
        bugs.push(b);
        let mut r = rev.clone();
        r["id"] = json!(id);
        r["bug"] = json!(id);
        r["status"] = json!(["needs-review", "accepted", "changes-planned"][i as usize % 3]);
        revs.push(r);
        links.insert(id.to_string(), json!([id]));
    }
    v["bugs"] = Value::Array(bugs);
    v["revisions"] = Value::Array(revs);
    v["bugRevisions"] = Value::Object(links);
    serde_json::from_value(v).unwrap()
}

fn app(w: u16, h: u16) -> App {
    let opts = Options {
        cache_minutes: None,
        agent: None,
        dir: PathBuf::from("/nonexistent/worklens-budget"),
        keys: Vec::new(),
    };
    let mut app = App::new(CoreConfig::default(), opts, Some(big()), Marks::default(), now());
    update(&mut app, Msg::Resize(w, h), now());
    app
}

fn slowest(app: &mut App, keys: &[KeyCode]) -> (Duration, KeyCode) {
    let (w, h) = app.size;
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut worst = (Duration::ZERO, KeyCode::Null);
    for &k in keys {
        let start = Instant::now();
        update(app, Msg::Key(KeyEvent::new(k, KeyModifiers::NONE)), now());
        terminal.draw(|f| view(app, f)).unwrap();
        let took = start.elapsed();
        if took > worst.0 {
            worst = (took, k);
        }
    }
    worst
}

#[test]
#[ignore = "timing; run with --profile hunt -- --ignored"]
fn every_keypress_redraws_within_a_frame() {
    let mut keys = Vec::new();
    keys.extend(std::iter::repeat_n(KeyCode::Down, 200));
    keys.extend([KeyCode::End, KeyCode::Home, KeyCode::PageDown, KeyCode::PageUp]);
    keys.extend([KeyCode::Tab, KeyCode::Tab, KeyCode::Tab, KeyCode::BackTab]);
    keys.extend((1..=9).map(|d| KeyCode::Char(char::from(b'0' + d))));
    keys.extend([KeyCode::Char('/'), KeyCode::Char('i'), KeyCode::Char('t'), KeyCode::Esc]);
    keys.extend([KeyCode::Char('?'), KeyCode::Esc, KeyCode::Enter, KeyCode::Esc]);
    for (w, h) in [(80, 24), (220, 60)] {
        let mut a = app(w, h);
        let (took, key) = slowest(&mut a, &keys);
        println!("{w}x{h}: slowest key {key:?} took {took:?}");
        assert!(took < BUDGET, "{key:?} took {took:?} at {w}x{h} with {ITEMS} items");
    }
}

#[test]
#[ignore = "timing; run with --profile hunt -- --ignored"]
fn a_refresh_lands_within_a_frame() {
    let mut a = app(160, 48);
    let start = Instant::now();
    update(&mut a, Msg::Fetched(Ok(Box::new(big()))), now());
    let took = start.elapsed();
    println!("refresh with {ITEMS} items took {took:?}");
    assert!(took < BUDGET * 4, "applying a fetch took {took:?}");
}
