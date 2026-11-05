use std::path::PathBuf;

use jiff::{SignedDuration, Timestamp};
use proptest::prelude::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use worklens_core::CoreConfig;
use worklens_core::marks::Marks;
use worklens_core::raw::Raw;
use worklens_testkit::{Mutation, WHITEBOARD, cache, mutated, mutations, screen_problems, text};
use worklens_tui::app::Mode;
use worklens_tui::{App, Check, Msg, Options, update, view};

#[derive(Clone, Debug)]
enum Step {
    Key(KeyCode, KeyModifiers),
    Resize(u16, u16),
    Tick,
    Focus(bool),
    Progress(String),
    Fetched(&'static Value, Vec<Mutation>),
    FetchFailed(String),
    Done(bool, String),
    Checks(Vec<(bool, bool, String)>),
}

fn now() -> Timestamp {
    "2026-11-03T20:00:00Z".parse().unwrap()
}

fn raw(seed: &Value, ms: &[Mutation]) -> Option<Raw> {
    serde_json::from_value(mutated(seed, ms)).ok()
}

fn code() -> impl Strategy<Value = KeyCode> {
    prop_oneof![
        4 => prop::sample::select(vec![
            KeyCode::Up, KeyCode::Down, KeyCode::Left, KeyCode::Right, KeyCode::Enter, KeyCode::Esc,
            KeyCode::Tab, KeyCode::BackTab, KeyCode::Backspace, KeyCode::Delete, KeyCode::Home,
            KeyCode::End, KeyCode::PageUp, KeyCode::PageDown, KeyCode::F(1),
        ]),
        6 => (b' '..=b'~').prop_map(|b| KeyCode::Char(char::from(b))),
        1 => prop::sample::select(vec!['é', '界', '\u{301}', '👩', '\u{0}', '\u{1b}']).prop_map(KeyCode::Char),
    ]
}

fn size() -> impl Strategy<Value = (u16, u16)> {
    prop_oneof![
        1 => (0u16..6, 0u16..6),
        3 => (6u16..60, 4u16..20),
        4 => (60u16..200, 20u16..60),
    ]
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        30 => (code(), prop::sample::select(vec![KeyModifiers::NONE, KeyModifiers::SHIFT, KeyModifiers::CONTROL]))
            .prop_map(|(c, m)| Step::Key(c, m)),
        3 => size().prop_map(|(w, h)| Step::Resize(w, h)),
        3 => Just(Step::Tick),
        1 => any::<bool>().prop_map(Step::Focus),
        1 => text().prop_map(Step::Progress),
        2 => (cache(), mutations(8)).prop_map(|(seed, ms)| Step::Fetched(seed, ms)),
        1 => text().prop_map(Step::FetchFailed),
        1 => (any::<bool>(), text()).prop_map(|(ok, t)| Step::Done(ok, t)),
        1 => prop::collection::vec((any::<bool>(), any::<bool>(), text()), 0..6).prop_map(Step::Checks),
    ]
}

fn start() -> impl Strategy<Value = (&'static Value, Vec<Mutation>, (u16, u16), Vec<Step>)> {
    (cache(), mutations(8), size(), prop::collection::vec(step(), 1..60))
}

fn app(seed: &Value, ms: &[Mutation], (w, h): (u16, u16)) -> App {
    let opts = Options {
        cache_minutes: None,
        agent: None,
        dir: PathBuf::from("/nonexistent/worklens-robust"),
        keys: Vec::new(),
    };
    let cfg = CoreConfig {
        whiteboard: Some(WHITEBOARD.into()),
        ..CoreConfig::default()
    };
    let mut app = App::new(cfg, opts, raw(seed, ms), Marks::default(), now());
    update(&mut app, Msg::Resize(w, h), now());
    app
}

fn msg(step: &Step) -> Option<Msg> {
    Some(match step {
        Step::Key(c, m) => Msg::Key(KeyEvent::new(*c, *m)),
        Step::Resize(w, h) => Msg::Resize(*w, *h),
        Step::Tick => Msg::Tick,
        Step::Focus(b) => Msg::Focus(*b),
        Step::Progress(t) => Msg::Progress(t.clone()),
        Step::Fetched(seed, ms) => Msg::Fetched(Ok(Box::new(raw(seed, ms)?))),
        Step::FetchFailed(t) => Msg::Fetched(Err(t.clone())),
        Step::Done(true, t) => Msg::Done(Ok(t.clone())),
        Step::Done(false, t) => Msg::Done(Err(t.clone())),
        Step::Checks(cs) => Msg::Checks(
            cs.iter()
                .enumerate()
                .map(|(i, (ok, pending, detail))| Check {
                    name: format!("check {i}"),
                    ok: *ok,
                    required: i % 2 == 0,
                    detail: detail.clone(),
                    fix: (!ok).then(|| detail.clone()),
                    pending: *pending,
                })
                .collect(),
        ),
    })
}

fn honest(app: &App, screen: &str) -> Result<(), TestCaseError> {
    let (Mode::List, Some(s)) = (app.mode, &app.snapshot) else {
        return Ok(());
    };
    let off: Vec<&str> = ["jira", "phabricator"]
        .into_iter()
        .filter(|source| {
            let mut errors = s.raw.errors.iter().filter(|e| e.source == *source).peekable();
            errors.peek().is_some() && errors.all(|e| e.unset)
        })
        .collect();
    let failed = s.raw.errors.iter().any(|e| !e.unset) || app.error.is_some() || app.action_error.is_some();
    let problems = screen_problems(screen, &off, failed);
    prop_assert!(problems.is_empty(), "{problems:#?}\n{screen}");
    Ok(())
}

fn draw(app: &App) -> Result<(), TestCaseError> {
    let (w, h) = app.size;
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| view(app, f)).unwrap();
    let buf = terminal.backend().buffer();
    let screen: String = (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>() + "\n")
        .collect();
    honest(app, &screen)?;
    for y in 0..h {
        for x in 0..w {
            let symbol = buf[(x, y)].symbol();
            prop_assert!(
                !symbol.chars().any(char::is_control),
                "control character {:?} drawn at {},{} in mode {:?}",
                symbol,
                x,
                y,
                app.mode
            );
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(worklens_testkit::config(64))]

    #[test]
    fn any_input_sequence_draws_cleanly((seed, ms, size, steps) in start()) {
        let mut app = app(seed, &ms, size);
        draw(&app)?;
        let mut t = now();
        for step in &steps {
            t += SignedDuration::from_secs(7);
            if let Some(m) = msg(step) {
                update(&mut app, m, t);
            }
            if let Step::Resize(w, h) = step {
                prop_assert_eq!(app.size, (*w, *h));
            }
            draw(&app)?;
        }
    }
}
