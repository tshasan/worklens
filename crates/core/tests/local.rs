use jiff::Timestamp;
use serde_json::{Value, json};
use worklens_core::local::{Sync, is_wip, title_of};
use worklens_core::rules::{NextStep, Reason};
use worklens_core::snapshot::ItemId;
use worklens_core::{BugId, CoreConfig, Entry, Raw, Section, Snapshot, build_snapshot};

const NOW: &str = "2026-11-03T19:00:00Z";

fn busy_json() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../store/fixtures/busy.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn snapshot(v: Value) -> Snapshot {
    let raw: Raw = serde_json::from_value(v).unwrap();
    build_snapshot(raw, &CoreConfig::default(), NOW.parse::<Timestamp>().unwrap())
}

fn busy() -> Snapshot {
    snapshot(busy_json())
}

fn by_id<'a>(s: &'a Snapshot, id: &str) -> &'a Entry {
    s.entries
        .iter()
        .find(|e| s.id(e).to_string() == id)
        .unwrap_or_else(|| panic!("no entry {id}"))
}

fn local_ids(s: &Snapshot) -> Vec<String> {
    s.section(Section::Local, true).map(|i| s.id(s.entry(i)).to_string()).collect()
}

fn shas(s: &Snapshot, e: &Entry) -> Vec<String> {
    let commits = &s.raw.local.as_ref().unwrap().commits;
    e.local.iter().map(|i| commits[i.get()].sha.clone()).collect()
}

#[test]
fn commits_without_a_known_bug_are_listed_by_branch() {
    let s = busy();
    let e = by_id(&s, "wip-ranking");
    assert!(matches!(e.id, ItemId::Branch(_)));
    assert_eq!(s.title(e), "WIP: try ranking tabs by recency", "newest commit first");
    assert_eq!(shas(&s, e).len(), 2);
    assert_eq!(e.placement.unwrap().section, Section::Local);
    assert_eq!(
        s.reason(e).to_string(),
        "2 commits on wip-ranking not on Phabricator, newest today, marked WIP"
    );
    assert_eq!(s.next_step(e), Some(NextStep::Upload));
}

#[test]
fn a_bug_found_only_through_a_local_commit_is_listed_until_it_closes() {
    let s = busy();
    let e = by_id(&s, "2020001");
    assert!(matches!(e.placement.unwrap().reason, Reason::NotUploaded(_)));
    assert_eq!(shas(&s, e), ["b2e0000000000000000000000000000000000001"]);
    assert_eq!(e.placement.unwrap().since, Some("2026-11-01T09:00:00Z".parse().unwrap()));

    let mut v = busy_json();
    let bugs = v["bugs"].as_array_mut().unwrap();
    let bug = bugs.iter_mut().find(|b| b["id"] == 2_020_001).unwrap();
    bug["status"] = json!("RESOLVED");
    bug["resolution"] = json!("FIXED");
    let s = snapshot(v);
    assert!(!local_ids(&s).contains(&"2020001".to_owned()));
    assert!(
        s.entries.iter().all(|e| s.id(e).to_string() != "2020001"),
        "a closed local-only bug is dropped"
    );
}

#[test]
fn an_edit_after_upload_and_an_unsquashed_fixup_are_drift() {
    let s = busy();
    let e = by_id(&s, "1998555");
    assert_eq!(
        shas(&s, e),
        [
            "c3d0000000000000000000000000000000000002",
            "c3d0000000000000000000000000000000000001"
        ]
    );
    let messages: Vec<&str> = e.drift.iter().map(|d| d.message.as_str()).collect();
    assert!(
        messages.contains(&"D342300's latest diff differs from local commit c3d000000000"),
        "{messages:?}"
    );
    assert!(
        messages.contains(&"fixup c3d000000000 for D342300 isn't squashed and uploaded"),
        "{messages:?}"
    );
    let differs = e.drift.iter().find(|d| d.message.contains("differs")).unwrap();
    assert!(differs.cause.contains("webgpu-cleanup is newer than the diff"), "{}", differs.cause);
    assert!(differs.cause.contains("moz-phab submit"), "{}", differs.cause);
    assert!(
        !messages.iter().any(|m| m.contains("title")),
        "a reviewer suffix isn't a title change: {messages:?}"
    );
    assert!(s.section(Section::Drift, true).any(|i| s.entry(i).id == e.id));
    assert!(
        s.section(Section::Local, true).any(|i| s.entry(i).id == e.id),
        "the fixup isn't uploaded"
    );
    assert_eq!(e.placement.unwrap().section, Section::Returned, "its review state still places it");
}

#[test]
fn a_newer_diff_on_phabricator_says_to_pull_it() {
    let mut v = busy_json();
    v["local"]["uploads"]["342300"]["uploaded"] = json!("2026-11-03T18:00:00Z");
    let s = snapshot(v);
    let e = by_id(&s, "1998555");
    let differs = e.drift.iter().find(|d| d.message.contains("differs")).unwrap();
    assert!(differs.cause.contains("after your commit"), "{}", differs.cause);
    assert!(differs.cause.contains("moz-phab patch D342300"), "{}", differs.cause);
}

#[test]
fn same_changes_with_a_new_title_only_flag_the_title() {
    let s = busy();
    let e = by_id(&s, "2002472");
    assert_eq!(
        shas(&s, e),
        ["d4c0000000000000000000000000000000000001"],
        "the older worktree copy of D348881 is superseded"
    );
    let commits = &s.raw.local.as_ref().unwrap().commits;
    let older = commits
        .iter()
        .position(|c| c.sha == "d4c0000000000000000000000000000000000002")
        .unwrap();
    assert_eq!(s.syncs[older], Sync::Superseded);
    assert_eq!(s.syncs[e.local[0].get()], Sync::Same);
    let messages: Vec<&str> = e.drift.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(messages, ["D348881's title differs from local commit d4c000000000"]);
    assert!(
        e.drift[0]
            .cause
            .contains("Phabricator says \"Bug 2002472 - Throttle WebGPU backend\"")
    );
    assert!(
        e.drift[0]
            .cause
            .contains("\"Bug 2002472 - Throttle the WebGPU backend while hidden\"")
    );
}

#[test]
fn a_commit_for_a_closed_revision_is_left_alone() {
    let s = busy();
    let commits = &s.raw.local.as_ref().unwrap().commits;
    let landed = commits
        .iter()
        .position(|c| c.sha == "e5b0000000000000000000000000000000000001")
        .unwrap();
    assert_eq!(s.syncs[landed], Sync::Closed);
    assert!(s.entries.iter().all(|e| e.local.iter().all(|i| i.get() != landed)));
}

#[test]
fn a_copy_of_uploaded_changes_on_another_branch_is_not_unsent() {
    let mut v = busy_json();
    v["local"]["commits"].as_array_mut().unwrap().push(json!({
        "sha": "f600000000000000000000000000000000000001",
        "place": "pagewindow",
        "subject": "Bug 2002472 - an unrelated wording",
        "bug": 2002472,
        "committed": "2026-11-03T18:00:00Z",
        "fingerprint": "00000000000000d4"
    }));
    let s = snapshot(v);
    let at = s.syncs.len() - 1;
    assert_eq!(s.syncs[at], Sync::Superseded, "same fingerprint as the uploaded D348881 commit");
}

#[test]
fn an_unscanned_checkout_turns_the_section_off_and_a_failed_scan_says_so() {
    let mut v = busy_json();
    v.as_object_mut().unwrap().remove("local");
    let s = snapshot(v);
    assert!(s.off(Section::Local));
    assert!(s.not_checked(Section::Local).unwrap().starts_with("Off:"));
    assert!(s.entries.iter().all(|e| e.local.is_empty()));

    let mut v = busy_json();
    v["local"]["error"] = json!("git rev-parse in /home/harbor/firefox: not a git repository");
    v["local"]["commits"] = json!([]);
    let s = snapshot(v);
    assert!(!s.off(Section::Local));
    assert!(s.not_checked(Section::Local).unwrap().starts_with("Not checked:"));
}

#[test]
fn titles_compare_without_reviewers_wip_or_a_final_period() {
    assert_eq!(title_of("Bug 1 - Do it. r?gale,#ai-platform-reviewers!"), "Bug 1 - Do it");
    assert_eq!(title_of("WIP: Bug 1 - Do it r=gale"), "Bug 1 - Do it");
    assert_eq!(title_of("Bug 1 - Use r? syntax in docs"), "Bug 1 - Use r? syntax in docs");
    assert_eq!(title_of("Bug 1 - Do it r?gale r?quill"), "Bug 1 - Do it");
    assert!(is_wip("No bug - WIP: prototype"));
    assert!(is_wip("No bug - DO NOT LAND: enable by default"));
    assert!(!is_wip("Bug 1 - Wipe the cache"));
}

#[test]
fn local_bugs_are_backlog_only_when_nothing_is_local() {
    let s = busy();
    let in_backlog = s
        .section(Section::Backlog, true)
        .any(|i| s.entry(i).bug.is_some_and(|b| s.bug(b).id == BugId(2_020_001)));
    assert!(!in_backlog);
}

#[test]
fn an_older_unlinked_copy_of_an_uploaded_bug_is_not_unsent_but_a_newer_follow_up_is() {
    let mut v = busy_json();
    let commits = v["local"]["commits"].as_array_mut().unwrap();
    for (sha, committed) in [
        ("f700000000000000000000000000000000000001", "2026-10-19T10:00:00Z"),
        ("f700000000000000000000000000000000000002", "2026-11-03T18:30:00Z"),
    ] {
        commits.push(json!({
            "sha": sha,
            "place": "worktree firefox-captcha-verify",
            "subject": "Bug 2002472 - Throttle WebGPU, reworded before upload",
            "bug": 2002472,
            "committed": committed,
            "fingerprint": format!("{:0>16}", &sha[36..])
        }));
    }
    let s = snapshot(v);
    let n = s.syncs.len();
    assert_eq!(s.syncs[n - 2], Sync::Superseded, "older than D348881's commit");
    assert_eq!(s.syncs[n - 1], Sync::NotUploaded, "newer than D348881's commit");
    let e = by_id(&s, "2002472");
    assert!(e.drift.iter().any(|d| d.message == "1 local commit isn't on Phabricator"));
}
