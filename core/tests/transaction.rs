//! The all-or-nothing write. Half-applied is the worst outcome available: the person believes
//! they moved, and one tool is still billing the old provider.

use std::path::Path;
use tapkey_core::fs::{FailOnce, FileSystem, RealFs};
use tapkey_core::transaction::{Action, Transaction};

mod support;
use support::TempDir;

fn write(path: &Path, bytes: &str) -> Action {
    Action::Write {
        path: path.to_path_buf(),
        bytes: bytes.as_bytes().to_vec(),
        mode: 0o600,
    }
}

#[test]
fn every_write_lands_when_nothing_fails() {
    let dir = TempDir::new("tx-happy");
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    std::fs::write(&a, "old a").expect("seed");
    let mut fs = RealFs;

    Transaction::new(vec![write(&a, "new a"), write(&b, "new b")])
        .apply(&mut fs)
        .expect("apply");

    assert_eq!(std::fs::read(&a).expect("a"), b"new a");
    assert_eq!(std::fs::read(&b).expect("b"), b"new b");
}

/// The guarantee ADR-0005 actually makes, and the only way to prove it: fail midway through
/// three files and check the earlier ones came back byte for byte.
#[test]
fn a_failure_on_the_second_of_three_files_puts_the_first_one_back() {
    let dir = TempDir::new("tx-rollback");
    let (a, b, c) = (
        dir.path().join("a"),
        dir.path().join("b"),
        dir.path().join("c"),
    );
    std::fs::write(&a, "original a").expect("seed");
    std::fs::write(&b, "original b").expect("seed");
    std::fs::write(&c, "original c").expect("seed");
    let mut fs = FailOnce::after(1);

    let rollback = Transaction::new(vec![
        write(&a, "new a"),
        write(&b, "new b"),
        write(&c, "new c"),
    ])
    .apply(&mut fs)
    .expect_err("the second write fails");

    assert_eq!(std::fs::read(&a).expect("a"), b"original a", "restored");
    assert_eq!(
        std::fs::read(&b).expect("b"),
        b"original b",
        "never changed"
    );
    assert_eq!(
        std::fs::read(&c).expect("c"),
        b"original c",
        "never reached"
    );
    assert_eq!(
        rollback.failed_at, b,
        "the report names the file that stopped it"
    );
    assert_eq!(rollback.restored, 1);
}

/// A file the switch created has no earlier contents to put back, so rolling back means
/// removing it. Leaving it would be a file the user never had and tapkey no longer tracks.
#[test]
fn rolling_back_removes_a_file_the_transaction_created() {
    let dir = TempDir::new("tx-created");
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    let mut fs = FailOnce::after(1);

    Transaction::new(vec![write(&a, "new a"), write(&b, "new b")])
        .apply(&mut fs)
        .expect_err("the second write fails");

    assert!(
        !a.exists(),
        "it did not exist before, so it must not exist after"
    );
}

#[test]
fn a_delete_is_undone_by_putting_the_file_back() {
    let dir = TempDir::new("tx-delete");
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    std::fs::write(&a, "original a").expect("seed");
    let mut fs = FailOnce::after(1);

    Transaction::new(vec![Action::Delete { path: a.clone() }, write(&b, "new b")])
        .apply(&mut fs)
        .expect_err("the second action fails");

    assert_eq!(std::fs::read(&a).expect("a"), b"original a");
}

/// The capture a rollback needs and the copy a backup needs are the same reading of the same
/// files, so the transaction produces it once.
#[test]
fn capturing_records_contents_and_absence_alike() {
    let dir = TempDir::new("tx-capture");
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    std::fs::write(&a, "original a").expect("seed");
    let fs = RealFs;

    let captured = Transaction::new(vec![write(&a, "new a"), write(&b, "new b")])
        .capture(&fs, "claude")
        .expect("capture");

    assert_eq!(captured.len(), 2);
    assert_eq!(captured[0].content.as_deref(), Some(&b"original a"[..]));
    assert_eq!(captured[1].content, None, "b did not exist");
}

/// The other half of the story: when the filesystem is genuinely broken rather than one file
/// refusing, the rollback cannot put everything back — and the count says so rather than the
/// report claiming a clean undo.
#[test]
fn a_rollback_that_cannot_finish_reports_how_far_it_got() {
    struct Dead;
    impl FileSystem for Dead {
        fn write(&mut self, _p: &std::path::Path, _b: &[u8], _m: u32) -> std::io::Result<()> {
            Err(std::io::Error::other("the volume went away"))
        }
        fn remove(&mut self, _p: &std::path::Path) -> std::io::Result<()> {
            Err(std::io::Error::other("the volume went away"))
        }
        fn read(&self, _p: &std::path::Path) -> std::io::Result<Option<Vec<u8>>> {
            Ok(Some(b"original".to_vec()))
        }
        fn mode(&self, _p: &std::path::Path) -> Option<u32> {
            Some(0o600)
        }
    }

    let dir = TempDir::new("tx-dead");
    let rollback = Transaction::new(vec![write(&dir.path().join("a"), "new a")])
        .apply(&mut Dead)
        .expect_err("nothing can be written");

    assert_eq!(rollback.restored, 0, "an honest zero beats a claimed undo");
}

/// A rollback that has to *recreate* a file must give it back the mode it had. Overwriting an
/// existing file cannot show this, because the atomic write keeps whatever mode is already
/// there — so the captured mode is dead weight on that path and decisive on this one. A
/// mutation run made the difference visible: replacing the mode reader with a constant left
/// every overwrite-shaped test passing.
#[cfg(unix)]
#[test]
fn rolling_back_a_delete_recreates_the_file_with_the_mode_it_had() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("tx-mode");
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    std::fs::write(&a, "original a").expect("seed");
    std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o644)).expect("chmod");

    Transaction::new(vec![Action::Delete { path: a.clone() }, write(&b, "new b")])
        .apply(&mut FailOnce::after(1))
        .expect_err("the second action fails");

    let mode = std::fs::metadata(&a).expect("stat").permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o644,
        "0o600 here would be tapkey tightening a file it does not own"
    );
    assert_eq!(std::fs::read(&a).expect("a"), b"original a");
}

/// Only a missing file is a non-error. Anything else has to travel, or a switch would read a
/// directory as an empty file and cheerfully back up nothing.
#[test]
fn a_read_that_fails_for_any_other_reason_is_still_an_error() {
    let fs = RealFs;
    let dir = TempDir::new("tx-read-error");
    assert!(
        fs.read(dir.path()).is_err(),
        "a directory is not an absent file"
    );
}

/// Undoing a create means removing a file, and the same undo runs over entries whose file the
/// failure never reached. Treating "already gone" as an error would abandon the rest of them.
#[test]
fn removing_a_file_that_is_not_there_is_not_a_failure() {
    let dir = TempDir::new("tx-remove-missing");
    let mut fs = RealFs;
    assert!(fs.remove(&dir.path().join("never-existed")).is_ok());
}

/// And the same asymmetry on the way out: absent is fine, anything else must travel. A removal
/// that swallowed every error would report a rollback it had not performed.
#[test]
fn a_removal_that_fails_for_any_other_reason_is_still_an_error() {
    let dir = TempDir::new("tx-remove-error");
    let mut fs = RealFs;
    assert!(
        fs.remove(dir.path()).is_err(),
        "a directory is not a file that was already gone"
    );
}
