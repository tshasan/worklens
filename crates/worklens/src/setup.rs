use std::io::{BufRead as _, IsTerminal as _, Write as _};

use anyhow::{Context as _, anyhow, bail};
use serde_json::{Map, Value};
use worklens_sources::{Http, JiraAuth, JiraConfig};
use worklens_tui::{Prefs, Setting};

use crate::config::{self, JIRA_SITE, PHABRICATOR};
use crate::doctor::{bugzilla_whoami, jira_whoami, phab_whoami};
use crate::secrets::{self, Origin, Service};
use crate::text::Style;

pub fn needed(cfg: &config::Config) -> bool {
    let s = &cfg.sources;
    s.email.is_none() && s.jira.is_none() && s.phab.is_none() && s.bugzilla_api_key.is_none()
}

fn ask(prompt: &str, default: Option<&str>) -> anyhow::Result<String> {
    match default {
        Some(d) => eprint!("{prompt} [{d}]: "),
        None => eprint!("{prompt}: "),
    }
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let line = line.trim();
    Ok(if line.is_empty() {
        default.unwrap_or_default().to_owned()
    } else {
        line.to_owned()
    })
}

fn yes(prompt: &str) -> anyhow::Result<bool> {
    let answer = ask(&format!("{prompt} [Y/n]"), None)?;
    Ok(!answer.to_lowercase().starts_with('n'))
}

pub fn update_config(change: impl FnOnce(&mut Map<String, Value>)) -> anyhow::Result<()> {
    let path = config::path().ok_or_else(|| anyhow!("no home directory"))?;
    let mut map: Map<String, Value> = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("{} doesn't parse", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(e.into()),
    };
    change(&mut map);
    worklens_store::write_atomic(&path, |out| {
        serde_json::to_writer_pretty(&mut *out, &map).map_err(std::io::Error::other)?;
        out.write_all(b"\n")
    })
    .with_context(|| format!("can't write {}", path.display()))
}

pub fn save_setting(prefs: &Prefs, setting: Setting) -> anyhow::Result<()> {
    update_config(|map| put_setting(map, prefs, setting))
}

fn put_setting(map: &mut Map<String, Value>, prefs: &Prefs, setting: Setting) {
    let key = setting.key();
    if let Some(tab) = setting.tab_key() {
        let mut tabs = map.remove(key).and_then(|v| v.as_object().cloned()).unwrap_or_default();
        if prefs.changed(setting) {
            tabs.insert(tab.to_owned(), prefs.value(setting).into());
        } else {
            tabs.remove(tab);
        }
        if !tabs.is_empty() {
            map.insert(key.to_owned(), tabs.into());
        }
        return;
    }
    if !prefs.changed(setting) {
        map.remove(key);
        return;
    }
    let value: Value = match setting {
        Setting::Whiteboard => prefs.whiteboard.clone().into(),
        Setting::SprintBudget => prefs.sprint_budget.to_string().parse::<f64>().unwrap_or(10.0).into(),
        Setting::Telemetry => prefs.telemetry.into(),
        Setting::LandingChecks => prefs.landing_checks.into(),
        Setting::DriftDays => prefs.drift_days.into(),
        Setting::CacheMinutes => prefs.cache_minutes.into(),
        Setting::Theme => prefs.theme.word().into(),
        Setting::ShowHidden => prefs.show_hidden.into(),
        Setting::Agent => prefs.agent.clone().into(),
        Setting::AgentDir => prefs.agent_dir.clone().into(),
        Setting::Section(_) | Setting::Pane(_) => return,
    };
    map.insert(key.to_owned(), value);
}

pub fn run(st: Style) -> anyhow::Result<()> {
    if !std::io::stdin().is_terminal() {
        bail!("setup asks questions, so run it in a terminal; or use `worklens auth set <service>` with the key on stdin");
    }
    let cfg = config::load()?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let http = Http::new()?;
    let ok = |text: String| eprintln!("  {} {text}", st.green("✓"));
    let bad = |text: String| eprintln!("  {} {text}", st.red("✗"));
    let note = |text: &str| eprintln!("  {}", st.dim(text));

    eprintln!("{}", st.bold("worklens setup"));
    eprintln!("Each key is checked, then kept in {}.", secrets::keychain_name());
    eprintln!("Press Enter to skip anything you don't use.\n");

    let mut email: Option<String> = cfg.sources.email.clone();

    eprintln!("{}", st.bold("Bugzilla"));
    note("A key lets worklens see restricted (security, confidential) bugs and find you by it.");
    note(&format!("Make one at {}", Service::Bugzilla.where_to_get()));
    loop {
        let key = secrets::read_secret("  API key: ")?;
        if key.is_empty() {
            note("skipped: public bugs only");
            break;
        }
        match runtime.block_on(bugzilla_whoami(&http, Some(&key))) {
            Ok(name) => {
                let at = secrets::set(Service::Bugzilla, &key)?;
                ok(format!("signed in as {}; saved in {at}", name.as_deref().unwrap_or("?")));
                email = email.or(name);
                break;
            }
            Err(e) => bad(format!("{}; try again or press Enter to skip", e.summary())),
        }
    }

    eprintln!("\n{}", st.bold("Phabricator"));
    match &cfg.sources.phab {
        Some(p)
            if cfg
                .credentials
                .iter()
                .any(|c| c.service == Service::Phabricator && c.origin.is_some()) =>
        {
            let from = cfg
                .credentials
                .iter()
                .find(|c| c.service == Service::Phabricator)
                .and_then(|c| c.origin.as_ref())
                .map_or_else(String::new, ToString::to_string);
            match runtime.block_on(phab_whoami(&http, &p.host, &p.token)) {
                Ok(who) => {
                    ok(format!("signed in as {who}, with the token from {from}"));
                    email = email.or(Some(who));
                }
                Err(e) => bad(format!("the token from {from} doesn't work: {}", e.summary())),
            }
        }
        _ => {
            note("Reviews and your revisions. A Conduit API token, from");
            note(Service::Phabricator.where_to_get());
            loop {
                let token = secrets::read_secret("  Conduit token: ")?;
                if token.is_empty() {
                    note("skipped: no reviews or revisions");
                    break;
                }
                match runtime.block_on(phab_whoami(&http, PHABRICATOR, &token)) {
                    Ok(who) => {
                        let at = secrets::set(Service::Phabricator, &token)?;
                        ok(format!("signed in as {who}; saved in {at}"));
                        email = email.or(Some(who));
                        break;
                    }
                    Err(e) => bad(format!("{}; try again or press Enter to skip", e.summary())),
                }
            }
        }
    }

    eprintln!("\n{}", st.bold("Jira"));
    note("Optional: your sprint, and where Bugzilla and Jira disagree.");
    let mut jira_settings: Option<(String, String, JiraAuth, worklens_sources::jira::FieldIds)> = None;
    if cfg.sources.jira.is_some() && !yes("  A Jira token is already set up. Set up a different one?")? {
        note("kept");
    } else if yes("  Set up Jira?")? {
        let site = ask("  Jira site", Some(JIRA_SITE))?.trim_end_matches('/').to_owned();
        let account = ask("  Atlassian account email", email.as_deref())?;
        note(&format!("An API token, from {}", Service::Jira.where_to_get()));
        loop {
            let token = secrets::read_secret("  API token: ")?;
            if token.is_empty() {
                note("skipped");
                break;
            }
            let mut found = None;
            for auth in [
                JiraAuth::Classic { email: account.clone() },
                JiraAuth::Scoped { email: account.clone() },
            ] {
                let jira = JiraConfig {
                    site: site.clone(),
                    token: token.clone(),
                    auth: auth.clone(),
                    points_field: String::new(),
                    sprint_field: String::new(),
                };
                if let Ok(who) = runtime.block_on(jira_whoami(&http, &jira)) {
                    let fields = runtime
                        .block_on(async { worklens_sources::jira::Jira::connect(&http, &jira).await?.discover_fields().await })
                        .unwrap_or_default();
                    found = Some((who, auth, fields));
                    break;
                }
            }
            match found {
                Some((who, auth, fields)) => {
                    let at = secrets::set(Service::Jira, &token)?;
                    ok(format!("signed in as {who} ({} token); saved in {at}", auth.name()));
                    match (&fields.story_points, &fields.sprint) {
                        (Some(points), Some(sprint)) => ok(format!("story points are {points}, the sprint is {sprint}")),
                        _ => note("couldn't find the story points or sprint field; set storyPointsField and sprintField in the config"),
                    }
                    email = email.or(Some(who));
                    jira_settings = Some((site, account, auth, fields));
                    break;
                }
                None => bad("neither a classic nor a scoped token works with that email and site; try again or press Enter".into()),
            }
        }
    }

    if email.is_none() {
        eprintln!();
        let typed = ask("Your Bugzilla email (whose bugs to show)", None)?;
        email = (!typed.is_empty()).then_some(typed);
    }

    eprintln!("\n{}", st.bold("Team"));
    note("Your team's Bugzilla whiteboard tag, e.g. [aiplatform]. Team drift is off without one.");
    let whiteboard = ask("  Whiteboard", cfg.core.whiteboard.as_deref())?;
    if whiteboard.is_empty() {
        note("skipped: no team drift");
    }

    let mut sprint_budget: Option<f32> = None;
    if jira_settings.is_some() || cfg.sources.jira.is_some() {
        let current = cfg.core.sprint_budget.to_string();
        loop {
            let typed = ask("  Story points per sprint", Some(&current))?;
            match typed.parse::<f32>() {
                Ok(points) if points > 0.0 => {
                    sprint_budget = Some(points);
                    break;
                }
                _ => bad(format!("{typed} isn't a positive number")),
            }
        }
    }

    update_config(|map| {
        if let Some(email) = &email {
            map.insert("email".into(), email.clone().into());
        }
        if !whiteboard.is_empty() {
            map.insert("whiteboard".into(), whiteboard.clone().into());
        }
        if let Some(points) = sprint_budget {
            map.insert("sprintBudget".into(), points.into());
        }
        if let Some((site, account, auth, fields)) = &jira_settings {
            map.insert("jiraSite".into(), site.clone().into());
            map.insert("jiraEmail".into(), account.clone().into());
            map.insert("jiraAuth".into(), auth.name().into());
            if let Some(points) = &fields.story_points {
                map.insert("storyPointsField".into(), points.clone().into());
            }
            if let Some(sprint) = &fields.sprint {
                map.insert("sprintField".into(), sprint.clone().into());
            }
        }
    })?;
    eprintln!();
    match &email {
        Some(email) => ok(format!("set up for {email}; settings in ~/.config/worklens.json")),
        None => bad("no email: worklens can't tell whose bugs to show; run `worklens setup` again".into()),
    }
    eprintln!(
        "{}",
        st.dim("`worklens doctor` checks everything again; `worklens auth status` shows where each key is.")
    );
    Ok(())
}

pub fn status(st: Style) -> anyhow::Result<()> {
    let cfg = config::load()?;
    for c in &cfg.credentials {
        let line = match (&c.origin, &c.problem) {
            (_, Some(problem)) => format!("{} {:<12} {problem}", st.red("✗"), c.service.label()),
            (Some(origin @ Origin::Borrowed(_)), None) => format!(
                "{} {:<12} {origin}; `worklens auth import` keeps a copy in {}",
                st.yellow("~"),
                c.service.label(),
                secrets::keychain_name()
            ),
            (Some(origin), None) => format!("{} {:<12} {origin}", st.green("✓"), c.service.label()),
            (None, None) => format!(
                "{} {:<12} not set; `worklens auth set {}`",
                st.dim("·"),
                c.service.label(),
                c.service.id()
            ),
        };
        println!("{line}");
    }
    Ok(())
}

pub fn set(service: &str) -> anyhow::Result<()> {
    let service = parse_service(service)?;
    eprintln!("{}: {}", service.label(), service.where_to_get());
    let key = secrets::read_secret(&format!("{} key: ", service.label()))?;
    let at = secrets::set(service, &key)?;
    eprintln!("Saved in {at}. `worklens doctor` checks it.");
    Ok(())
}

pub fn remove(service: &str) -> anyhow::Result<()> {
    let service = parse_service(service)?;
    if secrets::remove(service)? {
        eprintln!("Removed worklens's {} key.", service.label());
    } else {
        eprintln!("worklens had no {} key of its own.", service.label());
    }
    if let Ok(Some(s)) = secrets::get(service)
        && let Origin::Env(var) = s.origin
    {
        eprintln!("${var} is still set and is used first.");
    }
    Ok(())
}

pub fn import() -> anyhow::Result<()> {
    let cfg = config::load()?;
    let mut copied = 0;
    for c in &cfg.credentials {
        let Some(Origin::Borrowed(from)) = &c.origin else { continue };
        let value = match c.service {
            Service::Bugzilla => cfg.sources.bugzilla_api_key.clone(),
            Service::Jira => cfg.sources.jira.as_ref().map(|j| j.token.clone()),
            Service::Phabricator => cfg.sources.phab.as_ref().map(|p| p.token.clone()),
        };
        if let Some(value) = value {
            if let (Service::Jira, Some(j)) = (c.service, &cfg.sources.jira) {
                let auth = j.auth.name();
                update_config(|map| {
                    map.insert("jiraAuth".into(), auth.into());
                })?;
            }
            let at = secrets::set(c.service, &value)?;
            eprintln!("{}: copied from {from} to {at}", c.service.label());
            copied += 1;
        }
    }
    if copied == 0 {
        eprintln!("Nothing to import: every key is already worklens's own, or there are none.");
    }
    Ok(())
}

fn parse_service(name: &str) -> anyhow::Result<Service> {
    Service::parse(name).ok_or_else(|| {
        let names: Vec<&str> = Service::ALL.iter().map(|s| s.id()).collect();
        anyhow!("unknown service {name:?}; one of {}", names.join(", "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_store_only_what_differs_from_the_default() {
        let mut map = Map::new();
        map.insert("email".into(), "me@mozilla.com".into());
        let team = worklens_core::Section::Team.index();
        let mut sections = Prefs::default().sections;
        sections[team] = worklens_tui::Show::Hidden;
        let prefs = Prefs {
            landing_checks: false,
            sprint_budget: 7.5,
            sections,
            ..Prefs::default()
        };
        for s in [
            Setting::LandingChecks,
            Setting::SprintBudget,
            Setting::Section(team),
            Setting::Pane(3),
        ] {
            put_setting(&mut map, &prefs, s);
        }
        assert_eq!(
            Value::Object(map.clone()),
            serde_json::json!({"email": "me@mozilla.com", "landingChecks": false, "sprintBudget": 7.5, "sectionTabs": {"team": "hidden"}})
        );
        let defaults = Prefs::default();
        for s in [Setting::LandingChecks, Setting::SprintBudget, Setting::Section(team)] {
            put_setting(&mut map, &defaults, s);
        }
        assert_eq!(Value::Object(map), serde_json::json!({"email": "me@mozilla.com"}));
    }
}
