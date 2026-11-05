use std::path::PathBuf;

use jiff::Timestamp;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use worklens_core::CoreConfig;
use worklens_core::marks::Marks;
use worklens_core::raw::Raw;
use worklens_core::section::Section;
use worklens_tui::{App, Msg, Options, update, view};

fn now() -> Timestamp {
    "2026-11-03T20:00:00Z".parse().unwrap()
}

fn app(w: u16, h: u16) -> App {
    app_from("raw.json", w, h)
}

fn app_from(fixture: &str, w: u16, h: u16) -> App {
    let path = format!("{}/../store/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"));
    let raw: Raw = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let opts = Options {
        cache_minutes: None,
        agent: None,
        dir: PathBuf::from("/tmp"),
        keys: Vec::new(),
    };
    let mut app = App::new(CoreConfig::default(), opts, Some(raw), Marks::default(), now());
    update(&mut app, Msg::Resize(w, h), now());
    app
}

fn screen(app: &App) -> String {
    let (w, h) = app.size;
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| view(app, f)).unwrap();
    let buf = terminal.backend().buffer();
    (0..h)
        .map(|y| {
            let mut row = String::new();
            let mut x = 0;
            while x < w {
                let symbol = buf[(x, y)].symbol();
                row.push_str(symbol);
                x += u16::try_from(unicode_width::UnicodeWidthStr::width(symbol).max(1)).unwrap_or(1);
            }
            row.trim_end().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn at_the_end(s: &str, hint: &str) -> bool {
    let line = s.lines().find(|l| l.contains(hint)).expect("a scroll hint");
    let range = line.split(" lines").next().unwrap().rsplit("│ ").next().unwrap().trim();
    let (shown, total) = range.split_once(" of ").unwrap();
    shown.split_once('–').unwrap().1 == total
}

fn press(app: &mut App, code: KeyCode) {
    update(app, Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)), now());
}

#[test]
fn narrow_terminal_stacks_the_list_over_the_detail() {
    let s = screen(&app(80, 24));
    assert!(s.lines().next().unwrap().contains("worklens"), "{s}");
    assert!(s.contains("Waiting on you 1"), "full tab titles at 80 columns:\n{s}");
    assert!(s.contains("ni? "), "{s}");
    assert!(s.contains("1995008"), "{s}");
    assert!(
        s.contains("needinfo from a@mozilla.com, 4 days"),
        "the reason is in the detail:\n{s}"
    );
    assert!(s.lines().last().unwrap().contains("↑↓ move"), "{s}");
}

#[test]
fn wide_terminal_puts_them_side_by_side_with_full_tab_titles() {
    let s = screen(&app(170, 30));
    assert!(s.contains("Waiting on you 1"), "{s}");
    let tabs = line_with(&s, "Waiting on you 1");
    assert!(tabs.contains("Overview   Timeline"), "detail tabs share the section tab row:\n{s}");
    assert!(s.contains("D321987 · needs-review"), "{s}");
}

#[test]
fn help_fits_80x24() {
    let mut a = app(80, 24);
    press(&mut a, KeyCode::Char('?'));
    let s = screen(&a);
    assert!(s.lines().any(|l| l.contains("Keys") && l.contains("On the selected item")), "{s}");
    assert!(s.lines().any(|l| l.contains("Tab        Switch detail tab")), "{s}");
    assert!(s.contains("Keys · vim too: j k l h ^d ^u g G"), "{s}");
    assert!(s.lines().any(|l| l.contains("B  Open in Bugzilla")), "{s}");
    assert!(s.contains("*   important"), "the last help line still fits:\n{s}");
    assert!(!s.contains("scroll ·"), "{s}");
}

#[test]
fn the_telemetry_tab_shows_only_on_an_item_with_metrics() {
    let metrics = |bugs: serde_json::Value| {
        serde_json::from_value(serde_json::json!({
            "fetchedAt":"2026-11-03T19:00:00Z","metrics":[{
                "name":"example.counter","expires":"161","disabled":false,
                "notificationEmails":["me@mozilla.com"],"bugs":bugs,"dataReviews":[],
                "sourceUrl":"https://example.org/metrics.yaml","definitionLastSeen":"2026-11-02 00:00:00"
            }]
        }))
        .unwrap()
    };
    for (w, h) in [(80, 24), (140, 30)] {
        let mut a = app(w, h);
        let mut raw = a.snapshot.take().unwrap().raw;
        raw.telemetry = Some(metrics(serde_json::json!([42])));
        update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
        assert!(!screen(&a).contains("Telemetry"), "{}", screen(&a));
        assert!(!screen(&a).contains("Drift   "), "no drift, so no Drift tab:\n{}", screen(&a));
        for _ in 0..4 {
            press(&mut a, KeyCode::Tab);
        }
        assert!(
            screen(&a).contains("needinfo from"),
            "four tabs on wraps to Overview:\n{}",
            screen(&a)
        );

        let mut a = app(w, h);
        let mut raw = a.snapshot.take().unwrap().raw;
        raw.telemetry = Some(metrics(serde_json::json!([1995008])));
        update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
        assert!(screen(&a).contains("Telemetry"), "{}", screen(&a));
        for _ in 0..4 {
            press(&mut a, KeyCode::Tab);
        }
        let s = screen(&a);
        assert!(s.contains("name this bug") && s.contains("example.counter"), "{s}");
    }
}

#[test]
fn approval_channel_and_metric_evidence_remain_reachable_at_both_sizes() {
    for (w, h) in [(80, 24), (140, 30)] {
        let mut a = app(w, h);
        let mut raw = a.snapshot.take().unwrap().raw;
        raw.revisions[0].projects = Some(vec!["PHID-PROJ-data".into()]);
        raw.bugs[0].release_flags.insert("cf_status_firefox161".into(), "fixed".into());
        raw.users.insert("PHID-PROJ-data".into(), "#needs-data-classification".into());
        raw.telemetry = Some(
            serde_json::from_value(serde_json::json!({
                "fetchedAt":"2026-11-03T19:00:00Z","metrics":[{
                    "name":"example.counter","expires":"161","disabled":false,
                    "notificationEmails":["me@mozilla.com"],"bugs":[1995008],"dataReviews":[],
                    "sourceUrl":"https://example.org/metrics.yaml","definitionLastSeen":"2026-11-02 00:00:00"
                }]
            }))
            .unwrap(),
        );
        update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
        let mut seen = String::new();
        for _ in 0..30 {
            seen.push_str(&screen(&a));
            press(&mut a, KeyCode::PageDown);
        }
        assert!(
            seen.contains("classification required"),
            "classification evidence unreachable at {w}x{h}"
        );
        assert!(seen.contains("Bugzilla reports fixed"), "channel evidence unreachable at {w}x{h}");
        for _ in 0..4 {
            press(&mut a, KeyCode::Tab);
        }
        let mut seen = String::new();
        for _ in 0..10 {
            seen.push_str(&screen(&a));
            press(&mut a, KeyCode::PageDown);
        }
        assert!(seen.contains("example.counter"), "metric identity unreachable at {w}x{h}");
        assert!(
            seen.contains("Expired on current Nightly"),
            "expiry evidence unreachable at {w}x{h}"
        );
    }
}

#[test]
fn integration_evidence_and_menu_only_destination_fit_both_sizes() {
    for (w, h) in [(80, 24), (140, 30)] {
        let mut a = app(w, h);
        let mut raw = a.snapshot.take().unwrap().raw;
        raw.landings = Some(
            serde_json::from_value(serde_json::json!({
                "revisions":[321987],"repositories":[{
                    "repository":"autoland","checkedAt":"2026-11-03T19:00:00Z",
                    "pushLimit":100,"scannedPushes":1,"oldestPush":"2026-11-03T10:00:00Z","error":null,
                    "matches":[{"revision":321987,"commit":"1".repeat(40),"pushRevision":"2".repeat(40),
                        "url":format!("https://treeherder.mozilla.org/jobs?repo=autoland&revision={}", "2".repeat(40)),
                        "pushedAt":"2026-11-03T10:00:00Z","health":null,"healthError":"No jobs available"}]
                }]
            }))
            .unwrap(),
        );
        update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
        let mut seen = String::new();
        for _ in 0..25 {
            seen.push_str(&screen(&a));
            let previous = a.pane_scroll;
            press(&mut a, KeyCode::PageDown);
            if previous == a.pane_scroll {
                break;
            }
        }
        assert!(seen.contains(&"1".repeat(40)), "matched commit unreachable at {w}x{h}");
        assert!(seen.contains("No jobs"), "missing health must remain visible at {w}x{h}: {seen}");
        let at = a
            .actions()
            .iter()
            .position(|action| action.label.contains("autoland push"))
            .unwrap();
        press(&mut a, KeyCode::Enter);
        for _ in 0..at {
            press(&mut a, KeyCode::Down);
        }
        let s = screen(&a);
        assert!(s.contains("autoland push"), "{s}");
        assert!(!s.contains("[]"), "a menu-only action must not advertise a shortcut: {s}");
    }
}

#[test]
fn landing_guidance_and_the_first_action_target_the_same_requirement() {
    for (w, h) in [(80, 24), (140, 30)] {
        let mut a = landing_app(w, h);
        let mut raw = a.snapshot.take().unwrap().raw;
        raw.revisions[0].projects = Some(vec!["PHID-PROJ-data".into()]);
        raw.users.insert("PHID-PROJ-data".into(), "#needs-data-classification".into());
        update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
        assert_eq!(a.section(), Section::Waiting);
        let s = screen(&a);
        assert!(s.contains("Resolve data classification"), "{s}");
        assert!(a.actions()[0].label.contains("Resolve data classification"));
        press(&mut a, KeyCode::Enter);
        let effects = update(&mut a, Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)), now());
        assert!(matches!(&effects[..], [worklens_tui::Effect::Open(url)] if url.ends_with("/D321987")));
        let effects = update(&mut a, Msg::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)), now());
        assert!(matches!(&effects[..], [worklens_tui::Effect::Open(url)] if url.ends_with("/D321987")));
    }
}

#[test]
fn dependency_owner_and_destination_remain_reachable_at_both_sizes() {
    for (w, h) in [(80, 24), (140, 30)] {
        let mut a = app(w, h);
        let mut raw = a.snapshot.take().unwrap().raw;
        raw.bugs[0].depends_on = vec![worklens_core::BugId(123)];
        let mut bug: worklens_core::raw::Bug = serde_json::from_value(serde_json::to_value(&raw.bugs[0]).unwrap()).unwrap();
        bug.id = worklens_core::BugId(123);
        bug.assigned_to = "alice@mozilla.com".into();
        raw.dependencies.bugs.insert(bug.id, bug);
        update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
        let seen = paged(&mut a);
        assert!(seen.iter().any(|l| l.contains("Depends: bug 123")), "{seen:?}");
        assert!(seen.iter().any(|l| l.contains("alice@mozilla.com")), "{seen:?}");
        assert!(a.actions().iter().any(|a| a.label.contains("dependency bug 123")));
    }
}

#[test]
fn help_on_a_large_screen_is_centered_rows_with_a_tag_grid() {
    let mut a = app(140, 30);
    press(&mut a, KeyCode::Char('?'));
    let s = screen(&a);
    let keys = s.lines().find(|l| l.contains("Keys")).unwrap();
    assert!(keys.starts_with(&" ".repeat(20)), "centered, not stretched:\n{s}");
    assert!(!s.contains('╭'), "no boxes:\n{s}");
    assert!(s.lines().any(|l| l.contains("r?# group review")), "{s}");
}

#[test]
fn detail_evidence_is_a_table_by_source() {
    let s = screen(&app(140, 30));
    assert!(s.lines().any(|l| l.contains("Bugzilla     1995008 · ASSIGNED")), "{s}");
    assert!(s.lines().any(|l| l.contains("Phabricator  D321987 · needs-review")), "{s}");
}

fn landing_app(w: u16, h: u16) -> App {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/raw.json");
    let mut raw: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    for bug in raw["bugs"].as_array_mut().unwrap() {
        if bug["id"] == 1995008 {
            bug["flags"] = serde_json::json!([]);
        }
    }
    for rev in raw["revisions"].as_array_mut().unwrap() {
        if rev["id"] == 321987 {
            rev["status"] = "accepted".into();
            rev["reviewers"][0]["status"] = "accepted".into();
        }
    }
    raw["tryPushes"]["321987"]["builds"] = "pass".into();
    raw["tryPushes"]["321987"]["lint"] = "pass".into();
    raw["tryPushes"]["321987"]["tests"] = "pass".into();
    raw["tryPushes"]["321987"]["failed"] = 0.into();
    raw["tryPushes"]["321987"]["running"] = 0.into();
    raw["tryPushes"]["321987"]["pushedAt"] = "2026-11-03T19:00:00Z".into();
    let raw: Raw = serde_json::from_value(raw).unwrap();
    let opts = Options {
        cache_minutes: None,
        agent: None,
        dir: PathBuf::from("/tmp"),
        keys: Vec::new(),
    };
    let mut app = App::new(CoreConfig::default(), opts, Some(raw), Marks::default(), now());
    update(&mut app, Msg::Resize(w, h), now());
    app
}

fn paged(a: &mut App) -> std::collections::BTreeSet<String> {
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..10 {
        seen.extend(screen(a).lines().map(str::to_owned));
        press(a, KeyCode::PageDown);
    }
    seen
}

#[test]
fn a_needinfo_shows_no_unknown_approval_evidence() {
    for (w, h) in [(80, 24), (140, 30)] {
        let seen = paged(&mut app(w, h));
        assert!(!seen.iter().any(|l| l.contains("Approvals and fix presence")), "{seen:#?}");
        assert!(!seen.iter().any(|l| l.contains("fix presence unknown")), "{seen:#?}");
        assert!(!seen.iter().any(|l| l.contains("not fetched")), "{seen:#?}");
        assert!(!seen.iter().any(|l| l.contains("Release channels")), "{seen:#?}");
        assert!(seen.iter().any(|l| l.contains("waiting on quillfox")), "{seen:#?}");
    }
}

#[test]
fn evidence_labels_keep_a_gap_and_repeated_unknowns_collapse() {
    for (w, h) in [(80, 24), (140, 30)] {
        let mut a = landing_app(w, h);
        press(&mut a, KeyCode::Char('2'));
        assert_eq!(a.section(), Section::Land, "the item is ready to land");
        let seen = paged(&mut a);
        assert!(seen.iter().any(|l| l.contains("Approvals and fix presence")), "{seen:#?}");
        let label = seen.iter().find(|l| l.contains("D321987 data")).unwrap();
        assert!(!label.contains("…Project") && label.contains(" Project"), "{label}");
        assert!(seen.iter().any(|l| l.contains("Release channels")), "{seen:#?}");
        assert!(!seen.iter().any(|l| l.contains("fix presence unknown")), "{seen:#?}");
    }
}

#[test]
fn tabs_hide_empty_sections_and_underline_the_current_one() {
    let mut a = app(80, 24);
    a.tone = worklens_tui::Tone::Dark;
    let s = screen(&a);
    let tabs = s.lines().nth(1).unwrap().to_owned();
    assert!(tabs.contains("Waiting on you 1   Sprint 1"), "spaced apart: {tabs}");
    assert!(
        !tabs.contains("Land") && !tabs.contains("Team") && !tabs.contains("empty"),
        "{tabs}"
    );
    let border = s.lines().nth(2).unwrap();
    let start = tabs.find("Waiting").unwrap();
    let width = "Waiting on you 1".len();
    assert_eq!(
        border.chars().skip(start).take(width).collect::<String>(),
        "━".repeat(width),
        "\n{tabs}\n{border}"
    );
    assert_eq!(border.matches('━').count(), width, "{border}");
    press(&mut a, KeyCode::Char('0'));
    let tabs = screen(&a).lines().nth(1).unwrap().to_owned();
    assert!(tabs.contains("Team drift 0"), "{tabs}");
}

#[test]
fn the_menu_shows_numbered_actions() {
    let mut a = app(140, 30);
    press(&mut a, KeyCode::Enter);
    let s = screen(&a);
    assert!(s.contains("> 1 Open bug 1995008 in Bugzilla"), "{s}");
    assert!(s.contains("9 Done locally"), "{s}");
    let snooze = s.lines().find(|l| l.contains("Snooze locally until")).unwrap();
    assert!(
        snooze.contains("│   0 Snooze") && !snooze.contains("10"),
        "0 picks the tenth, as with sections: {s}"
    );
    assert!(snooze.ends_with(" z │"), "the key sits in a right-aligned column: {s}");
    let bugzilla = s.lines().find(|l| l.contains("Open bug 1995008 in Bugzilla")).unwrap();
    assert_eq!(bugzilla.chars().count(), snooze.chars().count(), "{s}");
    assert!(bugzilla.ends_with(" B │") && !bugzilla.contains("[B]"), "{s}");
    assert!(!s.contains("Action 1"), "a menu that fits doesn't repeat the footer: {s}");
    press(&mut a, KeyCode::Char('0'));
    assert!(screen(&a).contains("Snooze 1995008 locally:"), "0 runs the tenth action");
}

#[test]
fn long_menu_labels_wrap_under_themselves_within_80_columns() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/raw.json");
    let mut data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    data["issues"]["AIPLAT-1416"]["key"] = "FIREFOX-PLATFORM-123456789".into();
    let mut a = app(80, 24);
    update(&mut a, Msg::Fetched(Ok(Box::new(serde_json::from_value(data).unwrap()))), now());
    a.opts.keys.push(worklens_tui::CustomKey {
        key: 'x',
        label: "Run the long custom command that explains itself in many more words than fit".into(),
        run: "echo example".into(),
        terminal: false,
    });
    press(&mut a, KeyCode::Enter);
    let s = screen(&a);
    let fix = s
        .lines()
        .find(|l| l.contains("Open bug 1995008 and copy the See Also URL"))
        .unwrap();
    assert!(fix.ends_with(" F │"), "{s}");
    let at = s.lines().position(|l| l.contains("Run the long custom")).unwrap();
    assert!(s.lines().nth(at).unwrap().ends_with(" x │"), "the key is on the first line:\n{s}");
    let next = s.lines().nth(at + 1).unwrap();
    assert!(next.starts_with("│     ") && !next.starts_with("│      "), "a hanging indent:\n{s}");
    assert!(s.lines().all(|l| l.chars().count() <= 80), "{s}");
}

#[test]
fn before_the_first_fetch_nothing_claims_zero_work() {
    let mut a = app(80, 24);
    a.snapshot = None;
    a.rows.clear();
    a.busy = Some("Refreshing".into());
    let s = screen(&a);
    let tabs = s.lines().nth(1).unwrap();
    assert_eq!(
        tabs.trim(),
        "Waiting on you",
        "only the first tab, until counts say which others have work: {s}"
    );
    assert!(s.lines().nth(2).unwrap().starts_with("╭━━━━"), "it is underlined: {s}");
    assert_eq!(s.matches("Fetching").count(), 1, "one line says it's fetching:\n{s}");
    assert!(!s.contains("Select an item"), "nothing to select yet:\n{s}");
    a.busy = Some("Identifying you".into());
    let s = screen(&a);
    assert!(s.contains("Identifying you"), "the current step shows: {s}");
}

#[test]
fn an_empty_section_leaves_the_detail_blank() {
    let mut a = app(80, 24);
    press(&mut a, KeyCode::Right);
    let s = screen(&a);
    assert!(!s.contains("Select an item"), "{s}");
}

#[test]
fn the_merge_countdown_is_quiet_not_a_warning() {
    use ratatui::style::Color;
    let mut a = app(140, 30);
    a.tone = worklens_tui::Tone::Dark;
    let mut terminal = Terminal::new(TestBackend::new(140, 30)).unwrap();
    terminal.draw(|f| view(&a, f)).unwrap();
    let buf = terminal.backend().buffer();
    let header: String = (0..140).map(|x| buf[(x, 0)].symbol()).collect();
    let at = header.find("Beta merge").unwrap();
    let x = header[..at].chars().count() as u16;
    assert_eq!(buf[(x, 0)].fg, Color::Indexed(246), "quiet, like the cache age");
}

#[test]
fn help_lists_custom_keys_and_still_fits_80x24() {
    let mut a = app(80, 24);
    for (key, label) in [('S', "Searchfox"), ('x', "Run tests")] {
        a.opts.keys.push(worklens_tui::CustomKey {
            key,
            label: label.into(),
            run: "echo {{bug}}".into(),
            terminal: false,
        });
    }
    press(&mut a, KeyCode::Char('?'));
    let s = screen(&a);
    assert!(s.lines().any(|l| l.contains("S  Searchfox")), "{s}");
    assert!(s.lines().any(|l| l.contains("x  Run tests")), "{s}");
    assert!(!at_the_end(&s, "lines · ↑↓ scroll"), "{s}");
    for _ in 0..5 {
        press(&mut a, KeyCode::Down);
    }
    let s = screen(&a);
    assert!(
        s.contains("*   important") && at_the_end(&s, "lines · ↑↓ scroll"),
        "the last help line is reachable:\n{s}"
    );
    let mut wide = app(140, 30);
    wide.opts.keys = a.opts.keys.clone();
    press(&mut wide, KeyCode::Char('?'));
    assert!(screen(&wide).contains("x  Run tests"), "{}", screen(&wide));
}

#[test]
fn an_empty_section_points_at_the_next_one() {
    let mut a = app(80, 24);
    press(&mut a, KeyCode::Char('2'));
    let s = screen(&a);
    assert!(s.contains("None of your revisions are accepted"), "{s}");
}

#[test]
fn the_header_counts_down_to_the_merge() {
    let s = screen(&app(140, 30));
    assert!(s.lines().next().unwrap().contains("Beta merge in 2 days"), "{s}");
}

#[test]
fn decision_precedes_evidence_and_overview_reaches_the_end() {
    for (w, h) in [(80, 24), (140, 30)] {
        let mut a = app(w, h);
        let s = screen(&a);
        let title = s.find("Support view-source:").unwrap();
        let reason = s.find("needinfo from").unwrap();
        let next = s.find("Next: Answer the needinfo").unwrap();
        let evidence = s.find("1995008 · ASSIGNED").unwrap();
        assert!(title < reason && reason < next && next < evidence, "{s}");
        for _ in 0..20 {
            press(&mut a, KeyCode::PageDown);
        }
        let end = screen(&a);
        assert!(end.contains("Can you look at this?"), "{end}");
        let stopped = a.pane_scroll;
        press(&mut a, KeyCode::PageDown);
        assert_eq!(a.pane_scroll, stopped);
        for _ in 0..20 {
            press(&mut a, KeyCode::PageUp);
        }
        assert_eq!(a.pane_scroll, 0);
        assert_eq!(a.cursor(), 0, "paging reads details without changing selection");
    }
}

#[test]
fn empty_states_distinguish_filter_hidden_failure_and_first_load() {
    let mut a = app(80, 24);
    press(&mut a, KeyCode::Char('/'));
    for c in "absent".chars() {
        press(&mut a, KeyCode::Char(c));
    }
    press(&mut a, KeyCode::Enter);
    let filtered = screen(&a);
    assert!(filtered.contains("No matches for “absent”"), "{filtered}");
    assert!(filtered.contains("0 of 1 match"), "{filtered}");
    press(&mut a, KeyCode::Esc);
    press(&mut a, KeyCode::Char('d'));
    a.toast = None;
    let hidden = screen(&a);
    assert!(hidden.contains("1 items hidden locally"), "{hidden}");
    assert!(hidden.contains("Z shows"), "{hidden}");
    a.error = Some("Offline".into());
    press(&mut a, KeyCode::Right);
    let failed = screen(&a);
    assert!(failed.contains("Refresh failed"), "{failed}");
    assert!(!failed.contains("None of your revisions are accepted"), "{failed}");
    a.snapshot = None;
    a.rows.clear();
    a.busy = Some("Fetching Bugzilla".into());
    let loading = screen(&a);
    assert!(loading.contains("No cached work yet"), "{loading}");
}

#[test]
fn source_health_and_cache_age_survive_refresh_and_errors_are_readable() {
    let mut a = app(80, 24);
    let mut raw = a.snapshot.take().unwrap().raw;
    raw.errors.push(worklens_core::raw::SourceError {
        source: "Bugzilla".into(),
        message: format!("{} END-OF-ERROR", "Connection failed; check credentials. ".repeat(35)),
        unset: false,
    });
    update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
    press(&mut a, KeyCode::Char('r'));
    let s = screen(&a);
    let header = s.lines().next().unwrap();
    assert!(
        header.contains("Bugzilla") && header.contains("Updated") && header.contains("Refreshing"),
        "{s}"
    );
    press(&mut a, KeyCode::Char('s'));
    for _ in 0..30 {
        press(&mut a, KeyCode::PageDown);
    }
    let s = screen(&a);
    assert!(s.contains("END-OF-ERROR"), "{s}");
    let stopped = a.pane_scroll;
    press(&mut a, KeyCode::PageDown);
    assert_eq!(a.pane_scroll, stopped);
}

#[test]
fn menu_wraps_long_actions_and_reaches_the_last_action_without_bleed() {
    let mut a = app(80, 24);
    a.opts.keys.push(worklens_tui::CustomKey {
        key: 'x',
        label: format!("{} END-OF-ACTION", "A descriptive custom command ".repeat(60)),
        run: "echo example".into(),
        terminal: false,
    });
    press(&mut a, KeyCode::Enter);
    for _ in 0..30 {
        press(&mut a, KeyCode::Down);
    }
    for _ in 0..30 {
        press(&mut a, KeyCode::PageDown);
    }
    let s = screen(&a);
    assert!(s.contains("END-OF-ACTION"), "{s}");
    assert!(
        s.contains("Action 11 of 11 · PgUp PgDn read"),
        "a scrolling menu says where you are: {s}"
    );
    assert!(!s.contains("Support view-source:"), "the full-body overlay clears the list: {s}");
    assert!(s.contains("Esc back"), "{s}");
    press(&mut a, KeyCode::Esc);
    assert!(screen(&a).contains("Next: Answer the needinfo"));
}

#[test]
fn selection_has_a_marker_and_reverse_video_without_relying_on_hue() {
    use ratatui::style::Modifier;
    let a = app(80, 24);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| view(&a, f)).unwrap();
    let buf = terminal.backend().buffer();
    assert_eq!(buf[(1, 3)].symbol(), ">");
    assert!(buf[(25, 3)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn on_a_known_dark_background_selection_is_a_raised_band_and_structure_recedes() {
    use ratatui::style::{Color, Modifier};
    let mut a = app(80, 24);
    a.tone = worklens_tui::Tone::Dark;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| view(&a, f)).unwrap();
    let buf = terminal.backend().buffer();
    assert_eq!(buf[(1, 3)].symbol(), "▌");
    let title = &buf[(25, 3)];
    assert_eq!(title.bg, Color::Indexed(237));
    assert!(!title.modifier.contains(Modifier::REVERSED));
    assert!(title.modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(3, 3)].fg, Color::Reset, "age on the selected row");
    assert_eq!(buf[(0, 2)].fg, Color::Indexed(240));
    assert_eq!(buf[(1, 1)].symbol(), "W");
    assert!(!buf[(1, 1)].modifier.contains(Modifier::REVERSED));
    assert!(buf[(1, 1)].modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(1, 1)].fg, Color::Cyan);
    let tabs: Vec<_> = (0..80).map(|x| &buf[(x, 1)]).collect();
    assert!(
        tabs.iter()
            .all(|c| c.bg == Color::Reset && !c.modifier.contains(Modifier::REVERSED)),
        "no tab shares the selected row's band"
    );
    assert_eq!(buf[(1, 23)].symbol(), "↑");
    assert_eq!(buf[(1, 23)].fg, Color::Reset);
    assert_eq!(buf[(4, 23)].symbol(), "m");
    assert_eq!(buf[(4, 23)].fg, Color::Indexed(246));
}

#[test]
fn without_color_status_and_selection_still_read() {
    use ratatui::style::{Color, Modifier};
    let mut a = app(80, 24);
    a.tone = worklens_tui::Tone::NoColor;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| view(&a, f)).unwrap();
    let buf = terminal.backend().buffer();
    assert_eq!(buf[(1, 3)].symbol(), ">");
    assert!(buf[(25, 3)].modifier.contains(Modifier::REVERSED));
    let hues = (0..24)
        .flat_map(|y| (0..80).map(move |x| (x, y)))
        .filter(|p| !matches!(buf[*p].fg, Color::Reset) || !matches!(buf[*p].bg, Color::Reset))
        .count();
    assert_eq!(hues, 0, "NO_COLOR draws no colors at all");
}

#[test]
fn small_terminals_degrade_without_panicking() {
    for (w, h) in [(1, 1), (20, 8), (79, 23)] {
        let mut a = app(w, h);
        for key in [KeyCode::PageDown, KeyCode::Enter, KeyCode::Down, KeyCode::Esc, KeyCode::Char('s')] {
            press(&mut a, key);
            let _ = screen(&a);
        }
    }
}

#[test]
fn concurrent_failures_do_not_push_freshness_or_busy_state_out_of_the_header() {
    let mut a = app(80, 24);
    let mut raw = a.snapshot.take().unwrap().raw;
    for source in ["Bugzilla", "Jira", "Phabricator", "Treeherder"] {
        raw.errors.push(worklens_core::raw::SourceError {
            source: source.into(),
            message: "Offline".into(),
            unset: false,
        });
    }
    update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
    a.error = Some("Refresh failed".into());
    a.action_error = Some("Open failed".into());
    a.busy = Some("Retrying".into());
    let s = screen(&a);
    let header = s.lines().next().unwrap();
    for fact in ["worklens", "Updated", "Refreshing", "s Sources", "failed"] {
        assert!(header.contains(fact), "{s}");
    }
}

#[test]
fn snooze_keeps_its_prompt_recovery_and_cancel_visible() {
    let mut a = app(80, 24);
    press(&mut a, KeyCode::Char('z'));
    assert!(screen(&a).contains("Enter alone: until new activity"));
    for c in "not-a-date".chars() {
        press(&mut a, KeyCode::Char(c));
    }
    press(&mut a, KeyCode::Enter);
    let s = screen(&a);
    assert!(s.contains("Date not understood"), "{s}");
    assert!(s.contains("> not-a-date"), "{s}");
    assert!(s.contains("Esc cancel"), "{s}");
    press(&mut a, KeyCode::Esc);
    assert!(screen(&a).contains("Next: Answer the needinfo"));
}

#[test]
fn what_changed_shows_in_overview_and_stays_while_reading() {
    let mut a = app(140, 30);
    let mut raw = a.snapshot.take().unwrap().raw;
    raw.revisions[0].reviewers[0].status = worklens_core::raw::ReviewerStatus::Accepted;
    update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
    let s = screen(&a);
    assert!(
        s.contains("Since you looked:") && s.contains("D321987: quillfox blocking → accepted"),
        "{s}"
    );

    update(&mut a, Msg::Tick, now());
    let later = now() + jiff::SignedDuration::from_secs(1);
    update(&mut a, Msg::Tick, later);
    assert!(!a.selected().unwrap().marks.is_new, "looking at it marked it seen");
    assert!(screen(&a).contains("D321987: quillfox blocking → accepted"), "{}", screen(&a));

    let raw = a.snapshot.take().unwrap().raw;
    update(&mut a, Msg::Fetched(Ok(Box::new(raw))), later);
    assert!(!screen(&a).contains("Since you looked:"), "{}", screen(&a));
}

#[test]
fn the_drift_title_shows_the_trend_and_a_fetch_records_today() {
    let mut a = app(80, 24);
    let today = now().to_zoned(jiff::tz::TimeZone::system()).date();
    let five_days_ago = today.checked_sub(jiff::Span::new().days(5)).unwrap();
    a.drift_log.record(five_days_ago, 3, 0);
    press(&mut a, KeyCode::Char('8'));
    let s = screen(&a);
    let on = five_days_ago.strftime("%b %d").to_string();
    let title = s.lines().find(|l| l.contains("down from")).unwrap_or_default();
    assert!(title.starts_with('╰') && title.contains(&format!("down from 3 on {on}")), "{s}");

    let raw = a.snapshot.take().unwrap().raw;
    let effects = update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
    let [worklens_tui::app::Effect::RecordDrift(day)] = effects.as_slice() else {
        panic!("{effects:?}")
    };
    assert_eq!((day.date, day.mine), (today, 0));
    assert!(screen(&a).contains("down from 3"), "{}", screen(&a));
}

fn landed_app(w: u16, h: u16) -> App {
    let mut a = landing_app(w, h);
    let mut raw = a.snapshot.take().unwrap().raw;
    let rev = raw.revisions.iter_mut().find(|r| r.id.0 == 321987).unwrap();
    rev.reviewers[0].status = worklens_core::raw::ReviewerStatus::Accepted;
    let bug = raw.bugs.iter_mut().find(|b| b.id.0 == 1995008).unwrap();
    bug.release_flags.insert("cf_status_firefox161".into(), "fixed".into());
    bug.release_flags.insert("cf_status_firefox160".into(), "affected".into());
    bug.release_flags.insert("cf_status_firefox159".into(), "---".into());
    bug.release_flags.insert("cf_status_firefox_esr153".into(), "unaffected".into());
    let id = bug.id;
    raw.uplift.insert(
        id,
        worklens_core::raw::Uplift {
            patch: true,
            approvals: vec![("beta".into(), "?".into())],
            flags: Vec::new(),
        },
    );
    raw.landings = Some(
        serde_json::from_value(serde_json::json!({
            "revisions":[321987],"repositories":[{
                "repository":"autoland","checkedAt":"2026-11-03T19:00:00Z",
                "pushLimit":100,"scannedPushes":1,"oldestPush":"2026-11-03T10:00:00Z","error":null,
                "matches":[{"revision":321987,"commit":"1".repeat(40),"pushRevision":"2".repeat(40),
                    "url":"https://treeherder.mozilla.org/jobs?repo=autoland","pushedAt":"2026-11-03T10:00:00Z",
                    "health":null,"healthError":"No jobs available"}]
            }]
        }))
        .unwrap(),
    );
    update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
    press(&mut a, KeyCode::Char('2'));
    a
}

fn detail(screen: &str, wide: bool) -> Vec<String> {
    screen
        .lines()
        .filter_map(|l| {
            let l = if wide { l.split("││").nth(1)? } else { l };
            Some(l.trim_matches(|c: char| c == '│' || c == ' ').to_owned())
        })
        .collect()
}

#[test]
fn review_and_try_strips_sit_under_their_revision() {
    for (w, h) in [(80, 40), (140, 30)] {
        let lines = detail(&screen(&app(w, h)), w >= 110);
        let at = |prefix: &str| {
            lines
                .iter()
                .position(|l| l.starts_with(prefix))
                .unwrap_or_else(|| panic!("{prefix} at {w}x{h}: {lines:#?}"))
        };
        let (rev, reviewers, tries) = (at("Phabricator  D321987"), at("Reviewers"), at("Try"));
        assert!(rev < reviewers && reviewers < tries, "{lines:#?}");
        assert!(lines[reviewers].contains("waiting on quillfox (blocking)"), "{lines:#?}");
        assert!(lines[tries].contains("builds passed · lint passed · tests failed"), "{lines:#?}");
    }
}

#[test]
fn the_release_matrix_follows_the_decision_and_keeps_every_field_at_any_width() {
    for (w, h) in [(80, 24), (110, 30), (140, 30)] {
        let mut a = landed_app(w, h);
        let first = detail(&screen(&a), w >= 110);
        let next = first.iter().position(|l| l.starts_with("Next: Land D321987"));
        assert!(next.is_some(), "the decision is on the first screen at {w}x{h}: {first:#?}");
        let seen = paged(&mut a);
        let lines: Vec<String> = seen.iter().flat_map(|l| detail(l, w >= 110)).collect();
        assert!(lines.iter().any(|l| l.contains("Release channels")), "{lines:#?}");
        for word in [
            "Nightly 161",
            "Beta 160",
            "Release 159",
            "ESR 153",
            "ESR 115",
            "Reported",
            "Uplift",
            "Landing",
        ] {
            assert!(lines.iter().any(|l| l.contains(word)), "{word} at {w}x{h}: {lines:#?}");
        }
        for word in ["fixed", "affected", "not set", "unaffected", "requested", "autoland", "not checked"] {
            assert!(lines.iter().any(|l| l.contains(word)), "{word} at {w}x{h}: {lines:#?}");
        }
        assert!(!lines.iter().any(|l| l.contains("Bugzilla reports")), "{lines:#?}");
        assert!(!lines.iter().any(|l| l.contains("beta uplift")), "{lines:#?}");
        assert!(lines.iter().any(|l| l.contains(&"1".repeat(20))), "{lines:#?}");
    }
}

#[test]
fn the_release_matrix_has_channels_as_columns_when_they_fit() {
    let lines = detail(&screen(&landed_app(80, 50)), false);
    let header = lines
        .iter()
        .position(|l| l.starts_with("Nightly 161  Beta 160"))
        .expect("a header of channels");
    assert!(lines[header + 1].starts_with("Reported     fixed        affected"), "{lines:#?}");
    assert!(lines[header + 2].starts_with("Uplift       —            requested"), "{lines:#?}");
    assert!(lines[header + 3].starts_with("Landing      autoland     not checked"), "{lines:#?}");
}

#[test]
fn a_narrow_detail_pivots_the_matrix_to_a_row_per_channel() {
    let lines = detail(&screen(&landed_app(110, 50)), true);
    let header = lines.iter().position(|l| l.starts_with("Reported")).expect("a header of fields");
    assert!(lines[header].contains("Uplift") && lines[header].contains("Landing"), "{lines:#?}");
    assert!(lines[header + 1].starts_with("Nightly 161  fixed"), "{lines:#?}");
    assert!(lines[header + 5].starts_with("ESR 115"), "{lines:#?}");
}

fn line_with<'a>(screen: &'a str, text: &str) -> &'a str {
    screen
        .lines()
        .find(|l| l.contains(text))
        .unwrap_or_else(|| panic!("no {text:?} in\n{screen}"))
}

#[test]
fn settings_is_a_tab_beside_sources_and_fits_80x24() {
    let mut a = app(80, 24);
    press(&mut a, KeyCode::Char('s'));
    assert!(line_with(&screen(&a), "Sources   Settings").contains("Tab ⇥"));
    press(&mut a, KeyCode::Tab);
    let s = screen(&a);
    assert!(
        line_with(&s, "Whiteboard").contains(">Whiteboard"),
        "the cursor starts on the first row:\n{s}"
    );
    assert!(line_with(&s, "Sprint budget").contains("10 pts"), "{s}");
    assert!(line_with(&s, "Drift window").contains("30 days"), "{s}");
    assert!(!s.contains("changed"), "defaults are not marked:\n{s}");
    assert!(
        s.contains("1–18 of 39 lines · PgUp PgDn scroll"),
        "the rest is below the fold and says so:\n{s}"
    );
    assert!(
        s.lines().last().unwrap().contains("Enter edit"),
        "the footer says what Enter does here:\n{s}"
    );
    assert!(s.lines().all(|l| l.chars().count() <= 80), "{s}");

    for _ in 0..3 {
        press(&mut a, KeyCode::Down);
    }
    let s = screen(&a);
    assert!(
        line_with(&s, ">Landing checks").contains("‹ on ›"),
        "the choice looks pickable:\n{s}"
    );
    assert!(s.lines().last().unwrap().contains("Enter → off · ←→ choose"), "{s}");
    press(&mut a, KeyCode::Right);
    let row = line_with(&screen(&a), "Landing checks").to_owned();
    assert!(row.contains("‹ off ›") && row.ends_with("changed │"), "{row}");
    update(&mut a, Msg::Done(Ok("Landing checks off; refreshing".into())), now());
    let s = screen(&a);
    assert!(line_with(&s, "Landing checks off; refreshing").contains("PgUp PgDn scroll"), "{s}");
    assert!(
        s.lines().last().unwrap().contains("Esc back"),
        "the save message leaves the keys:\n{s}"
    );

    press(&mut a, KeyCode::End);
    let s = screen(&a);
    assert!(line_with(&s, ">Telemetry").contains("shown when a metric names the bug"), "{s}");
    assert!(s.contains("custom keys: ~/.config/worklens.json"), "the end is reachable:\n{s}");
}

#[test]
fn a_typed_setting_shows_a_caret_and_bad_input_keeps_the_prompt() {
    let mut a = app(80, 24);
    press(&mut a, KeyCode::Char('s'));
    press(&mut a, KeyCode::Tab);
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Enter);
    assert!(line_with(&screen(&a), "Sprint budget").contains("10▏"), "{}", screen(&a));
    assert!(screen(&a).lines().last().unwrap().contains("Esc cancel"));
    press(&mut a, KeyCode::Backspace);
    press(&mut a, KeyCode::Backspace);
    press(&mut a, KeyCode::Char('x'));
    press(&mut a, KeyCode::Enter);
    let s = screen(&a);
    assert!(line_with(&s, "Sprint budget").contains("x▏"), "still editing:\n{s}");
    assert!(s.contains("number of points"), "the reason shows in the page:\n{s}");
    assert!(s.lines().last().unwrap().contains("Esc cancel"), "and the keys stay:\n{s}");
    press(&mut a, KeyCode::Esc);
    assert!(line_with(&screen(&a), "Sprint budget").contains("10 pts"));
}

#[test]
fn a_hidden_section_leaves_the_tabs_but_keeps_its_count_in_sight() {
    let mut a = app(80, 24);
    assert!(screen(&a).contains("Sprint 1"), "{}", screen(&a));
    let mut prefs = a.prefs.clone();
    prefs.sections[Section::Sprint.index()] = worklens_tui::Show::Hidden;
    a.set_prefs(prefs);
    let s = screen(&a);
    let tabs = s.lines().nth(1).unwrap();
    assert!(tabs.starts_with(" Waiting on you 1") && !tabs.contains("   Sprint 1"), "{s}");
    assert!(tabs.ends_with("hidden: Sprint 1"), "{s}");
    for _ in 0..Section::COUNT {
        press(&mut a, KeyCode::Right);
        assert_eq!(a.section(), Section::Waiting, "arrows skip the hidden section");
    }
}

#[test]
fn the_selected_setting_reads_without_a_known_background() {
    let mut a = app(80, 24);
    press(&mut a, KeyCode::Char('s'));
    press(&mut a, KeyCode::Tab);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| view(&a, f)).unwrap();
    let buf = terminal.backend().buffer();
    let y = (0..24)
        .find(|y| (0..80).map(|x| buf[(x, *y)].symbol()).collect::<String>().contains(">Whiteboard"))
        .unwrap();
    assert!(
        buf[(5, y)].modifier.contains(ratatui::style::Modifier::REVERSED),
        "reverse video like the list"
    );
}

fn app_with_idea(w: u16, h: u16) -> App {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/raw.json");
    let mut raw: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    raw["issues"]["AIPLAT-1416"]["parent"] = "AIPLAT-1040".into();
    raw["roadmap"] = serde_json::json!({
        "parents": { "AIPLAT-1040": {
            "key": "AIPLAT-1040", "summary": "Example epic: text extraction", "status": "In Progress",
            "statusCategory": "indeterminate", "assignee": null, "type": "Epic",
            "updated": "2026-11-01T12:00:00Z", "points": null, "sprint": null, "ideas": ["AIDI-79"],
        }},
        "ideas": { "AIDI-79": {
            "key": "AIDI-79", "summary": "Example idea: page extraction", "status": "Developing",
            "statusCategory": "indeterminate", "assignee": "pm@mozilla.com", "updated": "2026-11-03T09:00:00Z",
            "weeklyStatus": "Dev on track", "target": "Fx158.0.1 (Oct 20)", "quarter": "Q4 | 2026",
        }},
    });
    let raw: Raw = serde_json::from_value(raw).unwrap();
    let opts = Options {
        cache_minutes: None,
        agent: None,
        dir: PathBuf::from("/tmp"),
        keys: Vec::new(),
    };
    let mut app = App::new(CoreConfig::default(), opts, Some(raw), Marks::default(), now());
    update(&mut app, Msg::Resize(w, h), now());
    app
}

#[test]
fn the_ideas_tab_shows_the_idea_and_the_work_under_it_at_80x24() {
    let mut a = app_with_idea(80, 24);
    assert!(!line_with(&screen(&a), "Waiting on you").contains("AIDI"), "the idea is not a turn");
    press(&mut a, KeyCode::Char('7'));
    let s = screen(&a);
    assert!(line_with(&s, "Waiting on you").contains("Ideas 1"), "{s}");
    assert!(line_with(&s, "AIDI-79").contains("Example idea: page extraction"), "{s}");
    assert!(
        s.contains("Developing, Dev on track, target Fx158.0.1 (Oct 20)"),
        "management's view is the reason:\n{s}"
    );
    assert!(s.contains("Next: Tell the idea's owner where your work stands"), "{s}");
    assert!(s.lines().all(|l| l.chars().count() <= 80), "{s}");
}

#[test]
fn work_names_its_epic_and_idea_and_the_idea_names_its_work() {
    let mut a = app_with_idea(140, 44);
    let s = screen(&a);
    assert!(line_with(&s, "Epic ").contains("AIPLAT-1040 Example epic: text extraction"), "{s}");
    assert!(line_with(&s, "Idea ").contains("AIDI-79 Example idea: page extraction"), "{s}");
    press(&mut a, KeyCode::Char('7'));
    let s = screen(&a);
    assert!(s.contains("Your work under it"), "{s}");
    assert!(line_with(&s, "1995008").contains("Support view-source"), "{s}");
    assert!(
        s.contains("needinfo from a@mozilla.com"),
        "each item's own reason sits under it:\n{s}"
    );
    assert!(line_with(&s, "~ AIDI-79 targets").contains("14 days ago"), "{s}");
    assert!(s.contains("The idea and your work disagree"), "{s}");
}

#[test]
fn an_idea_offers_an_engineering_update_instead_of_an_agent_brief() {
    let mut a = app_with_idea(140, 44);
    press(&mut a, KeyCode::Char('7'));
    press(&mut a, KeyCode::Enter);
    let s = screen(&a);
    assert!(s.contains("Copy an engineering update for AIDI-79"), "{s}");
    assert!(!s.contains("Copy a brief for a coding agent"), "{s}");
    assert!(s.contains("Open AIDI-79 in Jira"), "{s}");
}

#[test]
fn the_timeline_tab_lists_dated_events_and_where_the_fix_ships() {
    for (w, h) in [(80, 24), (140, 30)] {
        let mut a = app(w, h);
        let mut raw = a.snapshot.take().unwrap().raw;
        raw.bugs[0].creation_time = Some("2026-10-01T09:00:00Z".parse().unwrap());
        raw.bugs[0].release_flags.insert("cf_status_firefox161".into(), "fixed".into());
        update(&mut a, Msg::Fetched(Ok(Box::new(raw))), now());
        assert!(screen(&a).contains("Timeline"), "{}", screen(&a));
        assert!(
            screen(&a).contains("Fixed in Nightly 161"),
            "Overview says where it ships:\n{}",
            screen(&a)
        );
        press(&mut a, KeyCode::Tab);
        let s = screen(&a);
        assert!(s.contains("Oct 1") && s.contains("Bug 1995008 filed"), "{s}");
        assert!(s.contains("D321987 pushed to try"), "{s}");
        assert!(s.contains("Ships") && s.contains("becomes Beta"), "{s}");
        assert!(s.lines().all(|l| l.chars().count() <= w as usize), "{s}");
    }
}

#[test]
fn opening_says_what_changed_since_the_last_look_until_a_key_is_pressed() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/raw.json");
    let raw = || -> Raw { serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap() };
    let mut marks = app(80, 24).marks;
    marks.last_look = Some("2026-11-03T12:00:00Z".parse().unwrap());
    let mut changed = raw();
    changed.revisions[0].reviewers[0].status = worklens_core::raw::ReviewerStatus::Accepted;
    let opts = || Options {
        cache_minutes: None,
        agent: None,
        dir: PathBuf::from("/tmp"),
        keys: Vec::new(),
    };
    let mut a = App::new(CoreConfig::default(), opts(), Some(changed), marks.clone(), now());
    update(&mut a, Msg::Resize(80, 24), now());
    let last = screen(&a).lines().last().unwrap().to_owned();
    assert!(last.contains("Since you looked, 8h ago: 1 changed"), "{last}");
    assert!(
        last.contains("· Bug 1995008 D321987: quillfox"),
        "the first change fits, cut short:\n{last}"
    );
    press(&mut a, KeyCode::Down);
    assert!(
        screen(&a).lines().last().unwrap().contains("↑↓ move"),
        "a key brings the hints back"
    );

    let mut quiet = App::new(CoreConfig::default(), opts(), Some(raw()), marks, now());
    update(&mut quiet, Msg::Resize(80, 24), now());
    assert!(
        screen(&quiet)
            .lines()
            .last()
            .unwrap()
            .contains("Nothing new since you looked, 8h ago")
    );
}

#[test]
fn the_needinfo_comment_is_shown_once_as_the_question() {
    let s = screen(&app(160, 40));
    assert_eq!(s.matches("Can you look at this?").count(), 1, "{s}");
    assert!(s.contains("a asked, 4d ago"), "{s}");
}

#[test]
fn the_legend_gives_each_glyph_one_meaning() {
    let mut a = app(80, 24);
    press(&mut a, KeyCode::Char('?'));
    let s = screen(&a);
    assert_eq!(s.matches("~   ").count(), 1, "{s}");
    assert!(s.contains("~   old try pass") && s.contains("*   important"), "{s}");
}

#[test]
fn the_header_groups_identity_with_the_sprint_and_release_with_freshness() {
    let s = screen(&app(160, 24));
    let header = s.lines().next().unwrap();
    assert!(header.starts_with(" worklens   Sprint  "), "{header}");
    assert!(header.ends_with("Beta merge in 2 days   Updated 19m ago"), "{header}");
    let s = screen(&app(80, 24));
    let header = s.lines().next().unwrap();
    assert!(header.starts_with(" worklens   Sprint 3 of 10 pts"), "{header}");
    assert!(header.ends_with("Updated 19m ago"), "{header}");
}

#[test]
fn the_sprint_bar_tells_committed_from_empty_cells_in_every_theme() {
    use ratatui::style::Color;
    for tone in [worklens_tui::Tone::Dark, worklens_tui::Tone::Light, worklens_tui::Tone::NoColor] {
        let mut a = app(160, 24);
        a.tone = tone;
        let mut terminal = Terminal::new(TestBackend::new(160, 24)).unwrap();
        terminal.draw(|f| view(&a, f)).unwrap();
        let buf = terminal.backend().buffer();
        let cells: Vec<(String, Color)> = (0..160)
            .map(|x| (buf[(x, 0)].symbol().to_owned(), buf[(x, 0)].fg))
            .filter(|(sym, _)| matches!(sym.as_str(), "█" | "▓" | "░"))
            .collect();
        assert_eq!(cells.len(), 10, "{tone:?}");
        let (committed, empty) = (&cells[0], &cells[9]);
        assert_ne!(committed.0, empty.0, "{tone:?}: shape alone tells them apart");
        assert_eq!(empty.0, "░", "{tone:?}");
    }
}

#[test]
fn every_footer_and_hint_separates_keys_the_same_way() {
    use ratatui::style::Color;
    let mut a = app(80, 24);
    a.tone = worklens_tui::Tone::Dark;
    let s = screen(&a);
    assert_eq!(
        s.lines().last().unwrap(),
        " ↑↓ move · ←→ section · Tab detail · Enter actions · / filter · ? help"
    );
    let hint = line_with(&s, "lines · PgUp PgDn scroll");
    assert!(
        hint.contains("│ 1–10 of 13 lines · PgUp PgDn scroll "),
        "no more ↓ beside the range: {s}"
    );
    assert!(!s.contains("more ↓"), "{s}");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| view(&a, f)).unwrap();
    let buf = terminal.backend().buffer();
    let y = s.lines().position(|l| l.contains("PgUp PgDn scroll")).unwrap() as u16;
    let x = line_with(&s, "PgUp PgDn scroll").find("PgUp").unwrap();
    let x = line_with(&s, "PgUp PgDn scroll")[..x].chars().count() as u16;
    assert_eq!(buf[(x, y)].fg, Color::Reset, "keys in the default colour");
    assert_eq!(buf[(x + 10, y)].fg, Color::Indexed(246), "labels quiet");
    assert_eq!(buf[(1, 23)].fg, Color::Reset);
    for (keys, footer) in [
        (vec![KeyCode::Enter], "↑↓ choose · Enter or key run · Esc back"),
        (
            vec![KeyCode::Char('s')],
            "PgUp PgDn scroll · Tab settings · r retry · Esc back · q quit",
        ),
        (vec![KeyCode::Char('?')], "Esc close · q quit"),
    ] {
        let mut a = app(80, 24);
        for k in keys {
            press(&mut a, k);
        }
        let s = screen(&a);
        assert!(s.lines().last().unwrap().ends_with(footer), "{s}");
    }
}

fn underlined(s: &str, name: &str) -> bool {
    let lines: Vec<&str> = s.lines().collect();
    lines.iter().enumerate().any(|(y, l)| {
        let Some(at) = l.find(&format!(" {name} ")) else { return false };
        let x = l[..at].chars().count() + 1;
        let w = name.chars().count();
        lines.get(y + 1).is_some_and(|below| {
            let cells: Vec<char> = below.chars().collect();
            cells.get(x..x + w).is_some_and(|u| u.iter().all(|c| *c == '━'))
                && cells.get(x + w) != Some(&'━')
                && matches!(cells.first(), Some('╭'))
        })
    })
}

#[test]
fn detail_and_settings_tabs_are_underlined_in_their_box_border_like_sections() {
    for (w, h) in [(80, 24), (160, 30)] {
        let mut a = app(w, h);
        let s = screen(&a);
        assert!(underlined(&s, "Waiting on you 1"), "{s}");
        assert!(underlined(&s, "Overview"), "{w}x{h}:\n{s}");
        assert!(!s.lines().any(|l| l.starts_with("│ ━")), "no rule row inside the box:\n{s}");
        press(&mut a, KeyCode::Tab);
        assert!(underlined(&screen(&a), "Timeline"), "{w}x{h}");
        press(&mut a, KeyCode::Char('s'));
        let s = screen(&a);
        assert!(underlined(&s, "Sources"), "{s}");
        press(&mut a, KeyCode::Tab);
        assert!(underlined(&screen(&a), "Settings"), "{w}x{h}");
    }
}

#[test]
fn sources_show_their_rows_at_once_and_spin_only_the_pending_ones() {
    let mut a = app(80, 24);
    press(&mut a, KeyCode::Char('s'));
    let s = screen(&a);
    assert!(line_with(&s, "Reading your sources and tokens").starts_with("│ ⠋ "), "{s}");
    let check = |name: &str, ok: bool, pending: bool, detail: &str| worklens_tui::Check {
        name: name.into(),
        ok,
        required: name == "bugzilla",
        detail: detail.into(),
        fix: Some("a fix".into()),
        pending,
    };
    update(
        &mut a,
        Msg::Checks(vec![
            check("bugzilla", true, true, "checking Bugzilla answers"),
            check("jira", false, false, "no token; sprint and Jira drift are skipped"),
        ]),
        now(),
    );
    let s = screen(&a);
    assert!(line_with(&s, "bugzilla").contains("⠋ bugzilla checking Bugzilla answers"), "{s}");
    assert!(line_with(&s, "jira").contains("~ jira"), "{s}");
    assert_eq!(s.matches("fix: a fix").count(), 1, "a pending row has no fix yet:\n{s}");
    update(&mut a, Msg::Tick, now());
    assert!(line_with(&screen(&a), "bugzilla").contains("⠙ bugzilla"), "the pending mark spins");
    update(&mut a, Msg::Checks(vec![check("bugzilla", true, false, "signed in as me")]), now());
    let frame = a.frame;
    update(&mut a, Msg::Tick, now());
    assert_eq!(a.frame, frame, "nothing animates once every check is in");
    assert!(line_with(&screen(&a), "bugzilla").contains("✓ bugzilla"));
}

fn says_off_never_failed(screen: &str) {
    let problems = worklens_testkit::screen_problems(screen, &["jira", "phabricator"], false);
    assert!(problems.is_empty(), "{problems:#?}\n{screen}");
}

#[test]
fn a_bugzilla_only_user_sees_what_is_off_and_never_that_it_failed() {
    for (w, h) in [(80, 24), (160, 48)] {
        let mut a = app_from("bugzilla_only.json", w, h);
        for _ in 0..Section::COUNT {
            says_off_never_failed(&screen(&a));
            press(&mut a, KeyCode::Tab);
            says_off_never_failed(&screen(&a));
            press(&mut a, KeyCode::Right);
        }
        a.snapshot = None;
        a.rows.clear();
        a.busy = Some("Fetching bugs".into());
        let s = screen(&a);
        assert!(s.contains("Fetching your work…"), "{s}");
        says_off_never_failed(&s);
    }
}

#[test]
fn source_rows_wrap_under_their_detail_and_keep_long_names_whole() {
    let mut a = app(60, 24);
    press(&mut a, KeyCode::Char('s'));
    let check = |name: &str, detail: &str| worklens_tui::Check {
        name: name.into(),
        ok: false,
        required: false,
        detail: detail.into(),
        fix: Some("`worklens auth set jira` (an API token from https://id.atlassian.com/manage-profile/security/api-tokens)".into()),
        pending: false,
    };
    update(
        &mut a,
        Msg::Checks(vec![
            check("jira", "no token; sprint and Jira drift are skipped · last fetch: not set up"),
            check("release trains", "last fetch: 1 request, slowest 114 ms"),
        ]),
        now(),
    );
    let s = screen(&a);
    assert!(s.contains("~ release trains last fetch"), "{s}");
    let column = line_with(&s, "~ jira").find("no token").unwrap();
    let body: Vec<&str> = s
        .lines()
        .skip_while(|l| !l.contains("~ jira"))
        .take_while(|l| !l.contains("needed"))
        .collect();
    for line in &body[1..] {
        let inner = line.trim_start_matches('│').trim_end_matches('│').trim_end();
        if inner.is_empty() || inner.contains("~ release trains") {
            continue;
        }
        let at = line.find(inner.trim_start()).unwrap();
        assert_eq!(at, column, "wrapped text starts under the detail:\n{s}");
    }
}

#[test]
fn a_long_sources_page_scrolls_to_its_last_line() {
    let mut a = app(80, 24);
    press(&mut a, KeyCode::Char('s'));
    let checks = (0..30)
        .map(|i| worklens_tui::Check {
            name: format!("source {i}"),
            ok: true,
            required: false,
            detail: "reachable · last fetch: 3 requests, slowest 120 ms".into(),
            fix: None,
            pending: false,
        })
        .collect();
    update(&mut a, Msg::Checks(checks), now());
    assert!(!screen(&a).contains("✗ needed"), "the legend starts below the fold");
    for _ in 0..20 {
        press(&mut a, KeyCode::PageDown);
    }
    let s = screen(&a);
    assert!(s.contains("✗ needed"), "paging down reaches the legend:\n{s}");
    assert!(s.contains("source 29"), "{s}");
}

fn overloaded(w: u16, h: u16) -> App {
    overloaded_with(w, h, |_| {})
}

fn overloaded_with(w: u16, h: u16, edit: impl FnOnce(&mut serde_json::Value)) -> App {
    let path = format!("{}/../store/fixtures/busy.json", env!("CARGO_MANIFEST_DIR"));
    let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    edit(&mut value);
    let raw: Raw = serde_json::from_value(value).unwrap();
    let opts = Options {
        cache_minutes: None,
        agent: None,
        dir: PathBuf::from("/tmp"),
        keys: Vec::new(),
    };
    let cfg = CoreConfig {
        whiteboard: Some("[aiplatform]".into()),
        ..CoreConfig::default()
    };
    let mut app = App::new(cfg, opts, Some(raw), Marks::default(), now());
    update(&mut app, Msg::Resize(w, h), now());
    app
}

fn filter_to(app: &mut App, text: &str) {
    press(app, KeyCode::Char('/'));
    for c in text.chars() {
        press(app, KeyCode::Char(c));
    }
    press(app, KeyCode::Enter);
}

fn open_section(app: &mut App, tab: &str) {
    for _ in 0..Section::COUNT {
        let s = screen(app);
        let tabs: Vec<char> = s.lines().nth(1).unwrap_or_default().chars().collect();
        let border: Vec<char> = s.lines().nth(2).unwrap_or_default().chars().collect();
        let name: Vec<char> = tab.chars().collect();
        let at = tabs.windows(name.len()).position(|w| w == name.as_slice());
        if at.is_some_and(|c| border.get(c) == Some(&'━')) {
            return;
        }
        press(app, KeyCode::Right);
    }
    panic!("no {tab} section in\n{}", screen(app));
}

#[test]
fn an_approved_uplift_does_not_promise_a_nightly_it_already_shipped_in() {
    for (w, h) in [(80, 24), (200, 50)] {
        let mut a = overloaded(w, h);
        open_section(&mut a, "Land 4");
        filter_to(&mut a, "2000469");
        let s = screen(&a);
        assert!(s.contains("uplift to Beta 146 approved"), "{s}");
        assert!(s.contains("Fixed in Nightly 147"), "{s}");
        assert!(
            !s.contains("Lands in 147"),
            "the fix is already in Nightly 147 and the step is a Beta uplift:\n{s}"
        );
        let mut a = overloaded(w, h);
        open_section(&mut a, "Land 4");
        filter_to(&mut a, "1998253");
        let s = screen(&a);
        assert!(
            s.contains("Lands in 147 if it lands by Nov 9 "),
            "an accepted revision still gets the countdown:\n{s}"
        );
    }
}

#[test]
fn jira_keys_in_the_list_are_never_cut() {
    for (w, h) in [(80, 24), (200, 50)] {
        let mut a = overloaded(w, h);
        open_section(&mut a, "Sprint 15");
        let s = screen(&a);
        assert!(!s.contains("AIPLAT-15…"), "rows can't share a cut key:\n{s}");
        assert!(s.contains("AIPLAT-1528"), "{s}");
    }
}

#[test]
fn sprint_done_carries_its_unit() {
    let s = screen(&overloaded(200, 50));
    let header = line_with(&s, "of 10 pts");
    assert!(header.contains(" · 13 pts done"), "13 is points, not tickets:\n{header}");
}

#[test]
fn an_idea_names_its_target_quarter_once() {
    for (w, h) in [(80, 24), (200, 50)] {
        let mut a = overloaded(w, h);
        open_section(&mut a, "Ideas 4");
        filter_to(&mut a, "AIDI-81");
        let s = screen(&a);
        let target = line_with(&s, "Target ");
        assert_eq!(target.matches("Q3").count(), 1, "{target}");
        assert!(target.contains("Q3 (Sep 30, 2026)"), "the dated target stays:\n{target}");
        let mut a = overloaded_with(w, h, |v| v["roadmap"]["ideas"]["AIDI-81"]["target"] = serde_json::Value::Null);
        open_section(&mut a, "Ideas 4");
        filter_to(&mut a, "AIDI-81");
        let s = screen(&a);
        assert!(
            line_with(&s, "Target ").contains("Q3"),
            "with no target, the quarter is the target:\n{s}"
        );
    }
}

#[test]
fn a_sprint_with_nothing_done_says_nothing_about_done() {
    let s = screen(&app(160, 40));
    let header = line_with(&s, "of 10 pts");
    assert!(header.contains("3 of 10 pts"), "{header}");
    assert!(!header.contains("done"), "{header}");
}
