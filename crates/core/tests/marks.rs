use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use serde_json::{Value, json};
use worklens_core::marks::{Hidden, Marks, Signature, apply, mark_all_seen, mark_done, mark_snoozed, parse_when, restore};
use worklens_core::{CoreConfig, Raw, Snapshot, build_snapshot};

const ME: &str = "me@mozilla.com";

fn now() -> Timestamp {
    "2026-11-02T12:00:00Z".parse().unwrap()
}

fn bug(id: u32, extra: &Value) -> Value {
    let mut b = json!({
        "id": id, "summary": format!("bug {id}"), "status": "ASSIGNED", "resolution": "",
        "assigned_to": ME, "severity": "S3", "priority": "P3", "keywords": [], "whiteboard": "",
        "see_also": [], "flags": [], "last_change_time": "2026-11-01T12:00:00Z",
        "product": "Core", "component": "ML", "via": ["assigned"],
    });
    for (k, v) in extra.as_object().unwrap() {
        b[k] = v.clone();
    }
    b
}

fn snapshot(bugs: &[Value], last_comments: &Value) -> Snapshot {
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-02T12:00:00Z",
        "me": { "email": ME, "jiraEmail": ME, "phid": "PHID-ME" },
        "bugs": bugs, "issues": {}, "revisions": [], "toReview": [], "bugRevisions": {},
        "lastComments": last_comments, "errors": [],
    }))
    .unwrap();
    build_snapshot(raw, &CoreConfig::default(), now())
}

fn hidden_after(s: &mut Snapshot, marks: &Marks, at: Timestamp) -> Option<Hidden> {
    apply(s, marks, at);
    s.entries[0].marks.hidden
}

#[test]
fn done_comes_back_on_new_activity_from_others_not_on_my_own_comment() {
    let mut marks = Marks::default();
    let first = snapshot(&[bug(1, &json!({}))], &json!({}));
    mark_done(&mut marks, &first, &first.entries[0], now());
    let later = now() + 1.second();
    let comment = |author: &str| json!({ "1": { "author": author, "time": "2026-11-02T12:00:00.500Z", "text": "reply" } });

    assert_eq!(
        hidden_after(&mut snapshot(&[bug(1, &json!({}))], &json!({})), &marks, later),
        Some(Hidden::Done)
    );
    assert_eq!(
        hidden_after(&mut snapshot(&[bug(1, &json!({}))], &comment(ME)), &marks, later),
        Some(Hidden::Done),
        "my own comment isn't new activity"
    );
    assert_eq!(
        hidden_after(&mut snapshot(&[bug(1, &json!({}))], &comment("x@mozilla.com")), &marks, later),
        None,
        "someone else's is"
    );
    assert_eq!(
        hidden_after(
            &mut snapshot(&[bug(1, &json!({ "status": "REOPENED" }))], &json!({})),
            &marks,
            later
        ),
        None,
        "so is a change to the facts"
    );
}

#[test]
fn a_snooze_ends_at_its_date_or_on_new_activity() {
    let mut s = snapshot(&[bug(1, &json!({}))], &json!({}));
    let mut marks = Marks::default();
    mark_snoozed(&mut marks, &s, &s.entries[0], Some(now() + 24.hours()), now());
    assert_eq!(hidden_after(&mut s, &marks, now() + 1.second()), Some(Hidden::Snoozed));
    assert_eq!(s.entries[0].marks.snoozed_until, Some(now() + 24.hours()));
    assert_eq!(hidden_after(&mut s, &marks, now() + 48.hours()), None);
    restore(&mut marks, &s, &s.entries[0]);
    assert_eq!(hidden_after(&mut s, &marks, now() + 1.second()), None, "restore lifts it");
}

#[test]
fn new_means_activity_since_i_last_looked_and_the_first_run_isnt_all_new() {
    let mut s = snapshot(&[bug(1, &json!({}))], &json!({}));
    apply(&mut s, &Marks::default(), now());
    assert!(!s.entries[0].marks.is_new);

    let mut looked = Marks::default();
    mark_all_seen(&mut looked, &s, now());
    looked.last_look = Some(now());
    apply(&mut s, &looked, now());
    assert!(!s.entries[0].marks.is_new);

    let mut later = snapshot(&[bug(1, &json!({ "status": "REOPENED" })), bug(2, &json!({}))], &json!({}));
    apply(&mut later, &looked, now());
    let new: Vec<_> = later.entries.iter().map(|e| (later.id(e).to_string(), e.marks.is_new)).collect();
    assert_eq!(new, [("1".to_owned(), true), ("2".to_owned(), true)]);
}

#[test]
fn hidden_entries_leave_their_section_unless_asked_for() {
    use worklens_core::section::Section;
    let mut s = snapshot(&[bug(1, &json!({})), bug(2, &json!({}))], &json!({}));
    let mut marks = Marks::default();
    mark_done(&mut marks, &s, &s.entries[0], now());
    apply(&mut s, &marks, now());
    assert_eq!(s.section(Section::Other, false).count(), 1);
    assert_eq!(s.section(Section::Other, true).count(), 2);
}

#[test]
fn marks_files_from_the_typescript_version_read_and_round_trip() {
    let ts = r#"{"version":1,"lastLook":"2026-11-02T12:00:00.000Z","items":{"1":{"done":{"at":"2026-11-02T12:00:00.000Z","sig":"[]"},"snooze":{"at":"2026-11-02T12:00:00.000Z","sig":"[]","until":null}}}}"#;
    let marks: Marks = serde_json::from_str(ts).unwrap();
    assert_eq!(marks.last_look, Some(now()));
    assert!(marks.items["1"].snooze.as_ref().unwrap().until.is_none());
    let again: Marks = serde_json::from_str(&serde_json::to_string(&marks).unwrap()).unwrap();
    assert_eq!(again.items["1"].done.as_ref().unwrap().sig, "[]");
}

#[test]
fn snooze_dates_read_the_way_people_write_them() {
    let monday = now().to_zoned(TimeZone::UTC);
    let day = |w: &str| parse_when(w, &monday).map(|t| t.to_zoned(TimeZone::UTC).date().to_string());
    assert_eq!(day("tomorrow").as_deref(), Some("2026-11-03"));
    assert_eq!(day("mon").as_deref(), Some("2026-11-09"), "the next Monday, not today");
    assert_eq!(day("next week").as_deref(), Some("2026-11-09"));
    assert_eq!(day("fri").as_deref(), Some("2026-11-06"));
    assert_eq!(day("3d").as_deref(), Some("2026-11-05"));
    assert_eq!(day("2w").as_deref(), Some("2026-11-16"));
    assert_eq!(day("nov 10").as_deref(), Some("2026-11-10"));
    assert_eq!(day("10 nov").as_deref(), Some("2026-11-10"));
    assert_eq!(day("November 10").as_deref(), Some("2026-11-10"));
    assert_eq!(day("jan 5").as_deref(), Some("2027-01-05"), "a date that's passed means next year");
    assert_eq!(day("2026-12-01").as_deref(), Some("2026-12-01"));
    assert_eq!(day("someday"), None);
    assert_eq!(day("feb 31"), None);
    assert_eq!(day("4294967296d"), None, "too far ahead is no date, not a crash");
    assert_eq!(day("9223372036854775807w"), None);
    let nine = parse_when("tomorrow", &monday).unwrap().to_zoned(TimeZone::UTC);
    assert_eq!((nine.hour(), nine.minute()), (9, 0), "snoozes end at 9:00");
}

#[test]
fn find_accepts_bug_numbers_jira_keys_and_d_numbers() {
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-02T12:00:00Z",
        "me": { "email": ME, "jiraEmail": ME, "phid": "PHID-ME" },
        "bugs": [bug(7, &json!({ "see_also": ["https://mozilla-hub.atlassian.net/browse/AI-7"] }))],
        "issues": { "AI-7": { "key": "AI-7", "summary": "AI-7", "status": "In Progress", "statusCategory": "indeterminate",
                              "assignee": ME, "updated": "2026-11-01T12:00:00Z", "points": 3, "sprint": null } },
        "revisions": [{ "id": 70, "title": "D70", "status": "needs-review", "author": "PHID-ME", "bug": 7,
                        "modified": "2026-11-01T12:00:00Z", "reviewers": [] }],
        "toReview": [], "bugRevisions": {}, "lastComments": {}, "errors": [],
    }))
    .unwrap();
    let s = build_snapshot(raw, &CoreConfig::default(), now());
    for reference in ["7", "bug 7", "Bug7", "ai-7", "D70", "d70"] {
        let found = s.find(reference).map(|i| s.id(s.entry(i)).to_string());
        assert_eq!(found.as_deref(), Some("7"), "{reference}");
    }
    assert_eq!(s.find("D999"), None);
}

#[test]
fn find_accepts_links_pasted_from_each_tool() {
    let raw: Raw = serde_json::from_value(json!({
        "version": 1, "fetchedAt": "2026-11-02T12:00:00Z",
        "me": { "email": ME, "jiraEmail": ME, "phid": "PHID-ME" },
        "bugs": [bug(7, &json!({ "see_also": ["https://mozilla-hub.atlassian.net/browse/AI-7"] }))],
        "issues": { "AI-7": { "key": "AI-7", "summary": "AI-7", "status": "In Progress", "statusCategory": "indeterminate",
                              "assignee": ME, "updated": "2026-11-01T12:00:00Z", "points": 3, "sprint": null } },
        "revisions": [{ "id": 70, "title": "D70", "status": "needs-review", "author": "PHID-ME", "bug": 7,
                        "modified": "2026-11-01T12:00:00Z", "reviewers": [] }],
        "toReview": [], "bugRevisions": {}, "lastComments": {}, "errors": [],
    }))
    .unwrap();
    let s = build_snapshot(raw, &CoreConfig::default(), now());
    for link in [
        "https://bugzilla.mozilla.org/show_bug.cgi?id=7",
        "https://bugzilla.mozilla.org/show_bug.cgi?id=7#c12",
        "https://bugzilla.mozilla.org/show_bug.cgi?format=default&id=7",
        "bugzilla.mozilla.org/show_bug.cgi?id=7",
        "https://bugzil.la/7",
        "https://bugzilla.mozilla.org/7",
        "https://phabricator.services.mozilla.com/D70",
        "https://phabricator.services.mozilla.com/D70/",
        "https://phabricator.services.mozilla.com/D70#inline-123",
        "https://mozilla-hub.atlassian.net/browse/AI-7",
        "https://mozilla-hub.atlassian.net/browse/AI-7?focusedCommentId=1",
        "https://mozilla-hub.atlassian.net/jira/software/c/projects/AI/boards/1?selectedIssue=AI-7",
        "  <https://bugzilla.mozilla.org/show_bug.cgi?id=7>  ",
    ] {
        let found = s.find(link).map(|i| s.id(s.entry(i)).to_string());
        assert_eq!(found.as_deref(), Some("7"), "{link}");
    }
    assert_eq!(s.find("https://bugzilla.mozilla.org/show_bug.cgi?id=8"), None);
    assert_eq!(s.find("https://example.com/"), None);
    assert_eq!(s.find("/"), None);
}

#[test]
fn a_done_mark_saved_before_typed_signatures_still_holds() {
    let old = r#"["waiting",["a@mozilla.com","2026-10-30T10:00:00Z"],null,[[321987,"needs-review",["quillfox:blocking"],"fail"]],["AIPLAT-1416","ENG REVIEW","me@mozilla.com"],["ASSIGNED","me@mozilla.com"],[]]"#;
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/raw.json");
    let raw: Raw = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let mut s = build_snapshot(raw, &CoreConfig::default(), "2026-11-03T12:00:00Z".parse().unwrap());
    let e = &s.entries[0];
    let mut current = Signature::of(&s, e);
    current.facts = None;
    assert_eq!(current.to_sig(), old, "the legacy fields are unchanged");
    let parsed = Signature::parse(old).expect("an old signature parses");
    assert_eq!(parsed, current);
    assert_eq!(parsed.revisions[0].reviewers, ["quillfox:blocking"]);
    let marks: Marks = serde_json::from_value(json!({
        "version": 1, "lastLook": null,
        "items": { "1995008": { "done": { "at": "2026-11-03T12:00:00Z", "sig": old } } },
    }))
    .unwrap();
    assert_eq!(
        hidden_after(&mut s, &marks, "2026-11-03T13:00:00Z".parse().unwrap()),
        Some(Hidden::Done)
    );
}

#[test]
fn signatures_that_are_not_seven_facts_do_not_parse() {
    assert_eq!(Signature::parse("[]"), None);
    assert_eq!(Signature::parse(r#"["waiting",null,null,[],null,null,[],"extra"]"#), None);
    assert!(Signature::parse(r#"["waiting",null,null,[],null,null,[]]"#).is_some());
}
