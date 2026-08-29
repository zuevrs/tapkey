//! The backup store on disk.
//!
//! One directory holds everything tapkey owns. A backup is whole copies of the files one
//! change touched, grouped by that change, named by the instant it was taken. The manifest is
//! written last and is the commit point: a directory without one is an interrupted write, our
//! own garbage rather than the user's state.
//!
//! The snapshot lives apart because its retention rule is different — it is kept forever, so
//! the sweep does not see it at all rather than being taught to skip it.

use crate::atomic::write_atomically;
use crate::instant::format_utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A file as it was found, ready to be copied into the store.
#[derive(Debug, Clone)]
pub struct Captured {
    pub path: PathBuf,
    pub tool: String,
    /// `None` means the file did not exist. Back-fill can create a config that was never
    /// there, so without this "returns you to how it was" is false from the first switch.
    pub content: Option<Vec<u8>>,
    pub mode: Option<u32>,
}

/// What restoring one backup would do. The store plans; the transactional writer acts, so the
/// all-or-nothing guarantee exists once rather than twice.
#[derive(Debug, PartialEq, Eq)]
pub enum RestoreAction {
    Write {
        path: PathBuf,
        bytes: Vec<u8>,
        mode: Option<u32>,
    },
    Delete {
        path: PathBuf,
    },
}

/// Which stored state to go back to. Tagged rather than inferred from the shape of an id:
/// "snapshot cannot look like a timestamp" is a guard that lasts until the format changes.
#[derive(Debug, Clone)]
pub enum Target {
    Snapshot,
    Backup(BackupId),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BackupId(String);

impl BackupId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for BackupId {
    fn from(s: &str) -> Self {
        BackupId(s.to_string())
    }
}

#[derive(Debug)]
pub struct BackupSummary {
    pub id: BackupId,
    pub profile: String,
    pub instant: String,
    /// False when the manifest cannot be read or a copy it names is gone. Such a backup is
    /// kept and listed: erasing somebody's only way back because we failed to parse it is
    /// worse than any amount of clutter.
    pub restorable: bool,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    instant: String,
    /// The profile's name as it was. Not a reference: profiles get renamed and deleted, and a
    /// history that dereferences begins to lie retroactively.
    #[serde(default)]
    profile: String,
    files: Vec<Entry>,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    tool: String,
    /// Absent from the store when the file did not exist.
    stored: Option<String>,
    mode: Option<u32>,
    size: u64,
}

const MANIFEST_VERSION: u32 = 1;
#[cfg(unix)]
const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Open the store, creating it closed. The store is ours, unlike the tools' files, and a
    /// copy inside it may contain a key.
    pub fn open(root: &Path) -> io::Result<Store> {
        create_dir_closed(root)?;
        create_dir_closed(&root.join("backups"))?;
        Ok(Store {
            root: root.to_path_buf(),
        })
    }

    pub fn has_snapshot(&self) -> bool {
        self.snapshot_dir().join("manifest.json").exists()
    }

    /// Record the configs as they were before tapkey first changed anything.
    pub fn take_snapshot(&self, files: &[Captured], at: SystemTime) -> io::Result<()> {
        let dir = self.snapshot_dir();
        write_group(&dir, files, at, "")?;
        Ok(())
    }

    /// Save the files one change is about to touch.
    pub fn take_backup(
        &self,
        files: &[Captured],
        profile: &str,
        at: SystemTime,
    ) -> io::Result<BackupId> {
        let id = self.free_name(format_utc(at));
        write_group(&self.backups_dir().join(&id.0), files, at, profile)?;
        Ok(id)
    }

    /// Two changes inside the same millisecond would otherwise share a directory, and the
    /// second would silently overwrite the first. An ugly name is a much smaller price than a
    /// lost way back, and a suffix keeps the ordering the name is chosen for.
    fn free_name(&self, base: String) -> BackupId {
        if !self.backups_dir().join(&base).exists() {
            return BackupId(base);
        }
        for n in 2.. {
            let candidate = format!("{base}-{n}");
            if !self.backups_dir().join(&candidate).exists() {
                return BackupId(candidate);
            }
        }
        unreachable!("the range is unbounded")
    }

    /// Every readable backup, oldest first, sweeping away any interrupted write it finds.
    pub fn backups(&self) -> io::Result<Vec<BackupSummary>> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(self.backups_dir()) else {
            return Ok(out);
        };
        for entry in entries {
            let path = entry?.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("manifest.json");
            if !manifest_path.exists() {
                // No manifest means the write never committed. Ours, not theirs.
                fs::remove_dir_all(&path)?;
                continue;
            }
            let id = BackupId(file_name(&path));
            out.push(match read_manifest(&manifest_path) {
                Ok(m) => BackupSummary {
                    profile: m.profile,
                    instant: m.instant,
                    restorable: m.files.iter().all(|e| match &e.stored {
                        Some(name) => path.join("files").join(name).exists(),
                        None => true,
                    }),
                    id,
                },
                Err(_) => BackupSummary {
                    profile: String::new(),
                    instant: id.0.clone(),
                    restorable: false,
                    id,
                },
            });
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    /// What going back to `target` would do to the filesystem.
    pub fn restore_plan(&self, target: Target) -> io::Result<Vec<RestoreAction>> {
        let dir = match target {
            Target::Snapshot => self.snapshot_dir(),
            Target::Backup(id) => self.backups_dir().join(&id.0),
        };
        let manifest = read_manifest(&dir.join("manifest.json"))?;
        manifest
            .files
            .into_iter()
            .map(|e| match e.stored {
                Some(name) => Ok(RestoreAction::Write {
                    bytes: fs::read(dir.join("files").join(name))?,
                    path: e.path,
                    mode: e.mode,
                }),
                None => Ok(RestoreAction::Delete { path: e.path }),
            })
            .collect()
    }

    /// Two ceilings, both binding, oldest first. Fifty guards against a long history, fifty
    /// megabytes against one enormous config, and each alone misses what the other catches.
    /// The snapshot is not counted and not reachable: counting it would let a large one evict
    /// the whole history, or make it the suspect when the budget is reached.
    pub fn sweep(&self, max_backups: usize, max_bytes: u64) -> io::Result<()> {
        let listed = self.backups()?; // oldest first, and sweeps torn writes on the way
        let mut sizes: Vec<(BackupId, u64)> = Vec::new();
        for summary in &listed {
            let dir = self.backups_dir().join(summary.id.as_str());
            sizes.push((summary.id.clone(), directory_size(&dir)?));
        }

        let mut total: u64 = sizes.iter().map(|(_, n)| *n).sum();
        let mut count = sizes.len();
        for (id, size) in sizes {
            if count <= max_backups && total <= max_bytes {
                break;
            }
            fs::remove_dir_all(self.backups_dir().join(id.as_str()))?;
            total = total.saturating_sub(size);
            count -= 1;
        }
        Ok(())
    }

    fn snapshot_dir(&self) -> PathBuf {
        self.root.join("snapshot")
    }

    fn backups_dir(&self) -> PathBuf {
        self.root.join("backups")
    }
}

/// Copies first, manifest last by atomic rename: one point of commit rather than a transaction
/// spanning the store.
fn write_group(dir: &Path, files: &[Captured], at: SystemTime, profile: &str) -> io::Result<()> {
    create_dir_closed(dir)?;
    create_dir_closed(&dir.join("files"))?;

    let mut entries = Vec::new();
    for (n, file) in files.iter().enumerate() {
        let stored = match &file.content {
            Some(bytes) => {
                let name = format!("{:02}", n + 1);
                write_atomically(&dir.join("files").join(&name), bytes, FILE_MODE)?;
                Some(name)
            }
            None => None,
        };
        entries.push(Entry {
            path: file.path.clone(),
            tool: file.tool.clone(),
            size: file.content.as_ref().map(|b| b.len() as u64).unwrap_or(0),
            mode: file.mode,
            stored,
        });
    }

    let manifest = Manifest {
        version: MANIFEST_VERSION,
        instant: format_utc(at),
        profile: profile.to_string(),
        files: entries,
    };
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    write_atomically(&dir.join("manifest.json"), &bytes, FILE_MODE)
}

fn read_manifest(path: &Path) -> io::Result<Manifest> {
    let bytes = fs::read(path)?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if manifest.version > MANIFEST_VERSION {
        // A version from the future is kept and marked, never reinterpreted by guesswork.
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "manifest version {} is newer than this build reads",
                manifest.version
            ),
        ));
    }
    Ok(manifest)
}

fn create_dir_closed(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    set_dir_mode(path)
}

#[cfg(unix)]
fn set_dir_mode(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(DIR_MODE))
}

#[cfg(not(unix))]
fn set_dir_mode(_path: &Path) -> io::Result<()> {
    // Windows has no equivalent; revisited with the Windows platform seam.
    Ok(())
}

fn directory_size(dir: &Path) -> io::Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        total += if meta.is_dir() {
            directory_size(&entry.path())?
        } else {
            meta.len()
        };
    }
    Ok(total)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}
