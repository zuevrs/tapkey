//! One writer at a time.
//!
//! Three entry points into one core means the menu bar and a terminal can switch at once. The
//! lock is advisory and the operating system releases it when the holding process dies, so a
//! stale lock cannot exist — unlike a pid file, which somebody has to judge stale.
//!
//! A second caller is refused rather than queued: a switch takes milliseconds, and a queue
//! would mean the profile applies after the person changed their mind.

use std::fs::{File, OpenOptions};
#[cfg(unix)]
use std::io;
use std::path::Path;

/// Held for the duration of one operation that writes.
///
/// There is no explicit unlock, and that is the point rather than an omission: closing the
/// descriptor releases the lock, so dropping this releases it and so does the process dying.
/// A pid file would need somebody to decide when it had gone stale.
pub struct Lock {
    #[allow(dead_code)]
    file: File,
}

impl Lock {
    /// Take the lock, or report that somebody else has it.
    pub fn acquire(store: &Path) -> Result<Lock, Busy> {
        let path = store.join("lock");
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| Busy(e.to_string()))?;
        take(&file)?;
        Ok(Lock { file })
    }
}

/// Somebody else is writing. Not an error in the file system sense — a fact about timing.
#[derive(Debug)]
pub struct Busy(pub String);

#[cfg(unix)]
fn take(file: &File) -> Result<(), Busy> {
    use std::os::unix::io::AsRawFd;
    // SAFETY: the descriptor is owned by `file` and outlives the call.
    // Exclusive and non-blocking. A mutation run reports the `|` as survivable, and it is:
    // the two flags share no bits, so exclusive-or has the same value here.
    let taken = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if taken == 0 {
        return Ok(());
    }
    Err(Busy(io::Error::last_os_error().to_string()))
}

#[cfg(windows)]
fn take(file: &File) -> Result<(), Busy> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;

    // Exclusive and immediate, over the whole 64-bit range — the byte-range form of what `flock`
    // does on Unix. Like the descriptor there, the handle releases the lock when the process
    // exits, which is why a dead process cannot hold one: the property locking.rs asserts.
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    let taken = unsafe {
        LockFileEx(
            file.as_raw_handle(),
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if taken != 0 {
        return Ok(());
    }
    Err(Busy(std::io::Error::last_os_error().to_string()))
}

#[cfg(not(any(unix, windows)))]
fn take(_file: &File) -> Result<(), Busy> {
    // No platform seam on this target. Refusing rather than succeeding: "we cannot lock" is
    // nearer to "busy" than to "free", and the side to be wrong on is the one where two processes
    // do not write the same file.
    Err(Busy("locking is not implemented on this platform".into()))
}
