use std::collections::HashMap;
use std::path::Path;

use futures::{StreamExt, stream};
use jiff::Timestamp;
use worklens_core::raw::{LocalCommit, Revision, SourceError, Upload};
use worklens_core::{BugId, RevId};

use crate::MAX_IN_FLIGHT;
use crate::phab::Phab;

pub const DAYS: u32 = 90;

const BASES: [&str; 3] = ["origin/main", "upstream/main", "origin/master"];

const PINNED: [&str; 6] = [
    "log.showSignature=false",
    "diff.noprefix=false",
    "diff.mnemonicPrefix=false",
    "diff.relative=false",
    "core.quotePath=false",
    "color.ui=false",
];

pub struct Scan {
    pub base: String,
    pub commits: Vec<LocalCommit>,
}

async fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(PINNED.iter().flat_map(|c| ["-c", c]))
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("can't run git: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(format!(
        "git {} in {}: {}",
        args.first().copied().unwrap_or_default(),
        repo.display(),
        stderr.lines().next().unwrap_or("failed")
    ))
}

async fn base(repo: &Path) -> Result<String, String> {
    if let Ok(head) = git(repo, &["rev-parse", "--abbrev-ref", "origin/HEAD"]).await {
        let head = head.trim();
        if !head.is_empty() && head != "origin/HEAD" {
            return Ok(head.to_owned());
        }
    }
    for candidate in BASES {
        if git(repo, &["rev-parse", "--verify", "--quiet", candidate]).await.is_ok() {
            return Ok(candidate.to_owned());
        }
    }
    Err(format!("{} has no origin/main to compare branches against", repo.display()))
}

pub fn is_backup(branch: &str) -> bool {
    branch.contains("backup")
}

pub fn detached_worktrees(porcelain: &str) -> Vec<(String, String)> {
    porcelain
        .split("\n\n")
        .filter(|block| block.lines().any(|l| l == "detached"))
        .filter_map(|block| {
            let path = block.lines().find_map(|l| l.strip_prefix("worktree "))?;
            let head = block.lines().find_map(|l| l.strip_prefix("HEAD "))?;
            let name = Path::new(path).file_name()?.to_string_lossy().into_owned();
            Some((head.to_owned(), format!("worktree {name}")))
        })
        .collect()
}

pub async fn scan(repo: &Path, fallback_email: &str) -> Result<Scan, String> {
    let email = git(repo, &["config", "user.email"])
        .await
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback_email.to_owned());
    git(repo, &["rev-parse", "--git-dir"]).await?;
    let base = base(repo).await?;
    let mut refs: Vec<String> = git(
        repo,
        &["for-each-ref", "--sort=-committerdate", "--format=%(refname:short)", "refs/heads"],
    )
    .await?
    .lines()
    .filter(|b| !b.is_empty() && !is_backup(b))
    .map(str::to_owned)
    .collect();
    let worktrees = git(repo, &["worktree", "list", "--porcelain"])
        .await
        .map(|out| detached_worktrees(&out))
        .unwrap_or_default();
    refs.extend(worktrees.iter().map(|(sha, _)| sha.clone()));
    if refs.is_empty() {
        return Ok(Scan { base, commits: Vec::new() });
    }
    let author = format!("--author={email}");
    let since = format!("--since={DAYS}.days.ago");
    let lists: Vec<Result<String, String>> = stream::iter(&refs)
        .map(|r| {
            let args = [
                "rev-list",
                "--no-merges",
                "-F",
                author.as_str(),
                since.as_str(),
                r.as_str(),
                "--not",
                base.as_str(),
                "--",
            ];
            async move { git(repo, &args).await }
        })
        .buffered(MAX_IN_FLIGHT)
        .collect()
        .await;
    let labels: HashMap<&str, &str> = worktrees.iter().map(|(sha, label)| (sha.as_str(), label.as_str())).collect();
    let mut places: HashMap<String, String> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for (r, list) in refs.iter().zip(lists) {
        let place = labels.get(r.as_str()).copied().unwrap_or(r);
        for sha in list?.lines().filter(|l| !l.is_empty()) {
            if !places.contains_key(sha) {
                places.insert(sha.to_owned(), place.to_owned());
                order.push(sha.to_owned());
            }
        }
    }
    if order.is_empty() {
        return Ok(Scan { base, commits: Vec::new() });
    }
    let mut args = vec!["log", "--no-walk=unsorted", "--no-color", "--format=%x1e%H%x1f%ct%x1f%B"];
    args.extend(order.iter().map(String::as_str));
    args.push("--");
    let log = git(repo, &args).await?;
    let mut commits = parse_log(&log, &places);
    if commits.is_empty() {
        return Ok(Scan { base, commits });
    }
    let mut show = vec!["show", "-U0", "--no-color", "--no-ext-diff", "--no-textconv", "--format=%x1e%H"];
    show.extend(commits.iter().map(|c| c.sha.as_str()));
    let diffs = git(repo, &show).await?;
    let prints = fingerprints(&diffs);
    for c in &mut commits {
        if let Some(f) = prints.get(c.sha.as_str()) {
            c.fingerprint.clone_from(f);
        }
    }
    Ok(Scan { base, commits })
}

pub fn parse_log(log: &str, places: &HashMap<String, String>) -> Vec<LocalCommit> {
    log.split('\u{1e}')
        .filter_map(|record| {
            let mut parts = record.splitn(3, '\u{1f}');
            let sha = parts.next()?.trim();
            let seconds: i64 = parts.next()?.trim().parse().ok()?;
            let message = parts.next()?;
            let place = places.get(sha)?.clone();
            let subject = message.lines().next().unwrap_or_default().trim().to_owned();
            Some(LocalCommit {
                sha: sha.to_owned(),
                place,
                bug: bug_in(&subject),
                revision: revision_in(message),
                subject,
                committed: Timestamp::from_second(seconds).ok()?,
                fingerprint: String::new(),
            })
        })
        .collect()
}

pub fn bug_in(subject: &str) -> Option<BugId> {
    let lower = subject.to_ascii_lowercase();
    lower.match_indices("bug").find_map(|(at, _)| {
        let before = lower[..at].chars().next_back();
        if before.is_some_and(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        let rest = lower[at + 3..].trim_start_matches([' ', '#']);
        let digits: &str = &rest[..rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len())];
        digits.parse().ok().filter(|n| *n > 0).map(BugId)
    })
}

pub fn revision_in(message: &str) -> Option<RevId> {
    message.lines().rev().find_map(|line| {
        let url = line.trim().strip_prefix("Differential Revision:")?.trim();
        let tail = &url[url.rfind("/D")? + 2..];
        let digits = &tail[..tail.find(|c: char| !c.is_ascii_digit()).unwrap_or(tail.len())];
        digits.parse().ok().filter(|n| *n > 0).map(RevId)
    })
}

pub fn fingerprints(show: &str) -> HashMap<&str, String> {
    show.split('\u{1e}')
        .filter_map(|record| {
            let (sha, diff) = record.split_once('\n').unwrap_or((record, ""));
            let sha = sha.trim();
            (!sha.is_empty()).then(|| (sha, fingerprint(diff)))
        })
        .collect()
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

fn hunk_counts(header: &str) -> Option<(u32, u32)> {
    let mut words = header.strip_prefix("@@ ")?.split(' ');
    let count = |w: &str| -> Option<u32> {
        match w.split_once(',') {
            Some((_, n)) => n.parse().ok(),
            None => Some(1),
        }
    };
    let old = count(words.next()?.strip_prefix('-')?)?;
    let new = count(words.next()?.strip_prefix('+')?)?;
    Some((old, new))
}

fn path_of(line: &str, prefix: &str) -> Option<String> {
    let path = line.strip_prefix(prefix)?;
    let path = path.split('\t').next().unwrap_or(path).trim_end();
    (path != "/dev/null").then(|| {
        path.strip_prefix("a/")
            .or_else(|| path.strip_prefix("b/"))
            .unwrap_or(path)
            .to_owned()
    })
}

pub fn fingerprint(diff: &str) -> String {
    let mut files: Vec<(String, Vec<String>)> = Vec::new();
    let (mut old_path, mut new_path) = (None::<String>, None::<String>);
    let (mut old_left, mut new_left) = (0u32, 0u32);
    let mut current: Option<usize> = None;
    for line in diff.lines() {
        if old_left > 0 || new_left > 0 {
            let Some(i) = current else { break };
            let (sign, body) = line.split_at(line.len().min(1));
            match sign {
                "-" => old_left = old_left.saturating_sub(1),
                "+" => new_left = new_left.saturating_sub(1),
                " " | "" => {
                    old_left = old_left.saturating_sub(1);
                    new_left = new_left.saturating_sub(1);
                    continue;
                }
                _ => continue,
            }
            files[i].1.push(format!("{sign}{}", body.trim_end()));
            continue;
        }
        if line.starts_with("diff ") {
            (old_path, new_path, current) = (None, None, None);
        } else if let Some(p) = path_of(line, "--- ") {
            old_path = Some(p);
        } else if line.starts_with("--- ") {
            old_path = None;
        } else if line.starts_with("+++ ") {
            new_path = path_of(line, "+++ ");
            current = None;
        } else if let Some((old, new)) = hunk_counts(line) {
            let Some(path) = new_path.clone().or_else(|| old_path.clone()) else {
                continue;
            };
            let i = current.unwrap_or_else(|| {
                files.push((path, Vec::new()));
                files.len() - 1
            });
            current = Some(i);
            (old_left, new_left) = (old, new);
        }
    }
    for (_, lines) in &mut files {
        lines.sort_unstable();
    }
    files.sort();
    let all = files.iter().fold(FNV_OFFSET, |h, (path, lines)| {
        let h = fnv(fnv(h, path.as_bytes()), b"\0");
        lines.iter().fold(h, |h, l| fnv(fnv(h, l.as_bytes()), b"\n"))
    });
    format!("{all:016x}")
}

pub struct Cached {
    pub uploaded: Option<Timestamp>,
    pub fingerprint: String,
}

pub async fn uploads(
    phab: Option<&Phab<'_>>,
    commits: &[LocalCommit],
    revisions: &[Revision],
    cached: &HashMap<String, Cached>,
    errors: &mut Vec<SourceError>,
) -> HashMap<RevId, Upload> {
    let mut wanted: Vec<RevId> = commits.iter().filter_map(|c| c.revision).collect();
    wanted.sort_unstable();
    wanted.dedup();
    let mut out: HashMap<RevId, Upload> = HashMap::with_capacity(wanted.len());
    let Some(phab) = phab else { return out };
    let known: HashMap<RevId, &Revision> = revisions.iter().map(|r| (r.id, r)).collect();
    let missing: Vec<RevId> = wanted.iter().copied().filter(|r| !known.contains_key(r)).collect();
    let fetched = if missing.is_empty() {
        Vec::new()
    } else {
        phab.by_ids(&missing).await.unwrap_or_else(|e| {
            errors.push(local_error(&e));
            Vec::new()
        })
    };
    for r in known.values().copied().filter(|r| wanted.contains(&r.id)).chain(&fetched) {
        out.insert(
            r.id,
            Upload {
                title: r.title.clone(),
                status: r.status,
                bug: r.bug,
                diff_phid: r.diff_phid.clone(),
                uploaded: None,
                fingerprint: None,
            },
        );
    }
    let mut todo: Vec<&str> = Vec::new();
    for up in out.values_mut() {
        let Some(phid) = up.diff_phid.as_deref() else { continue };
        match cached.get(phid) {
            Some(c) => {
                up.uploaded = c.uploaded;
                up.fingerprint = Some(c.fingerprint.clone());
            }
            None => todo.push(phid),
        }
    }
    if todo.is_empty() {
        return out;
    }
    let found = match phab.diffs(&todo).await {
        Ok(found) => found,
        Err(e) => {
            errors.push(local_error(&e));
            return out;
        }
    };
    let raw: Vec<(String, crate::http::Result<String>)> = stream::iter(found.iter())
        .map(|(phid, (id, _))| async move { (phid.clone(), phab.raw_diff(*id).await) })
        .buffer_unordered(MAX_IN_FLIGHT)
        .collect()
        .await;
    let mut prints: HashMap<String, String> = HashMap::with_capacity(raw.len());
    for (phid, text) in raw {
        match text {
            Ok(text) => {
                prints.insert(phid, fingerprint(&text));
            }
            Err(e) => errors.push(local_error(&e)),
        }
    }
    for up in out.values_mut() {
        let Some(phid) = up.diff_phid.as_deref() else { continue };
        if let Some((_, at)) = found.get(phid) {
            up.uploaded = Some(*at);
        }
        if let Some(f) = prints.remove(phid) {
            up.fingerprint = Some(f);
        }
    }
    out
}

fn local_error(e: &crate::http::Error) -> SourceError {
    SourceError {
        source: "phabricator-diffs".into(),
        message: e.summary(),
        unset: false,
        retry_at: e.retry_at(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bug_numbers_come_from_the_subject() {
        assert_eq!(
            bug_in("Bug 2074080 - Give static embeddings their own engineId"),
            Some(BugId(2_074_080))
        );
        assert_eq!(bug_in("WIP: bug 2090291 - idle"), Some(BugId(2_090_291)));
        assert_eq!(bug_in("fixup! Bug 2090291 - idle"), Some(BugId(2_090_291)));
        assert_eq!(bug_in("Bug XXXXXXX - not filed yet"), None);
        assert_eq!(bug_in("Bug TBF-captcha - to be filed"), None);
        assert_eq!(bug_in("No bug - WIP: prototype"), None);
        assert_eq!(bug_in("Debug 123 output"), None);
        assert_eq!(bug_in("Bug 0 - zero"), None);
        assert_eq!(bug_in("Bug 99999999999999999999 - overflow"), None);
    }

    #[test]
    fn the_revision_comes_from_the_last_differential_trailer() {
        let message = "Bug 1 - x\n\nDifferential Revision: https://phabricator.services.mozilla.com/D336313\n";
        assert_eq!(revision_in(message), Some(RevId(336_313)));
        assert_eq!(revision_in("Bug 1 - x\n\nSee D123 for context"), None);
        assert_eq!(
            revision_in("Differential Revision: https://phabricator.services.mozilla.com/D"),
            None
        );
        let two = "Differential Revision: https://phabricator.services.mozilla.com/D1\nDifferential Revision: https://phabricator.services.mozilla.com/D2";
        assert_eq!(revision_in(two), Some(RevId(2)));
    }

    #[test]
    fn worktree_porcelain_yields_detached_heads_only() {
        let porcelain = "worktree /home/me/firefox\nHEAD aaa\nbranch refs/heads/main\n\nworktree /home/me/firefox-D9\nHEAD bbb\ndetached\n\nworktree /home/me/gone\nHEAD ccc\ndetached\nprunable gitdir file points to non-existent location\n";
        assert_eq!(
            detached_worktrees(porcelain),
            [
                ("bbb".to_owned(), "worktree firefox-D9".to_owned()),
                ("ccc".to_owned(), "worktree gone".to_owned())
            ]
        );
    }

    const PHAB: &str = include_str!("../fixtures/phabricator/getrawdiff.json");

    fn phab_diff() -> String {
        crate::phab::parse_raw_diff(PHAB.as_bytes()).unwrap()
    }

    #[test]
    fn a_full_context_patch_and_a_zero_context_one_fingerprint_alike() {
        let local = "diff --git a/toolkit/components/ml/new.txt b/toolkit/components/ml/new.txt\nnew file mode 100644\nindex 0000000..1111111\n--- /dev/null\n+++ b/toolkit/components/ml/new.txt\n@@ -0,0 +1,2 @@\n+-- a line that looks like a header\n+second\ndiff --git a/toolkit/components/ml/a.txt b/toolkit/components/ml/a.txt\nindex 2222222..3333333 100644\n--- a/toolkit/components/ml/a.txt\n+++ b/toolkit/components/ml/a.txt\n@@ -5 +5 @@ four\n-five\n+FIVE\n";
        assert_eq!(fingerprint(local), fingerprint(&phab_diff()));
    }

    #[test]
    fn content_paths_and_signs_all_count() {
        let base = phab_diff();
        for changed in [
            base.replace("+FIVE", "+Five"),
            base.replace("-- a line", "-- another line"),
            base.replace("b/toolkit/components/ml/a.txt", "b/toolkit/components/ml/b.txt")
                .replace("a/toolkit/components/ml/a.txt", "a/toolkit/components/ml/b.txt"),
            base.replace("+second\n", "").replace("+1,2 @@", "+1,1 @@"),
        ] {
            assert_ne!(fingerprint(&changed), fingerprint(&base), "{changed}");
        }
        assert_eq!(
            fingerprint(&base.replace("+FIVE\n", "+FIVE   \r\n")),
            fingerprint(&base),
            "trailing whitespace is ignored"
        );
        assert_eq!(
            fingerprint(&base.replace("+second\n", "+second\n\\ No newline at end of file\n")),
            fingerprint(&base)
        );
    }

    #[test]
    fn a_different_diff_algorithm_ordering_the_same_lines_fingerprints_alike() {
        let myers = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n-a\n-b\n+c\n+d\n";
        let other = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+c\n@@ -2 +2 @@\n-b\n+d\n";
        assert_eq!(fingerprint(myers), fingerprint(other));
        assert_ne!(fingerprint(myers), fingerprint(&myers.replace("+c", "-c").replace("-a", "+a")));
    }

    #[test]
    fn files_and_hunks_without_content_dont_fingerprint() {
        let empty = fingerprint("");
        assert_eq!(
            fingerprint("diff --git a/x.png b/x.png\nBinary files a/x.png and b/x.png differ\n"),
            empty
        );
        assert_eq!(fingerprint("diff --git a/x b/x\nold mode 100644\nnew mode 100755\n"), empty);
        assert_eq!(fingerprint("@@ -1 +1 @@\n-a\n+b\n"), empty, "a hunk outside any file is ignored");
        assert_eq!(fingerprint("garbage\n@@ nonsense\n+++\n---\n"), empty);
    }

    #[test]
    fn show_output_splits_into_one_fingerprint_per_commit() {
        let show = "\u{1e}aaa\n\ndiff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n\u{1e}bbb\n";
        let prints = fingerprints(show);
        assert_eq!(prints.len(), 2);
        assert_ne!(prints["aaa"], prints["bbb"]);
        assert_eq!(prints["bbb"], fingerprint(""));
    }
}
