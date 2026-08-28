//! The backup store on disk.

use std::time::{Duration, UNIX_EPOCH};
use tapkey_core::store::{Captured, RestoreAction, Store, Target};

mod support;
use support::TempDir;

fn at(ms: u64) -> std::time::SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn captured(path: &std::path::Path, content: &str) -> Captured {
    Captured {
        path: path.to_path_buf(),
        tool: "claude".into(),
        content: Some(content.as_bytes().to_vec()),
        mode: Some(0o644),
    }
}

#[test]
fn a_backup_holds_whole_copies_and_can_be_listed() {
    let dir = TempDir::new("store-backup");
    let store = Store::open(&dir.path().join("tapkey")).expect("open");
    let target = dir.path().join("settings.json");

    let id = store
        .take_backup(
            &[captured(&target, "original")],
            "zai",
            at(1_787_866_640_123),
        )
        .expect("backup");

    let listed = store.backups().expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);
    assert_eq!(listed[0].profile, "zai");
    assert_eq!(
        id.as_str(),
        "20260827T213720.123Z",
        "named by the instant it was taken"
    );
}

#[test]
fn restoring_returns_the_original_bytes_rather_than_writing_them() {
    let dir = TempDir::new("store-restore");
    let store = Store::open(&dir.path().join("tapkey")).expect("open");
    let target = dir.path().join("settings.json");
    let id = store
        .take_backup(&[captured(&target, "original")], "zai", at(1))
        .expect("backup");

    let plan = store.restore_plan(Target::Backup(id)).expect("plan");

    match &plan[..] {
        [RestoreAction::Write { path, bytes, mode }] => {
            assert_eq!(path, &target);
            assert_eq!(bytes, b"original");
            assert_eq!(
                *mode,
                Some(0o644),
                "the file's own mode comes back, not ours"
            );
        }
        other => panic!("unexpected plan: {other:?}"),
    }
    assert!(
        !target.exists(),
        "the store plans; the transactional writer acts"
    );
}

/// Back-fill can create a config that was never there, so without an absence marker returning
/// you to how it was is false from the first switch on a clean machine.
#[test]
fn a_file_that_did_not_exist_is_recorded_absent_and_restoring_deletes_it() {
    let dir = TempDir::new("store-absent");
    let store = Store::open(&dir.path().join("tapkey")).expect("open");
    let target = dir.path().join("settings.json");
    let id = store
        .take_backup(
            &[Captured {
                path: target.clone(),
                tool: "claude".into(),
                content: None,
                mode: None,
            }],
            "zai",
            at(1),
        )
        .expect("backup");

    let plan = store.restore_plan(Target::Backup(id)).expect("plan");

    assert!(matches!(&plan[..], [RestoreAction::Delete { path }] if path == &target));
}

/// The manifest is written last and is the commit point, so a directory without one is an
/// interrupted write: our own garbage, not the user's state.
#[test]
fn a_backup_without_a_manifest_is_swept_in_silence() {
    let dir = TempDir::new("store-torn");
    let root = dir.path().join("tapkey");
    let store = Store::open(&root).expect("open");
    store
        .take_backup(&[captured(&dir.path().join("a.json"), "x")], "zai", at(1))
        .expect("backup");
    let torn = root.join("backups").join("20990101T000000.000Z");
    std::fs::create_dir_all(torn.join("files")).expect("torn dir");
    std::fs::write(torn.join("files").join("01"), b"half a write").expect("torn file");

    let listed = store.backups().expect("list");

    assert_eq!(listed.len(), 1, "the torn one is not listed");
    assert!(!torn.exists(), "and it is gone");
}

/// Erasing somebody's only way back because we failed to parse it is worse than any amount of
/// clutter. It stays, it is listed, and it says it cannot be restored.
#[test]
fn a_backup_we_cannot_read_is_kept_and_marked_rather_than_deleted() {
    let dir = TempDir::new("store-unreadable");
    let root = dir.path().join("tapkey");
    let store = Store::open(&root).expect("open");
    let broken = root.join("backups").join("20250101T000000.000Z");
    std::fs::create_dir_all(&broken).expect("mkdir");
    std::fs::write(broken.join("manifest.json"), b"{ this is not json").expect("write");

    let listed = store.backups().expect("list");

    assert_eq!(listed.len(), 1);
    assert!(!listed[0].restorable, "listed, and honestly");
    assert!(broken.exists(), "and never deleted for being unreadable");
}

#[test]
fn restoring_an_unreadable_backup_refuses_rather_than_half_doing_it() {
    let dir = TempDir::new("store-refuse");
    let root = dir.path().join("tapkey");
    let store = Store::open(&root).expect("open");
    let broken = root.join("backups").join("20250101T000000.000Z");
    std::fs::create_dir_all(&broken).expect("mkdir");
    std::fs::write(broken.join("manifest.json"), b"nonsense").expect("write");

    assert!(
        store
            .restore_plan(Target::Backup("20250101T000000.000Z".into()))
            .is_err()
    );
}

/// Different retention rules mean different places: the sweep does not see the snapshot at
/// all, rather than being taught to skip it.
#[test]
fn the_snapshot_lives_apart_and_the_sweep_cannot_reach_it() {
    let dir = TempDir::new("store-snapshot");
    let root = dir.path().join("tapkey");
    let store = Store::open(&root).expect("open");
    let target = dir.path().join("settings.json");
    store
        .take_snapshot(&[captured(&target, "as found")], at(1))
        .expect("snapshot");
    for i in 0..60u64 {
        store
            .take_backup(&[captured(&target, "x")], "zai", at(1000 + i * 1000))
            .expect("backup");
    }

    store.sweep(3, 50 * 1024 * 1024).expect("sweep");

    assert!(
        store.has_snapshot(),
        "the snapshot is the floor under every restore"
    );
    assert_eq!(store.backups().expect("list").len(), 3);
    let plan = store.restore_plan(Target::Snapshot).expect("plan");
    assert!(matches!(&plan[..], [RestoreAction::Write { bytes, .. }] if bytes == b"as found"));
}

#[test]
fn the_sweep_removes_the_oldest_first() {
    let dir = TempDir::new("store-sweep-order");
    let root = dir.path().join("tapkey");
    let store = Store::open(&root).expect("open");
    let target = dir.path().join("settings.json");
    for i in 0..5u64 {
        store
            .take_backup(
                &[captured(&target, "x")],
                &format!("p{i}"),
                at(1000 + i * 1000),
            )
            .expect("backup");
    }

    store.sweep(2, 50 * 1024 * 1024).expect("sweep");

    let kept: Vec<String> = store
        .backups()
        .expect("list")
        .into_iter()
        .map(|b| b.profile)
        .collect();
    assert_eq!(kept, vec!["p3", "p4"], "newest kept, oldest gone");
}

/// A byte ceiling as well as a count: fifty guards against a long history, fifty megabytes
/// against one enormous config, and each alone misses what the other catches.
#[test]
fn the_byte_ceiling_binds_as_well_as_the_count() {
    let dir = TempDir::new("store-sweep-bytes");
    let root = dir.path().join("tapkey");
    let store = Store::open(&root).expect("open");
    let target = dir.path().join("settings.json");
    let big = "x".repeat(4096);
    for i in 0..5u64 {
        store
            .take_backup(
                &[Captured {
                    path: target.clone(),
                    tool: "claude".into(),
                    content: Some(big.as_bytes().to_vec()),
                    mode: Some(0o644),
                }],
                &format!("p{i}"),
                at(1000 + i * 1000),
            )
            .expect("backup");
    }

    store.sweep(50, 9000).expect("sweep");

    assert!(
        store.backups().expect("list").len() <= 2,
        "the byte ceiling bound first"
    );
}

#[cfg(unix)]
#[test]
fn the_store_is_ours_so_it_is_created_closed() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("store-modes");
    let root = dir.path().join("tapkey");
    let store = Store::open(&root).expect("open");
    let target = dir.path().join("settings.json");
    let id = store
        .take_backup(&[captured(&target, "holds a key perhaps")], "zai", at(1))
        .expect("backup");

    let mode =
        |p: &std::path::Path| std::fs::metadata(p).expect("stat").permissions().mode() & 0o777;
    assert_eq!(mode(&root), 0o700);
    let copy = root
        .join("backups")
        .join(id.as_str())
        .join("files")
        .join("01");
    assert_eq!(
        mode(&copy),
        0o600,
        "a copy inside the store may contain a key"
    );
}

/// The one state where the restore promise is broken has to be distinguishable from the
/// ordinary first run, and the distinction is exactly this: backups present, snapshot absent.
#[test]
fn a_missing_snapshot_is_distinguishable_from_a_first_run() {
    let dir = TempDir::new("store-no-snapshot");
    let store = Store::open(&dir.path().join("tapkey")).expect("open");

    assert!(!store.has_snapshot(), "nothing has happened yet");
    assert!(store.backups().expect("list").is_empty());

    store
        .take_backup(&[captured(&dir.path().join("a.json"), "x")], "zai", at(1))
        .expect("backup");

    assert!(
        !store.has_snapshot(),
        "backups but no snapshot: the promise is not kept"
    );
    assert_eq!(store.backups().expect("list").len(), 1);

    store
        .take_snapshot(&[captured(&dir.path().join("a.json"), "as found")], at(2))
        .expect("snapshot");

    assert!(store.has_snapshot());
}

/// A manifest written by a newer build is kept and marked, never reinterpreted by guesswork —
/// the same rule as one we cannot parse at all.
#[test]
fn a_manifest_from_the_future_is_unrestorable_rather_than_guessed_at() {
    let dir = TempDir::new("store-future");
    let root = dir.path().join("tapkey");
    let store = Store::open(&root).expect("open");
    let future = root.join("backups").join("20990101T000000.000Z");
    std::fs::create_dir_all(&future).expect("mkdir");
    std::fs::write(
        future.join("manifest.json"),
        br#"{"version": 999, "instant": "20990101T000000.000Z", "profile": "later", "files": []}"#,
    )
    .expect("write");

    let listed = store.backups().expect("list");

    assert_eq!(listed.len(), 1);
    assert!(!listed[0].restorable);
    assert!(
        store
            .restore_plan(Target::Backup("20990101T000000.000Z".into()))
            .is_err()
    );
    assert!(
        future.exists(),
        "kept, because we do not delete what we cannot read"
    );
}
