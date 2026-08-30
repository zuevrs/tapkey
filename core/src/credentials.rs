//! Reading and storing a secret, with the narrowest possible path between the two.
//!
//! The rule harvest obeys: a secret goes **file → buffer → helper stdin**, and stops. It never
//! enters a response, a log, or a structure that outlives the call. The reading half is per tool,
//! because each tool keeps its plaintext key in its own shape; the storing half is the helper's
//! `set`, which is the only writer of secrets by decision.

use crate::env::Env;
use std::io::Write;
use std::process::{Command, Stdio};

/// Re-read the inline plaintext key for one candidate, from the tool's user-level file, now.
pub fn read_inline(env: &Env, tool: &str, id: &str) -> Option<Vec<u8>> {
    match tool {
        "claude" => {
            let bytes = std::fs::read(env.home().join(".claude").join("settings.json")).ok()?;
            let document = crate::json::Document::parse(&bytes).ok()?;
            document
                .get_string(&["env", "ANTHROPIC_AUTH_TOKEN"])
                .or_else(|| document.get_string(&["env", "ANTHROPIC_API_KEY"]))
                .map(str::to_owned)
                .map(Vec::from)
        }
        "opencode" => {
            // The candidate was found by id, and the id names the provider in the tool's own file.
            for name in crate::adapters::opencode::GLOBAL_FILES {
                let path = env.home().join(".config").join("opencode").join(name);
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let Ok(document) = crate::json::Document::parse_jsonc(&bytes) else {
                    continue;
                };
                match document.get_string(&["provider", id, "options", "apiKey"]) {
                    Some(key) if !key.starts_with("{env:") && !key.starts_with("{file:") => {
                        return Some(Vec::from(key));
                    }
                    _ => continue,
                }
            }
            None
        }
        // Codex keeps no plaintext key in `config.toml`, so there is nothing to re-read.
        _ => None,
    }
}

/// Store a secret through the helper's `set`, on stdin.
pub fn store(env: &Env, id: &str, secret: &[u8]) -> Result<(), String> {
    let mut command = Command::new(crate::env::helper_path(env.store()));
    // The path names where files go; only the flag chooses files at all. A real env keeps the
    // platform's store — the Keychain the catalogue promises and the first shipped iteration
    // silently skipped.
    command
        .env("TAPKEY_STORE", env.store())
        .env_remove("TAPKEY_FILE_STORE");
    if env.file_key_store() {
        command.env("TAPKEY_FILE_STORE", "1");
    }
    let mut child = command
        .arg("set")
        .arg(id)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run the credential helper: {e}"))?;
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(secret)
        .map_err(|e| format!("could not hand the secret over: {e}"))?;
    match child.wait() {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err("the credential helper refused the secret".into()),
        Err(e) => Err(format!("the credential helper failed: {e}")),
    }
}

/// Delete the stored key for one provider, through the helper's `forget`.
pub fn forget(env: &Env, id: &str) -> Result<(), String> {
    let mut command = Command::new(crate::env::helper_path(env.store()));
    command
        .env("TAPKEY_STORE", env.store())
        .env_remove("TAPKEY_FILE_STORE");
    if env.file_key_store() {
        command.env("TAPKEY_FILE_STORE", "1");
    }
    let mut child = command
        .arg("forget")
        .arg(id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run the credential helper: {e}"))?;
    match child.wait() {
        Ok(status) if status.success() => Ok(()),
        // Exit 1 is the helper's *no such item* — nothing stored, nothing to forget, which is
        // success for a removal. A provider that never had a key still has to be removable.
        Ok(status) if status.code() == Some(1) => Ok(()),
        Ok(_) => Err("the credential helper refused".into()),
        Err(e) => Err(format!("the credential helper failed: {e}")),
    }
}

/// The stored secret for one provider, for the one request that needs it on the wire: the
/// provider's own catalogue. File → buffer → header, and stops — never a log, never a
/// response, never a structure that outlives the call.
pub fn read_stored(env: &Env, id: &str) -> Option<String> {
    let output = std::process::Command::new(crate::env::helper_path(env.store()))
        .env_remove("TAPKEY_FILE_STORE")
        .arg("get")
        .arg(id)
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}
