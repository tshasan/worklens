use std::path::PathBuf;

use jiff::{SignedDuration, Timestamp};
use proptest::strategy::{Strategy as _, ValueTree as _};
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use worklens_core::CoreConfig;
use worklens_core::marks::Marks;
use worklens_core::raw::Raw;
use worklens_core::section::Section;
use worklens_testkit::{Mutation, fixture, mutated, mutations, screen_problems};
use worklens_tui::app::Mode;
use worklens_tui::{App, Msg, Options, update, view};

const USAGE: &str = "\
Drive the TUI headlessly and print each frame.

cargo run -q -p worklens-tui --example drive -- [OPTIONS] [STEPS...]

Options:
  --cache NAME     fixture in crates/store/fixtures (raw.json, busy.json,
                   bugzilla_only.json) or a path; default raw.json
  --size WxH       terminal size; default 80x24
  --mutate SEED    damage the cache with testkit mutations from this seed
  --whiteboard TAG team whiteboard tag, as if set in Settings
  --section ID     start on this section (waiting, land, returned, uplift,
                   others, sprint, ideas, drift, other, team)
  --last           print only the final frame

Steps: up down left right enter esc tab backtab pgup pgdn home end space
bksp, any single character (? / s q d z ...), ctrl+X, resize:WxH, tick,
fail:TEXT (a refresh fails), fetch:NAME (a refresh returns that fixture).

Lines starting with !! are rule breaks found by testkit::screen_problems.
Effects (open a URL, copy, run the agent) are printed, never performed.
The clock is fixed at 2026-11-03T20:00Z. Sources checks and action
feedback come from the binary; judge those through the CLI.";

fn now() -> Timestamp {
    "2026-11-03T20:00:00Z".parse().unwrap()
}

fn load(name: &str, seed: Option<u64>) -> (Raw, Vec<Mutation>) {
    let value: Value = if name.contains('/') {
        serde_json::from_slice(&std::fs::read(name).unwrap_or_else(|e| panic!("{name}: {e}"))).unwrap()
    } else {
        fixture(&format!("crates/store/fixtures/{name}"))
    };
    let ms = seed.map_or_else(Vec::new, |seed| {
        let mut bytes = [0u8; 32];
        for (i, b) in seed.to_le_bytes().iter().cycle().take(32).enumerate() {
            bytes[i] = *b;
        }
        let mut runner = TestRunner::new_with_rng(Config::default(), TestRng::from_seed(RngAlgorithm::ChaCha, &bytes));
        mutations(8).new_tree(&mut runner).unwrap().current()
    });
    match serde_json::from_value(mutated(&value, &ms)) {
        Ok(raw) => (raw, ms),
        Err(e) => {
            println!("the mutated cache doesn't parse ({e}); worklens would refetch. Try another seed.");
            std::process::exit(2);
        }
    }
}

fn size(text: &str) -> (u16, u16) {
    let (w, h) = text.split_once('x').unwrap_or_else(|| panic!("size {text:?} isn't WxH"));
    (w.parse().unwrap(), h.parse().unwrap())
}

fn key(step: &str) -> Option<KeyEvent> {
    let plain = |code| Some(KeyEvent::new(code, KeyModifiers::NONE));
    match step {
        "up" => plain(KeyCode::Up),
        "down" => plain(KeyCode::Down),
        "left" => plain(KeyCode::Left),
        "right" => plain(KeyCode::Right),
        "enter" => plain(KeyCode::Enter),
        "esc" => plain(KeyCode::Esc),
        "tab" => plain(KeyCode::Tab),
        "backtab" => plain(KeyCode::BackTab),
        "pgup" => plain(KeyCode::PageUp),
        "pgdn" => plain(KeyCode::PageDown),
        "home" => plain(KeyCode::Home),
        "end" => plain(KeyCode::End),
        "space" => plain(KeyCode::Char(' ')),
        "bksp" => plain(KeyCode::Backspace),
        _ => {
            if let Some(c) = step.strip_prefix("ctrl+").and_then(|c| c.chars().next()) {
                return Some(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
            }
            let mut chars = step.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => plain(KeyCode::Char(c)),
                _ => None,
            }
        }
    }
}

fn msg(step: &str) -> Msg {
    if let Some(k) = key(step) {
        return Msg::Key(k);
    }
    if let Some(s) = step.strip_prefix("resize:") {
        let (w, h) = size(s);
        return Msg::Resize(w, h);
    }
    if let Some(text) = step.strip_prefix("fail:") {
        return Msg::Fetched(Err(text.to_owned()));
    }
    if let Some(name) = step.strip_prefix("fetch:") {
        return Msg::Fetched(Ok(Box::new(load(name, None).0)));
    }
    if step == "tick" {
        return Msg::Tick;
    }
    eprintln!("unknown step {step:?}\n\n{USAGE}");
    std::process::exit(2);
}

fn row(buf: &ratatui::buffer::Buffer, y: u16, w: u16) -> String {
    let mut out = String::new();
    let mut x = 0;
    while x < w {
        let symbol = buf[(x, y)].symbol();
        out.push_str(symbol);
        x += u16::try_from(unicode_width::UnicodeWidthStr::width(symbol).max(1)).unwrap_or(1);
    }
    out.trim_end().to_owned()
}

fn frame(app: &App, label: &str) {
    let (w, h) = app.size;
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| view(app, f)).unwrap();
    let buf = terminal.backend().buffer();
    let screen: String = (0..h).map(|y| row(buf, y, w) + "\n").collect();
    println!("── {label} · {:?} · {w}x{h}", app.mode);
    print!("{screen}");
    if let (Mode::List, Some(s)) = (app.mode, &app.snapshot) {
        let off: Vec<&str> = ["jira", "phabricator"]
            .into_iter()
            .filter(|source| {
                let mut errors = s.raw.errors.iter().filter(|e| e.source == *source).peekable();
                errors.peek().is_some() && errors.all(|e| e.unset)
            })
            .collect();
        let failed = s.raw.errors.iter().any(|e| !e.unset) || app.error.is_some() || app.action_error.is_some();
        for problem in screen_problems(&screen, &off, failed) {
            println!("!! {problem}");
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut cache, mut dims, mut seed, mut whiteboard, mut last) = ("raw.json".to_owned(), (80, 24), None, None, false);
    let mut section: Option<String> = None;
    let mut steps = Vec::new();
    while let Some(arg) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| panic!("{arg} needs a value"));
        match arg.as_str() {
            "--cache" => cache = value(),
            "--size" => dims = size(&value()),
            "--mutate" => seed = Some(value().parse().expect("a number")),
            "--whiteboard" => whiteboard = Some(value()),
            "--section" => section = Some(value()),
            "--last" => last = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            _ => steps.push(arg),
        }
    }
    let (raw, ms) = load(&cache, seed);
    if !ms.is_empty() {
        println!("mutations: {ms:?}");
    }
    let opts = Options {
        cache_minutes: None,
        agent: None,
        dir: PathBuf::from("/nonexistent/worklens-drive"),
        keys: Vec::new(),
    };
    let cfg = CoreConfig {
        whiteboard: whiteboard.clone(),
        ..CoreConfig::default()
    };
    let mut app = App::new(cfg, opts, Some(raw), Marks::default(), now());
    if let Some(tag) = whiteboard {
        let mut prefs = app.prefs.clone();
        prefs.whiteboard = tag;
        app.set_prefs(prefs);
    }
    let mut t = now();
    update(&mut app, Msg::Resize(dims.0, dims.1), t);
    if let Some(id) = section {
        for _ in 0..Section::COUNT {
            if app.section().id() == id {
                break;
            }
            update(&mut app, Msg::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)), t);
        }
        if app.section().id() != id {
            println!("no {id} section with work in this cache; sections are hidden while empty");
            std::process::exit(2);
        }
    }
    if !last || steps.is_empty() {
        frame(&app, "start");
    }
    for (i, step) in steps.iter().enumerate() {
        t += SignedDuration::from_secs(1);
        let effects = update(&mut app, msg(step), t);
        if !last || i + 1 == steps.len() {
            frame(&app, step);
            for effect in effects {
                println!(">> {effect:?}");
            }
        }
    }
}
