use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use proptest::prelude::*;
use serde_json::{Value, json};
use worklens_testkit::{WHITEBOARD, cache, fixture, mutated, mutations, text};

const BUSY: &str = "crates/store/fixtures/busy.json";

const TOOLS: &[&str] = &[
    "worklens_telemetry",
    "worklens_sections",
    "worklens_next",
    "worklens_sprint",
    "worklens_drift",
    "worklens_show",
    "worklens_brief",
    "worklens_health",
    "worklens_status",
];

const REFS: &[&str] = &[
    "1990298",
    "bug 1990361",
    "D330108",
    "d330982",
    "AIPLAT-1420",
    "aiplat-1437",
    "AIDI-79",
    "https://bugzilla.mozilla.org/show_bug.cgi?id=1990396",
    "https://phabricator.services.mozilla.com/D331010",
    "https://mozilla-hub.atlassian.net/browse/AIPLAT-1448",
];

struct Home(PathBuf);

impl Home {
    fn new(cache: &Value, config: &Value) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "worklens-robust-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        write(&dir.join(".cache/worklens/state.json"), cache);
        write(&dir.join(".config/worklens.json"), config);
        Self(dir)
    }

    fn run(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_worklens"))
            .args(args)
            .env("HOME", &self.0)
            .env("XDG_STATE_HOME", self.0.join("state"))
            .env("NO_COLOR", "1")
            .env("WORKLENS_KEYCHAIN", "off")
            .env_remove("BUGZILLA_API_KEY")
            .env_remove("WORKLENS_BUGZILLA_KEY")
            .env_remove("WORKLENS_JIRA_TOKEN")
            .env_remove("WORKLENS_PHABRICATOR_TOKEN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        std::io::Write::write_all(&mut child.stdin.take().unwrap(), stdin.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write(path: &Path, value: &Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    worklens_store::write_atomic(path, |w| serde_json::to_writer(w, value).map_err(std::io::Error::other)).unwrap();
}

fn config() -> Value {
    json!({
        "cacheMinutes": 1e12, "whiteboard": WHITEBOARD, "sprintBudget": 10, "email": "harbor@mozilla.com",
        "jiraEmail": "harbor@mozilla.com", "jiraSite": "https://mozilla-hub.atlassian.net", "jiraAuth": "bearer",
        "storyPointsField": "customfield_10037", "sprintField": "customfield_10020", "driftDays": 14,
        "theme": "dark", "telemetry": true, "landingChecks": true, "showHidden": false,
        "sectionTabs": { "team": "never", "ideas": "always" }, "detailTabs": { "telemetry": "auto" },
        "jiraStatuses": { "done": ["Closed"], "review": ["ENG REVIEW"], "inProgress": ["In Progress"], "blocked": ["Blocked"] },
        "jbiSyncs": ["summary"],
        "keys": { "x": { "label": "Open", "run": "echo {id}", "terminal": false } },
    })
}

fn panicked(out: &Output) -> Option<String> {
    let err = String::from_utf8_lossy(&out.stderr);
    (out.status.code() == Some(101) || err.contains("panicked")).then(|| err.into_owned())
}

fn arg() -> impl Strategy<Value = Value> {
    prop_oneof![
        3 => prop::sample::select(REFS).prop_map(Value::from),
        2 => text().prop_map(Value::from),
        1 => prop::sample::select(vec![
            json!(0), json!(-1), json!(1.5), json!(u64::MAX), json!(1e300), json!(null), json!(true),
            json!([]), json!({}), json!("waiting"), json!("team"), json!("mine"), json!("ideas"),
        ]),
    ]
}

fn request() -> impl Strategy<Value = String> {
    let id = prop_oneof![
        any::<u32>().prop_map(Value::from),
        text().prop_map(Value::from),
        Just(json!(-1)),
        Just(json!(1.5)),
    ];
    let args = prop::collection::btree_map(
        prop::sample::select(vec!["ref", "limit", "section", "scope", "owner", "other"]),
        arg(),
        0..4,
    )
    .prop_map(|m| Value::Object(m.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()));
    let call = (id.clone(), prop::sample::select(TOOLS), args).prop_map(|(id, name, arguments)| {
        json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": name, "arguments": arguments } }).to_string()
    });
    let other = (
        id,
        prop::sample::select(vec!["initialize", "ping", "tools/list", "tools/call", "resources/list", ""]),
        arg(),
    )
        .prop_map(|(id, method, params)| json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string());
    prop_oneof![
        6 => call,
        2 => other,
        1 => Just(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned()),
        1 => text().prop_map(|t| t.replace(['\n', '\r'], " ")),
    ]
}

fn replies_expected(line: &str) -> Option<Option<Value>> {
    if line.trim().is_empty() {
        return None;
    }
    match serde_json::from_str::<Value>(line) {
        Ok(v) if v.get("method").is_some_and(Value::is_string) => v.get("id").filter(|id| !id.is_null()).map(|id| Some(id.clone())),
        _ => Some(None),
    }
}

proptest! {
    #![proptest_config(worklens_testkit::config(16))]

    #[test]
    fn mcp_answers_every_request_exactly_once(lines in prop::collection::vec(request(), 1..30)) {
        let home = Home::new(&fixture(BUSY), &config());
        let out = home.run(&["mcp"], &(lines.join("\n") + "\n"));
        if let Some(err) = panicked(&out) {
            return Err(TestCaseError::fail(err));
        }
        prop_assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let expected: Vec<Option<Value>> = lines.iter().filter_map(|l| replies_expected(l)).collect();
        let stdout = String::from_utf8(out.stdout).unwrap();
        let replies: Vec<Value> = stdout.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        prop_assert_eq!(replies.len(), expected.len(), "{}", stdout);
        for (reply, id) in replies.iter().zip(&expected) {
            prop_assert_eq!(&reply["jsonrpc"], "2.0");
            prop_assert_eq!(&reply["id"], id.as_ref().unwrap_or(&Value::Null));
            prop_assert!(reply.get("result").is_some() != reply.get("error").is_some(), "{}", reply);
            if let Some(text) = reply["result"]["content"][0]["text"].as_str()
                && reply["result"]["isError"] != true
                && text.starts_with('{')
            {
                let doc: Value = serde_json::from_str(text).map_err(|e| TestCaseError::fail(format!("{e}: {text}")))?;
                prop_assert!(doc.get("ageMinutes").is_some(), "{}", text);
            }
        }
    }

    #[test]
    fn a_damaged_config_never_panics(ms in mutations(10)) {
        let home = Home::new(&fixture(BUSY), &mutated(&config(), &ms));
        for args in [&["sections", "--cached", "--json"][..], &["drift", "--team", "--cached"], &["auth", "status"]] {
            if let Some(err) = panicked(&home.run(args, "")) {
                return Err(TestCaseError::fail(format!("{args:?}: {err}")));
            }
        }
    }

    #[test]
    fn a_damaged_cache_never_panics(seed in cache(), ms in mutations(12), r in prop::sample::select(REFS)) {
        let home = Home::new(&mutated(seed, &ms), &config());
        for args in [
            &["sections", "--cached", "--json"][..],
            &["next", "--all", "--cached", "--compact"],
            &["drift", "--team", "--cached", "--json"],
            &["sprint", "--cached"],
            &["show", r, "--cached", "--json"],
            &["brief", r, "--cached"],
            &["health", "--cached", "--json"],
        ] {
            if let Some(err) = panicked(&home.run(args, "")) {
                return Err(TestCaseError::fail(format!("{args:?}: {err}")));
            }
        }
    }
}

#[test]
fn every_item_carries_the_fields_the_skill_promises() {
    let home = Home::new(&fixture(BUSY), &config());
    let out = home.run(&["sections", "--cached", "--json"], "");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["schema"], "worklens/2");
    for key in ["partial", "errors", "stale", "ageMinutes"] {
        assert!(doc.get(key).is_some(), "no {key}");
    }
    let mut items = 0;
    for section in doc["sections"].as_array().unwrap() {
        for item in section["items"].as_array().unwrap() {
            items += 1;
            let keys: &[&str] = if section["id"] == "team" {
                &["id", "title", "owner", "drift", "isNew"]
            } else {
                &[
                    "id", "title", "section", "reason", "since", "stale", "tag", "action", "isNew", "changes", "drift",
                ]
            };
            for key in keys {
                assert!(item.get(key).is_some(), "{} has no {key}: {item}", item["id"]);
            }
        }
    }
    assert!(items > 80, "{items}");
}
