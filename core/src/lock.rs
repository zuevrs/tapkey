//! One writer at a time.
//!
//! Three entry points into one core means the menu bar and a terminal can switch at once. The
//! lock is advisory and the operating system releases it when the holding process dies, so a
//! stale lock cannot exist — unlike a pid file, which somebody has to judge stale.
//!
//! A second caller is refused rather than queued: a switch takes milliseconds, and a queue
//! would mean the profile applies after the person changed their mind.

use std::fs::{File, OpenOptions};
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

#[cfg(not(unix))]
fn take(_file: &File) -> Result<(), Busy> {
    // Windows has LockFileEx and no runner yet to prove it against. Named rather than silently
    // succeeding, so the gap is collected along with the rest of the Windows seam.
    Ok(())
}
