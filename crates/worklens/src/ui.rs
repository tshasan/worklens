use std::cell::OnceCell;
use std::collections::VecDeque;
use std::future::Future;
use std::io::BufRead as _;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Context as _;
use crossterm::event::{DisableFocusChange, EnableFocusChange, Event, EventStream, KeyEventKind};
use crossterm::execute;
use futures::StreamExt as _;
use futures::stream::FuturesUnordered;
use jiff::Timestamp;
use worklens_core::Raw;
use worklens_core::marks::Marks;
use worklens_store::{cache_file, marks_file, read_marks, read_raw, write_raw};
use worklens_tui::{App, Command, Effect, Msg, Options, Setting, Touched, update, view};

use crate::config::Config;

type Job<'a> = Pin<Box<dyn Future<Output = Msg> + 'a>>;

type Child = Pin<Box<dyn Future<Output = Option<Msg>>>>;

pub fn run(cfg: &Config, cached: bool) -> anyhow::Result<()> {
    let raw = cache_file().and_then(|p| read_raw(&p).ok());
    let marks = match marks_file().map(|p| read_marks(&p)) {
        Some(Ok(m)) => m,
        _ => Marks::default(),
    };
    let opts = Options {
        cache_minutes: (!cached).then_some(cfg.cache_minutes),
        agent: cfg.agent.clone(),
        dir: cfg.agent_dir.clone(),
        keys: cfg.keys.clone(),
    };
    let saved = marks.clone();
    let mut app = App::new(cfg.core.clone(), opts, raw, marks, Timestamp::now());
    app.set_prefs(cfg.prefs.clone());
    if let Some(path) = worklens_store::drift_file() {
        app.drift_log = worklens_store::read_drift_log(&path);
    }
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(event_loop(app, cfg, saved))
}

fn enter() -> anyhow::Result<ratatui::DefaultTerminal> {
    let terminal = ratatui::try_init()?;
    execute!(std::io::stdout(), EnableFocusChange)?;
    Ok(terminal)
}

fn leave() {
    let _ = execute!(std::io::stdout(), DisableFocusChange);
    ratatui::restore();
}

async fn event_loop(mut app: App, cfg: &Config, mut saved: Marks) -> anyhow::Result<()> {
    let full: OnceCell<Config> = OnceCell::new();
    let mut sources = cfg.sources.clone();
    let mut terminal = enter()?;
    app.tone = crate::tone::detect(cfg.theme.as_deref());
    let size = terminal.size()?;
    app.size = (size.width, size.height);
    let mut events = EventStream::new();
    let (progress_tx, mut progress) = tokio::sync::mpsc::channel::<String>(16);
    let mut job: Option<Job<'_>> = None;
    let mut checks: Option<Job<'_>> = None;
    let mut children: FuturesUnordered<Child> = FuturesUnordered::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut pending: VecDeque<Effect> = app.start(Timestamp::now()).into();
    let mut follow_ups: VecDeque<Msg> = VecDeque::new();

    let result: anyhow::Result<()> = async {
        loop {
            while let Some(effect) = pending.pop_front() {
                match effect {
                    Effect::Quit => return Ok(()),
                    Effect::Refresh if job.is_none() => {
                        with_secrets(&full, cfg, &mut sources);
                        job = Some(Box::pin(fetch(sources.clone(), progress_tx.clone())));
                    }
                    Effect::Refetch(items) if job.is_none() => {
                        with_secrets(&full, cfg, &mut sources);
                        job = Some(Box::pin(refetch(sources.clone(), items)));
                    }
                    Effect::SaveSetting { setting, done } => match crate::setup::save_setting(&app.prefs, setting) {
                        Ok(()) => {
                            if setting == Setting::Theme {
                                drop(events);
                                app.tone = crate::tone::detect(Some(app.prefs.theme.word()));
                                events = EventStream::new();
                                app.dirty = true;
                            }
                            apply_setting(&mut app, &mut sources, setting);
                            follow_ups.push_back(Msg::Done(Ok(done)));
                        }
                        Err(e) => follow_ups.push_back(Msg::Done(Err(format!("Couldn't save {}: {e:#}", setting.label())))),
                    },
                    Effect::Refresh | Effect::Refetch(_) => {}
                    Effect::RunChecks => {
                        let resolved = with_secrets(&full, cfg, &mut sources);
                        follow_ups.push_back(Msg::Checks(crate::doctor::pending_checks(resolved)));
                        checks = Some(Box::pin(async { Msg::Checks(crate::doctor::run_checks(resolved).await) }));
                    }
                    Effect::RunInTerminal { command, pause } => {
                        drop(events);
                        leave();
                        let outcome = run_in_terminal(&command, pause);
                        terminal = enter()?;
                        terminal.clear()?;
                        events = EventStream::new();
                        if let Err(e) = outcome {
                            follow_ups.push_back(Msg::Done(Err(format!("{}: {e}", command.title))));
                        }
                        app.dirty = true;
                    }
                    other => match run_effect(other, &app.marks, &mut saved) {
                        Started::Nothing => {}
                        Started::Now(msg) => follow_ups.push_back(msg),
                        Started::Later(child) => children.push(child),
                    },
                }
            }
            if let Some(msg) = follow_ups.pop_front() {
                pending.extend(update(&mut app, msg, Timestamp::now()));
                continue;
            }
            if app.dirty {
                terminal.draw(|f| view(&app, f))?;
                app.dirty = false;
            }
            let msg = tokio::select! {
                ev = events.next() => match ev {
                    Some(Ok(Event::Key(k))) if k.kind != KeyEventKind::Release => Msg::Key(k),
                    Some(Ok(Event::Resize(w, h))) => Msg::Resize(w, h),
                    Some(Ok(Event::FocusGained)) => Msg::Focus(true),
                    Some(Ok(Event::FocusLost)) => Msg::Focus(false),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return Err(e.into()),
                    None => return Ok(()),
                },
                msg = async { job.as_mut().expect("guarded by the precondition").await }, if job.is_some() => {
                    job = None;
                    msg
                }
                msg = async { checks.as_mut().expect("guarded by the precondition").await }, if checks.is_some() => {
                    checks = None;
                    msg
                }
                Some(done) = children.next(), if !children.is_empty() => match done {
                    Some(msg) => msg,
                    None => continue,
                },
                Some(text) = progress.recv() => Msg::Progress(text),
                _ = tick.tick() => Msg::Tick,
            };
            pending.extend(update(&mut app, msg, Timestamp::now()));
        }
    }
    .await;
    leave();
    if let Some(path) = marks_file() {
        worklens_store::save_marks(&path, &saved, &app.marks).context("saving marks")?;
    }
    result
}

fn with_secrets<'c>(full: &'c OnceCell<Config>, cfg: &Config, sources: &mut worklens_sources::Config) -> &'c Config {
    let resolved = full.get_or_init(|| crate::config::with_secrets(cfg.clone()));
    sources.bugzilla_api_key.clone_from(&resolved.sources.bugzilla_api_key);
    sources.jira.clone_from(&resolved.sources.jira);
    sources.phab.clone_from(&resolved.sources.phab);
    resolved
}

fn apply_setting(app: &mut App, sources: &mut worklens_sources::Config, setting: Setting) {
    let p = &app.prefs;
    match setting {
        Setting::Whiteboard => sources.whiteboard = Some(p.whiteboard.clone()).filter(|w| !w.is_empty()),
        Setting::Telemetry => sources.telemetry = p.telemetry,
        Setting::LandingChecks => sources.landing_checks = p.landing_checks,
        Setting::DriftDays => sources.drift_days = p.drift_days,
        Setting::Agent => app.opts.agent = crate::config::find_agent(Some(&p.agent)),
        Setting::AgentDir => {
            let home = crate::config::home();
            app.opts.dir = crate::config::agent_dir(home.as_deref(), &p.agent_dir);
            sources.repo = crate::config::local_repo(home.as_deref(), &p.agent_dir);
        }
        _ => {}
    }
}

async fn fetch(cfg: worklens_sources::Config, progress: tokio::sync::mpsc::Sender<String>) -> Msg {
    let result = async {
        let http = worklens_sources::Http::new()?;
        let previous = cache_file().and_then(|path| read_raw(&path).ok());
        let mut raw = Box::pin(worklens_sources::fetch_raw(&http, &cfg, previous, |text| {
            drop(progress.try_send(text.to_owned()))
        }))
        .await?;
        raw.calls = http.take_calls();
        save(&raw)?;
        anyhow::Ok(raw)
    }
    .await;
    Msg::Fetched(result.map(Box::new).map_err(|e| format!("{e:#}")))
}

async fn refetch(cfg: worklens_sources::Config, items: Vec<Touched>) -> Msg {
    let result = async {
        let raw = read_raw(&cache_file().context("no home directory")?)?;
        let refs: Vec<worklens_sources::ItemRef> = items
            .into_iter()
            .map(|t| worklens_sources::ItemRef {
                bug: t.bug,
                revisions: t.revisions,
                jira: t.jira,
            })
            .collect();
        let http = worklens_sources::Http::new()?;
        let raw = Box::pin(worklens_sources::refetch_items(&http, &cfg, raw, &refs)).await?;
        save(&raw)?;
        anyhow::Ok(raw)
    }
    .await;
    Msg::Fetched(result.map(Box::new).map_err(|e| format!("{e:#}")))
}

fn save(raw: &Raw) -> anyhow::Result<()> {
    write_raw(&cache_file().context("no home directory")?, raw)?;
    Ok(())
}

const QUIET_TIMEOUT: Duration = Duration::from_secs(60);
const COPY_TIMEOUT: Duration = Duration::from_secs(5);
const OPEN_TIMEOUT: Duration = Duration::from_secs(60);

enum Started {
    Nothing,
    Now(Msg),
    Later(Child),
}

fn run_effect(effect: Effect, marks: &Marks, saved: &mut Marks) -> Started {
    match effect {
        Effect::Open(url) => Started::Later(Box::pin(open(url))),
        Effect::Copy { text, done } => Started::Later(Box::pin(async move {
            match copy(&text).await {
                Ok(()) => Some(Msg::Done(Ok(done))),
                Err(e) => Some(Msg::Done(Err(e))),
            }
        })),
        Effect::SaveMarks => match marks_file().map(|p| worklens_store::save_marks(&p, saved, marks)) {
            Some(Err(e)) => Started::Now(Msg::Done(Err(format!("Couldn't save marks: {e}")))),
            Some(Ok(())) => {
                saved.clone_from(marks);
                Started::Nothing
            }
            None => Started::Nothing,
        },
        Effect::RecordDrift(day) => match worklens_store::drift_file().map(|p| worklens_store::record_drift(&p, day)) {
            Some(Err(e)) => Started::Now(Msg::Done(Err(format!("Couldn't save the drift count: {e}")))),
            _ => Started::Nothing,
        },
        Effect::RunQuietly { command, label } => {
            Started::Later(Box::pin(async move { Some(run_quietly(&command, &label, QUIET_TIMEOUT).await) }))
        }
        Effect::Refresh
        | Effect::Refetch(_)
        | Effect::RunChecks
        | Effect::RunInTerminal { .. }
        | Effect::SaveSetting { .. }
        | Effect::Quit => Started::Nothing,
    }
}

async fn open(url: String) -> Option<Msg> {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let status = tokio::process::Command::new(opener)
        .arg(&url)
        .stdin(std::process::Stdio::null())
        .status();
    match tokio::time::timeout(OPEN_TIMEOUT, status).await {
        Ok(Ok(s)) if s.success() => None,
        Ok(Ok(s)) => Some(Msg::Done(Err(format!("{opener} exited {s}")))),
        Ok(Err(e)) => Some(Msg::Done(Err(format!("{opener}: {e}")))),
        Err(_) => None,
    }
}

async fn run_quietly(command: &Command, label: &str, limit: Duration) -> Msg {
    let output = tokio::process::Command::new(&command.program)
        .args(&command.args)
        .current_dir(&command.cwd)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    Msg::Done(match tokio::time::timeout(limit, output).await {
        Ok(Ok(o)) if o.status.success() => Ok(format!("{label}: done")),
        Ok(Ok(o)) => Err(String::from_utf8_lossy(&o.stderr)
            .lines()
            .next()
            .map_or_else(|| format!("{label} exited {}", o.status), str::to_owned)),
        Ok(Err(e)) => Err(format!("{label}: {e}")),
        Err(_) => Err(format!("{label}: still running after {}s, stopped it", limit.as_secs())),
    })
}

async fn copy(text: &str) -> Result<(), String> {
    use tokio::io::AsyncWriteExt as _;
    let tools: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else {
        &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"])]
    };
    for (tool, args) in tools {
        let Ok(mut child) = tokio::process::Command::new(tool)
            .args(*args)
            .stdin(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        else {
            continue;
        };
        let feed_and_wait = async {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(text.as_bytes()).await.map_err(|e| e.to_string())?;
            }
            child.wait().await.map_err(|e| e.to_string())
        };
        return match tokio::time::timeout(COPY_TIMEOUT, feed_and_wait).await {
            Ok(Ok(s)) if s.success() => Ok(()),
            Ok(Ok(s)) => Err(format!("{tool} exited {s}")),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(format!("{tool} didn't finish in {}s", COPY_TIMEOUT.as_secs())),
        };
    }
    Err("No clipboard tool found (pbcopy, wl-copy or xclip)".into())
}

fn run_in_terminal(command: &Command, pause: bool) -> anyhow::Result<()> {
    eprintln!("\x1b[2m$ {}\x1b[0m", command.title);
    let status = std::process::Command::new(&command.program)
        .args(&command.args)
        .current_dir(&command.cwd)
        .status();
    let note = match &status {
        Ok(s) if s.success() => "done".to_owned(),
        Ok(s) => s.code().map_or_else(|| format!("ended: {s}"), |c| format!("exited {c}")),
        Err(e) => format!("failed: {e}"),
    };
    if pause || status.is_err() {
        eprint!("\n\x1b[2m{note} · press enter to go back to worklens\x1b[0m");
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
    }
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => anyhow::bail!("{note}"),
        Err(e) => Err(e).with_context(|| format!("couldn't start {}", command.program)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        Command {
            title: script.into(),
            program: "sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir(),
        }
    }

    fn block_on<F: Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn a_quiet_command_reports_success_failure_and_a_hang() {
        let ok = block_on(run_quietly(&sh("true"), "Ok", QUIET_TIMEOUT));
        assert!(matches!(ok, Msg::Done(Ok(ref t)) if t == "Ok: done"), "{ok:?}");
        let failed = block_on(run_quietly(&sh("echo nope >&2; exit 3"), "Fails", QUIET_TIMEOUT));
        assert!(matches!(failed, Msg::Done(Err(ref t)) if t == "nope"), "{failed:?}");
        let started = std::time::Instant::now();
        let hung = block_on(run_quietly(&sh("sleep 30"), "Hangs", Duration::from_millis(200)));
        assert!(matches!(hung, Msg::Done(Err(ref t)) if t.contains("stopped it")), "{hung:?}");
        assert!(started.elapsed() < Duration::from_secs(5), "the timeout ends the wait");
    }

    #[test]
    fn a_terminal_command_that_exits_non_zero_is_an_error() {
        assert!(run_in_terminal(&sh("true"), false).is_ok());
        let e = run_in_terminal(&sh("exit 4"), false).unwrap_err();
        assert_eq!(e.to_string(), "exited 4");
    }
}
