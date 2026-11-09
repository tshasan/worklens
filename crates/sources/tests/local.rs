use std::path::{Path, PathBuf};
use std::process::Command;

use worklens_core::{BugId, RevId};
use worklens_sources::local::{fingerprint, scan};

struct Repo {
    root: PathBuf,
}

impl Repo {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("worklens-local-{name}-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&root));
        std::fs::create_dir_all(root.join("firefox")).unwrap();
        let repo = Self { root };
        repo.git(&["init", "-q", "-b", "main"]);
        repo.git(&["config", "user.email", "me@mozilla.com"]);
        repo.git(&["config", "user.name", "Me"]);
        repo
    }

    fn dir(&self) -> PathBuf {
        self.root.join("firefox")
    }

    fn git_in(&self, dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }

    fn git(&self, args: &[&str]) -> String {
        self.git_in(&self.dir(), args)
    }

    fn write(&self, file: &str, text: &str) {
        write(&self.dir().join(file), text);
    }

    fn commit(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", message]);
        self.git(&["rev-parse", "HEAD"]).trim().to_owned()
    }

    fn commit_as(&self, author: &str, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--author", author, "-m", message]);
        self.git(&["rev-parse", "HEAD"]).trim().to_owned()
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.root));
    }
}

fn write(path: &Path, text: &str) {
    worklens_store::write_atomic(path, |w| std::io::Write::write_all(w, text.as_bytes())).unwrap();
}

fn run<T>(f: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

const LINES: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";

#[test]
fn branches_and_detached_worktrees_are_scanned_for_my_unlanded_commits_only() {
    let repo = Repo::new("scan");
    repo.write("a.txt", LINES);
    repo.commit("Initial");
    repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);

    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.write("a.txt", &LINES.replace("five", "FIVE"));
    let uploaded = repo.commit(
        "Bug 2074080 - Give static embeddings their own engineId r?gale\n\nWhy.\n\nDifferential Revision: https://phabricator.services.mozilla.com/D336313",
    );
    repo.write("b.txt", "wip\n");
    let wip = repo.commit("WIP: try something");

    repo.git(&["checkout", "-q", "-b", "backup/feature-pre-rebase"]);
    repo.write("c.txt", "old\n");
    let backed_up = repo.commit("Bug 2074080 - an old copy");

    repo.git(&["checkout", "-q", "-b", "theirs", "origin/main"]);
    repo.write("d.txt", "not mine\n");
    let theirs = repo.commit_as("Someone <someone@example.com>", "Bug 1 - Their work");

    repo.git(&["checkout", "-q", "main"]);
    let worktree = repo.root.join("firefox-D9");
    repo.git(&["worktree", "add", "-q", "--detach", worktree.to_str().unwrap(), "origin/main"]);
    write(&worktree.join("e.txt"), "detached\n");
    repo.git_in(&worktree, &["add", "-A"]);
    repo.git_in(
        &worktree,
        &["-c", "commit.gpgsign=false", "commit", "-q", "-m", "No bug - Detached experiment"],
    );
    let detached = repo.git_in(&worktree, &["rev-parse", "HEAD"]).trim().to_owned();

    let s = run(scan(&repo.dir(), "fallback@mozilla.com")).unwrap();
    assert_eq!(s.base, "origin/main");
    let shas: Vec<&str> = s.commits.iter().map(|c| c.sha.as_str()).collect();
    assert!(shas.contains(&uploaded.as_str()) && shas.contains(&wip.as_str()), "{shas:?}");
    assert!(!shas.contains(&backed_up.as_str()), "backup branches are skipped");
    assert!(!shas.contains(&theirs.as_str()), "other people's commits are skipped");
    assert_eq!(s.commits.len(), 3, "{:?}", s.commits);

    let up = s.commits.iter().find(|c| c.sha == uploaded).unwrap();
    assert_eq!(up.place, "feature");
    assert_eq!(up.bug, Some(BugId(2_074_080)));
    assert_eq!(up.revision, Some(RevId(336_313)));
    assert_eq!(up.subject, "Bug 2074080 - Give static embeddings their own engineId r?gale");
    let w = s.commits.iter().find(|c| c.sha == wip).unwrap();
    assert_eq!((w.bug, w.revision), (None, None));
    let d = s.commits.iter().find(|c| c.sha == detached).unwrap();
    assert_eq!(d.place, "worktree firefox-D9");

    let full_context = repo.git(&["diff", "-U99", "origin/main", &uploaded]);
    assert_eq!(
        up.fingerprint,
        fingerprint(&full_context),
        "context lines don't change the fingerprint"
    );
    assert_ne!(up.fingerprint, w.fingerprint);
}

#[test]
fn a_rebase_keeps_the_fingerprint_and_an_edit_changes_it() {
    let repo = Repo::new("rebase");
    repo.write("a.txt", LINES);
    repo.commit("Initial");
    repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.write("a.txt", &LINES.replace("eight", "EIGHT"));
    let before = repo.commit("Bug 5 - Change eight\n\nDifferential Revision: https://phabricator.services.mozilla.com/D5");
    let print = |sha: &str| fingerprint(&repo.git(&["show", "--format=", sha]));
    let original = print(&before);

    repo.git(&["checkout", "-q", "main"]);
    repo.write("a.txt", &format!("zero\n{}", LINES.replace("five", "FIVE")));
    repo.commit("Upstream shifts the lines and changes the context");
    repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    repo.git(&["checkout", "-q", "feature"]);
    repo.git(&["-c", "commit.gpgsign=false", "rebase", "-q", "main"]);
    let rebased = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
    assert_ne!(before, rebased);

    let s = run(scan(&repo.dir(), "x")).unwrap();
    assert_eq!(s.commits.len(), 1);
    assert_eq!(s.commits[0].sha, rebased);
    assert!(
        repo.git(&["show", &rebased]).contains(" FIVE"),
        "the rebased diff's context differs"
    );
    assert_eq!(
        s.commits[0].fingerprint, original,
        "a rebase with no edits matches the uploaded diff"
    );

    repo.write(
        "a.txt",
        &format!("zero\n{}", LINES.replace("five", "FIVE").replace("eight", "Eight!")),
    );
    repo.git(&["add", "-A"]);
    repo.git(&["-c", "commit.gpgsign=false", "commit", "-q", "--amend", "--no-edit"]);
    let s = run(scan(&repo.dir(), "x")).unwrap();
    assert_ne!(s.commits[0].fingerprint, original, "an edit after upload shows");
}

#[test]
fn a_checkout_without_an_upstream_is_an_error_not_an_empty_list() {
    let repo = Repo::new("no-origin");
    repo.write("a.txt", "x\n");
    repo.commit("Initial");
    let err = run(scan(&repo.dir(), "x")).err().unwrap();
    assert!(err.contains("origin/main"), "{err}");
    let missing = run(scan(&repo.root.join("nowhere"), "x")).err().unwrap();
    assert!(!missing.contains("origin/main"), "{missing}");
    assert!(missing.contains("nowhere"), "{missing}");
}
