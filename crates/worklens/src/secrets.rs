use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow, bail};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Service {
    Bugzilla,
    Jira,
    Phabricator,
}

impl Service {
    pub const ALL: [Service; 3] = [Service::Bugzilla, Service::Jira, Service::Phabricator];

    pub fn id(self) -> &'static str {
        match self {
            Service::Bugzilla => "bugzilla",
            Service::Jira => "jira",
            Service::Phabricator => "phabricator",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Service::Bugzilla => "Bugzilla",
            Service::Jira => "Jira",
            Service::Phabricator => "Phabricator",
        }
    }

    pub fn env_var(self) -> &'static str {
        match self {
            Service::Bugzilla => "WORKLENS_BUGZILLA_KEY",
            Service::Jira => "WORKLENS_JIRA_TOKEN",
            Service::Phabricator => "WORKLENS_PHABRICATOR_TOKEN",
        }
    }

    pub fn where_to_get(self) -> &'static str {
        match self {
            Service::Bugzilla => "https://bugzilla.mozilla.org/userprefs.cgi?tab=apikey",
            Service::Jira => "https://id.atlassian.com/manage-profile/security/api-tokens",
            Service::Phabricator => "https://phabricator.services.mozilla.com/settings/ (Conduit API Tokens)",
        }
    }

    pub fn parse(name: &str) -> Option<Service> {
        Service::ALL.into_iter().find(|s| s.id().eq_ignore_ascii_case(name))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Env(&'static str),
    Keychain,
    File,
    Borrowed(String),
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Origin::Env(var) => write!(f, "${var}"),
            Origin::Keychain => f.write_str(keychain_name()),
            Origin::File => f.write_str("~/.config/worklens/credentials.json"),
            Origin::Borrowed(what) => write!(f, "{what} (borrowed)"),
        }
    }
}

pub struct Secret {
    pub value: String,
    pub origin: Origin,
}

pub fn keychain_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS Keychain"
    } else if cfg!(windows) {
        "Windows Credential Manager"
    } else {
        "the Secret Service keyring"
    }
}

const KEYCHAIN_SERVICE: &str = "worklens";

fn keychain_enabled() -> bool {
    std::env::var("WORKLENS_KEYCHAIN").map_or(true, |v| v != "off" && v != "0")
}

fn keychain_entry(service: Service) -> Option<keyring::Entry> {
    if !keychain_enabled() {
        return None;
    }
    keyring::Entry::new(KEYCHAIN_SERVICE, service.id()).ok()
}

fn from_keychain(service: Service) -> Option<String> {
    keychain_entry(service)?.get_password().ok().filter(|v| !v.is_empty())
}

pub fn credentials_file() -> Option<PathBuf> {
    std::env::home_dir().map(|h| h.join(".config/worklens/credentials.json"))
}

fn read_file(path: &Path) -> anyhow::Result<BTreeMap<String, String>> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e).with_context(|| format!("can't read {}", path.display())),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(path)?.permissions().mode();
        if mode & 0o077 != 0 {
            bail!(
                "{} can be read by other users (mode {:o}); run `chmod 600 {}`",
                path.display(),
                mode & 0o777,
                path.display()
            );
        }
    }
    serde_json::from_slice(&bytes).with_context(|| format!("{} doesn't parse", path.display()))
}

fn write_file(path: &Path, keys: &BTreeMap<String, String>) -> anyhow::Result<()> {
    worklens_store::write_atomic(path, |out| serde_json::to_writer_pretty(out, keys).map_err(std::io::Error::other))
        .with_context(|| format!("can't write {}", path.display()))
}

pub fn get(service: Service) -> anyhow::Result<Option<Secret>> {
    if let Ok(value) = std::env::var(service.env_var())
        && !value.trim().is_empty()
    {
        return Ok(Some(Secret {
            value: value.trim().to_owned(),
            origin: Origin::Env(service.env_var()),
        }));
    }
    if let Some(value) = from_keychain(service) {
        return Ok(Some(Secret {
            value,
            origin: Origin::Keychain,
        }));
    }
    let Some(path) = credentials_file() else { return Ok(None) };
    Ok(read_file(&path)?
        .remove(service.id())
        .filter(|v| !v.is_empty())
        .map(|value| Secret {
            value,
            origin: Origin::File,
        }))
}

pub fn set(service: Service, value: &str) -> anyhow::Result<Origin> {
    let value = value.trim();
    if value.is_empty() {
        bail!("empty key");
    }
    if let Some(entry) = keychain_entry(service)
        && entry.set_password(value).is_ok()
    {
        remove_from_file(service)?;
        return Ok(Origin::Keychain);
    }
    let path = credentials_file().ok_or_else(|| anyhow!("no home directory"))?;
    let mut keys = read_file(&path)?;
    keys.insert(service.id().to_owned(), value.to_owned());
    write_file(&path, &keys)?;
    Ok(Origin::File)
}

pub fn remove(service: Service) -> anyhow::Result<bool> {
    let in_keychain = keychain_entry(service).is_some_and(|e| e.delete_credential().is_ok());
    let in_file = remove_from_file(service)?;
    Ok(in_keychain || in_file)
}

fn remove_from_file(service: Service) -> anyhow::Result<bool> {
    let Some(path) = credentials_file() else { return Ok(false) };
    let mut keys = match read_file(&path) {
        Ok(k) => k,
        Err(_) => return Ok(false),
    };
    if keys.remove(service.id()).is_none() {
        return Ok(false);
    }
    write_file(&path, &keys)?;
    Ok(true)
}

pub fn read_secret(prompt: &str) -> anyhow::Result<String> {
    use std::io::{BufRead as _, IsTerminal as _, Write as _};
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        let mut line = String::new();
        stdin.lock().read_line(&mut line)?;
        return Ok(line.trim().to_owned());
    }
    eprint!("{prompt}");
    std::io::stderr().flush()?;
    let line = without_echo(|| {
        let mut line = String::new();
        stdin.lock().read_line(&mut line).map(|_| line)
    })?;
    eprintln!();
    Ok(line.trim().to_owned())
}

#[cfg(unix)]
fn without_echo(read: impl FnOnce() -> std::io::Result<String>) -> std::io::Result<String> {
    use rustix::termios::{LocalModes, OptionalActions, tcgetattr, tcsetattr};
    let stdin = std::io::stdin();
    let before = tcgetattr(&stdin)?;
    let mut quiet = before.clone();
    quiet.local_modes.remove(LocalModes::ECHO);
    tcsetattr(&stdin, OptionalActions::Now, &quiet)?;
    let result = read();
    tcsetattr(&stdin, OptionalActions::Now, &before)?;
    result
}

#[cfg(not(unix))]
fn without_echo(read: impl FnOnce() -> std::io::Result<String>) -> std::io::Result<String> {
    read()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_round_trip_through_their_names() {
        for s in Service::ALL {
            assert_eq!(Service::parse(s.id()), Some(s));
            assert_eq!(Service::parse(&s.id().to_uppercase()), Some(s));
        }
        assert_eq!(Service::parse("github"), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_credentials_file_others_can_read_is_refused() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("worklens-secrets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("credentials.json");
        worklens_store::write_atomic(&path, |out| std::io::Write::write_all(out, br#"{"jira":"t"}"#)).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = read_file(&path).unwrap_err().to_string();
        assert!(err.contains("chmod 600"), "{err}");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_file(&path).unwrap()["jira"], "t");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_file_has_no_keys() {
        assert!(read_file(Path::new("/nonexistent/worklens/credentials.json")).unwrap().is_empty());
    }

    #[test]
    fn origins_say_where_without_the_value() {
        assert_eq!(Origin::Env("WORKLENS_JIRA_TOKEN").to_string(), "$WORKLENS_JIRA_TOKEN");
        assert_eq!(Origin::Borrowed("~/.arcrc".into()).to_string(), "~/.arcrc (borrowed)");
    }
}
