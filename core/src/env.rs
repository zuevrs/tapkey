//! What the core is given rather than what it goes and finds.
//!
//! The public entry point wraps a form that accepts this: overriding `HOME` in the process is
//! global while tests run on threads, and a test-only field in the wire envelope would put
//! scaffolding in the contract three consumers depend on.

use crate::fs::{FileSystem, RealFs};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where Claude Code reads an administrator's policy from on this platform.
#[cfg(target_os = "macos")]
const MANAGED_SETTINGS: &str = "/Library/Application Support/ClaudeCode/managed-settings.json";
#[cfg(all(unix, not(target_os = "macos")))]
const MANAGED_SETTINGS: &str = "/etc/claude-code/managed-settings.json";
#[cfg(windows)]
const MANAGED_SETTINGS: &str = r"C:\Program Files\ClaudeCode\managed-settings.json";

/// Asking about a credential, and never being handed one.
///
/// ADR-0016 records why this is an interface rather than a call: a test must never raise an access
/// dialog, and the Linux runner has no Keychain at all. The default implementation spawns the helper
/// binary; a test substitutes its own and goes near neither. Presence is the only question asked —
/// a credential tapkey does not need is a credential tapkey does not hold.
pub trait Credentials {
    /// Whether a credential is stored for this provider id, answered as the helper's exit code
    /// distinguishes: found, no such item, or refused.
    fn check(&self, provider_id: &str) -> CredentialState;
}

/// The three answers, which is why the helper answers with three exit codes rather than two: an
/// absent item and a denial lead to different outcomes, and one non-zero code would make the core
/// guess — the guess becoming "add a key" said to somebody whose key was withheld.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialState {
    Found,
    Absent,
    Denied,
}

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

/// Asking the network one narrow question: is there a route here?
///
/// HTTP is one of the five platform seams, and this is the only thing the core ever asks of it.
/// A Test probes a format's own path and reads the **status**, never the body and never a real
/// completion — those cost tokens and need a credential, and a format was measured to be
/// establishable without either: an absent path answers 404 and a present one 401.
pub trait Http {
    fn post(&self, url: &str) -> Result<ProbeStatus, NetworkUnreachable>;
}

/// What came back, reduced to what a Test can use. A body would be a secret-carrying surface and
/// a parser; neither belongs here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeStatus {
    Answered(u16),
    /// No HTTP answer at all — connection refused, DNS failure, timeout. Not an answer, and never
    /// to be recorded as one: a network failure is our outage, not a verdict on the endpoint.
    NoAnswer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkUnreachable;

/// The world the core acts in.
pub struct Env {
    credentials: Box<dyn Credentials>,
    http: Box<dyn Http>,
    home: PathBuf,
    store: PathBuf,
    project: Option<PathBuf>,
    managed: PathBuf,
    shell: BTreeMap<String, ShellVar>,
    now: std::time::SystemTime,
    /// Given rather than constructed, for one reason: the transactional guarantee is provable
    /// only by failing midway through several files, and a failure that cannot be injected is
    /// a guarantee nobody has tested end to end.
    filesystem: RefCell<Box<dyn FileSystem>>,
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
            now: std::time::SystemTime::now(),
            filesystem: RefCell::new(Box::new(RealFs)),
            credentials: Box::new(HelperCredentials),
            http: Box::new(RealHttp),
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
            now: std::time::SystemTime::now(),
            filesystem: RefCell::new(Box::new(RealFs)),
            // Nothing is stored, which is the safe default for a test: every switch needing a
            // credential refuses rather than proceeding on one nobody put there.
            credentials: Box::new(NoCredentials),
            // The safe default for a test: the network does not exist, so a Test comes back
            // *untested* rather than reaching for anything.
            http: Box::new(NoNetwork),
        }
    }

    /// Substitute the HTTP seam. Tests do this; nothing else needs to.
    pub fn with_http(mut self, http: Box<dyn Http>) -> Self {
        self.http = http;
        self
    }

    pub fn http(&self) -> &dyn Http {
        &*self.http
    }

    /// Substitute the credential seam. Tests do this; nothing else needs to.
    pub fn with_credentials(mut self, credentials: Box<dyn Credentials>) -> Self {
        self.credentials = credentials;
        self
    }

    pub fn credentials(&self) -> &dyn Credentials {
        &*self.credentials
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

    /// The clock, given rather than read, so a backup's name is deterministic under test and
    /// the store's own layout can be compared byte for byte.
    pub fn now(&self) -> std::time::SystemTime {
        self.now
    }

    /// Substitute the filesystem, so a fixture can declare a failure partway through.
    pub fn with_filesystem(mut self, filesystem: Box<dyn FileSystem>) -> Self {
        self.filesystem = RefCell::new(filesystem);
        self
    }

    /// Borrow the filesystem for the duration of one operation.
    pub fn filesystem(&self) -> std::cell::RefMut<'_, Box<dyn FileSystem>> {
        self.filesystem.borrow_mut()
    }

    /// Fix the clock. Without this a fixture could not compare the store it produced.
    pub fn with_clock(mut self, now: std::time::SystemTime) -> Self {
        self.now = now;
        self
    }

    /// What the login shell exports under `name`, if anything.
    pub fn shell_var(&self, name: &str) -> Option<&ShellVar> {
        self.shell.get(name)
    }
}

/// Nothing is stored. The default in tests, and the honest answer on a machine where the helper has
/// never run.
pub struct NoCredentials;

impl Credentials for NoCredentials {
    fn check(&self, _provider_id: &str) -> CredentialState {
        CredentialState::Absent
    }
}

/// The real one: asks the helper binary, which owns every Keychain operation (ADR-0007).
pub struct HelperCredentials;

impl Credentials for HelperCredentials {
    fn check(&self, provider_id: &str) -> CredentialState {
        // `has` prints nothing and answers by exit code, so that asking whether a credential exists
        // never passes through the code that can print one.
        match std::process::Command::new(helper_path())
            .arg("has")
            .arg(provider_id)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
        {
            Ok(status) if status.success() => CredentialState::Found,
            Ok(status) if status.code() == Some(1) => CredentialState::Absent,
            // Anything else, including the helper being missing, is a refusal rather than an
            // absence: we could not find out, and "add a key" is the wrong thing to say to somebody
            // whose key may be sitting there.
            _ => CredentialState::Denied,
        }
    }
}

fn helper_path() -> PathBuf {
    Env::real().store().join("bin").join("tapkey-helper")
}

/// Nothing answers. The default in tests, and the honest state offline.
pub struct NoNetwork;

impl Http for NoNetwork {
    fn post(&self, _url: &str) -> Result<ProbeStatus, NetworkUnreachable> {
        Err(NetworkUnreachable)
    }
}

/// The real one, through `ureq` with the platform's own trust store: a bundled CA list would go
/// stale, and this tool's requests are credential-adjacent. Five seconds, because a Test runs
/// while somebody watches, and a status code is all it wants.
pub struct RealHttp;

impl Http for RealHttp {
    fn post(&self, url: &str) -> Result<ProbeStatus, NetworkUnreachable> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(5)))
            .build()
            .into();
        // A non-2xx status is an *answer* here, not an error: 404 is exactly what a Test is asking
        // about, so the error carrying a status is unpacked rather than failed on.
        match agent.post(url).send(&b"{}"[..]) {
            Ok(response) => Ok(ProbeStatus::Answered(response.status().as_u16())),
            Err(ureq::Error::StatusCode(code)) => Ok(ProbeStatus::Answered(code)),
            Err(_) => Ok(ProbeStatus::NoAnswer),
        }
    }
}
