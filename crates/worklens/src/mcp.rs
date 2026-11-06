use std::io::{BufRead as _, Write as _};

use serde::Deserialize;
use serde_json::{Value, json};
use worklens_core::section::Section;
use worklens_core::{EntryIdx, Snapshot, brief, present};

use crate::json as shapes;
use crate::{Freshness, load};

const REF: &str = "Bug number (2066590), Jira key (AIPLAT-1305), revision (D321987), or a link to any of them";

struct Tool {
    name: &'static str,
    description: &'static str,
    input_schema: fn() -> Value,
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "worklens_telemetry",
        description: "Personal Firefox Desktop Glean definitions matched by contact email or loaded bug: expiry evidence, definition source, and data-review references. References do not establish approval. Never contains collected telemetry values.",
        input_schema: || json!({ "type": "object", "properties": {} }),
    },
    Tool {
        name: "worklens_sections",
        description: "The user's work by whose turn it is: waiting (needinfos and reviews asked of them), land (their accepted revisions), returned (changes requested), others (their revisions waiting on reviewers, with who), sprint, drift, other, team. Each item has `reason` (one checkable fact), `since`, `stale` (waited past Mozilla's 7-day thresholds) and `action` (the suggested next step). Start here when asked what's on their plate, what's waiting on them, or what to work on.",
        input_schema: || {
            let ids: Vec<&str> = Section::ALL.iter().map(|s| s.id()).collect();
            json!({ "type": "object", "properties": { "section": { "type": "string", "enum": ids, "description": "Only this section" } } })
        },
    },
    Tool {
        name: "worklens_next",
        description: "What's waiting on the user, ready to land, returned to them and waiting on others, in that order; within each, backed-out landings (`bo`) first, then critical items, then important, then the rest, oldest first. Each item has `reason` and `action` (suggested next step). Use the order as given; it's the user's.",
        input_schema: || json!({ "type": "object", "properties": { "limit": { "type": "number", "description": "Max items, default 10; `more` counts the rest" } } }),
    },
    Tool {
        name: "worklens_sprint",
        description: "Active sprint: story points committed vs the user's budget, points done, unestimated tickets, and the sprint's items.",
        input_schema: || json!({ "type": "object", "properties": {} }),
    },
    Tool {
        name: "worklens_drift",
        description: "Places where a Bugzilla bug, its Phabricator revisions and its mirrored Jira ticket disagree (status, assignee, missing links), each with its likely cause. scope 'mine' is the user's bugs; 'team' is every whiteboard-tagged team bug.",
        input_schema: || {
            json!({ "type": "object", "properties": {
                "scope": { "type": "string", "enum": ["mine", "team"] },
                "owner": { "type": "string", "description": "Filter team drift by owner email" },
            } })
        },
    },
    Tool {
        name: "worklens_show",
        description: "Everything worklens knows about one item: bug, Jira ticket, revisions with reviewers and try status, needinfo, latest comment, drift, why it's in its section, and the next step.",
        input_schema: || json!({ "type": "object", "properties": { "ref": { "type": "string", "description": REF } }, "required": ["ref"] }),
    },
    Tool {
        name: "worklens_brief",
        description: "A markdown handoff for starting work on one item: bug description, latest comments, revisions with checkout commands, Jira state, why it's on the list, and the next step. Fetch this before working on a bug.",
        input_schema: || json!({ "type": "object", "properties": { "ref": { "type": "string", "description": REF } }, "required": ["ref"] }),
    },
    Tool {
        name: "worklens_health",
        description: "Each source (Bugzilla, Phabricator, Jira, Treeherder, release trains, crash stats, Glean) as the last fetch saw it: state ok/failed/stale/off, requests and failures, its error, since when stale data is shown, and where its key comes from (never the key). Every other tool's result also has `partial`, `errors`, `stale` and `ageMinutes`; check them before saying something is empty.",
        input_schema: || json!({ "type": "object", "properties": {} }),
    },
    Tool {
        name: "worklens_refresh",
        description: "Refetch from Bugzilla, Jira and Phabricator, bypassing the cache (about 10 seconds). Only needed after the user changed something upstream. A source that fails keeps its previous data, listed in `stale`.",
        input_schema: || json!({ "type": "object", "properties": {} }),
    },
];

#[derive(Deserialize)]
struct Request {
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Deserialize, Default)]
struct Args {
    limit: Option<usize>,
    scope: Option<String>,
    owner: Option<String>,
    #[serde(rename = "ref")]
    reference: Option<String>,
    section: Option<String>,
}

pub fn serve() -> anyhow::Result<()> {
    let stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Request>(&line) {
            Err(_) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "Parse error" } })),
            Ok(Request { id: None, .. }) => None,
            Ok(Request {
                id: Some(id),
                method,
                params,
            }) => Some(match handle(&method, &params) {
                Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
            }),
        };
        if let Some(reply) = reply {
            serde_json::to_writer(&mut stdout, &reply)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

fn handle(method: &str, params: &Value) -> Result<Value, (i32, String)> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": params.get("protocolVersion").cloned().unwrap_or(Value::from("2025-06-18")),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "worklens", "version": env!("CARGO_PKG_VERSION") },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => {
            let tools: Vec<Value> = TOOLS
                .iter()
                .map(|t| json!({ "name": t.name, "description": t.description, "inputSchema": (t.input_schema)() }))
                .collect();
            Ok(json!({ "tools": tools }))
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            if !TOOLS.iter().any(|t| t.name == name) {
                return Err((-32602, format!("Unknown tool {name}")));
            }
            let args: Args = params
                .get("arguments")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .unwrap_or_default()
                .unwrap_or_default();
            Ok(match call(name, &args) {
                Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
                Err(e) => json!({ "content": [{ "type": "text", "text": format!("{e:#}") }], "isError": true }),
            })
        }
        _ => Err((-32601, format!("Method not found: {method}"))),
    }
}

fn find(s: &Snapshot, reference: Option<&str>) -> anyhow::Result<EntryIdx> {
    let reference = reference.ok_or_else(|| anyhow::anyhow!("ref is required"))?;
    s.find(reference)
        .ok_or_else(|| anyhow::anyhow!("{reference} isn't in the user's work or team drift. Call worklens_sections to see what is."))
}

fn compacts(s: &Snapshot, ids: impl Iterator<Item = EntryIdx>) -> Vec<Value> {
    ids.map(|i| shapes::compact(s, s.entry(i))).collect()
}

fn call(name: &str, args: &Args) -> anyhow::Result<String> {
    let freshness = if name == "worklens_refresh" {
        Freshness::Fetch
    } else {
        Freshness::Auto
    };
    let (s, _) = load(freshness)?;
    let s = &s;
    let errors = serde_json::to_value(&s.raw.errors)?;
    let out = match name {
        "worklens_telemetry" => json!({ "definitions": s.raw.telemetry,
            "evidence": worklens_core::evidence::telemetry(s), "errors": errors }),
        "worklens_sections" => {
            let sections: Vec<Value> = Section::ALL
                .iter()
                .filter(|x| args.section.as_deref().is_none_or(|want| want == x.id()))
                .map(|x| json!({ "id": x.id(), "title": x.title(), "items": compacts(s, s.section(*x, false)) }))
                .collect();
            json!({ "fetchedAt": s.raw.fetched_at, "sprint": shapes::sprint(s), "sections": sections, "errors": errors })
        }
        "worklens_next" => {
            let turns = [Section::Waiting, Section::Land, Section::Returned, Section::Others];
            let limit = args.limit.unwrap_or(10);
            let every: Vec<EntryIdx> = turns.iter().flat_map(|t| s.section(*t, false)).collect();
            let more = every.len().saturating_sub(limit);
            let ids = every.into_iter().take(limit);
            json!({ "fetchedAt": s.raw.fetched_at, "sprint": shapes::sprint(s), "items": compacts(s, ids), "more": more, "errors": errors })
        }
        "worklens_sprint" => json!({ "sprint": shapes::sprint(s), "items": compacts(s, s.section(Section::Sprint, true)) }),
        "worklens_drift" => {
            let team = args.scope.as_deref() == Some("team");
            let ids = s.section(if team { Section::Team } else { Section::Drift }, true);
            let list: Vec<Value> = ids
                .filter(|i| args.owner.as_deref().is_none_or(|o| present::owner(s, s.entry(*i)) == Some(o)))
                .map(|i| shapes::compact_drift(s, s.entry(i)))
                .collect();
            Value::from(list)
        }
        "worklens_show" => serde_json::to_value(shapes::item(s, s.entry(find(s, args.reference.as_deref())?)))?,
        "worklens_brief" => return Ok(brief::render(s, s.entry(find(s, args.reference.as_deref())?))),
        "worklens_health" => json!({ "sources": crate::health::from_cache(&s.raw, &crate::config::load()?) }),
        _ => {
            let mine = s.entries.iter().filter(|e| e.placement.is_some()).count();
            json!({ "fetchedAt": s.raw.fetched_at, "items": mine, "team": s.section(Section::Team, true).count(), "errors": errors })
        }
    };
    Ok(serde_json::to_string(&shapes::stamp(
        out.into(),
        &shapes::status(s, jiff::Timestamp::now()),
    ))?)
}
