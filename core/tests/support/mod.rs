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
}

impl Machine {
    pub fn new(label: &str) -> Self {
        let dir = TempDir::new(label);
        std::fs::create_dir_all(dir.path().join("home")).expect("home");
        Machine {
            dir,
            shell: BTreeMap::new(),
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

    pub fn env(&self) -> Env {
        // A fixed clock, so a backup's name is the same on every run.
        Env::for_test(self.home(), self.store())
            .with_clock(std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_787_866_640_123))
            .with_project(self.project())
            .with_managed(self.managed())
            .with_shell(self.shell.clone())
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
