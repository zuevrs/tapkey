//! The filesystem as something the core is handed rather than something it reaches for.
//!
//! One reason only: the transactional guarantee is provable solely by failing midway through
//! several files, and a failure that cannot be injected is a guarantee nobody has tested.

use crate::atomic::write_atomically;
use std::io;
use std::path::{Path, PathBuf};

pub trait FileSystem {
    fn write(&mut self, path: &Path, bytes: &[u8], mode: u32) -> io::Result<()>;
    fn remove(&mut self, path: &Path) -> io::Result<()>;
    /// `Ok(None)` when the file is not there, which is a fact rather than an error.
    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>>;
    fn mode(&self, path: &Path) -> Option<u32>;
}

/// The real one. Every write goes through the atomic seam.
pub struct RealFs;

impl FileSystem for RealFs {
    fn write(&mut self, path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
        write_atomically(path, bytes, mode)
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    #[cfg(unix)]
    fn mode(&self, path: &Path) -> Option<u32> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .ok()
            .map(|m| m.permissions().mode() & 0o7777)
    }

    #[cfg(not(unix))]
    fn mode(&self, _path: &Path) -> Option<u32> {
        None
    }
}

/// Fails exactly one operation — the one after `successes` have gone through — and behaves
/// normally before and after it.
///
/// Failing everything from that point on was the first shape, and it was wrong: the rollback
/// writes through the same filesystem, so a permanently broken one meant nothing could be put
/// back and the test could not tell a working rollback from a missing one. It is also the
/// wrong scenario. What ADR-0005 is about is one file refusing — a read-only config, a denied
/// permission, a full disk — while the rest of the disk is perfectly able to accept the
/// restore.
pub struct FailOnce {
    real: RealFs,
    countdown: Option<usize>,
    pub attempted: Vec<PathBuf>,
}

impl FailOnce {
    pub fn after(successes: usize) -> Self {
        FailOnce {
            real: RealFs,
            countdown: Some(successes),
            attempted: Vec::new(),
        }
    }

    /// True once the injected failure has been delivered, so nothing further is refused.
    fn should_fail(&mut self) -> bool {
        match self.countdown {
            Some(0) => {
                self.countdown = None;
                true
            }
            Some(n) => {
                self.countdown = Some(n - 1);
                false
            }
            None => false,
        }
    }
}

impl FileSystem for FailOnce {
    fn write(&mut self, path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
        self.attempted.push(path.to_path_buf());
        if self.should_fail() {
            return Err(io::Error::other("injected write failure"));
        }
        self.real.write(path, bytes, mode)
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
        self.attempted.push(path.to_path_buf());
        if self.should_fail() {
            return Err(io::Error::other("injected removal failure"));
        }
        self.real.remove(path)
    }

    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        self.real.read(path)
    }

    fn mode(&self, path: &Path) -> Option<u32> {
        self.real.mode(path)
    }
}
