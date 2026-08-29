//! All or nothing.
//!
//! A switch usually touches several files across several tools, and any one of them can fail.
//! Half-applied is the worst outcome available — the person believes they moved, and one tool
//! is still billing the old provider — so every action is undone if any of them fails.

use crate::fs::FileSystem;
use crate::store::Captured;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum Action {
    Write {
        path: PathBuf,
        bytes: Vec<u8>,
        mode: u32,
    },
    Delete {
        path: PathBuf,
    },
}

impl Action {
    pub fn path(&self) -> &PathBuf {
        match self {
            Action::Write { path, .. } | Action::Delete { path } => path,
        }
    }
}

/// What a failed switch reports. The core returns facts: which file stopped it, why, and how
/// many were put back — the sentence belongs to whoever is speaking to the person.
#[derive(Debug)]
pub struct RolledBack {
    pub failed_at: PathBuf,
    pub reason: String,
    pub restored: usize,
}

pub struct Transaction {
    actions: Vec<Action>,
}

/// What a file held before the transaction touched it. One reading serves two purposes: the
/// rollback needs it in memory and the backup needs it on disk.
struct Before {
    path: PathBuf,
    content: Option<Vec<u8>>,
    mode: Option<u32>,
}

impl Transaction {
    pub fn new(actions: Vec<Action>) -> Self {
        Transaction { actions }
    }

    /// Read every file this transaction would touch, as it stands now.
    /// A switch now spans several tools, so which tool a file belongs to is looked up by path
    /// rather than passed as one name for all of them. Tagging Codex's `config.toml` as Claude
    /// Code's would put it under the wrong heading in a backup's manifest, and a restore reads
    /// that manifest.
    pub fn capture(
        &self,
        fs: &dyn FileSystem,
        tool_of: &std::collections::BTreeMap<PathBuf, &'static str>,
    ) -> io::Result<Vec<Captured>> {
        self.before(fs).map(|before| {
            before
                .into_iter()
                .map(|b| Captured {
                    tool: tool_of
                        .get(&b.path)
                        .copied()
                        .unwrap_or("unknown")
                        .to_string(),
                    path: b.path,
                    content: b.content,
                    mode: b.mode,
                })
                .collect()
        })
    }

    /// Apply every action, or none of them.
    pub fn apply(&self, fs: &mut dyn FileSystem) -> Result<(), RolledBack> {
        let before = self.before(fs).map_err(|e| RolledBack {
            failed_at: PathBuf::new(),
            reason: format!("could not read the files to be changed: {e}"),
            restored: 0,
        })?;

        for (index, action) in self.actions.iter().enumerate() {
            let result = match action {
                Action::Write { path, bytes, mode } => fs.write(path, bytes, *mode),
                Action::Delete { path } => fs.remove(path),
            };
            if let Err(e) = result {
                let restored = undo(fs, &before[..index]);
                return Err(RolledBack {
                    failed_at: action.path().clone(),
                    reason: e.to_string(),
                    restored,
                });
            }
        }
        Ok(())
    }

    fn before(&self, fs: &dyn FileSystem) -> io::Result<Vec<Before>> {
        self.actions
            .iter()
            .map(|a| {
                let path = a.path().clone();
                Ok(Before {
                    content: fs.read(&path)?,
                    mode: fs.mode(&path),
                    path,
                })
            })
            .collect()
    }
}

/// Put back everything already changed. A file that did not exist before is removed rather
/// than left behind: it would be a file the user never had and tapkey no longer tracks.
///
/// An error here is deliberately not propagated. The transaction has already failed, and the
/// only useful thing left is to put back as much as possible and say how much that was.
fn undo(fs: &mut dyn FileSystem, done: &[Before]) -> usize {
    let mut restored = 0;
    for entry in done.iter().rev() {
        let outcome = match &entry.content {
            Some(bytes) => fs.write(&entry.path, bytes, entry.mode.unwrap_or(0o600)),
            None => fs.remove(&entry.path),
        };
        if outcome.is_ok() {
            restored += 1;
        }
    }
    restored
}
