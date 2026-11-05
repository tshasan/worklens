use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use proptest::prelude::*;
use worklens_core::RAW_VERSION;
use worklens_core::raw::Raw;
use worklens_store::{read_marks, read_raw, write_raw};
use worklens_testkit::{cache, mutated, mutations};

fn scratch() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "worklens-robust-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("cache.json")
}

proptest! {
    #![proptest_config(worklens_testkit::config(256))]

    #[test]
    fn corrupt_files_read_as_errors(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        let path = scratch();
        worklens_store::write_atomic(&path, |w| std::io::Write::write_all(w, &bytes)).unwrap();
        let _ = read_raw(&path);
        let _ = read_marks(&path);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn every_readable_cache_survives_a_write(seed in cache(), ms in mutations(12)) {
        let Ok(raw) = serde_json::from_value::<Raw>(mutated(seed, &ms)) else { return Ok(()) };
        if raw.version != RAW_VERSION {
            return Ok(());
        }
        let before = serde_json::to_value(&raw).unwrap();
        let path = scratch();
        write_raw(&path, &raw).unwrap();
        let again = read_raw(&path);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        let again = again.map_err(|e| TestCaseError::fail(format!("written cache does not read back: {e}")))?;
        prop_assert_eq!(before, serde_json::to_value(&again).unwrap());
    }
}
