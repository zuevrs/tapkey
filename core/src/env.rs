//! What the core is given rather than what it goes and finds.
//!
//! The public entry point wraps a form that accepts this: overriding `HOME` in the process is
//! global while tests run on threads, and a test-only field in the wire envelope would put
//! scaffolding in the contract three consumers depend on.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where Claude Code reads an administrator's policy from on this platform.
#[cfg(target_os = "macos")]
const MANAGED_SETTINGS: &str = "/Library/Application Support/ClaudeCode/managed-settings.json";
#[cfg(all(unix, not(target_os = "macos")))]
const MANAGED_SETTINGS: &str = "/etc/claude-code/managed-settings.json";
#[cfg(windows)]
const MANAGED_SETTINGS: &str = r"C:\Program Files\ClaudeCode\managed-settings.json";

/// What a login shell was found to export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellVar {
    /// Set, and the value is safe to hold and to show.
    Value(String),
    /// Set, and deliberately not read. A credential tapkey does not need is a credential
    /// tapkey does not hold: once in the process it is in a buffer, and from there eventually
    /// in a log or a crash report.
    SetButWithheld,
}

/// The world the core acts in.
pub struct Env {
    home: PathBuf,
    store: PathBuf,
    project: Option<PathBuf>,
    managed: PathBuf,
    shell: BTreeMap<String, ShellVar>,
}

impl Env {
    /// The real machine.
    pub fn real() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        let store = home
            .join("Library")
            .join("Application Support")
            .join("tapkey");
        Env {
            home,
            store,
            project: None,
            managed: PathBuf::from(MANAGED_SETTINGS),
            shell: BTreeMap::new(),
        }
    }

    /// The environment a test builds by hand.
    pub fn for_test(home: PathBuf, store: PathBuf) -> Self {
        Env {
            home,
            store,
            project: None,
            managed: PathBuf::from(MANAGED_SETTINGS),
            shell: BTreeMap::new(),
        }
    }

    /// Declare what the login shell exports. Values are only supplied for variables it is safe
    /// to hold; a credential is recorded as present and nothing more.
    pub fn with_shell(mut self, shell: BTreeMap<String, ShellVar>) -> Self {
        self.shell = shell;
        self
    }

    /// Where the administrator's managed settings would be, if there are any.
    pub fn with_managed(mut self, managed: PathBuf) -> Self {
        self.managed = managed;
        self
    }

    /// The project directory a tool would resolve its project scope against.
    pub fn with_project(mut self, project: PathBuf) -> Self {
        self.project = Some(project);
        self
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    #[allow(dead_code)]
    pub fn store(&self) -> &Path {
        &self.store
    }

    pub fn project(&self) -> Option<&Path> {
        self.project.as_deref()
    }

    pub fn managed(&self) -> &Path {
        &self.managed
    }

    /// What the login shell exports under `name`, if anything.
    pub fn shell_var(&self, name: &str) -> Option<&ShellVar> {
        self.shell.get(name)
    }
}
