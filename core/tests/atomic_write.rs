//! Tests for the atomic write seam, at its public interface.

use std::fs;
use tapkey_core::atomic::write_atomically;

mod support;
use support::TempDir;

#[test]
fn writes_the_bytes_to_a_new_file() {
    let dir = TempDir::new("atomic-new");
    let target = dir.path().join("settings.json");

    write_atomically(&target, b"{\"model\":\"opus\"}", 0o600).expect("write");

    assert_eq!(
        fs::read(&target).expect("read back"),
        b"{\"model\":\"opus\"}"
    );
}

#[test]
fn replaces_the_contents_of_an_existing_file() {
    let dir = TempDir::new("atomic-replace");
    let target = dir.path().join("settings.json");
    fs::write(&target, b"old").expect("seed");

    write_atomically(&target, b"new", 0o600).expect("write");

    assert_eq!(fs::read(&target).expect("read back"), b"new");
}

#[cfg(unix)]
#[test]
fn a_new_file_is_created_at_the_requested_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("atomic-mode-new");
    let target = dir.path().join("fresh.json");

    write_atomically(&target, b"{}", 0o600).expect("write");

    let mode = fs::metadata(&target).expect("stat").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "a file tapkey creates may hold a credential");
}

#[cfg(unix)]
#[test]
fn an_existing_file_keeps_the_mode_it_had() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("atomic-mode-kept");
    let target = dir.path().join("theirs.json");
    fs::write(&target, b"old").expect("seed");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).expect("chmod");

    write_atomically(&target, b"new", 0o600).expect("write");

    let mode = fs::metadata(&target).expect("stat").permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o644,
        "tapkey does not own this file and must not tighten it"
    );
}

/// Temp-plus-rename needs permission on the *directory*, not on the file, so a writable file
/// in an unwritable directory is a refusal.
///
/// This test does **not** establish where the temp file was staged: an implementation staging
/// in `/tmp` fails here too, at the rename instead of at the create. That claim is checked by
/// a unit test against `temp_path_beside`, an internal seam, because no portable observation
/// through this interface distinguishes the two.
#[cfg(unix)]
#[test]
fn refuses_when_the_destination_directory_cannot_be_written_even_if_the_file_can() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("atomic-locked-dir");
    let sealed = dir.path().join("sealed");
    fs::create_dir(&sealed).expect("mkdir");
    let target = sealed.join("settings.json");
    fs::write(&target, b"original").expect("seed");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o666)).expect("chmod file");
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o555)).expect("chmod dir");

    let result = write_atomically(&target, b"replacement", 0o600);

    // Restore before asserting, so a failure does not leave an undeletable temp directory.
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o755)).expect("restore dir");
    assert!(
        result.is_err(),
        "temp-plus-rename needs permission on the directory"
    );
    assert_eq!(
        fs::read(&target).expect("read back"),
        b"original",
        "and changed nothing"
    );
}

/// The failure has to land *after* the temp file exists, or the cleanup path is never taken.
/// Renaming onto an existing directory does exactly that: the temp file is created and written,
/// and only then does the rename refuse.
#[test]
fn leaves_no_debris_when_the_write_fails_after_the_temp_file_exists() {
    let dir = TempDir::new("atomic-debris");
    let target = dir.path().join("occupied");
    fs::create_dir(&target).expect("a directory stands where the file should go");

    write_atomically(&target, b"{}", 0o600).expect_err("cannot rename a file onto a directory");

    let strays: Vec<_> = fs::read_dir(dir.path())
        .expect("list")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|n| n != "occupied")
        .collect();
    assert!(strays.is_empty(), "a failed write left {strays:?} behind");
}

/// Dotfile managers make config files symlinks routinely. `rename` over a link replaces the
/// link with a regular file, so the link is resolved first and the target is what gets written.
#[cfg(unix)]
#[test]
fn follows_a_symlink_instead_of_replacing_it() {
    let dir = TempDir::new("atomic-symlink");
    let real = dir.path().join("real.json");
    let link = dir.path().join("settings.json");
    fs::write(&real, b"old").expect("seed");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");

    write_atomically(&link, b"new", 0o600).expect("write");

    assert!(
        fs::symlink_metadata(&link)
            .expect("stat link")
            .file_type()
            .is_symlink(),
        "the link was dissolved into a regular file"
    );
    assert_eq!(
        fs::read(&real).expect("read target"),
        b"new",
        "and the target was written"
    );
}

/// `stow` and `chezmoi` create *relative* links, which the kernel resolves against the
/// directory holding the link — not against the process's working directory.
#[cfg(unix)]
#[test]
fn resolves_a_relative_symlink_against_the_directory_holding_it() {
    let dir = TempDir::new("atomic-relative-symlink");
    let store = dir.path().join("dotfiles");
    fs::create_dir(&store).expect("mkdir");
    let real = store.join("claude-settings.json");
    fs::write(&real, b"old").expect("seed");

    let link = dir.path().join("settings.json");
    std::os::unix::fs::symlink("dotfiles/claude-settings.json", &link).expect("relative symlink");

    write_atomically(&link, b"new", 0o600).expect("write");

    assert!(
        fs::symlink_metadata(&link)
            .expect("stat link")
            .file_type()
            .is_symlink(),
        "the link was dissolved into a regular file"
    );
    assert_eq!(fs::read(&real).expect("read target"), b"new");
}
