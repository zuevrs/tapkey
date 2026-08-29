//! The bridge's own logic, at its seam: the helper refresh.
//!
//! Everything else on this side of the bridge is either the core (227 tests) or Tauri. What is
//! ours is the startup swap.

use std::path::PathBuf;
use tapkey_app_lib::refresh_helper_from;

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tapkey-app-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn bundled_helper() -> PathBuf {
    let mut path = std::env::current_exe().expect("test binary location");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join(format!("tapkey-helper{}", std::env::consts::EXE_SUFFIX))
}

/// The whole swap, against the real helper binary cargo just built.
#[test]
fn a_stored_helper_is_swapped_to_the_bundles_content() {
    let bin = scratch("swap");
    let stored = bin.join(format!("tapkey-helper{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&stored, b"stale").expect("seed a stale helper");

    refresh_helper_from(&bundled_helper(), &bin);

    let want = std::fs::read(bundled_helper()).expect("read");
    assert_eq!(std::fs::read(&stored).expect("read"), want, "swapped");
    assert!(
        !bin.join("tapkey-helper.old").exists(),
        "the old copy is cleaned"
    );
    let _ = std::fs::remove_dir_all(&bin);
}

/// Identical content stands down. A running tool's helper must not be renamed out from under it
/// for no reason — this is the whole point of comparing by hash.
#[test]
fn an_up_to_date_helper_is_not_touched() {
    let bin = scratch("standdown");
    let stored = bin.join(format!("tapkey-helper{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(bundled_helper(), &stored).expect("seed an up-to-date helper");
    let mtime = std::fs::metadata(&stored)
        .expect("meta")
        .modified()
        .unwrap();

    std::thread::sleep(std::time::Duration::from_millis(50));
    refresh_helper_from(&bundled_helper(), &bin);

    assert_eq!(
        std::fs::metadata(&stored)
            .expect("meta")
            .modified()
            .unwrap(),
        mtime,
        "identical content must not rewrite the file"
    );
}
