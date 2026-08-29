//! Shared by several test binaries; each one uses a different part of it, so what any one
//! of them leaves unused is not dead code.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A directory that deletes itself. Named per test so a failure leaves a legible path, and
/// counted so that concurrently running tests cannot collide.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(label: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("tapkey-test-{label}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

#[allow(dead_code)]
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------------------------
// A machine the core can be pointed at: a fake home, a fake project, a fixed clock, and a
// login shell that exports nothing unless a test says so.
// ---------------------------------------------------------------------------------------

use serde_json::Value;
use std::collections::BTreeMap;
use tapkey_core::env::{Env, ShellVar};

pub struct Machine {
    dir: TempDir,
    shell: BTreeMap<String, ShellVar>,
    fail_after: Option<usize>,
    credentials: Credentials,
    /// A factory, because every `call` builds a fresh `Env` and a seam cannot be handed out
    /// twice.
    http: Option<Box<dyn Fn() -> Box<dyn tapkey_core::env::Http> + Send + Sync>>,
}

/// What the credential seam answers in this test. The default is *everything stored*, so the
/// sixty-odd tests that are not about credentials keep testing what they were about; the three
/// that are about credentials opt into absence and denial explicitly.
enum Credentials {
    All,
    Stored(Vec<String>),
    None,
    Denied,
}

impl Machine {
    pub fn new(label: &str) -> Self {
        let dir = TempDir::new(label);
        std::fs::create_dir_all(dir.path().join("home")).expect("home");
        Machine {
            dir,
            shell: BTreeMap::new(),
            fail_after: None,
            credentials: Credentials::All,
            http: None,
        }
    }

    pub fn project(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    pub fn managed(&self) -> PathBuf {
        self.dir
            .path()
            .join("managed")
            .join("managed-settings.json")
    }

    /// Substitute the HTTP seam. Nothing by default, which is the offline state.
    pub fn http(
        mut self,
        http: impl Fn() -> Box<dyn tapkey_core::env::Http> + Send + Sync + 'static,
    ) -> Self {
        self.http = Some(Box::new(http));
        self
    }

    /// Declare which providers have a credential stored.
    pub fn with_credentials(mut self, providers: &[&str]) -> Self {
        self.credentials = Credentials::Stored(providers.iter().map(|s| s.to_string()).collect());
        self
    }

    /// Declare that the Keychain refuses. Distinct from nothing being stored, because the two lead
    /// to different sentences on screen.
    pub fn denying_credentials(mut self) -> Self {
        self.credentials = Credentials::Denied;
        self
    }

    /// Declare that the filesystem refuses one operation, after this many have succeeded.
    pub fn failing_after(mut self, successes: usize) -> Self {
        self.fail_after = Some(successes);
        self
    }

    /// Declare what a login shell exports. Values only for what is safe to hold.
    pub fn exporting(mut self, name: &str, value: ShellVar) -> Self {
        self.shell.insert(name.to_string(), value);
        self
    }

    pub fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    pub fn write_user_settings(&self, value: Value) {
        write_json(&self.home().join(".claude").join("settings.json"), value);
    }

    pub fn write_project_settings(&self, value: Value) {
        write_json(&self.project().join(".claude").join("settings.json"), value);
    }

    pub fn write_project_local_settings(&self, value: Value) {
        write_json(
            &self.project().join(".claude").join("settings.local.json"),
            value,
        );
    }

    pub fn store(&self) -> PathBuf {
        self.dir.path().join("store")
    }

    pub fn write_profiles(&self, value: Value) {
        write_json(&self.store().join("profiles.json"), value);
    }

    pub fn write_managed_settings(&self, value: Value) {
        write_json(&self.managed(), value);
    }

    /// Write bytes verbatim, for the cases where the exact layout is the point.
    pub fn write_user_settings_raw(&self, bytes: &[u8]) {
        let path = self.home().join(".claude").join("settings.json");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    // -- Codex. Always raw bytes: TOML's layout is the thing the editor promises to keep, so a
    // -- helper that serialised a structure would be testing something other than the file.

    pub fn write_codex_config(&self, bytes: &[u8]) {
        let path = self.home().join(".codex").join("config.toml");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    pub fn write_codex_project_config(&self, bytes: &[u8]) {
        let path = self.project().join(".codex").join("config.toml");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    // -- OpenCode. Raw bytes again: JSONC layout is what the splicer promises to keep.

    /// One of the three global files, which the tool reads and merges rather than choosing between.
    pub fn write_opencode_config(&self, name: &str, bytes: &[u8]) {
        let path = self.home().join(".config").join("opencode").join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    /// A project file, which OpenCode obeys with nothing to grant — unlike Codex's, which is
    /// ignored in silence until the user's own config trusts the repository root.
    pub fn write_opencode_project_config(&self, bytes: &[u8]) {
        let path = self.project().join("opencode.jsonc");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    /// Measured: the gate keys on the **repo root**, and the entry lives in the *user's* config.
    /// Without it the project file is ignored entirely, and Codex says nothing about it.
    pub fn trust_codex_project(&self) {
        let path = self.home().join(".codex").join("config.toml");
        let mut bytes = std::fs::read(&path).unwrap_or_default();
        bytes.extend_from_slice(
            format!(
                "\n[projects.\"{}\"]\ntrust_level = \"trusted\"\n",
                self.project().display()
            )
            .as_bytes(),
        );
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    pub fn env(&self) -> Env {
        // A fixed clock, so a backup's name is the same on every run.
        let env = Env::for_test(self.home(), self.store())
            .with_clock(std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_787_866_640_123))
            .with_project(self.project())
            .with_managed(self.managed())
            .with_shell(self.shell.clone())
            .with_http(match &self.http {
                Some(build) => build(),
                None => Box::new(OfflineHttp),
            })
            .with_credentials(match &self.credentials {
                Credentials::All => {
                    Box::new(AllCredentials) as Box<dyn tapkey_core::env::Credentials>
                }
                Credentials::Stored(ids) => Box::new(StoredCredentials(ids.clone())),
                Credentials::None => Box::new(tapkey_core::env::NoCredentials),
                Credentials::Denied => Box::new(DenyingCredentials),
            });
        match self.fail_after {
            Some(n) => env.with_filesystem(Box::new(tapkey_core::fs::FailOnce::after(n))),
            None => env,
        }
    }
}

fn write_json(path: &Path, value: Value) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, serde_json::to_vec_pretty(&value).expect("serialise")).expect("write");
}

/// Send one request through the single entry point and parse what comes back.
pub fn call(machine: &Machine, request: Value) -> Value {
    let text = tapkey_core::handle_with(&machine.env(), &request.to_string());
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("response was not JSON: {e}\n{text}"))
}

struct AllCredentials;

impl tapkey_core::env::Credentials for AllCredentials {
    fn check(&self, _provider_id: &str) -> tapkey_core::env::CredentialState {
        tapkey_core::env::CredentialState::Found
    }
}

struct StoredCredentials(Vec<String>);

impl tapkey_core::env::Credentials for StoredCredentials {
    fn check(&self, provider_id: &str) -> tapkey_core::env::CredentialState {
        if self.0.iter().any(|id| id == provider_id) {
            tapkey_core::env::CredentialState::Found
        } else {
            tapkey_core::env::CredentialState::Absent
        }
    }
}

struct DenyingCredentials;

impl tapkey_core::env::Credentials for DenyingCredentials {
    fn check(&self, _provider_id: &str) -> tapkey_core::env::CredentialState {
        tapkey_core::env::CredentialState::Denied
    }
}

/// Nothing answers. The machine's default seam, and the honest state offline.
struct OfflineHttp;

impl tapkey_core::env::Http for OfflineHttp {
    fn post(
        &self,
        _url: &str,
    ) -> Result<tapkey_core::env::ProbeStatus, tapkey_core::env::NetworkUnreachable> {
        Ok(tapkey_core::env::ProbeStatus::NoAnswer)
    }
}
