mod config;
mod doctor;
mod health;
mod json;
mod mcp;
mod secrets;
mod setup;
mod text;
mod tone;
mod ui;

use std::process::ExitCode;

use anyhow::{Context as _, anyhow, bail};
use clap::{Parser, Subcommand};
use jiff::{Timestamp, Zoned};
use serde_json::{Value, json};
use worklens_core::marks::{self, Marks};
use worklens_core::raw::SourceState;
use worklens_core::section::Section;
use worklens_core::{EntryIdx, Snapshot, brief, build_snapshot, present};
use worklens_store::{cache_file, marks_file, read_marks, read_raw, write_raw};

use crate::json::{Doc, Field};
use crate::text::Style;

#[derive(Parser)]
#[command(
    name = "worklens",
    version,
    about = "Your Bugzilla, Jira and Phabricator work, by whose turn it is",
    after_help = "With no command, worklens opens the interactive UI.\n\
                  Also: waiting, land, returned, uplift, others, other, landed, backlog, health, log, agent-instructions.\n\
                  `worklens help <command>` explains any of them."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[arg(long, global = true, help = "JSON output (schema \"worklens/2\")")]
    json: bool,
    #[arg(long, global = true, help = "Smaller JSON, for agents")]
    compact: bool,
    #[arg(long, global = true, conflicts_with = "cached", help = "Fetch before showing")]
    fresh: bool,
    #[arg(long, global = true, help = "Never fetch; use the cache however old")]
    cached: bool,
}

#[derive(Subcommand)]
enum Command {
    #[command(hide = true, about = "The interactive UI; the default in a terminal")]
    Ui,
    #[command(
        about = "What needs you, critical first",
        long_about = "What needs you, critical first.\n\nWaiting on you, ready to land, returned to you, and waiting on others. Within each, backed-out landings first, then critical items (S1, sec-critical or tracked) come first, then important (S2, sec-high), then the rest; oldest first within each, group reviews last. A Phabricator stack stays together, bottom first, where its first member would sort."
    )]
    Next {
        #[arg(long, help = "Every item, not just the first 10")]
        all: bool,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    #[command(about = "Every section")]
    Sections,
    #[command(hide = true, about = "Needinfos and reviews asked of you")]
    Waiting,
    #[command(hide = true, about = "Your accepted revisions")]
    Land,
    #[command(hide = true, about = "Changes requested on your revisions")]
    Returned,
    #[command(hide = true, about = "Your revisions in review")]
    Others,
    #[command(hide = true, about = "Everything else assigned to you")]
    Other,
    #[command(
        about = "Recent fixes assigned to you, with fixed status and expected train",
        long_about = "Recent fixes assigned to you, with fixed status and expected train.\n\nThe fixed version comes from Bugzilla status flags. Train position is expected from the calendar; build inclusion and who resolved a bug are unverified."
    )]
    Landed,
    #[command(
        about = "What's assigned to you that you haven't started",
        long_about = "What's assigned to you that you haven't started.\n\nJira tickets in To Do outside an open sprint, and open bugs with no revision and no ticket in progress; highest priority first, then oldest."
    )]
    Backlog,
    #[command(
        about = "Local commits that aren't on Phabricator",
        long_about = "Local commits that aren't on Phabricator.\n\nScans your branches and detached worktrees in the agent directory (default ~/firefox) for your commits from the last 90 days that aren't on origin/main. Branches with \"backup\" in their name are skipped. Commits whose changes differ from their revision's latest diff are listed under drift."
    )]
    Local,
    #[command(about = "Sprint points and tickets")]
    Sprint,
    #[command(about = "Where Bugzilla, Phabricator and Jira disagree")]
    Drift {
        #[arg(long, help = "The whole team's bugs, not just yours")]
        team: bool,
        #[arg(long, help = "Only this person's (with --team)")]
        owner: Option<String>,
    },
    #[command(
        about = "Your Glean metrics and when they expire",
        long_about = "Your Glean metrics and when they expire.\n\nFirefox Desktop metric definitions that name your bugs: expiry and data-review references."
    )]
    Telemetry,
    #[command(about = "One item in full")]
    Show {
        #[arg(help = "Bug number, AIPLAT-123, D123, or a link to one")]
        reference: String,
    },
    #[command(about = "Markdown handoff for a coding agent")]
    Brief { reference: String },
    #[command(
        about = "Start your coding agent with the brief",
        long_about = "Start your coding agent with the brief.\n\nThe agent is claude or codex, whichever is installed, or \"agent\" in the config."
    )]
    Start { reference: String },
    #[command(
        about = "Check out an item's revision",
        long_about = "Check out an item's revision.\n\nRuns moz-phab patch in agentDir."
    )]
    Checkout { reference: String },
    #[command(
        about = "Have your agent draft a review. It never posts",
        long_about = "Have your agent draft a review. It never posts.\n\nThe agent checks out a review asked of you and drafts feedback for you to read."
    )]
    Review { reference: String },
    #[command(about = "Hide an item until it has new activity")]
    Done { reference: String },
    #[command(
        about = "Hide an item until a date",
        long_about = "Hide an item until a date.\n\nThe date is like tomorrow, mon, 3d or nov 10. The item comes back at the date or on new activity, whichever is first."
    )]
    Snooze { reference: String, when: Vec<String> },
    #[command(about = "Undo done or snooze")]
    Restore { reference: String },
    #[command(
        about = "Fetch now and update the cache",
        long_about = "Fetch now and update the cache.\n\nRefreshes the primary sources; supplemental sources keep their own intervals."
    )]
    Refresh,
    #[command(
        about = "Set up your API keys",
        long_about = "Set up your API keys.\n\nEach source's key, checked and kept in your keychain."
    )]
    Setup,
    #[command(about = "Manage API keys one at a time")]
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    #[command(about = "Check each source live, with fixes")]
    Doctor,
    #[command(
        hide = true,
        about = "Each source as the last fetch saw it. No network",
        long_about = "Each source as the last fetch saw it. No network.\n\nRequests, failures, stale data and keys, from the cache alone."
    )]
    Health,
    #[command(hide = true, about = "Every API request behind the cached data")]
    Log,
    #[command(about = "Run as an MCP server over stdio")]
    Mcp,
    #[command(hide = true, about = "Text to paste into an agent's AGENTS.md")]
    AgentInstructions,
}

#[derive(Subcommand)]
enum AuthAction {
    #[command(about = "Where each key comes from (never the key itself)")]
    Status,
    #[command(about = "Save a key, typed (hidden) or piped on stdin: bugzilla, jira or phabricator")]
    Set { service: String },
    #[command(about = "Forget worklens's own copy of a key")]
    Remove { service: String },
    #[command(about = "Copy keys borrowed from ~/.arcrc, ~/.jira-aiplat.json or the config into the keychain")]
    Import,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("worklens: {e:#}");
            ExitCode::FAILURE
        }
    }
}

struct Out {
    json: bool,
    compact: bool,
    style: Style,
    status: serde_json::Map<String, Value>,
}

impl Out {
    fn print<'a>(
        &self,
        full: impl FnOnce() -> Doc<'a>,
        compact: impl FnOnce() -> Doc<'a>,
        text: impl FnOnce() -> String,
    ) -> anyhow::Result<()> {
        let mut output = if self.compact {
            serde_json::to_vec(&json::stamp(compact(), &self.status))?
        } else if self.json {
            serde_json::to_vec_pretty(&json::stamp(full(), &self.status))?
        } else {
            text().into_bytes()
        };
        output.push(b'\n');
        emit(&output)
    }
}

fn emit(bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write as _;
    let mut stdout = std::io::stdout().lock();
    match stdout.write_all(bytes).and_then(|()| stdout.flush()) {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => Ok(other?),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Freshness {
    Auto,
    Fetch,
    CacheOnly,
}

fn fetch(cfg: &config::Config) -> anyhow::Result<worklens_core::Raw> {
    use std::io::IsTerminal;
    let progress = std::io::stderr().is_terminal();
    let st = Style { color: progress };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let http = worklens_sources::Http::new()?;
    let previous = cache_file().and_then(|path| read_raw(&path).ok());
    let mut raw = runtime.block_on(worklens_sources::fetch_raw(&http, &cfg.sources, previous, |msg| {
        if progress {
            eprintln!("{}", st.dim(format!("… {msg}")));
        }
    }))?;
    raw.calls = http.take_calls();
    write_raw(&cache_file().context("no home directory")?, &raw)?;
    let s = build_snapshot(raw, &cfg.core, Timestamp::now());
    if let Some(day) = drift_day(&s)
        && let Some(path) = worklens_store::drift_file()
        && let Err(e) = worklens_store::record_drift(&path, day)
    {
        eprintln!("worklens: couldn't save the drift count: {e}");
    }
    Ok(s.raw)
}

pub fn drift_day(s: &Snapshot) -> Option<worklens_core::drift_log::DriftDay> {
    let (mine, team) = worklens_core::drift_log::counts(s)?;
    Some(worklens_core::drift_log::DriftDay {
        date: s.now.to_zoned(jiff::tz::TimeZone::system()).date(),
        mine,
        team,
    })
}

fn load(freshness: Freshness) -> anyhow::Result<(Snapshot, Marks)> {
    let cfg = config::load_without_secrets()?;
    let path = cache_file().ok_or_else(|| anyhow!("no home directory"))?;
    let now = Timestamp::now();
    let cached = match freshness {
        Freshness::Fetch => None,
        Freshness::CacheOnly => Some(read_raw(&path).map_err(|e| anyhow!("{e}; run `worklens refresh`"))?),
        Freshness::Auto => read_raw(&path)
            .ok()
            .filter(|raw| (now.as_millisecond() - raw.fetched_at.as_millisecond()) as f64 / 60_000.0 < cfg.cache_minutes),
    };
    let raw = match cached {
        Some(raw) => raw,
        None => fetch(&config::with_secrets(cfg.clone()))?,
    };
    let mut marks = match marks_file().map(|p| read_marks(&p)) {
        Some(Ok(m)) => m,
        Some(Err(e)) => {
            eprintln!("worklens: ignoring the marks file: {e}");
            Marks::default()
        }
        None => Marks::default(),
    };
    let mut s = build_snapshot(raw, &cfg.core, now);
    let base = marks.clone();
    if marks::upgrade(&mut marks, &s)
        && let Some(path) = marks_file()
    {
        worklens_store::save_marks(&path, &base, &marks)?;
    }
    marks::apply(&mut s, &marks, now);
    Ok((s, marks))
}

fn doctor(json: bool) -> anyhow::Result<()> {
    let cfg = config::load()?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let checks = runtime.block_on(doctor::run_checks(&cfg));
    if json {
        let list: Vec<Value> = checks
            .iter()
            .map(|c| json!({ "name": c.name, "ok": c.ok, "required": c.required, "detail": c.detail, "fix": c.fix }))
            .collect();
        emit(format!("{}\n", serde_json::to_string_pretty(&list)?).as_bytes())?;
    } else {
        let st = Style::detect();
        for c in &checks {
            let mark = match (c.ok, c.required) {
                (true, _) => st.green("✓"),
                (false, true) => st.red("✗"),
                (false, false) => st.yellow("~"),
            };
            println!("{mark} {:<15} {}", c.name, c.detail);
            if let (false, Some(fix)) = (c.ok, &c.fix) {
                println!("{}", st.dim(format!("  {:<15} fix: {fix}", "")));
            }
        }
    }
    if checks.iter().any(|c| c.required && !c.ok) {
        bail!("a required check failed");
    }
    Ok(())
}

fn health(json: bool) -> anyhow::Result<()> {
    let cfg = config::load()?;
    let path = cache_file().context("no home directory")?;
    let next = if setup::needed(&cfg) {
        "worklens setup"
    } else {
        "worklens refresh"
    };
    let raw = read_raw(&path).map_err(|e| anyhow!("{e}; run `{next}`"))?;
    let sources = health::from_cache(&raw, &cfg);
    let age = (Timestamp::now().as_millisecond() - raw.fetched_at.as_millisecond()).max(0) / 60_000;
    if json {
        let value = json!({
            "schema": json::SCHEMA, "fetchedAt": raw.fetched_at, "ageMinutes": age,
            "partial": raw.failures().next().is_some(), "sources": sources, "errors": raw.errors, "stale": raw.stale,
        });
        return emit(format!("{}\n", serde_json::to_string_pretty(&value)?).as_bytes());
    }
    let st = Style::detect();
    println!(
        "Updated {} ({})",
        present::ago(raw.fetched_at, Timestamp::now()),
        health::clock(raw.fetched_at)
    );
    for h in &sources {
        let mark = match h.state {
            "ok" => st.green("✓"),
            "stale" | "failed" => st.red("✗"),
            "off" => st.yellow("~"),
            _ => st.dim("·"),
        };
        println!("{mark} {:<15} {}", h.name, health::describe(h));
        if let Some(fix) = health::fix(h).filter(|_| h.state != "ok") {
            println!("{}", st.dim(format!("  {:<15} fix: {fix}", "")));
        }
    }
    Ok(())
}

fn find(s: &Snapshot, reference: &str) -> anyhow::Result<EntryIdx> {
    s.find(reference).ok_or_else(|| match s.unread_source(reference) {
        Some(why) => anyhow!("{reference} isn't in your work: {why}"),
        None => anyhow!("{reference} isn't in your work or team drift. Try `worklens next --all`."),
    })
}

fn run(cli: &Cli) -> anyhow::Result<()> {
    use std::io::IsTerminal;
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if matches!(cli.command, Some(Command::Ui)) || (cli.command.is_none() && interactive && !cli.json && !cli.compact) {
        let mut cfg = config::load_without_secrets()?;
        if interactive && setup::needed(&cfg) && setup::needed(&config::with_secrets(cfg.clone())) {
            setup::run(Style::detect())?;
            cfg = config::load_without_secrets()?;
        }
        return ui::run(&cfg, cli.cached);
    }
    match &cli.command {
        Some(Command::Setup) => return setup::run(Style::detect()),
        Some(Command::Auth { action }) => {
            return match action {
                AuthAction::Status => setup::status(Style::detect()),
                AuthAction::Set { service } => setup::set(service),
                AuthAction::Remove { service } => setup::remove(service),
                AuthAction::Import => setup::import(),
            };
        }
        Some(Command::Mcp) => return mcp::serve(),
        Some(Command::Doctor) => return doctor(cli.json || cli.compact),
        Some(Command::Health) => return health(cli.json || cli.compact),
        Some(Command::AgentInstructions) => {
            const SKILL: &str = include_str!("../SKILL.md");
            let body = SKILL
                .strip_prefix("---\n")
                .and_then(|rest| rest.split_once("\n---\n"))
                .map_or(SKILL, |(_, body)| body);
            return emit(format!("{}\n", body.trim()).as_bytes());
        }
        _ => {}
    }
    let mut out = Out {
        json: cli.json || cli.compact,
        compact: cli.compact,
        style: Style::detect(),
        status: serde_json::Map::new(),
    };
    let st = out.style;
    let freshness = match cli.command {
        Some(Command::Refresh) => Freshness::Fetch,
        Some(Command::Log) => Freshness::CacheOnly,
        _ if cli.fresh => Freshness::Fetch,
        _ if cli.cached => Freshness::CacheOnly,
        _ => Freshness::Auto,
    };
    let (s, mut marks) = load(freshness)?;
    let marks_as_read = marks.clone();
    let s: &Snapshot = Box::leak(Box::new(s));
    out.status = json::status(s, Timestamp::now());
    let out = out;
    let items = |ids: &[EntryIdx]| -> Field<'_> { Field::Items(ids.iter().map(|i| json::item(s, s.entry(*i))).collect()) };
    let value = |v: Value| Field::Value(v);
    let compacts = |ids: &[EntryIdx]| -> Vec<Value> { ids.iter().map(|i| json::compact(s, s.entry(*i))).collect() };
    let errors = || serde_json::to_value(&s.raw.errors).unwrap_or_default();
    let head = || {
        let trains = text::train_line(st, s).map(|t| format!("{t}\n")).unwrap_or_default();
        let sprint = text::sprint_line(st, s).map(|l| format!("{l}\n")).unwrap_or_default();
        format!("{}\n{sprint}{trains}", text::header(st, s))
    };

    match cli.command.as_ref().unwrap_or(&Command::Next { all: false, limit: 10 }) {
        Command::Next { all, limit } => {
            const TURNS: [Section; 4] = [Section::Waiting, Section::Land, Section::Returned, Section::Others];
            let limit = if *all { usize::MAX } else { *limit };
            let every: Vec<EntryIdx> = TURNS.iter().flat_map(|t| s.section(*t, false)).collect();
            let more = every.len().saturating_sub(limit);
            let ids: Vec<EntryIdx> = every.into_iter().take(limit).collect();
            out.print(
                || {
                    Doc::object([
                        ("schema", value(json!(json::SCHEMA))),
                        ("fetchedAt", value(json!(s.raw.fetched_at))),
                        ("me", value(json!(s.me()))),
                        ("sprint", value(json::sprint(s))),
                        ("trains", value(json!(s.raw.trains))),
                        ("items", items(&ids)),
                        ("more", value(json!(more))),
                        ("errors", value(errors())),
                    ])
                },
                || json!({ "fetchedAt": s.raw.fetched_at, "sprint": json::sprint(s), "items": compacts(&ids), "more": more, "errors": errors() }).into(),
                || {
                    let mut room = limit;
                    let blocks: Vec<String> = TURNS
                        .iter()
                        .filter(|t| !s.off(**t))
                        .map(|t| {
                            let block = text::section_block(st, s, *t, room);
                            room = room.saturating_sub(s.section(*t, false).count());
                            block
                        })
                        .collect();
                    format!("{}\n{}\n", head(), blocks.join("\n\n"))
                },
            )
        }
        Command::Sections => {
            let lists: Vec<(Section, Vec<EntryIdx>)> = Section::ALL.iter().map(|x| (*x, s.section(*x, false).collect())).collect();
            out.print(
                || {
                    let sections = lists
                        .iter()
                        .map(|(x, ids)| json::SectionItems {
                            id: x.id(),
                            title: x.title(),
                            items: ids.iter().map(|i| json::item(s, s.entry(*i))).collect(),
                        })
                        .collect();
                    Doc::object([
                        ("schema", value(json!(json::SCHEMA))),
                        ("fetchedAt", value(json!(s.raw.fetched_at))),
                        ("me", value(json!(s.me()))),
                        ("sprint", value(json::sprint(s))),
                        ("trains", value(json!(s.raw.trains))),
                        ("sections", Field::Sections(sections)),
                        ("errors", value(errors())),
                    ])
                },
                || {
                    let sections: Vec<Value> = lists
                        .iter()
                        .map(|(x, ids)| json!({ "id": x.id(), "title": x.title(), "items": compacts(ids) }))
                        .collect();
                    json!({ "fetchedAt": s.raw.fetched_at, "sprint": json::sprint(s), "sections": sections, "errors": errors() }).into()
                },
                || {
                    let blocks: Vec<String> = Section::ALL.iter().map(|x| text::section_block(st, s, *x, usize::MAX)).collect();
                    format!("{}\n{}\n", head(), blocks.join("\n\n"))
                },
            )
        }
        Command::Waiting
        | Command::Land
        | Command::Returned
        | Command::Others
        | Command::Other
        | Command::Landed
        | Command::Backlog
        | Command::Local => {
            let section = match cli.command {
                Some(Command::Local) => Section::Local,
                Some(Command::Landed) => Section::Landed,
                Some(Command::Backlog) => Section::Backlog,
                Some(Command::Waiting) => Section::Waiting,
                Some(Command::Land) => Section::Land,
                Some(Command::Returned) => Section::Returned,
                Some(Command::Others) => Section::Others,
                _ => Section::Other,
            };
            let ids: Vec<EntryIdx> = s.section(section, false).collect();
            out.print(
                || {
                    Doc::object([
                        ("schema", value(json!(json::SCHEMA))),
                        ("fetchedAt", value(json!(s.raw.fetched_at))),
                        ("section", value(json!(section.id()))),
                        ("items", items(&ids)),
                        ("errors", value(errors())),
                    ])
                },
                || json!({ "fetchedAt": s.raw.fetched_at, "section": section.id(), "items": compacts(&ids), "errors": errors() }).into(),
                || format!("{}\n\n{}", text::header(st, s), text::section_block(st, s, section, usize::MAX)),
            )
        }
        Command::Telemetry => {
            let doc = || -> Doc<'_> {
                json!({ "schema": json::SCHEMA, "definitions": s.raw.telemetry,
                "evidence": worklens_core::evidence::telemetry(s), "errors": errors() })
                .into()
            };
            out.print(doc, doc, || {
                let mut lines = vec![
                    text::header(st, s),
                    "Personal Firefox Desktop metric definitions (not collected values)".into(),
                ];
                if s.raw.telemetry.is_none() {
                    lines.push("Definitions unavailable, disabled, or not cached; refresh and check Sources.".into());
                } else if s.raw.telemetry.as_ref().is_some_and(|r| r.metrics.is_empty()) {
                    lines.push("No definitions matched your email or loaded personal bugs.".into());
                }
                for metric in worklens_core::evidence::telemetry(s) {
                    lines.push(format!("{}: {}\n  {}", metric.label, metric.summary, metric.url));
                }
                lines.join("\n")
            })
        }
        Command::Sprint => {
            let ids: Vec<EntryIdx> = s.section(Section::Sprint, true).collect();
            out.print(
                || {
                    Doc::object([
                        ("schema", value(json!(json::SCHEMA))),
                        ("fetchedAt", value(json!(s.raw.fetched_at))),
                        ("sprint", value(json::sprint(s))),
                        ("items", items(&ids)),
                    ])
                },
                || json!({ "sprint": json::sprint(s), "items": compacts(&ids) }).into(),
                || {
                    let entries: Vec<_> = ids.iter().map(|i| s.entry(*i)).collect();
                    let rows = text::sprint_rows(st, s, &entries);
                    let body = if rows.is_empty() {
                        st.dim(s.empty_text(Section::Sprint))
                    } else {
                        rows
                    };
                    format!("{}\n{body}\n", head())
                },
            )
        }
        Command::Drift { team, owner } => {
            let section = if *team { Section::Team } else { Section::Drift };
            let ids: Vec<EntryIdx> = s
                .section(section, true)
                .filter(|i| owner.as_deref().is_none_or(|o| present::owner(s, s.entry(*i)) == Some(o)))
                .collect();
            let drift_compacts = || -> Vec<Value> { ids.iter().map(|i| json::compact_drift(s, s.entry(*i))).collect() };
            out.print(
                || {
                    let log = worklens_store::drift_file()
                        .map(|p| worklens_store::read_drift_log(&p))
                        .unwrap_or_default();
                    let history: Vec<Value> = log
                        .days
                        .iter()
                        .map(|d| json!({ "date": d.date.to_string(), "count": if *team { d.team } else { d.mine } }))
                        .collect();
                    Doc::object([
                        ("schema", value(json!(json::SCHEMA))),
                        ("fetchedAt", value(json!(s.raw.fetched_at))),
                        ("scope", value(json!(if *team { "team" } else { "mine" }))),
                        ("items", items(&ids)),
                        ("history", value(Value::from(history))),
                    ])
                },
                || Value::from(drift_compacts()).into(),
                || {
                    let body = if ids.is_empty() {
                        match s.not_checked(Section::Drift) {
                            None => st.green("Everything is in sync."),
                            Some(why) if s.raw.state("jira") == SourceState::Off => st.dim(why),
                            Some(why) => st.yellow(format!("{why} See `worklens doctor`.")),
                        }
                    } else {
                        let blocks: Vec<String> = ids
                            .iter()
                            .map(|i| {
                                let e = s.entry(*i);
                                let who = if *team {
                                    st.dim(format!("{}  ", present::owner(s, e).unwrap_or("(nobody)")))
                                } else {
                                    String::new()
                                };
                                format!("{who}{}", text::item_block(st, s, e))
                            })
                            .collect();
                        blocks.join("\n\n")
                    };
                    format!("{}\n\n{body}", text::header(st, s))
                },
            )
        }
        Command::Show { reference } => {
            let e = s.entry(find(s, reference)?);
            out.print(
                || serde_json::to_value(json::item(s, e)).unwrap_or_default().into(),
                || json::compact(s, e).into(),
                || text::show(st, s, e),
            )
        }
        Command::Brief { reference } => {
            let md = brief::render(s, s.entry(find(s, reference)?));
            out.print(
                || json!({ "markdown": md }).into(),
                || json!({ "markdown": md }).into(),
                || md.clone(),
            )
        }
        Command::Done { reference } | Command::Snooze { reference, .. } | Command::Restore { reference } => {
            let e = s.entry(find(s, reference)?);
            let now = Timestamp::now();
            let id = s.id(e).to_string();
            let (what, key, until) = match &cli.command {
                Some(Command::Done { .. }) => {
                    marks::mark_done(&mut marks, s, e, now);
                    ("done; it comes back when it has new activity".to_owned(), "done", None)
                }
                Some(Command::Snooze { when, .. }) => {
                    let until = if when.is_empty() {
                        None
                    } else {
                        let text = when.join(" ");
                        Some(
                            marks::parse_when(&text, &Zoned::now())
                                .ok_or_else(|| anyhow!("Can't read \"{text}\"; try tomorrow, mon, 3d, 2w or nov 10."))?,
                        )
                    };
                    marks::mark_snoozed(&mut marks, s, e, until, now);
                    (
                        format!(
                            "snoozed until {}",
                            until.map_or_else(|| "it has new activity".to_owned(), text::date)
                        ),
                        "snooze",
                        until,
                    )
                }
                _ => {
                    marks::restore(&mut marks, s, e);
                    ("back in its section".to_owned(), "restore", None)
                }
            };
            worklens_store::save_marks(&marks_file().context("no home directory")?, &marks_as_read, &marks)?;
            let result = json!({ "id": id, key: true, "until": until });
            out.print(
                || result.clone().into(),
                || result.clone().into(),
                || format!("{} {what}.", st.bold(&id)),
            )
        }
        Command::Ui
        | Command::Mcp
        | Command::Doctor
        | Command::Health
        | Command::AgentInstructions
        | Command::Setup
        | Command::Auth { .. } => {
            unreachable!("handled before loading")
        }
        Command::Start { reference } | Command::Checkout { reference } | Command::Review { reference } => {
            let cfg = config::load_without_secrets()?;
            let e = s.entry(find(s, reference)?);
            let dir = &cfg.agent_dir;
            let no_agent = || anyhow!("No coding agent found: install claude or codex, or set \"agent\" in ~/.config/worklens.json.");
            let (title, program, args) = match &cli.command {
                Some(Command::Checkout { .. }) => {
                    let rev = s
                        .revision_of(e)
                        .map(|r| s.rev(r).id)
                        .ok_or_else(|| anyhow!("{} has no revision to check out.", s.id(e)))?;
                    (
                        format!("moz-phab patch {rev} in {}", dir.display()),
                        "moz-phab".to_owned(),
                        vec!["patch".to_owned(), rev.to_string()],
                    )
                }
                Some(Command::Review { .. }) => {
                    let prompt = brief::review_prompt(s, e).ok_or_else(|| anyhow!("{} has no review asked of you.", s.id(e)))?;
                    let agent = cfg.agent.as_ref().ok_or_else(no_agent)?;
                    let mut args = agent.args.clone();
                    args.push(prompt);
                    (
                        format!("{} in {} reviewing {}", agent.name, dir.display(), s.id(e)),
                        agent.program.clone(),
                        args,
                    )
                }
                _ => {
                    let agent = cfg.agent.as_ref().ok_or_else(no_agent)?;
                    let mut args = agent.args.clone();
                    args.push(brief::render(s, e));
                    (
                        format!("Starting {} in {} with the brief for {}", agent.name, dir.display(), s.id(e)),
                        agent.program.clone(),
                        args,
                    )
                }
            };
            eprintln!(
                "{}",
                Style {
                    color: std::io::stderr().is_terminal()
                }
                .dim(format!("$ {title}"))
            );
            let status = std::process::Command::new(&program)
                .args(&args)
                .current_dir(dir)
                .status()
                .with_context(|| format!("couldn't start {program}"))?;
            if !status.success() {
                bail!("{program} exited {status}");
            }
            Ok(())
        }
        Command::Refresh => {
            let mine = s.entries.iter().filter(|e| e.placement.is_some()).count();
            let team = s.section(Section::Team, true).count();
            let result = json!({ "fetchedAt": s.raw.fetched_at, "items": mine, "team": team, "errors": errors() });
            out.print(
                || result.clone().into(),
                || result.clone().into(),
                || {
                    let mut text = st.green(format!("Fetched {mine} items and {team} team drift entries."));
                    for e in &s.raw.errors {
                        let line = format!("{}: {}", e.source, e.message);
                        text.push('\n');
                        text.push_str(&if e.unset { st.dim(line) } else { st.yellow(format!("! {line}")) });
                    }
                    text
                },
            )
        }
        Command::Log => {
            let calls = &s.raw.calls;
            if calls.is_empty() {
                bail!("No requests recorded yet; the log starts with the next fetch.");
            }
            out.print(
                || json!({ "fetchedAt": s.raw.fetched_at, "calls": calls }).into(),
                || json!({ "fetchedAt": s.raw.fetched_at, "calls": calls }).into(),
                || {
                    let tz = jiff::tz::TimeZone::system();
                    let rows: Vec<String> = calls
                        .iter()
                        .map(|c| {
                            let at = c.at.to_zoned(tz.clone()).strftime("%H:%M:%S").to_string();
                            let status = c.status.map_or_else(|| "err".to_owned(), |x| x.to_string());
                            let url = c.url.trim_start_matches("https://").trim_start_matches("http://");
                            let error = c.error.as_ref().map(|e| st.red(format!(" ({e})"))).unwrap_or_default();
                            format!("{at} {:>5}ms {status:<4}{:<5}{url}{error}", c.ms, c.method)
                        })
                        .collect();
                    format!("{}\n\n{}", text::header(st, s), rows.join("\n"))
                },
            )
        }
    }
}
