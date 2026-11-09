use serde::Deserialize;
use worklens_sources::bugzilla::BZ;
use worklens_sources::http::{Http, parse};
use worklens_sources::jira::Jira;
use worklens_sources::phab::Phab;
use worklens_tui::Check;

use crate::config::Config;

pub async fn bugzilla_whoami(http: &Http, key: Option<&str>) -> worklens_sources::http::Result<Option<String>> {
    #[derive(Deserialize)]
    struct WhoAmI {
        name: String,
    }
    match key {
        None => http.send(http.client.get(format!("{BZ}/rest/version"))).await.map(|_| None),
        Some(key) => {
            let req = http.client.get(format!("{BZ}/rest/whoami")).header("X-BUGZILLA-API-KEY", key);
            let bytes = http.send(req).await?;
            Ok(Some(parse::<WhoAmI>("Bugzilla whoami", &bytes)?.name))
        }
    }
}

pub async fn phab_whoami(http: &Http, host: &str, token: &str) -> worklens_sources::http::Result<String> {
    Ok(Phab { http, host, token }.whoami().await?.primary_email)
}

pub async fn jira_whoami(http: &Http, cfg: &worklens_sources::JiraConfig) -> worklens_sources::http::Result<String> {
    Jira::connect(http, cfg).await?.me().await
}

fn with_last_fetch(checks: &mut Vec<Check>, cfg: &Config) {
    let Some(raw) = worklens_store::cache_file().and_then(|p| worklens_store::read_raw(&p).ok()) else {
        return;
    };
    for h in crate::health::from_cache(&raw, cfg) {
        let line = crate::health::describe(&h);
        let healthy = matches!(h.state, "ok" | "unused" | "off");
        match checks.iter_mut().find(|c| c.name.eq_ignore_ascii_case(h.name)) {
            Some(c) => {
                c.detail = format!("{} · last fetch: {line}", c.detail);
                if !healthy {
                    c.ok = false;
                    c.fix = c.fix.take().or_else(|| crate::health::fix(&h));
                }
            }
            None if h.state != "unused" => checks.push(check(
                &h.name.to_lowercase(),
                false,
                healthy,
                format!("last fetch: {line}"),
                crate::health::fix(&h).as_deref(),
            )),
            None => {}
        }
    }
}

pub fn pending_checks(cfg: &Config) -> Vec<Check> {
    let jira = match &cfg.sources.jira {
        Some(_) => pending("jira", false, "checking the Jira token"),
        None => no_jira(),
    };
    let phab = match &cfg.sources.phab {
        Some(_) => pending("phabricator", false, "checking the Conduit token"),
        None => no_phab(),
    };
    let you = match &cfg.sources.email {
        Some(email) => check("you", true, true, email.clone(), None),
        None => pending("you", true, "looking up your email"),
    };
    let bugzilla = pending("bugzilla", true, "checking Bugzilla answers");
    let local = match &cfg.sources.repo {
        Some(_) => pending("local", false, "reading your branches"),
        None => no_local(cfg),
    };
    let mut out = vec![you, bugzilla, jira, phab, local, agent_check(cfg)];
    with_last_fetch(&mut out, cfg);
    out.extend(keys_check(cfg));
    out
}

fn pending(name: &str, required: bool, detail: &str) -> Check {
    Check {
        pending: true,
        ..check(name, required, true, detail, None)
    }
}

const JIRA_FIX: &str =
    "`worklens auth set jira` (an API token from https://id.atlassian.com/manage-profile/security/api-tokens), or `worklens setup`";
const PHAB_FIX: &str = "`worklens auth set phabricator` (a Conduit API token from https://phabricator.services.mozilla.com/settings/), or `moz-phab install-certificate`";

fn no_jira() -> Check {
    check("jira", false, false, "no token; sprint and Jira drift are skipped", Some(JIRA_FIX))
}

fn no_phab() -> Check {
    check(
        "phabricator",
        false,
        false,
        "no token; reviews and revisions are skipped",
        Some(PHAB_FIX),
    )
}

const BUILT_IN: &str = "BJPLTbcyCRFdzZoqrs?/jkhlgG12345678@";

fn check(name: &str, required: bool, ok: bool, detail: impl Into<String>, fix: Option<&str>) -> Check {
    Check {
        name: name.into(),
        ok,
        required,
        detail: detail.into(),
        fix: fix.map(str::to_owned),
        pending: false,
    }
}

pub async fn run_checks(cfg: &Config) -> Vec<Check> {
    let http = match Http::new() {
        Ok(h) => h,
        Err(e) => return vec![check("network", true, false, e.summary(), None)],
    };
    let jira = async {
        let fix = Some(JIRA_FIX);
        let Some(j) = &cfg.sources.jira else {
            return (no_jira(), None);
        };
        match jira_whoami(&http, j).await {
            Ok(email) => (
                check("jira", false, true, format!("signed in as {email} ({} token)", j.auth.name()), None),
                Some(email),
            ),
            Err(e) => (check("jira", false, false, e.summary(), fix), None),
        }
    };
    let phab = async {
        let fix = Some(PHAB_FIX);
        let Some(p) = &cfg.sources.phab else {
            return (no_phab(), None);
        };
        match phab_whoami(&http, &p.host, &p.token).await {
            Ok(email) => (
                check("phabricator", false, true, format!("signed in as {email}"), None),
                Some(email),
            ),
            Err(e) => (check("phabricator", false, false, e.summary(), fix), None),
        }
    };
    let bugzilla = async {
        let fix = Some("check your network; if you set a Bugzilla key, check it's still valid (`worklens auth set bugzilla`)");
        match bugzilla_whoami(&http, cfg.sources.bugzilla_api_key.as_deref()).await {
            Ok(None) => check(
                "bugzilla",
                true,
                true,
                "reachable, anonymous (public bugs only; `worklens auth set bugzilla` for restricted bugs)",
                None,
            ),
            Ok(Some(name)) => check("bugzilla", true, true, format!("signed in as {name}"), None),
            Err(e) => check("bugzilla", true, false, e.summary(), fix),
        }
    };
    let ((jira, jira_email), (phab, phab_email), bugzilla, local) = futures::join!(jira, phab, bugzilla, local_check(cfg));

    let you = match cfg.sources.email.clone().or(jira_email).or(phab_email) {
        Some(email) => check("you", true, true, email, None),
        None => check(
            "you",
            true,
            false,
            "no email to look up your bugs with",
            Some("run `worklens setup`"),
        ),
    };
    let mut out = vec![you, bugzilla, jira, phab, local, agent_check(cfg)];
    with_last_fetch(&mut out, cfg);
    out.extend(keys_check(cfg));
    out
}

const LOCAL_FIX: &str = "set Agent › Directory in `worklens setup` to your Firefox git checkout";

fn no_local(cfg: &Config) -> Check {
    check(
        "local",
        false,
        false,
        format!("{} isn't a git checkout; local commits aren't tracked", cfg.prefs.agent_dir),
        Some(LOCAL_FIX),
    )
}

async fn local_check(cfg: &Config) -> Check {
    let Some(repo) = &cfg.sources.repo else { return no_local(cfg) };
    let email = cfg.sources.email.as_deref().unwrap_or_default();
    match worklens_sources::local::scan(repo, email).await {
        Ok(s) => check(
            "local",
            false,
            true,
            format!(
                "{}: {} commits of yours from the last {} days aren't on {}",
                repo.display(),
                s.commits.len(),
                worklens_sources::local::DAYS,
                s.base
            ),
            None,
        ),
        Err(e) => check(
            "local",
            false,
            false,
            e,
            Some("check that `git status` works there and the checkout has an origin remote"),
        ),
    }
}

fn agent_check(cfg: &Config) -> Check {
    match &cfg.agent {
        Some(a) => check(
            "agent",
            false,
            true,
            format!(
                "{} (c key, worklens start)",
                std::iter::once(a.program.as_str())
                    .chain(a.args.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            None,
        ),
        None => check(
            "agent",
            false,
            false,
            "no claude or codex on PATH; the c key copies the brief instead",
            Some("install one, or set \"agent\" in ~/.config/worklens.json"),
        ),
    }
}

fn keys_check(cfg: &Config) -> Option<Check> {
    if cfg.keys.is_empty() && cfg.bad_keys.is_empty() {
        return None;
    }
    let mut clash: Vec<String> = cfg
        .keys
        .iter()
        .filter(|k| BUILT_IN.contains(k.key))
        .map(|k| k.key.to_string())
        .collect();
    clash.extend(cfg.bad_keys.iter().cloned());
    Some(if clash.is_empty() {
        let keys: Vec<String> = cfg.keys.iter().map(|k| format!("{} {}", k.key, k.label)).collect();
        check("keys", false, true, keys.join(", "), None)
    } else {
        let fix = format!("use a single key worklens doesn't already use (not {BUILT_IN})");
        check("keys", false, false, format!("{} can't be used", clash.join(", ")), Some(&fix))
    })
}
