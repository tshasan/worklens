use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Home(PathBuf);

impl Home {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("worklens-cli-{name}-{}", std::process::id()));
        let cache = dir.join(".cache/worklens");
        fs::create_dir_all(&cache).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../store/fixtures/raw.json");
        fs::copy(fixture, cache.join("state.json")).unwrap();
        Self(dir)
    }

    fn run(&self, args: &[&str]) -> (bool, String, String) {
        let mut all = args.to_vec();
        all.push("--cached");
        self.run_exactly(&all)
    }

    fn run_exactly(&self, args: &[&str]) -> (bool, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_worklens"))
            .args(args)
            .env("HOME", &self.0)
            .env("XDG_STATE_HOME", self.0.join("state"))
            .env("NO_COLOR", "1")
            .env("WORKLENS_KEYCHAIN", "off")
            .env_remove("BUGZILLA_API_KEY")
            .env_remove("WORKLENS_BUGZILLA_KEY")
            .env_remove("WORKLENS_JIRA_TOKEN")
            .env_remove("WORKLENS_PHABRICATOR_TOKEN")
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    }

    fn run_with_stdin(&self, args: &[&str], input: &str) -> (bool, String, String) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_worklens"))
            .args(args)
            .env("HOME", &self.0)
            .env("XDG_STATE_HOME", self.0.join("state"))
            .env("NO_COLOR", "1")
            .env("WORKLENS_KEYCHAIN", "off")
            .env_remove("WORKLENS_JIRA_TOKEN")
            .env_remove("WORKLENS_BUGZILLA_KEY")
            .env_remove("WORKLENS_PHABRICATOR_TOKEN")
            .env_remove("BUGZILLA_API_KEY")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        std::io::Write::write_all(&mut child.stdin.take().unwrap(), input.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        (
            out.status.success(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    }

    fn state_file_exists(&self) -> bool {
        self.0.join("state/worklens/marks.json").exists()
    }

    fn mcp(&self, input: &str) -> String {
        worklens_store::write_atomic(&self.0.join(".config/worklens.json"), |out| {
            serde_json::to_writer(out, &serde_json::json!({"cacheMinutes":1e12})).map_err(std::io::Error::other)
        })
        .unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_worklens"))
            .arg("mcp")
            .env("HOME", &self.0)
            .env("XDG_STATE_HOME", self.0.join("state"))
            .env("WORKLENS_KEYCHAIN", "off")
            .env_remove("BUGZILLA_API_KEY")
            .env_remove("WORKLENS_BUGZILLA_KEY")
            .env_remove("WORKLENS_JIRA_TOKEN")
            .env_remove("WORKLENS_PHABRICATOR_TOKEN")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        std::io::Write::write_all(&mut child.stdin.take().unwrap(), input.as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn next_lists_the_turn_sections_with_reasons() {
    let home = Home::new("next");
    let (ok, out, err) = home.run(&["next"]);
    assert!(ok, "{err}");
    assert!(out.contains("Waiting on you 1"), "{out}");
    assert!(out.contains("needinfo from a@mozilla.com"), "{out}");
    assert!(
        out.contains("Waiting on others 0"),
        "the fixture's revision is on a bug waiting on me: {out}"
    );
}

#[test]
fn json_has_the_schema_and_keeps_field_order() {
    let home = Home::new("json");
    let (ok, out, err) = home.run(&["sections", "--json"]);
    assert!(ok, "{err}");
    assert!(out.starts_with("{\n  \"schema\": \"worklens/2\""), "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["sections"][0]["id"], "waiting");
    assert_eq!(v["sections"][0]["items"][0]["action"], "Answer the needinfo");
}

#[test]
fn compact_is_one_line() {
    let home = Home::new("compact");
    let (ok, out, _) = home.run(&["waiting", "--compact"]);
    assert!(ok);
    assert_eq!(out.trim_end().lines().count(), 1);
}

#[test]
fn workflow_evidence_is_additive_and_old_telemetry_is_unknown() {
    let home = Home::new("evidence");
    let (ok, out, err) = home.run(&["show", "1995008", "--json"]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(
        v["workflow"]["approvals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["state"] == "unknown")
    );
    let (ok, out, err) = home.run(&["telemetry", "--json"]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["definitions"].is_null());
    assert!(v["evidence"].as_array().unwrap().is_empty());
}

#[test]
fn brief_and_show_find_items_by_jira_key() {
    let home = Home::new("brief");
    let (ok, out, err) = home.run(&["brief", "aiplat-1416"]);
    assert!(ok, "{err}");
    assert!(out.starts_with("# Bug 1995008: Support view-source:"), "{out}");
    assert!(out.contains("## Description"), "{out}");
    let (ok, _, err) = home.run(&["show", "D999"]);
    assert!(!ok);
    assert!(err.contains("D999 isn't in your work"), "{err}");
}

#[test]
fn done_hides_an_item_and_restore_brings_it_back() {
    let home = Home::new("done");
    assert!(home.run(&["done", "1995008"]).0);
    assert!(home.state_file_exists());
    assert!(home.run(&["waiting"]).1.contains("Waiting on you 0 (+1 done or snoozed)"));
    assert!(home.run(&["restore", "1995008"]).0);
    assert!(home.run(&["waiting"]).1.contains("Waiting on you 1"));
    let (ok, _, err) = home.run(&["snooze", "1995008", "someday"]);
    assert!(!ok);
    assert!(err.contains("Can't read \"someday\""), "{err}");
}

#[test]
fn a_missing_cache_says_what_to_do() {
    let home = Home::new("missing");
    fs::remove_file(home.0.join(".cache/worklens/state.json")).unwrap();
    let (ok, _, err) = home.run(&["next"]);
    assert!(!ok);
    assert!(err.contains("run `worklens refresh`"), "{err}");
}

#[test]
fn fetching_without_any_token_or_email_says_how_to_set_one_up() {
    let home = Home::new("no-tokens");
    let (ok, _, err) = home.run_exactly(&["refresh"]);
    assert!(!ok);
    assert!(err.contains("Can't tell who you are"), "{err}");
}

#[test]
fn mcp_answers_tool_calls_from_the_cache() {
    let home = Home::new("mcp");
    let input = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"worklens_sections","arguments":{"section":"waiting"}}}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"worklens_brief","arguments":{"ref":"AIPLAT-1416"}}}"#,
        r#"{"jsonrpc":"2.0","id":4,"method":"nope"}"#,
    ]
    .join("\n");
    let out = home.mcp(&input);
    let replies: Vec<serde_json::Value> = out.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(replies.len(), 4, "the notification gets no reply: {out}");
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "worklens");
    let sections: serde_json::Value = serde_json::from_str(replies[1]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(sections["sections"][0]["items"][0]["action"], "Answer the needinfo");
    assert!(
        replies[2]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("# Bug 1995008")
    );
    assert_eq!(replies[3]["error"]["code"], -32601);
}

#[test]
fn recorded_stack_survives_cache_cli_compact_brief_and_mcp_with_another_source_failing() {
    use worklens_core::raw::{Landings, RepositoryLandings, SourceError};
    use worklens_core::{Raw, RevId};

    let home = Home::new("recorded-stack");
    let now = "2026-11-04T00:00:00Z".parse().unwrap();
    let mut scan = worklens_sources::landing::parse_scan(
        include_bytes!("../../sources/fixtures/treeherder/recorded-autoland-push.json"),
        "autoland",
        &[RevId(338940)],
        now,
    )
    .unwrap();
    assert_eq!(scan.matches.len(), 1);
    let found = &mut scan.matches[0];
    assert_ne!(found.commit, found.push_revision, "a stack's patch is not necessarily its push tip");
    let expected_commit = found.commit.clone();
    let expected_url = found.url.clone();
    found.health = worklens_sources::landing::parse_push_health(
        include_bytes!("../../sources/fixtures/treeherder/recorded-autoland-health.json"),
        "autoland",
        &found.push_revision,
    )
    .unwrap();
    found.health_error = None;
    assert_eq!(found.health.unwrap().running, 31);

    let cache = home.0.join(".cache/worklens/state.json");
    let mut raw: Raw = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
    raw.revisions[0].id = RevId(338940);
    raw.landings = Some(Landings {
        revisions: vec![RevId(338940)],
        repositories: vec![
            scan,
            RepositoryLandings {
                repository: "mozilla-central".into(),
                checked_at: now,
                push_limit: 100,
                scanned_pushes: 0,
                oldest_push: None,
                error: Some("503 service unavailable".into()),
                matches: Vec::new(),
            },
        ],
    });
    raw.errors.push(SourceError {
        source: "treeherder-landings".into(),
        message: "mozilla-central: 503 service unavailable".into(),
        unset: false,
    });
    worklens_store::write_raw(&cache, &raw).unwrap();

    let (ok, out, err) = home.run(&["show", "D338940", "--json"]);
    assert!(ok, "{err}");
    let full: serde_json::Value = serde_json::from_str(&out).unwrap();
    let observed = &full["workflow"]["landings"][0];
    assert_eq!(observed["state"], "observed_landing_running");
    assert_eq!(observed["url"], expected_url);
    assert!(observed["summary"].as_str().unwrap().contains(&expected_commit));
    assert_eq!(full["workflow"]["landings"][1]["state"], "lookup_failed");
    let (ok, out, err) = home.run(&["show", "D338940", "--compact"]);
    assert!(ok, "{err}");
    let compact: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(compact["workflow"]["landings"], full["workflow"]["landings"]);
    let (ok, brief, err) = home.run(&["brief", "D338940"]);
    assert!(ok, "{err}");
    assert!(brief.contains(&expected_commit) && brief.contains(&expected_url));
    assert!(brief.contains("landing unknown") && brief.contains("31 pending/running jobs"));

    let reply: serde_json::Value = serde_json::from_str(
        &home.mcp(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"worklens_show","arguments":{"ref":"D338940"}}}"#),
    )
    .unwrap();
    assert_ne!(reply["result"]["isError"], true);
    let mcp: serde_json::Value = serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(mcp["workflow"]["landings"], full["workflow"]["landings"]);
}

#[test]
fn a_piped_key_is_kept_owner_only_and_shown_only_by_where_it_is() {
    let home = Home::new("auth");
    let (ok, _, err) = home.run_with_stdin(&["auth", "set", "jira"], "tok-123\n");
    assert!(ok, "{err}");
    let file = home.0.join(".config/worklens/credentials.json");
    assert!(fs::read_to_string(&file).unwrap().contains("tok-123"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let (ok, status, _) = home.run_exactly(&["auth", "status"]);
    assert!(ok);
    assert!(status.contains("credentials.json"), "{status}");
    assert!(!status.contains("tok-123"), "the key itself is never shown: {status}");
    let (ok, _, _) = home.run_exactly(&["auth", "remove", "jira"]);
    assert!(ok);
    assert!(!fs::read_to_string(&file).unwrap().contains("tok-123"));
}

#[test]
fn every_json_result_says_whether_its_data_is_complete() {
    let home = Home::new("status");
    for args in [
        &["next", "--json"][..],
        &["sprint", "--json"],
        &["drift", "--json"],
        &["sections", "--compact"],
    ] {
        let (ok, out, err) = home.run(args);
        assert!(ok, "{args:?}: {err}");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        for key in ["partial", "errors", "stale", "ageMinutes", "fetchedAt"] {
            assert!(v.get(key).is_some(), "{args:?} lacks {key}");
        }
    }
}

#[test]
fn health_reads_only_the_cache() {
    let home = Home::new("health");
    let (ok, out, err) = home.run_exactly(&["health", "--json"]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let names: Vec<&str> = v["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(&names[..3], ["Bugzilla", "Phabricator", "Jira"]);
}

#[test]
fn drift_json_has_the_daily_history() {
    let home = Home::new("drift-history");
    let (_, out, _) = home.run(&["drift", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["history"], serde_json::json!([]), "no log yet: an empty history");

    let state = home.0.join("state/worklens");
    fs::create_dir_all(&state).unwrap();
    let log = r#"{"version":1,"days":[{"date":"2026-11-01","mine":6,"team":9},{"date":"2026-11-03","mine":4,"team":7}]}"#;
    worklens_store::write_atomic(&state.join("drift.json"), |w| std::io::Write::write_all(w, log.as_bytes())).unwrap();
    let (ok, out, err) = home.run(&["drift", "--json"]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["history"],
        serde_json::json!([{ "date": "2026-11-01", "count": 6 }, { "date": "2026-11-03", "count": 4 }])
    );
    let (_, out, _) = home.run(&["drift", "--team", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["history"][1]["count"], 7, "team scope counts team drift");
}

#[test]
fn an_accepted_revision_names_its_try_push_once() {
    let home = Home::new("try-once");
    let busy = Path::new(env!("CARGO_MANIFEST_DIR")).join("../store/fixtures/busy.json");
    fs::copy(busy, home.0.join(".cache/worklens/state.json")).unwrap();
    let (ok, out, err) = home.run(&["sections"]);
    assert!(ok, "{err}");
    let accepted: Vec<&str> = out.lines().filter(|l| l.contains(" accepted by ")).collect();
    assert!(accepted.iter().any(|l| l.contains("try")), "{out}");
    for line in accepted {
        assert_eq!(line.matches("try").count(), 1, "{line}");
    }
}

#[test]
fn blocker_guidance_and_dependencies_survive_cli_compact_brief_and_mcp() {
    use serde_json::{Value, json};
    let home = Home::new("dependency-guidance");
    let path = home.0.join(".cache/worklens/state.json");
    let mut raw: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    raw["bugs"][0]["flags"] = json!([]);
    raw["revisions"][0]["status"] = json!("accepted");
    raw["revisions"][0]["reviewers"] = json!([]);
    let mut parent = raw["revisions"][0].clone();
    parent["id"] = json!(123);
    parent["status"] = json!("needs-revision");
    parent["author"] = json!("alice");
    raw["dependencies"] = json!({"parents":{"321987":["PHID-parent"]},"revisions":{"PHID-parent":parent}});
    worklens_store::write_atomic(&path, |w| serde_json::to_writer(w, &raw).map_err(std::io::Error::other)).unwrap();
    let (ok, out, err) = home.run(&["show", "1995008", "--json"]);
    assert!(ok, "{err}");
    let full: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(full["action"], "Inspect dependency D123 in Phabricator");
    assert_eq!(full["actionUrl"], "https://phabricator.services.mozilla.com/D123");
    assert_eq!(full["dependencies"][0]["owner"], "alice");
    let (ok, out, err) = home.run(&["show", "1995008", "--compact"]);
    assert!(ok, "{err}");
    let compact: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(compact["dependencies"], full["dependencies"]);
    let (ok, out, err) = home.run(&["brief", "1995008"]);
    assert!(ok, "{err}");
    assert!(out.contains("D123:") && out.contains("needs-revision"), "{out}");
    let out = home.mcp("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"worklens_show\",\"arguments\":{\"ref\":\"1995008\"}}}\n");
    let response: Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    let result: Value = serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(result["dependencies"], full["dependencies"]);
    assert_eq!(result["actionUrl"], full["actionUrl"]);
}
