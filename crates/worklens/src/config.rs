use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::Deserialize;
use worklens_core::CoreConfig;
use worklens_sources::{JiraAuth, JiraConfig, PhabConfig};
use worklens_tui::{Prefs, Show, ThemeName};

use crate::secrets::{self, Origin, Secret, Service};

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct File {
    whiteboard: Option<String>,
    sprint_budget: Option<f32>,
    email: Option<String>,
    jira_site: Option<String>,
    jira_email: Option<String>,
    jira_auth: Option<String>,
    story_points_field: Option<String>,
    sprint_field: Option<String>,
    bugzilla_api_key: Option<String>,
    drift_days: Option<i64>,
    cache_minutes: Option<f64>,
    agent: Option<String>,
    agent_dir: Option<String>,
    theme: Option<String>,
    telemetry: Option<bool>,
    landing_checks: Option<bool>,
    show_hidden: Option<bool>,
    #[serde(default)]
    section_tabs: HashMap<String, String>,
    #[serde(default)]
    detail_tabs: HashMap<String, String>,
    jira_statuses: Option<JiraStatusesFile>,
    #[serde(default)]
    jbi_syncs: Vec<worklens_core::drift::Synced>,
    #[serde(default)]
    keys: std::collections::BTreeMap<String, KeyFile>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JiraStatusesFile {
    done: Option<Vec<String>>,
    review: Option<Vec<String>>,
    in_progress: Option<Vec<String>>,
    blocked: Option<Vec<String>>,
}

fn jira_statuses(file: Option<&JiraStatusesFile>) -> worklens_core::drift::JiraStatuses {
    let mut statuses = worklens_core::drift::JiraStatuses::default();
    let Some(file) = file else { return statuses };
    let pick = |list: &Option<Vec<String>>, default: &mut Vec<String>| {
        if let Some(list) = list.as_ref().filter(|l| !l.is_empty()) {
            default.clone_from(list);
        }
    };
    pick(&file.done, &mut statuses.done);
    pick(&file.review, &mut statuses.review);
    pick(&file.in_progress, &mut statuses.in_progress);
    pick(&file.blocked, &mut statuses.blocked);
    statuses
}

#[derive(Deserialize)]
struct KeyFile {
    label: String,
    run: String,
    #[serde(default)]
    terminal: bool,
}

#[derive(Clone)]
pub struct Config {
    pub core: CoreConfig,
    pub sources: worklens_sources::Config,
    pub cache_minutes: f64,
    pub agent: Option<worklens_tui::Agent>,
    pub agent_dir: PathBuf,
    pub keys: Vec<worklens_tui::CustomKey>,
    pub theme: Option<String>,
    pub prefs: Prefs,
    pub bad_keys: Vec<String>,
    pub credentials: Vec<Credential>,
    pending: Pending,
}

#[derive(Clone)]
struct Pending {
    bugzilla_api_key: Option<String>,
    jira_site: String,
    jira_email: Option<String>,
    jira_auth: Option<String>,
    points_field: String,
    sprint_field: String,
}

#[derive(Clone)]
pub struct Credential {
    pub service: Service,
    pub origin: Option<Origin>,
    pub problem: Option<String>,
}

pub fn home() -> Option<PathBuf> {
    std::env::home_dir()
}

pub fn path() -> Option<PathBuf> {
    home().map(|h| h.join(".config/worklens.json"))
}

pub const PHABRICATOR: &str = "https://phabricator.services.mozilla.com/api/";
pub const JIRA_SITE: &str = "https://mozilla-hub.atlassian.net";

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

pub fn load() -> anyhow::Result<Config> {
    load_without_secrets().map(with_secrets)
}

pub fn load_without_secrets() -> anyhow::Result<Config> {
    let file: File = match path().map(std::fs::read) {
        Some(Ok(bytes)) => serde_json::from_slice(&bytes).context("~/.config/worklens.json doesn't parse")?,
        _ => File::default(),
    };
    let prefs = prefs(&file);
    let whiteboard = file.whiteboard.clone().filter(|w| !w.trim().is_empty());
    let home = home();
    let keys = custom_keys(file.keys);
    let pending = Pending {
        bugzilla_api_key: file.bugzilla_api_key,
        jira_site: file.jira_site.unwrap_or_else(|| JIRA_SITE.to_owned()),
        jira_email: file.jira_email.or_else(|| file.email.clone()),
        jira_auth: file.jira_auth,
        points_field: file.story_points_field.unwrap_or_else(|| "customfield_10037".to_owned()),
        sprint_field: file.sprint_field.unwrap_or_else(|| "customfield_10020".to_owned()),
    };

    Ok(Config {
        core: CoreConfig {
            whiteboard: whiteboard.clone(),
            sprint_budget: prefs.sprint_budget,
            jira_statuses: jira_statuses(file.jira_statuses.as_ref()),
            jbi_syncs: file.jbi_syncs,
        },
        sources: worklens_sources::Config {
            email: file.email,
            bugzilla_api_key: None,
            jira: None,
            phab: None,
            whiteboard,
            drift_days: prefs.drift_days,
            telemetry: prefs.telemetry,
            landing_checks: prefs.landing_checks,
        },
        cache_minutes: prefs.cache_minutes,
        agent: find_agent(Some(&prefs.agent)),
        agent_dir: agent_dir(home.as_deref(), &prefs.agent_dir),
        keys: keys.0,
        theme: file.theme,
        prefs,
        bad_keys: keys.1,
        credentials: Vec::new(),
        pending,
    })
}

pub fn with_secrets(mut cfg: Config) -> Config {
    let home = home();
    let pending = &cfg.pending;
    let mut credentials = Vec::with_capacity(Service::ALL.len());
    let mut own = |service: Service, borrowed: Option<Secret>| -> Option<Secret> {
        let (found, problem) = match secrets::get(service) {
            Ok(found) => (found, None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let found = found.or(borrowed);
        credentials.push(Credential {
            service,
            origin: found.as_ref().map(|s| s.origin.clone()),
            problem,
        });
        found
    };

    let bugzilla = own(
        Service::Bugzilla,
        pending
            .bugzilla_api_key
            .clone()
            .map(|value| Secret {
                value,
                origin: Origin::Borrowed("bugzillaApiKey in ~/.config/worklens.json".into()),
            })
            .or_else(|| {
                std::env::var("BUGZILLA_API_KEY")
                    .ok()
                    .filter(|v| !v.is_empty())
                    .map(|value| Secret {
                        value,
                        origin: Origin::Borrowed("$BUGZILLA_API_KEY".into()),
                    })
            }),
    );

    let jira_aiplat = home.as_deref().and_then(|h| jira_token(h, &pending.jira_site));
    let jira = own(
        Service::Jira,
        jira_aiplat.map(|value| Secret {
            value,
            origin: Origin::Borrowed("~/.jira-aiplat.json".into()),
        }),
    )
    .map(|secret| JiraConfig {
        site: pending.jira_site.clone(),
        auth: jira_auth(pending.jira_auth.as_deref(), pending.jira_email.clone(), &secret.origin),
        token: secret.value,
        points_field: pending.points_field.clone(),
        sprint_field: pending.sprint_field.clone(),
    });

    let arcrc = home.as_deref().and_then(phab_token);
    let arc_host = arcrc.as_ref().map(|p| p.host.clone());
    let phab = own(
        Service::Phabricator,
        arcrc.map(|p| Secret {
            value: p.token,
            origin: Origin::Borrowed("~/.arcrc".into()),
        }),
    )
    .map(|secret| PhabConfig {
        host: match &secret.origin {
            Origin::Borrowed(_) => arc_host.unwrap_or_else(|| PHABRICATOR.to_owned()),
            _ => PHABRICATOR.to_owned(),
        },
        token: secret.value,
    });

    cfg.sources.bugzilla_api_key = bugzilla.map(|s| s.value);
    cfg.sources.jira = jira;
    cfg.sources.phab = phab;
    cfg.credentials = credentials;
    cfg
}

fn prefs(file: &File) -> Prefs {
    let d = Prefs::default();
    let mut sections = d.sections;
    for (i, s) in worklens_core::section::Section::ALL.iter().enumerate().skip(1) {
        if let Some(show) = file.section_tabs.get(s.id()).and_then(|v| Show::parse(v)) {
            sections[i] = show;
        }
    }
    let mut panes = d.panes;
    for (i, p) in worklens_tui::app::Pane::ALL.iter().enumerate().skip(1) {
        if let Some(show) = file.detail_tabs.get(p.id()).and_then(|v| Show::parse(v)) {
            panes[i] = show;
        }
    }
    Prefs {
        whiteboard: file.whiteboard.clone().map(|w| w.trim().to_owned()).unwrap_or_default(),
        sprint_budget: file.sprint_budget.unwrap_or(d.sprint_budget),
        telemetry: file.telemetry.unwrap_or(d.telemetry),
        landing_checks: file.landing_checks.unwrap_or(d.landing_checks),
        drift_days: file.drift_days.unwrap_or(d.drift_days),
        cache_minutes: file.cache_minutes.unwrap_or(d.cache_minutes),
        theme: file.theme.as_deref().and_then(ThemeName::parse).unwrap_or(d.theme),
        show_hidden: file.show_hidden.unwrap_or(d.show_hidden),
        agent: file.agent.clone().map(|a| a.trim().to_owned()).unwrap_or_default(),
        agent_dir: file.agent_dir.clone().filter(|a| !a.trim().is_empty()).unwrap_or(d.agent_dir),
        sections,
        panes,
    }
}

fn jira_auth(configured: Option<&str>, email: Option<String>, origin: &Origin) -> JiraAuth {
    let Some(email) = email else { return JiraAuth::Bearer };
    match configured {
        Some("bearer") => JiraAuth::Bearer,
        Some("scoped") => JiraAuth::Scoped { email },
        Some("classic") => JiraAuth::Classic { email },
        _ if matches!(origin, Origin::Borrowed(_)) => JiraAuth::Bearer,
        _ => JiraAuth::Classic { email },
    }
}

fn custom_keys(keys: std::collections::BTreeMap<String, KeyFile>) -> (Vec<worklens_tui::CustomKey>, Vec<String>) {
    let mut good = Vec::with_capacity(keys.len());
    let mut bad = Vec::new();
    for (name, k) in keys {
        let mut chars = name.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => good.push(worklens_tui::CustomKey {
                key: c,
                label: k.label,
                run: k.run,
                terminal: k.terminal,
            }),
            _ => bad.push(name),
        }
    }
    (good, bad)
}

const KNOWN_AGENTS: [&str; 2] = ["claude", "codex"];

pub fn find_agent(configured: Option<&str>) -> Option<worklens_tui::Agent> {
    if let Some(cmd) = configured.map(str::trim).filter(|c| !c.is_empty()) {
        let mut words = cmd.split_whitespace().map(str::to_owned);
        let program = words.next()?;
        let name = Path::new(&program)
            .file_name()
            .map_or_else(|| program.clone(), |n| n.to_string_lossy().into_owned());
        return Some(worklens_tui::Agent {
            name,
            program,
            args: words.collect(),
        });
    }
    let path = std::env::var_os("PATH")?;
    KNOWN_AGENTS
        .iter()
        .find(|cmd| std::env::split_paths(&path).any(|dir| dir.join(cmd).is_file()))
        .map(|cmd| worklens_tui::Agent {
            name: (*cmd).to_owned(),
            program: (*cmd).to_owned(),
            args: Vec::new(),
        })
}

pub fn agent_dir(home: Option<&Path>, configured: &str) -> PathBuf {
    let expanded = match (configured.strip_prefix('~'), home) {
        (Some(rest), Some(home)) => home.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(configured),
    };
    if expanded.is_dir() {
        expanded
    } else {
        std::env::current_dir().unwrap_or(expanded)
    }
}

fn jira_token(home: &Path, site: &str) -> Option<String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct JiraAiplat {
        jira_auth: Vec<Auth>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Auth {
        url: String,
        api_token: String,
    }
    let file: JiraAiplat = read_json(&home.join(".jira-aiplat.json"))?;
    file.jira_auth.into_iter().find(|a| a.url == site).map(|a| a.api_token)
}

fn phab_token(home: &Path) -> Option<PhabConfig> {
    #[derive(Deserialize)]
    struct Arcrc {
        hosts: HashMap<String, Host>,
    }
    #[derive(Deserialize)]
    struct Host {
        token: Option<String>,
    }
    let arc: Arcrc = read_json(&home.join(".arcrc"))?;
    let (host, entry) = arc.hosts.into_iter().find(|(h, _)| {
        h.strip_prefix("https://")
            .and_then(|rest| rest.split('/').next())
            .is_some_and(|host| host == "phabricator.services.mozilla.com")
    })?;
    let host = if host.ends_with('/') { host } else { format!("{host}/") };
    Some(PhabConfig { host, token: entry.token? })
}

#[cfg(test)]
mod tests {
    use super::File;

    #[test]
    fn jbi_syncs_reads_known_fields_and_names_them_for_an_unknown_one() {
        let ok: File = serde_json::from_str(r#"{ "jbiSyncs": ["summary"] }"#).unwrap();
        assert_eq!(ok.jbi_syncs, vec![worklens_core::drift::Synced::Summary]);
        let none: File = serde_json::from_str("{}").unwrap();
        assert!(none.jbi_syncs.is_empty(), "off unless the team says JBI syncs it");
        let Err(err) = serde_json::from_str::<File>(r#"{ "jbiSyncs": ["priority"] }"#) else {
            panic!("priority isn't checked yet, so it must not be accepted silently")
        };
        assert!(err.to_string().contains("expected `summary`"), "{err}");
    }
}
