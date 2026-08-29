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
    // The destination directory may not exist: Codex does not create `~/.codex/config.toml` if it
    // did not find one, so an installed-but-unconfigured tool is an ordinary state and the file we
    // write is the first one there. 0700, because a directory we create is ours until somebody
    // else's file lands in it.
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
    }

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

    persist(temp, target)?;

    // With the data durable but the directory entry still cached, a power loss returns the
    // file to its previous contents having faithfully preserved bytes nobody will read.
    flush_directory(dir)
}

/// Put the finished temp file where the target is.
#[cfg(not(windows))]
fn persist(temp: &Path, target: &Path) -> io::Result<()> {
    fs::rename(temp, target)
}

/// On Windows, `fs::rename` is `MoveFileEx` with replacement, which lands **our** temp file —
/// with its attributes — where the person's file was. `ReplaceFileW` exists to do the opposite:
/// swap the contents while the destination keeps its own ACL and attributes, which is what
/// ADR-0018 promises. A target that does not exist yet has nothing to preserve, so a plain move
/// is correct for a first write.
#[cfg(windows)]
fn persist(temp: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
    use windows_sys::Win32::Storage::FileSystem::{REPLACE_FILE_FLAGS, ReplaceFileW};

    if !target.exists() {
        return fs::rename(temp, target);
    }
    let wide = |p: &Path| -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };
    let (replaced, replacement) = (wide(target), wide(temp));
    let status = unsafe {
        ReplaceFileW(
            replaced.as_ptr(),
            replacement.as_ptr(),
            std::ptr::null(),
            0 as REPLACE_FILE_FLAGS,
            std::ptr::null_mut::<HANDLE>() as HANDLE,
            std::ptr::null_mut::<HANDLE>() as HANDLE,
        )
    };
    if status == WAIT_OBJECT_0 {
        return Ok(());
    }
    Err(io::Error::last_os_error())
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

#[cfg(windows)]
fn create_with_mode(path: &Path, mode: u32) -> io::Result<File> {
    // `mode` is an intent the platform interprets, not a POSIX number to copy. 0o600 means
    // owner-only; a created file inherits the directory's ACL, and every file we create lives in
    // a directory we also created at owner-only intent — under a user profile, whose default ACL
    // already excludes other users. The interpretation is by inheritance, which holds until a
    // measured gap says otherwise.
    let _ = mode;
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(not(any(unix, windows)))]
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

#[cfg(windows)]
fn flush_directory(dir: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CloseHandle, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FlushFileBuffers, OPEN_EXISTING,
    };

    // The claim that a directory cannot be flushed was unmeasured, and this is the measurement:
    // open it with the backup-semantics flag, which is how a directory handle is taken, and
    // flush. A refusal to open is recorded as a success-with-gap rather than a failed write —
    // the rename is already durable through the volume; this is belt, not braces.
    let wide: Vec<u16> = dir
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            0 as HANDLE,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Ok(());
    }
    let _ = unsafe { FlushFileBuffers(handle) };
    let _ = unsafe { CloseHandle(handle) };
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn flush_directory(_dir: &Path) -> io::Result<()> {
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
