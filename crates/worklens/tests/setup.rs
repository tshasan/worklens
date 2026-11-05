#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use expectrl::process::unix::WaitStatus;
use expectrl::session::OsSession as Session;
use expectrl::{Eof, Expect as _, Regex};

const EMAIL_QUESTION: &str = "Your Bugzilla email (whose bugs to show)";
const TUI_STARTS: &str = "\x1b[?1049h";

struct Home(PathBuf);

impl Home {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("worklens-setup-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("replay")).unwrap();
        worklens_store::write_atomic(&dir.join("replay/routes.json"), |out| std::io::Write::write_all(out, b"[]")).unwrap();
        Self(dir)
    }

    fn write(&self, name: &str, text: &str) {
        worklens_store::write_atomic(&self.0.join(name), |out| std::io::Write::write_all(out, text.as_bytes())).unwrap();
    }

    fn fake_hg(&self, username: &str) {
        self.write(
            "bin/hg",
            &format!("#!/bin/sh\n[ \"$1 $2\" = \"config ui.username\" ] && echo '{username}' && exit 0\nexit 1\n"),
        );
        fs::set_permissions(self.0.join("bin/hg"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    }

    fn config_path(&self) -> PathBuf {
        self.0.join(".config/worklens.json")
    }

    fn saved_email(&self) -> serde_json::Value {
        let config: serde_json::Value = serde_json::from_slice(&fs::read(self.config_path()).unwrap()).unwrap();
        config["email"].clone()
    }

    fn spawn(&self, args: &[&str]) -> Session {
        self.spawn_with(args, &[])
    }

    fn spawn_with(&self, args: &[&str], env: &[(&str, &str)]) -> Session {
        let mut command = Command::new(env!("CARGO_BIN_EXE_worklens"));
        command
            .args(args)
            .current_dir(&self.0)
            .env("HOME", &self.0)
            .env("XDG_STATE_HOME", self.0.join("state"))
            .env("NO_COLOR", "1")
            .env("WORKLENS_KEYCHAIN", "off")
            .env("WORKLENS_REPLAY", self.0.join("replay"))
            .env(
                "PATH",
                format!("{}:{}", self.0.join("bin").display(), std::env::var("PATH").unwrap_or_default()),
            )
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HGRCPATH", self.0.join(".hgrc"))
            .env_remove("GIT_CONFIG_GLOBAL")
            .env_remove("GIT_DIR")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("HGUSER")
            .env_remove("BUGZILLA_API_KEY")
            .env_remove("WORKLENS_BUGZILLA_KEY")
            .env_remove("WORKLENS_JIRA_TOKEN")
            .env_remove("WORKLENS_PHABRICATOR_TOKEN")
            .envs(env.iter().copied());
        let mut session = Session::spawn(command).unwrap();
        session.set_expect_timeout(Some(Duration::from_secs(8)));
        session
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn skip_every_key(p: &mut Session) {
    p.expect("API key: ").unwrap();
    p.send_line("").unwrap();
    p.expect("Conduit token: ").unwrap();
    p.send_line("").unwrap();
    p.expect("Set up Jira? [Y/n]: ").unwrap();
    p.send_line("n").unwrap();
}

fn email_prompt(p: &mut Session) -> String {
    let found = p.expect(Regex(r"Your Bugzilla email[^:]*: ")).unwrap();
    String::from_utf8_lossy(found.get(0).unwrap()).into_owned()
}

fn finish(p: &mut Session) -> (i32, String) {
    let rest = p.expect(Eof).unwrap();
    let text = String::from_utf8_lossy(rest.as_bytes()).into_owned();
    let code = match p.get_process().wait().unwrap() {
        WaitStatus::Exited(_, code) => code,
        other => panic!("worklens didn't exit: {other:?}\n{text}"),
    };
    (code, text)
}

fn skip_the_email_twice(p: &mut Session) -> (i32, String) {
    skip_every_key(p);
    assert_eq!(email_prompt(p), format!("{EMAIL_QUESTION}: "));
    p.send_line("").unwrap();
    p.expect("it finds your bugs by your Bugzilla email").unwrap();
    assert_eq!(email_prompt(p), format!("{EMAIL_QUESTION}: "));
    p.send_line("").unwrap();
    finish(p)
}

#[test]
fn a_first_run_that_skips_everything_twice_exits_without_a_tui_or_a_config() {
    let home = Home::new("skip-all");
    let mut p = home.spawn(&[]);
    p.expect("Each key is checked, then kept in ~/.config/worklens/credentials.json.")
        .unwrap();
    let (code, rest) = skip_the_email_twice(&mut p);
    assert_ne!(code, 0, "{rest}");
    assert!(rest.contains("run `worklens setup` again"), "{rest}");
    assert!(!rest.contains(TUI_STARTS), "the TUI must not start:\n{rest}");
    assert!(!rest.contains("Whiteboard"), "setup stops at the missing email:\n{rest}");
    assert!(!home.config_path().exists(), "nothing is written");
}

#[test]
fn a_failed_setup_leaves_an_existing_config_as_it_was() {
    let home = Home::new("skip-all-kept");
    let before = "{\"whiteboard\": \"[aiplatform]\", \"cacheMinutes\": 5}\n";
    home.write(".config/worklens.json", before);
    let mut p = home.spawn(&[]);
    let (code, rest) = skip_the_email_twice(&mut p);
    assert_ne!(code, 0, "{rest}");
    assert!(!rest.contains(TUI_STARTS), "the TUI must not start:\n{rest}");
    assert_eq!(fs::read_to_string(home.config_path()).unwrap(), before);
}

#[test]
fn enter_accepts_the_git_email_as_the_bugzilla_email() {
    let home = Home::new("accept");
    home.write(".gitconfig", "[user]\n\tname = Ada\n\temail = ada@example.com\n");
    let mut p = home.spawn(&["setup"]);
    skip_every_key(&mut p);
    assert_eq!(email_prompt(&mut p), format!("{EMAIL_QUESTION} [ada@example.com]: "));
    p.send_line("").unwrap();
    p.expect("Whiteboard").unwrap();
    p.send_line("").unwrap();
    let (code, rest) = finish(&mut p);
    assert_eq!(code, 0, "{rest}");
    assert_eq!(home.saved_email(), "ada@example.com");
}

#[test]
fn a_typed_email_replaces_the_suggestion() {
    let home = Home::new("replace");
    home.write(".gitconfig", "[user]\n\temail = ada@example.com\n");
    let mut p = home.spawn(&["setup"]);
    skip_every_key(&mut p);
    assert_eq!(email_prompt(&mut p), format!("{EMAIL_QUESTION} [ada@example.com]: "));
    p.send_line("grace@example.com").unwrap();
    p.expect("Whiteboard").unwrap();
    p.send_line("").unwrap();
    let (code, rest) = finish(&mut p);
    assert_eq!(code, 0, "{rest}");
    assert_eq!(home.saved_email(), "grace@example.com");
    assert!(!fs::read_to_string(home.config_path()).unwrap().contains("ada@example.com"));
}

#[test]
fn without_a_git_or_hg_email_the_question_has_no_default() {
    let home = Home::new("no-default");
    let mut p = home.spawn(&["setup"]);
    skip_every_key(&mut p);
    assert_eq!(email_prompt(&mut p), format!("{EMAIL_QUESTION}: "));
    p.send_line("grace@example.com").unwrap();
    p.expect("Whiteboard").unwrap();
    p.send_line("").unwrap();
    let (code, rest) = finish(&mut p);
    assert_eq!(code, 0, "{rest}");
    assert_eq!(home.saved_email(), "grace@example.com");
}

#[test]
fn without_git_the_hg_username_address_is_suggested() {
    let home = Home::new("hg");
    home.fake_hg("Ada Lovelace <ada@example.com>");
    let mut p = home.spawn(&["setup"]);
    skip_every_key(&mut p);
    assert_eq!(email_prompt(&mut p), format!("{EMAIL_QUESTION} [ada@example.com]: "));
    p.send_line("").unwrap();
    p.expect("Whiteboard").unwrap();
    p.send_line("").unwrap();
    let (code, rest) = finish(&mut p);
    assert_eq!(code, 0, "{rest}");
    assert_eq!(home.saved_email(), "ada@example.com");
}

#[test]
fn git_wins_over_hg() {
    let home = Home::new("git-first");
    home.write(".gitconfig", "[user]\n\temail = ada@example.com\n");
    home.fake_hg("Grace <grace@example.com>");
    let mut p = home.spawn(&["setup"]);
    skip_every_key(&mut p);
    assert_eq!(email_prompt(&mut p), format!("{EMAIL_QUESTION} [ada@example.com]: "));
}

#[test]
fn a_user_with_a_key_already_is_asked_the_email_only_once() {
    let home = Home::new("keyed");
    let mut p = home.spawn_with(&["setup"], &[("WORKLENS_BUGZILLA_KEY", "a-key-from-the-environment")]);
    skip_every_key(&mut p);
    assert_eq!(email_prompt(&mut p), format!("{EMAIL_QUESTION}: "));
    p.send_line("").unwrap();
    p.expect("Whiteboard").unwrap();
    p.send_line("").unwrap();
    let (code, rest) = finish(&mut p);
    assert_eq!(code, 0, "{rest}");
    assert!(!rest.contains("it finds your bugs"), "{rest}");
    assert!(home.config_path().exists());
}
