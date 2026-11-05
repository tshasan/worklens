use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use serde::Deserialize;
use worklens_core::marks::Marks;
use worklens_core::{RAW_VERSION, Raw};

pub fn cache_file() -> Option<PathBuf> {
    std::env::home_dir().map(|home| home.join(".cache/worklens/state.json"))
}

pub fn marks_file() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|h| h.join(".local/state")))?;
    Some(state.join("worklens/marks.json"))
}

pub fn drift_file() -> Option<PathBuf> {
    marks_file().map(|m| m.with_file_name("drift.json"))
}

pub fn read_drift_log(path: &Path) -> worklens_core::drift_log::DriftLog {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn record_drift(path: &Path, day: worklens_core::drift_log::DriftDay) -> io::Result<()> {
    with_lock(path, || {
        let mut log = read_drift_log(path);
        log.record(day.date, day.mine, day.team);
        write_atomic(path, |w| serde_json::to_writer(w, &log).map_err(io::Error::other))
    })
}

pub fn read_marks(path: &Path) -> Result<Marks, ReadError> {
    match fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Marks::default()),
        Err(e) => Err(e.into()),
    }
}

pub fn write_marks(path: &Path, marks: &Marks) -> io::Result<()> {
    write_atomic(path, |w| serde_json::to_writer(w, marks).map_err(io::Error::other))
}

pub fn save_marks(path: &Path, base: &Marks, mine: &Marks) -> io::Result<()> {
    with_lock(path, || {
        let mut current = read_marks(path).unwrap_or_default();
        for (key, item) in &mine.items {
            if base.items.get(key) != Some(item) {
                current.items.insert(key.clone(), item.clone());
            }
        }
        for key in base.items.keys() {
            if !mine.items.contains_key(key) {
                current.items.remove(key);
            }
        }
        current.last_look = current.last_look.max(mine.last_look);
        write_marks(path, &current)
    })
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("no cache yet at {0}")]
    Missing(PathBuf),
    #[error("cache is version {found}, this build reads version {RAW_VERSION}")]
    Version { found: u32 },
    #[error("cache doesn't parse: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("can't read the cache: {0}")]
    Io(#[from] io::Error),
}

impl ReadError {
    pub const fn refetch_fixes(&self) -> bool {
        matches!(self, Self::Missing(_) | Self::Version { .. } | Self::Parse(_))
    }
}

pub fn read_raw(path: &Path) -> Result<Raw, ReadError> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(ReadError::Missing(path.to_path_buf())),
        Err(e) => return Err(e.into()),
    };
    match serde_json::from_slice::<Raw>(&bytes) {
        Ok(raw) if raw.version == RAW_VERSION => Ok(raw),
        Ok(raw) => Err(ReadError::Version { found: raw.version }),
        Err(parse) => {
            #[derive(Deserialize)]
            struct VersionOnly {
                version: u32,
            }
            match serde_json::from_slice::<VersionOnly>(&bytes) {
                Ok(v) if v.version != RAW_VERSION => Err(ReadError::Version { found: v.version }),
                _ => Err(parse.into()),
            }
        }
    }
}

pub fn write_raw(path: &Path, raw: &Raw) -> io::Result<()> {
    write_atomic(path, |w| serde_json::to_writer(w, raw).map_err(io::Error::other))
}

pub fn write_atomic(path: &Path, write: impl FnOnce(&mut BufWriter<File>) -> io::Result<()>) -> io::Result<()> {
    let dir = parent_dir(path);
    if let Some(dir) = dir {
        create_private_dir(dir)?;
    }
    let tmp = sibling(path, &format!(".{}.{}.tmp", std::process::id(), next_temp_number()));
    if let Err(e) = write_and_rename(&tmp, path, write) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if let Some(dir) = dir {
        sync_dir(dir)?;
    }
    Ok(())
}

fn write_and_rename(tmp: &Path, path: &Path, write: impl FnOnce(&mut BufWriter<File>) -> io::Result<()>) -> io::Result<()> {
    let mut out = BufWriter::new(create_private_file(tmp)?);
    write(&mut out)?;
    out.into_inner().map_err(io::IntoInnerError::into_error)?.sync_all()?;
    fs::rename(tmp, path)
}

fn next_temp_number() -> u32 {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn parent_dir(path: &Path) -> Option<&Path> {
    path.parent().filter(|dir| !dir.as_os_str().is_empty())
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map(std::ffi::OsStr::to_os_string).unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn create_private_file(path: &Path) -> io::Result<File> {
    private_options().create_new(true).open(path)
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

pub fn with_lock<T>(path: &Path, f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    if let Some(dir) = parent_dir(path) {
        create_private_dir(dir)?;
    }
    let lock = private_options().create(true).truncate(false).open(sibling(path, ".lock"))?;
    lock.lock()?;
    let result = f();
    let unlocked = lock.unlock();
    let value = result?;
    unlocked?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use worklens_core::raw::{RevStatus, ReviewerStatus, TryResult};
    use worklens_core::{BugId, JiraKey, RevId};

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/raw.json");

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("worklens-store-{name}-{}", std::process::id()))
    }

    #[test]
    fn replaces_the_file() -> io::Result<()> {
        let dir = temp_dir("atomic");
        let path = dir.join("state.json");
        write_atomic(&path, |w| w.write_all(b"old"))?;
        write_atomic(&path, |w| w.write_all(b"new"))?;
        assert_eq!(fs::read(&path)?, b"new");
        assert_eq!(temp_files(&dir), Vec::<String>::new());
        fs::remove_dir_all(dir)
    }

    fn temp_files(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn a_failing_writer_leaves_the_old_file_and_no_temp_file() {
        let dir = temp_dir("failing");
        let path = dir.join("state.json");
        write_atomic(&path, |w| w.write_all(b"old")).unwrap();
        let err = write_atomic(&path, |w| {
            w.write_all(b"half")?;
            Err(io::Error::other("writer broke"))
        })
        .unwrap_err();
        assert_eq!(err.to_string(), "writer broke");
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(temp_files(&dir), Vec::<String>::new());
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn files_are_owner_only_and_a_new_dir_is_too() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let root = temp_dir("modes");
        let dir = root.join("worklens");
        let path = dir.join("state.json");
        write_atomic(&path, |w| w.write_all(b"secret")).unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&dir), 0o700);

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        write_atomic(&path, |w| w.write_all(b"secret")).unwrap();
        assert_eq!(mode(&path), 0o600);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_writers_never_interleave() {
        let dir = temp_dir("concurrent");
        let path = dir.join("state.json");
        let a = vec![b'a'; 200_000];
        let b = vec![b'b'; 200_000];
        std::thread::scope(|s| {
            for bytes in [&a, &b] {
                let path = &path;
                s.spawn(move || {
                    for _ in 0..10 {
                        write_atomic(path, |w| w.write_all(bytes)).unwrap();
                    }
                });
            }
        });
        let written = fs::read(&path).unwrap();
        assert!(written == a || written == b, "the file mixes two writes");
        assert_eq!(temp_files(&dir), Vec::<String>::new());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn with_lock_serializes_read_modify_write() {
        let dir = temp_dir("lock");
        let path = dir.join("count");
        write_atomic(&path, |w| w.write_all(b"0")).unwrap();
        let increment = || {
            with_lock(&path, || {
                let n: u32 = fs::read_to_string(&path)?.parse().map_err(io::Error::other)?;
                std::thread::yield_now();
                write_atomic(&path, |w| write!(w, "{}", n + 1))
            })
            .unwrap();
        };
        std::thread::scope(|s| {
            for _ in 0..2 {
                s.spawn(|| (0..50).for_each(|_| increment()));
            }
        });
        assert_eq!(fs::read_to_string(&path).unwrap(), "100");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_a_cache_written_by_the_typescript_version() {
        let raw = read_raw(Path::new(FIXTURE)).unwrap();
        assert_eq!(raw.me.email, "me@mozilla.com");
        assert_eq!(raw.bugs[0].id, BugId(1_995_008));
        assert_eq!(raw.revisions[0].status, RevStatus::NeedsReview);
        assert_eq!(raw.revisions[0].reviewers[0].status, ReviewerStatus::Blocking);
        assert_eq!(raw.issues[&JiraKey("AIPLAT-1416".into())].points, Some(3.0));
        assert_eq!(raw.try_pushes.as_ref().unwrap()[&RevId(321_987)].tests, TryResult::Fail);
        assert_eq!(raw.bug_revisions[&BugId(1_995_008)], [RevId(321_987)]);
    }

    #[test]
    fn round_trips() {
        let path = temp_dir("roundtrip").join("state.json");
        let raw = read_raw(Path::new(FIXTURE)).unwrap();
        write_raw(&path, &raw).unwrap();
        let again = read_raw(&path).unwrap();
        assert_eq!(again.bugs.len(), raw.bugs.len());
        assert_eq!(again.fetched_at, raw.fetched_at);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn says_why_an_old_cache_is_unreadable() {
        let path = temp_dir("version").join("state.json");
        write_atomic(&path, |w| w.write_all(br#"{"version": 0, "bugs": "not a list"}"#)).unwrap();
        let err = read_raw(&path).unwrap_err();
        assert!(matches!(err, ReadError::Version { found: 0 }), "{err}");
        assert!(err.refetch_fixes());
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn marks_round_trip_and_a_missing_file_is_empty() {
        let path = temp_dir("marks").join("marks.json");
        assert!(read_marks(&path).unwrap().items.is_empty());
        let mut marks = Marks::default();
        marks.items.entry("D1".to_owned()).or_default();
        write_marks(&path, &marks).unwrap();
        assert!(read_marks(&path).unwrap().items.contains_key("D1"));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn missing_cache_is_not_an_io_error() {
        let err = read_raw(Path::new("/nonexistent/state.json")).unwrap_err();
        assert!(matches!(err, ReadError::Missing(_)));
    }

    #[test]
    #[ignore = "reads ~/.cache/worklens/state.json"]
    fn reads_the_real_cache() {
        let raw = read_raw(&cache_file().unwrap()).unwrap();
        assert!(!raw.bugs.is_empty());
    }

    #[test]
    fn saving_marks_keeps_what_another_process_changed() {
        use worklens_core::marks::{ItemMarks, Mark};
        let dir = temp_dir("merge-marks");
        let path = dir.join("marks.json");
        let mark = |sig: &str| ItemMarks {
            done: Some(Mark {
                at: "2026-11-03T12:00:00Z".parse().unwrap(),
                sig: sig.into(),
            }),
            ..ItemMarks::default()
        };
        let mut start = Marks::default();
        start.items.insert("1".into(), mark("a"));
        write_marks(&path, &start).unwrap();
        let mut cli = start.clone();
        cli.items.insert("2".into(), mark("b"));
        save_marks(&path, &start, &cli).unwrap();
        let mut ui = start.clone();
        ui.items.remove("1");
        ui.items.insert("3".into(), mark("c"));
        save_marks(&path, &start, &ui).unwrap();
        let mut keys: Vec<String> = read_marks(&path).unwrap().items.into_keys().collect();
        keys.sort();
        assert_eq!(keys, ["2", "3"], "the CLI's 2 survives; the UI's restore of 1 and its 3 apply");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn drift_days_accumulate_in_the_file() -> io::Result<()> {
        use worklens_core::drift_log::DriftDay;
        let dir = temp_dir("drift");
        let path = dir.join("drift.json");
        assert!(read_drift_log(&path).days.is_empty());
        let day = |d: &str, mine| DriftDay {
            date: d.parse().unwrap(),
            mine,
            team: 0,
        };
        record_drift(&path, day("2026-11-01", 6))?;
        record_drift(&path, day("2026-11-03", 4))?;
        record_drift(&path, day("2026-11-03", 3))?;
        let counts: Vec<usize> = read_drift_log(&path).days.iter().map(|d| d.mine).collect();
        assert_eq!(counts, vec![6, 3], "one point per day, the latest that day");
        fs::remove_dir_all(dir)
    }
}
