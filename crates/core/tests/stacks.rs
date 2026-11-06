use jiff::Timestamp;
use serde_json::{Value, json};
use worklens_core::section::Section;
use worklens_core::{CoreConfig, Raw, Snapshot, build_snapshot};

const ME: &str = "PHID-ME";

fn rev(id: u32, status: &str, author: &str, day: u32) -> Value {
    json!({
        "id": id, "phid": format!("PHID-DREV-{id}"), "title": format!("patch {id}"), "status": status,
        "author": author, "bug": null, "modified": format!("2026-11-{day:02}T12:00:00Z"), "reviewers": [],
    })
}

fn snapshot(revisions: &[Value], parents: &Value, to_review: &[u32]) -> Snapshot {
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-06T12:00:00Z",
        "me": { "email": "me@mozilla.com", "jiraEmail": "me@mozilla.com", "phid": ME },
        "bugs": [], "issues": {}, "revisions": revisions, "toReview": to_review, "bugRevisions": {}, "lastComments": {},
        "errors": [], "users": { "PHID-ME": "me", "PHID-C": "cdipersio" },
        "dependencies": { "parents": parents },
    }))
    .unwrap();
    build_snapshot(raw, &CoreConfig::default(), "2026-11-06T12:00:00Z".parse::<Timestamp>().unwrap())
}

fn ids(s: &Snapshot, section: Section) -> Vec<String> {
    s.section(section, false).map(|i| s.id(s.entry(i)).to_string()).collect()
}

fn stacked(s: &Snapshot, section: Section) -> Vec<Option<usize>> {
    s.section(section, false).map(|i| s.entry(i).stack.map(|k| k.get())).collect()
}

fn on(child: u32, parent: u32) -> (String, Value) {
    (child.to_string(), json!([format!("PHID-DREV-{parent}")]))
}

fn parents(pairs: &[(u32, u32)]) -> Value {
    Value::Object(pairs.iter().map(|(c, p)| on(*c, *p)).collect())
}

#[test]
fn my_stack_is_one_group_in_landing_order_above_its_landed_base() {
    let s = snapshot(
        &[
            rev(970, "published", ME, 1),
            rev(974, "needs-review", ME, 3),
            rev(972, "needs-review", ME, 4),
            rev(500, "needs-review", ME, 2),
            rev(971, "needs-review", ME, 5),
            rev(973, "needs-review", ME, 1),
        ],
        &parents(&[(971, 970), (972, 971), (973, 972), (974, 973)]),
        &[],
    );
    assert_eq!(s.stacks.len(), 1, "D970 is published, so it isn't part of the open stack");
    assert_eq!(
        ids(&s, Section::Others),
        ["D971", "D972", "D973", "D974", "D500"],
        "D973 is the oldest, so the stack sorts before D500, and lists bottom first"
    );
    assert_eq!(stacked(&s, Section::Others), [Some(0), Some(0), Some(0), Some(0), None]);
    assert_eq!(
        worklens_core::stack::header(&s, worklens_core::StackIdx(0)),
        "Stack of 4 · bottom D971 needs-review"
    );
    let at = |id: &str| worklens_core::stack::position(&s, s.entry(s.find(id).unwrap()));
    assert_eq!(at("D971").as_deref(), Some("Stack: bottom of 4; it lands first"));
    assert_eq!(at("D973").as_deref(), Some("Stack: 3 of 4"));
    assert_eq!(at("D500"), None);
}

#[test]
fn a_stack_split_across_sections_keeps_its_order_in_each() {
    let s = snapshot(
        &[
            rev(11, "accepted", ME, 1),
            rev(12, "needs-review", ME, 2),
            rev(13, "needs-review", ME, 3),
            rev(14, "needs-revision", ME, 4),
        ],
        &parents(&[(12, 11), (13, 12), (14, 13)]),
        &[],
    );
    assert_eq!(ids(&s, Section::Land), ["D11"]);
    assert_eq!(ids(&s, Section::Others), ["D12", "D13"]);
    assert_eq!(ids(&s, Section::Returned), ["D14"]);
    let at = |id: &str| worklens_core::stack::position(&s, s.entry(s.find(id).unwrap()));
    assert_eq!(at("D14").as_deref(), Some("Stack: 4 of 4"));
}

#[test]
fn reviews_group_through_a_patch_that_is_not_in_my_queue() {
    let s = snapshot(
        &[
            rev(22, "needs-review", "PHID-C", 1),
            rev(24, "needs-review", "PHID-C", 2),
            rev(23, "needs-review", "PHID-C", 3),
            rev(30, "needs-review", "PHID-C", 4),
        ],
        &parents(&[(23, 22), (24, 23)]),
        &[24, 22, 30],
    );
    assert_eq!(ids(&s, Section::Waiting), ["D22", "D24", "D30"]);
    assert_eq!(stacked(&s, Section::Waiting), [Some(0), Some(0), None]);
    assert_eq!(
        s.stacks[0].revisions.len(),
        3,
        "D23 isn't asked of me but still counts in the stack"
    );
}

#[test]
fn a_lone_patch_or_a_parent_loop_is_not_a_stack() {
    let s = snapshot(
        &[
            rev(40, "needs-review", ME, 1),
            rev(41, "needs-review", ME, 2),
            rev(42, "needs-review", ME, 3),
        ],
        &parents(&[(41, 42), (42, 41), (40, 99)]),
        &[],
    );
    assert!(s.stacks.len() <= 1, "{:?}", s.stacks);
    assert_eq!(ids(&s, Section::Others).len(), 3, "every patch is still listed once");
    assert!(
        s.entry(s.find("D40").unwrap()).stack.is_none(),
        "D99 is unknown, so D40 stands alone"
    );
}
