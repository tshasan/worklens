use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Home(PathBuf);

impl Home {
    fn new(name: &str) -> Self {
        Self::with_cache(name, "raw.json")
    }

    fn with_cache(name: &str, fixture: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("worklens-cli-{name}-{}", std::process::id()));
        let cache = dir.join(".cache/worklens");
        fs::create_dir_all(&cache).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../store/fixtures").join(fixture);
        fs::copy(fixture, cache.join("state.json")).unwrap();
        Self(dir)
    }

    fn run(&self, args: &[&str]) -> (bool, String, String) {
        let mut all = args.to_vec();
        all.push("--cached");
        self.run_exactly(&all)
    }

    fn run_exactly(&self, args: &[&str]) -> (bool, String, String) {
        self.run_with(args, &[])
    }

    fn run_with(&self, args: &[&str], env: &[(&str, &Path)]) -> (bool, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_worklens"))
            .args(args)
            .envs(env.iter().copied())
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
                retry_at: None,
                repository: "mozilla-central".into(),
                checked_at: now,
                push_limit: 100,
                scanned_pushes: 0,
                oldest_push: None,
                error: Some("503 service unavailable".into()),
                matches: Vec::new(),
                backouts: Vec::new(),
            },
        ],
    });
    raw.errors.push(SourceError {
        source: "treeherder-landings".into(),
        message: "mozilla-central: 503 service unavailable".into(),
        unset: false,
        retry_at: None,
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

fn says_off_never_failed(args: &[&str], text: &str) {
    let problems = worklens_testkit::screen_problems(text, &["jira", "phabricator"], false);
    assert!(problems.is_empty(), "`worklens {}`: {problems:#?}\n{text}", args.join(" "));
}

#[test]
fn a_bugzilla_only_user_hears_what_is_off_and_never_that_it_failed() {
    let home = Home::with_cache("bugzilla-only", "bugzilla_only.json");
    let commands: [&[&str]; 6] = [
        &["next"],
        &["sections"],
        &["sprint"],
        &["drift"],
        &["drift", "--team"],
        &["show", "1995008"],
    ];
    for args in commands {
        let (ok, out, err) = home.run(args);
        assert!(ok, "`worklens {}` failed: {err}", args.join(" "));
        says_off_never_failed(args, &format!("{out}{err}"));
    }
    for (reference, source) in [("D123", "Phabricator"), ("AIPLAT-123", "Jira")] {
        let (ok, _, err) = home.run(&["show", reference]);
        assert!(!ok);
        assert!(err.contains(&format!("{source} isn't set up")), "{reference}: {err}");
    }
    let (_, out, _) = home.run(&["next", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        json["sourceStates"],
        serde_json::json!({"bugzilla": "ok", "jira": "off", "phabricator": "off"})
    );
    let (_, out, _) = Home::new("all-sources").run(&["next", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["sourceStates"]["phabricator"], "ok");
}

#[test]
fn next_text_keeps_to_the_limit_and_skips_sections_that_are_off() {
    let busy = Home::with_cache("busy-limit", "busy.json");
    let (_, all, _) = busy.run(&["next", "--all"]);
    let (_, two, _) = busy.run(&["next", "--limit", "2"]);
    let rows = |text: &str| {
        text.lines()
            .filter(|l| l.trim_start().split_once("d  ").is_some_and(|(age, _)| age.parse::<u32>().is_ok()))
            .count()
    };
    assert!(rows(&all) > 2, "{all}");
    assert_eq!(rows(&two), 2, "{two}");
    assert!(two.contains("more; `worklens next --all` lists them"), "{two}");
    assert!(!all.contains("more; `worklens next --all`"), "--all leaves nothing out:\n{all}");
    let (_, out, _) = Home::with_cache("off-sections", "bugzilla_only.json").run(&["next"]);
    for title in ["Ready to land", "Returned to you", "Waiting on others"] {
        assert!(!out.contains(title), "{title} is off and the header says so:\n{out}");
    }
    assert!(out.contains("Waiting on you"), "{out}");
    let jira_lines = out.lines().filter(|l| l.to_lowercase().contains("jira")).count();
    assert_eq!(jira_lines, 1, "that Jira is off is said once:\n{out}");
}

#[test]
fn show_text_has_no_machine_stamps_and_names_each_link_once() {
    let home = Home::new("show-text");
    let (ok, out, err) = home.run(&["show", "1995008"]);
    assert!(ok, "{err}");
    assert!(!out.contains("; observed 20"), "{out}");
    let link = "https://bugzilla.mozilla.org/show_bug.cgi?id=1995008";
    assert_eq!(out.matches(link).count(), 1, "{out}");
    assert!(
        out.contains("D321987 review: quillfox: blocking"),
        "known evidence is shown:\n{out}"
    );
    let folded = out
        .lines()
        .find(|l| l.contains("Nothing seen yet for"))
        .unwrap_or_else(|| panic!("{out}"));
    assert!(folded.contains("Nightly 161") && folded.contains("ESR 115"), "{folded}");
    assert!(
        !out.lines().any(|l| l.trim_start().starts_with("Nightly 161:")),
        "unknowns fold into one line:\n{out}"
    );

    let (ok, landing, err) = Home::with_cache("show-landing", "busy.json").run(&["show", "2000469"]);
    assert!(ok, "{err}");
    assert!(
        landing.contains("esr140 uplift: No approval flag observed"),
        "an item up for a landing decision shows every row, unknowns too:\n{landing}"
    );
    assert!(!landing.contains("Nothing seen yet"), "{landing}");
}

#[test]
fn a_first_run_with_no_keys_fetches_only_bugzilla_and_says_what_is_off() {
    let home = Home::new("first-run");
    fs::remove_file(home.0.join(".cache/worklens/state.json")).unwrap();
    worklens_store::write_atomic(&home.0.join(".config/worklens.json"), |out| {
        out.write_all(br#"{"email": "me@mozilla.com"}"#)
    })
    .unwrap();
    let replay = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replay/first_run");

    let (ok, out, err) = home.run_with(&["refresh"], &[("WORKLENS_REPLAY", &replay)]);
    assert!(ok, "{out}{err}");
    let raw: serde_json::Value = serde_json::from_slice(&fs::read(home.0.join(".cache/worklens/state.json")).unwrap()).unwrap();
    let asked: Vec<&str> = raw["calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["url"].as_str().unwrap())
        .collect();
    for call in raw["calls"].as_array().unwrap() {
        assert_eq!(call["status"], 200, "every request the real code made was answered: {call}");
    }
    for host in ["atlassian", "phabricator"] {
        assert!(!asked.iter().any(|u| u.contains(host)), "no key, so no {host} request: {asked:#?}");
    }
    assert!(
        asked.iter().any(|u| u.contains("v1=me%40mozilla.com")),
        "bugs are looked up by the email: {asked:#?}"
    );

    let hand_made: serde_json::Value =
        serde_json::from_slice(&fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../store/fixtures/bugzilla_only.json")).unwrap())
            .unwrap();
    assert_eq!(
        raw["errors"], hand_made["errors"],
        "bugzilla_only.json must match what a real first run writes"
    );
    assert_eq!(raw["me"]["phid"], hand_made["me"]["phid"]);

    let (ok, text, err) = home.run(&["next"]);
    assert!(ok, "{err}");
    says_off_never_failed(&["next"], &text);
    let waiting = text.split("Waiting on you").nth(1).unwrap_or_default();
    assert!(
        waiting.contains("2086888"),
        "the needinfo the fetch found is waiting on you:\n{text}"
    );
    let (_, json, _) = home.run(&["next", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(json["partial"], false, "{json}");
    assert_eq!(
        json["sourceStates"],
        serde_json::json!({"bugzilla": "ok", "jira": "off", "phabricator": "off"})
    );
}

#[test]
fn next_prints_in_the_order_its_help_describes() {
    let busy = Home::with_cache("busy-order", "busy.json");
    let (_, help, _) = busy.run_exactly(&["next", "--help"]);
    assert!(help.contains("critical first"), "{help}");
    assert!(help.contains("backed-out landings first"), "{help}");
    assert!(!help.contains("What needs you, oldest first"), "{help}");
    let (_, out, _) = busy.run(&["next", "--all", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    let rank = |tier: &serde_json::Value| ["critical", "important", "other"].iter().position(|t| tier == t).unwrap();
    assert!(help.contains("stack stays together, bottom first"), "{help}");
    let all = json["items"].as_array().unwrap();
    let key = |x: &serde_json::Value| (rank(&x["tier"]), x["since"].as_str().unwrap_or("~").to_owned());
    let mut units: Vec<Vec<&serde_json::Value>> = Vec::new();
    for x in all {
        let bottom = &x["stack"]["bottom"];
        match units.last_mut() {
            Some(unit) if !bottom.is_null() && unit[0]["stack"]["bottom"] == *bottom && unit[0]["section"] == x["section"] => unit.push(x),
            _ => units.push(vec![x]),
        }
    }
    let items: Vec<&serde_json::Value> = units.iter().map(|unit| *unit.iter().min_by_key(|x| key(x)).unwrap()).collect();
    for pair in items.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a["section"] != b["section"] || a["tag"] == "r?#" || b["tag"] == "r?#" || a["tag"] == "bo" {
            continue;
        }
        assert_ne!(b["tag"], "bo", "a backout comes before everything else in its section:\n{a}\n{b}");
        assert!(
            key(a) <= key(b),
            "critical first, then important, then the rest; oldest first within each; a stack sorts as its first member:\n{a}\n{b}"
        );
    }
}

#[test]
fn next_json_says_how_many_items_it_left_out() {
    let busy = Home::with_cache("busy-next-json", "busy.json");
    let count = |args: &[&str]| -> (usize, u64) {
        let (ok, out, err) = busy.run(args);
        assert!(ok, "{err}");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        (
            v["items"].as_array().unwrap().len(),
            v["more"].as_u64().unwrap_or_else(|| panic!("no `more` in {args:?}")),
        )
    };
    let (all, none_left) = count(&["next", "--all", "--json"]);
    assert_eq!(none_left, 0);
    for args in [&["next", "--json"][..], &["next", "--compact"], &["next", "--limit", "3", "--json"]] {
        let (shown, more) = count(args);
        assert!(shown < all, "{args:?}");
        assert_eq!(shown as u64 + more, all as u64, "{args:?}: shown plus more is every item");
    }
}

#[test]
fn show_lists_no_stack_dependency_the_tui_hides() {
    let busy = Home::with_cache("busy-show-stack", "busy.json");
    let (ok, out, err) = busy.run(&["show", "2000469"]);
    assert!(ok, "{err}");
    assert!(out.contains("Land the uplift to Beta 146"), "{out}");
    assert!(
        !out.contains("Depends: D346061 stack"),
        "an item doesn't depend on its own stack:\n{out}"
    );
    let (_, blocked, _) = busy.run(&["show", "1996607"]);
    assert!(
        blocked.contains("Depends: bug 1996929 · ASSIGNED"),
        "real dependencies stay:\n{blocked}"
    );
    assert!(blocked.contains("Depends: D338703 · needs-revision"), "{blocked}");
}

#[test]
fn cache_age_reads_like_the_tui_not_thousands_of_minutes() {
    let busy = Home::with_cache("busy-age", "busy.json");
    let fixture: serde_json::Value = serde_json::from_slice(&fs::read(busy.0.join(".cache/worklens/state.json")).unwrap()).unwrap();
    let fetched: jiff::Timestamp = fixture["fetchedAt"].as_str().unwrap().parse().unwrap();
    let days = ((jiff::Timestamp::now().as_second() - fetched.as_second()) as f64 / 86_400.0).round();
    let expected = format!("pdated {days}d ago");
    for cmd in ["next", "health"] {
        let (ok, out, err) = busy.run(&[cmd]);
        assert!(ok, "{err}");
        let first = out.lines().next().unwrap_or_default();
        assert!(
            first.contains(&expected),
            "`{cmd}` should say {expected:?} like the TUI does: {first}"
        );
    }
}

#[test]
fn sprint_text_aligns_titles_after_mixed_ids() {
    let busy = Home::with_cache("busy-sprint-align", "busy.json");
    let (ok, out, err) = busy.run(&["sprint"]);
    assert!(ok, "{err}");
    let column = |id: &str, title: &str| {
        let line = out.lines().find(|l| l.contains(id)).unwrap_or_else(|| panic!("{id} in\n{out}"));
        line.find(title).unwrap()
    };
    assert_eq!(
        column("2006319", "Handle offline mode"),
        column("AIPLAT-1528", "Investigate PageExtractor"),
        "{out}"
    );
}

#[test]
fn sprint_text_counts_points_like_the_tui() {
    let busy = Home::with_cache("busy-sprint-points", "busy.json");
    let (ok, out, err) = busy.run(&["sprint"]);
    assert!(ok, "{err}");
    let header = out.lines().find(|l| l.contains("AI Platform 2026.22")).unwrap();
    assert!(
        header.contains("52 of 10 pts · over budget"),
        "the TUI says \"52 of 10 pts · over budget\":\n{header}"
    );
    assert!(header.contains(" · 13 pts done"), "13 is points, not tickets:\n{header}");
}

#[test]
fn mcp_next_says_how_many_items_it_left_out() {
    let home = Home::new("mcp-more");
    let input = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"worklens_next","arguments":{"limit":0}}}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"worklens_next","arguments":{}}}"#,
    ]
    .join("\n");
    let out = home.mcp(&input);
    let replies: Vec<serde_json::Value> = out.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let result =
        |i: usize| -> serde_json::Value { serde_json::from_str(replies[i]["result"]["content"][0]["text"].as_str().unwrap()).unwrap() };
    let (cut, whole) = (result(1), result(2));
    let every = whole["items"].as_array().unwrap().len() as u64;
    assert!(every > 0, "{whole}");
    assert_eq!(cut["items"].as_array().unwrap().len(), 0);
    assert_eq!(cut["more"], every, "with no room, every item is counted in `more`");
    assert_eq!(whole["more"], 0);
}

#[test]
fn a_sprint_with_nothing_done_says_nothing_about_done() {
    let (ok, out, err) = Home::new("sprint-none-done").run(&["sprint"]);
    assert!(ok, "{err}");
    let header = out.lines().find(|l| l.contains("of 10 pts")).unwrap_or_else(|| panic!("{out}"));
    assert!(header.contains("3 of 10 pts"), "{header}");
    assert!(
        !header.contains("done"),
        "the TUI leaves out 0 pts done, and so does the CLI:\n{header}"
    );
}

#[test]
fn drift_tells_an_unset_jira_how_to_turn_it_on_and_a_broken_one_where_to_look() {
    let unset = Home::with_cache("drift-unset", "bugzilla_only.json");
    let (ok, out, err) = unset.run(&["drift"]);
    assert!(ok, "{err}");
    assert!(out.contains("`worklens auth set jira` turns it on"), "{out}");
    assert!(
        !out.contains("worklens doctor"),
        "nothing failed, so there's nothing for doctor to explain:\n{out}"
    );

    let broken = Home::with_cache("drift-broken", "bugzilla_only.json");
    let cache = broken.0.join(".cache/worklens/state.json");
    let mut raw: serde_json::Value = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
    raw["errors"][0] = serde_json::json!({"source": "jira", "message": "401 for https://mozilla-hub.atlassian.net/rest/api/3/myself"});
    worklens_store::write_atomic(&cache, |out| serde_json::to_writer(out, &raw).map_err(std::io::Error::other)).unwrap();
    let (ok, out, err) = broken.run(&["drift"]);
    assert!(ok, "{err}");
    assert!(out.contains("Jira couldn't be read. See `worklens doctor`."), "{out}");
}

fn first_run_home(name: &str) -> Home {
    let home = Home::new(name);
    fs::remove_file(home.0.join(".cache/worklens/state.json")).unwrap();
    worklens_store::write_atomic(&home.0.join(".config/worklens.json"), |out| {
        out.write_all(br#"{"email": "me@mozilla.com"}"#)
    })
    .unwrap();
    home
}

fn replay_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replay").join(name)
}

#[test]
fn a_limited_request_waits_as_long_as_asked_then_succeeds() {
    let home = first_run_home("rate-limited");
    let (ok, out, err) = home.run_with(&["refresh"], &[("WORKLENS_REPLAY", &replay_dir("rate_limited"))]);
    assert!(ok, "{out}{err}");
    let raw: serde_json::Value = serde_json::from_slice(&fs::read(home.0.join(".cache/worklens/state.json")).unwrap()).unwrap();
    let calls = raw["calls"].as_array().unwrap();
    let needinfo: Vec<&serde_json::Value> = calls
        .iter()
        .filter(|c| c["url"].as_str().unwrap().contains("requestees.login_name"))
        .collect();
    assert_eq!(needinfo.len(), 2, "one 429 and one retry: {needinfo:#?}");
    assert_eq!(needinfo[0]["status"], 429);
    assert_eq!(needinfo[1]["status"], 200);
    assert_eq!(needinfo[1]["waitMs"], 3000, "the fixture's Retry-After: 3");
    let other: Vec<&serde_json::Value> = calls.iter().filter(|c| c["status"] != 200).collect();
    assert_eq!(other.len(), 1, "only the recorded 429 failed: {other:#?}");
    let failed: Vec<&serde_json::Value> = raw["errors"].as_array().unwrap().iter().filter(|e| e["unset"] != true).collect();
    assert!(failed.is_empty(), "the retry succeeded, so nothing failed: {failed:#?}");
}

#[test]
fn a_limit_longer_than_a_refresh_says_when_to_try_again() {
    let home = first_run_home("rate-limited-long");
    let (ok, out, err) = home.run_with(&["refresh"], &[("WORKLENS_REPLAY", &replay_dir("rate_limited_long"))]);
    assert!(ok, "{out}{err}");
    assert!(out.contains("bugzilla: rate limited, try again after"), "{out}");
    let raw: serde_json::Value = serde_json::from_slice(&fs::read(home.0.join(".cache/worklens/state.json")).unwrap()).unwrap();
    let bugzilla: Vec<&serde_json::Value> = raw["calls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["url"].as_str().unwrap().contains("bugzilla"))
        .collect();
    assert_eq!(bugzilla.len(), 1, "after the 429 no other Bugzilla request is sent: {bugzilla:#?}");
    let (ok, out, err) = home.run_exactly(&["health", "--json"]);
    assert!(ok, "{err}");
    let health: serde_json::Value = serde_json::from_str(&out).unwrap();
    let source = health["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "Bugzilla")
        .unwrap()
        .clone();
    assert_eq!(source["state"], "failed", "{source}");
    assert_eq!(source["rateLimitedUntil"], "2100-01-01T00:00:00Z", "the fixture's Retry-After date");
    assert!(
        health["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["source"] == "bugzilla" && e["retryAt"] == "2100-01-01T00:00:00Z"),
        "{health}"
    );
    let (_, text, _) = home.run_exactly(&["health"]);
    assert!(text.contains("the service is rate-limiting; refresh after"), "{text}");
}

fn with_scanned_pushes(home: &Home, drop_push: Option<&str>) {
    use worklens_core::raw::Landings;
    use worklens_core::{Raw, RevId};
    let mut page: serde_json::Value =
        serde_json::from_slice(include_bytes!("../../sources/fixtures/treeherder/integration-backouts.json")).unwrap();
    if let Some(push) = drop_push {
        page["results"].as_array_mut().unwrap().retain(|p| p["revision"] != push);
    }
    let now = "2026-11-03T20:00:00Z".parse().unwrap();
    let scan = worklens_sources::landing::parse_scan(&serde_json::to_vec(&page).unwrap(), "autoland", &[RevId(321987)], now).unwrap();
    let cache = home.0.join(".cache/worklens/state.json");
    let mut raw: Raw = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
    raw.landings = Some(Landings {
        revisions: vec![RevId(321987)],
        repositories: vec![scan],
    });
    worklens_store::write_raw(&cache, &raw).unwrap();
}

#[test]
fn waiting_json_names_the_backout_from_the_recorded_push_log() {
    let home = Home::new("backout");
    with_scanned_pushes(&home, Some(&"3".repeat(40)));
    let (ok, out, err) = home.run(&["waiting", "--json"]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let item = &v["items"][0];
    assert_eq!(item["id"], "1995008");
    assert_eq!(
        item["reason"], "D321987 backed out of autoland 3h after landing: causing bc failures in browser_viewsource.js",
        "pushed 1793700000, backed out 1793710800, cause from the subject"
    );
    assert_eq!(item["tag"], "bo");
    let push = format!("https://treeherder.mozilla.org/jobs?repo=autoland&revision={}", "2".repeat(40));
    assert_eq!(item["actionUrl"], push);
    assert_eq!(item["backout"]["url"], push);
    assert_eq!(item["backout"]["revision"], 321987);
    assert_eq!(item["backout"]["landedCommit"], "1".repeat(40));
    assert_eq!(item["backout"]["cause"], "causing bc failures in browser_viewsource.js");
    let (_, out, _) = home.run(&["next", "--compact"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["items"][0]["backout"]["url"], push, "agents get the link too");
}

#[test]
fn a_reland_in_the_push_log_clears_the_backout() {
    let home = Home::new("backout-reland");
    with_scanned_pushes(&home, None);
    let (ok, out, err) = home.run(&["waiting", "--json"]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["items"][0]["tag"], "ni?", "{}", v["items"][0]);
    assert!(v["items"][0].get("backout").is_none());
}

fn busy_now(name: &str, edit: impl FnOnce(&mut serde_json::Value, jiff::Timestamp)) -> Home {
    let home = Home::with_cache(name, "busy.json");
    let path = home.0.join(".cache/worklens/state.json");
    let mut raw: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let now = jiff::Timestamp::now();
    raw["fetchedAt"] = serde_json::json!(now);
    edit(&mut raw, now);
    worklens_store::write_atomic(&path, |w| serde_json::to_writer(w, &raw).map_err(std::io::Error::other)).unwrap();
    home
}

fn days_from(now: jiff::Timestamp, days: i64) -> jiff::Timestamp {
    now.checked_add(jiff::SignedDuration::from_hours(24 * days)).unwrap()
}

#[test]
fn landed_lists_each_fix_with_where_it_is_on_the_train() {
    let merge = std::cell::Cell::new(String::new());
    let home = busy_now("landed", |raw, now| {
        let date = |days: i64| days_from(now, days).to_zoned(jiff::tz::TimeZone::UTC).date();
        raw["trains"]["nextMerge"] = serde_json::json!(date(5).to_string());
        raw["trains"]["nextRelease"] = serde_json::json!(date(6).to_string());
        merge.set(date(5).strftime("%b %-d").to_string());
        for bug in raw["bugs"].as_array_mut().unwrap() {
            let age = match bug["id"].as_u64() {
                Some(2010101..=2010104) => 1,
                Some(2010106) => 31,
                _ => continue,
            };
            bug["cf_last_resolved"] = serde_json::json!(days_from(now, -age));
        }
    });
    let (ok, out, err) = home.run(&["landed"]);
    assert!(ok, "{err}");
    assert!(out.contains("Landed "), "{out}");
    for line in [
        "Bugzilla marks Firefox 147 fixed; build inclusion unverified".to_owned(),
        "Bugzilla marks Firefox 145 fixed; build inclusion unverified".to_owned(),
        "Resolved FIXED; no Firefox status flag set; build inclusion unverified".to_owned(),
    ] {
        assert!(out.contains(&line), "{line} in\n{out}");
    }
    assert!(!out.contains("2010106"), "a fix older than 30 days is gone:\n{out}");
    let (ok, out, err) = home.run(&["landed", "--json"]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["section"], "landed");
    let item = v["items"].as_array().unwrap().iter().find(|i| i["id"] == "2010102").unwrap();
    assert_eq!(item["fixedIn"], 146, "the lowest version flagged fixed");
    assert_eq!(item["train"], "beta");
    let autoland = v["items"].as_array().unwrap().iter().find(|i| i["id"] == "2010105").unwrap();
    assert_eq!(autoland["train"], "autoland");
    assert!(autoland.get("fixedIn").is_none(), "{autoland}");
}

fn land_json_with_merge(name: &str, merge_in_days: i64) -> serde_json::Value {
    let home = busy_now(name, |raw, now| {
        let date = |days: i64| days_from(now, days).to_zoned(jiff::tz::TimeZone::UTC).date().to_string();
        let mut details: serde_json::Value = serde_json::from_slice(include_bytes!("../../sources/fixtures/product-details.json")).unwrap();
        details["NEXT_MERGE_DATE"] = serde_json::json!(date(merge_in_days));
        details["NEXT_RELEASE_DATE"] = serde_json::json!(date(merge_in_days + 1));
        let trains = worklens_sources::trains::parse_trains(&serde_json::to_vec(&details).unwrap()).unwrap();
        raw["trains"] = serde_json::to_value(trains).unwrap();
    });
    let (ok, out, err) = home.run(&["land", "--json"]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let item = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["tag"] == "r+")
        .unwrap_or_else(|| panic!("an accepted revision in {v}"))
        .clone();
    let (ok, out, _) = home.run(&["land", "--compact"]);
    assert!(ok);
    let compact: serde_json::Value = serde_json::from_str(&out).unwrap();
    let same = compact["items"].as_array().unwrap().iter().find(|i| i["id"] == item["id"]).unwrap();
    assert_eq!(same["landsIn"], item["landsIn"], "compact and full agree");
    let request =
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"worklens_show","arguments":{"ref":item["id"]}}});
    let reply: serde_json::Value = serde_json::from_str(&home.mcp(&request.to_string())).unwrap();
    let mcp: serde_json::Value = serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(mcp["landsIn"], item["landsIn"], "MCP and CLI agree");
    item["landsIn"].clone()
}

#[test]
fn land_json_says_which_release_on_merge_day_and_the_day_after() {
    assert_eq!(land_json_with_merge("lands-in-merge-day", 0), serde_json::Value::Null);
    assert_eq!(land_json_with_merge("lands-in-day-after", -1), serde_json::Value::Null);
    assert_eq!(land_json_with_merge("lands-in-before", 2)["version"], 161);
}

#[test]
fn backlog_lists_unstarted_work_for_planning() {
    let home = Home::with_cache("backlog", "busy.json");
    let (ok, out, err) = home.run(&["backlog", "--json"]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["section"], "backlog");
    let items = v["items"].as_array().unwrap();
    assert!(!items.is_empty(), "{v}");
    for item in items {
        assert_eq!(item["tag"], "bl", "{item}");
        assert!(item["reason"].as_str().unwrap().ends_with("not started"), "{item}");
        assert!(item["actionUrl"].as_str().is_some(), "{item}");
    }
    let (ok, text, _) = Home::with_cache("backlog-bz", "bugzilla_only.json").run(&["backlog"]);
    assert!(ok);
    assert!(text.contains("2057035"), "a Bugzilla-only user's unstarted bug:\n{text}");
}

#[test]
fn reviewer_availability_agrees_in_text_json_compact_and_mcp() {
    let home = busy_now("reviewer-away", |raw, now| {
        raw["revisions"].as_array_mut().unwrap().push(serde_json::json!({
            "id": 990001, "title": "Check reviewer availability", "author": "PHID-USER-harbor",
            "status": "needs-review", "modified": days_from(now, -1),
            "reviewers": [{"phid": "PHID-USER-away", "status": "blocking"}]
        }));
        raw["away"]["PHID-USER-away"] = serde_json::json!(days_from(now, 2));
        raw["users"]["PHID-USER-away"] = serde_json::json!("Fixture reviewer");
    });
    let (ok, text, err) = home.run(&["others"]);
    assert!(ok, "{err}");
    assert!(text.contains("Fixture reviewer, away until"), "{text}");
    let (ok, detail, err) = home.run(&["show", "D990001"]);
    assert!(ok, "{err}");
    assert!(detail.contains("Consider another reviewer"), "{detail}");
    let mut documents = Vec::new();
    for flag in ["--json", "--compact"] {
        let (ok, out, err) = home.run(&["others", flag]);
        assert!(ok, "{err}");
        let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
        documents.push(
            doc["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["id"] == "D990001")
                .unwrap()
                .clone(),
        );
    }
    let reply: serde_json::Value = serde_json::from_str(
        &home.mcp(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"worklens_show","arguments":{"ref":"D990001"}}}"#),
    )
    .unwrap();
    let doc: serde_json::Value = serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    documents.push(doc);
    for doc in &documents {
        assert_eq!(doc["reviewerAway"][0]["who"], "Fixture reviewer", "{doc}");
        assert!(doc["reviewerAway"][0]["until"].is_string(), "{doc}");
        assert_eq!(doc["reviewerAway"], documents[0]["reviewerAway"]);
        assert!(doc["reason"].as_str().unwrap().contains("away until"));
    }
}

#[test]
fn weekly_reporting_is_withdrawn_and_old_additive_cache_fields_are_readable() {
    let home = busy_now("withdraw-week", |raw, _| {
        raw["reviewed"] = serde_json::json!([{"old": "ignored"}]);
    });
    assert!(home.run(&["sections", "--json"]).0);
    assert!(!home.run(&["week"]).0);
    let tools = home.mcp(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
    assert!(!tools.contains("worklens_week"), "{tools}");
}

#[test]
fn availability_refresh_replays_the_recorded_shape_and_a_source_derived_away_response() {
    for (replay, away) in [("availability", false), ("availability-away", true)] {
        let home = first_run_home(replay);
        worklens_store::write_atomic(&home.0.join(".arcrc"), |out| {
            out.write_all(br#"{"hosts":{"https://phabricator.services.mozilla.com/api/":{"token":"fixture-token"}}}"#)
        })
        .unwrap();
        worklens_store::write_atomic(&home.0.join(".config/worklens.json"), |out| {
            out.write_all(br#"{"email":"me@mozilla.com","telemetry":false,"landingChecks":false}"#)
        })
        .unwrap();
        let (ok, out, err) = home.run_with(&["refresh"], &[("WORKLENS_REPLAY", &replay_dir(replay))]);
        assert!(ok, "{out}\n{err}");
        assert!(!out.contains("nothing recorded"), "{out}");
        let (ok, out, err) = home.run(&["others", "--json"]);
        assert!(ok, "{err}");
        let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(
            !doc["errors"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["source"].as_str().unwrap_or_default().starts_with("phabricator")),
            "{doc}"
        );
        let item = &doc["items"][0];
        assert_eq!(item["id"], "D990002", "{doc}");
        assert_eq!(item["reason"].as_str().unwrap().contains("away until"), away);
        if away {
            assert_eq!(item["reviewerAway"][0]["who"], "Fixture reviewer");
            assert_eq!(item["reviewerAway"][0]["until"], "2099-01-01T00:00:00Z");
        }
    }
}

#[test]
fn a_second_refresh_respects_the_cached_server_retry_time() {
    let home = first_run_home("rate-limit-second-refresh");
    let replay = replay_dir("rate_limited_long");
    for _ in 0..2 {
        let (ok, out, err) = home.run_with(&["refresh"], &[("WORKLENS_REPLAY", &replay)]);
        assert!(ok, "{out}\n{err}");
    }
    let raw: serde_json::Value = serde_json::from_slice(&fs::read(home.0.join(".cache/worklens/state.json")).unwrap()).unwrap();
    assert!(
        raw["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["source"] == "bugzilla" && e["retryAt"] == "2100-01-01T00:00:00Z"),
        "{raw}"
    );
    assert!(
        !raw["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["url"].as_str().unwrap().contains("bugzilla.mozilla.org")),
        "the second refresh must not send Bugzilla requests before 2100"
    );
}

#[test]
fn a_stack_reaches_agents_and_the_shell_bottom_first() {
    let home = Home::new("stack");
    let state = home.0.join(".cache/worklens/state.json");
    let mut raw: serde_json::Value = serde_json::from_slice(&fs::read(&state).unwrap()).unwrap();
    let rev = |id: u32| {
        serde_json::json!({
            "id": id, "phid": format!("PHID-DREV-{id}"), "title": format!("Captcha patch {id}"), "status": "needs-review",
            "author": "PHID-USER-me", "bug": null, "modified": "2026-11-02T12:00:00Z", "reviewers": [],
        })
    };
    raw["revisions"].as_array_mut().unwrap().extend([rev(338972), rev(338971)]);
    raw["dependencies"] = serde_json::json!({ "parents": { "338972": ["PHID-DREV-338971"] } });
    worklens_store::write_atomic(&state, |w| serde_json::to_writer(w, &raw).map_err(std::io::Error::other)).unwrap();

    let (ok, out, err) = home.run(&["show", "D338972", "--json"]);
    assert!(ok, "{err}");
    let shown: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        shown["stack"],
        serde_json::json!({
            "position": 2, "of": 2, "bottom": "D338971", "bottomStatus": "needs-review",
            "revisions": ["D338971", "D338972"],
        })
    );

    let (ok, out, err) = home.run(&["next", "--compact"]);
    assert!(ok, "{err}");
    let next: serde_json::Value = serde_json::from_str(&out).unwrap();
    let stacked: Vec<(&str, u64)> = next["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| Some((i["id"].as_str()?, i["stack"]["position"].as_u64()?)))
        .collect();
    assert_eq!(stacked, [("D338971", 1), ("D338972", 2)]);

    let (ok, out, err) = home.run(&["sections"]);
    assert!(ok, "{err}");
    let head = out
        .find("┌ Stack of 2 · bottom D338971 needs-review")
        .unwrap_or_else(|| panic!("{out}"));
    assert!(
        out[head..].find("D338971").unwrap() < out[head..].find("D338972 ").unwrap(),
        "{out}"
    );
}
