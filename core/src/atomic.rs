//! The atomic write seam.
//!
//! One function, with the durability step selected at compile time. Callers know nothing of
//! `F_FULLFSYNC` or step order; that is what the seam is for.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static TEMP_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Write `bytes` to `path` so that a reader sees either the old contents or the new ones,
/// never a partial file, and so that the new contents survive a power loss.
///
/// `mode` is the Unix permission bits to give the file **if it does not already exist**. An
/// existing file keeps the mode it had: tapkey does not own the tools' files, and tightening
/// what it does not own is a surprise rather than a service.
pub fn write_atomically(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    // Dotfile managers make config files symlinks routinely, and `rename` over a link replaces
    // the link itself with a regular file. So the link is resolved and its target is written —
    // which also keeps the temp file on the target's filesystem rather than the link's.
    let resolved = resolve_links(path)?;
    let path = resolved.as_path();

    let dir = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "a target must have a parent directory",
        )
    })?;

    // The temporary file shares a directory with the target, or `rename` crosses a filesystem
    // boundary and stops being atomic — which is why `/tmp` is wrong however convenient.
    let temp = temp_path_beside(path);
    let create_mode = existing_mode(path).unwrap_or(mode);

    let outcome = write_then_rename(&temp, path, dir, bytes, create_mode);
    if outcome.is_err() {
        // Leave nothing behind: a failed write must not turn into a directory of debris.
        let _ = fs::remove_file(&temp);
    }
    outcome
}

fn write_then_rename(
    temp: &Path,
    target: &Path,
    dir: &Path,
    bytes: &[u8],
    create_mode: u32,
) -> io::Result<()> {
    let mut file = create_with_mode(temp, create_mode)?;
    file.write_all(bytes)?;
    flush_to_medium(&file)?;
    drop(file);

    fs::rename(temp, target)?;

    // With the data durable but the directory entry still cached, a power loss returns the
    // file to its previous contents having faithfully preserved bytes nobody will read.
    flush_directory(dir)
}

/// Follow a chain of symlinks to the path that will actually be written. A relative link is
/// resolved against the directory holding the link, as the kernel does. A dangling link
/// resolves to where it points, so writing through it creates that file rather than replacing
/// the link with one.
fn resolve_links(path: &Path) -> io::Result<PathBuf> {
    const MAX_HOPS: usize = 32;
    let mut current = path.to_path_buf();
    for _ in 0..MAX_HOPS {
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let target = fs::read_link(&current)?;
                current = if target.is_absolute() {
                    target
                } else {
                    current.parent().unwrap_or(Path::new("")).join(target)
                };
            }
            // Not a link, or nothing there yet: this is the path to write.
            _ => return Ok(current),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "symlink chain is too deep to follow",
    ))
}

fn temp_path_beside(target: &Path) -> PathBuf {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("write");
    target.with_file_name(format!(".{name}.tapkey-{}-{n}.tmp", std::process::id()))
}

#[cfg(unix)]
fn existing_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .ok()
        .map(|m| m.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn existing_mode(_path: &Path) -> Option<u32> {
    None
}

#[cfg(unix)]
fn create_with_mode(path: &Path, mode: u32) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    // The mode is set at creation rather than afterwards: a chmod after the fact leaves a
    // window in which the file exists with the wrong permissions.
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
}

#[cfg(not(unix))]
fn create_with_mode(path: &Path, _mode: u32) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// Darwin's `fsync` returns once the data reaches the drive's cache rather than the platter,
/// so it is not enough here; `F_FULLFSYNC` is.
#[cfg(target_os = "macos")]
fn flush_to_medium(file: &File) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    // SAFETY: the descriptor is owned by `file` and outlives the call.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Elsewhere the standard library already issues the right call: `fdatasync` on Linux,
/// `FlushFileBuffers` on Windows.
#[cfg(not(target_os = "macos"))]
fn flush_to_medium(file: &File) -> io::Result<()> {
    file.sync_data()
}

#[cfg(unix)]
fn flush_directory(dir: &Path) -> io::Result<()> {
    // `F_FULLFSYNC` addresses the medium behind a file's data and means nothing on a
    // directory, so a plain fsync is what a directory entry gets, on Darwin too.
    File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn flush_directory(_dir: &Path) -> io::Result<()> {
    // Windows offers no handle to a directory's entry that can be flushed this way. The gap
    // is named rather than hidden; it is revisited with the Windows platform seam.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An internal seam. Where the temp file goes cannot be observed portably through
    /// `write_atomically`, and it is the one property that makes the rename atomic, so it is
    /// checked here rather than assumed.
    #[test]
    fn the_temp_file_is_a_sibling_of_the_target() {
        let target = Path::new("/some/dir/settings.json");
        let temp = temp_path_beside(target);

        assert_eq!(
            temp.parent(),
            target.parent(),
            "a rename across filesystems is not atomic"
        );
        assert_ne!(temp.file_name(), target.file_name());
    }
}
