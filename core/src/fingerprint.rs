//! Remembering what tapkey wrote, without keeping it.
//!
//! Drift asks one question — is this slot still what we put there — and a hash answers it.
//! Storing the values themselves would put a token, or a URL carrying a key, on disk against
//! the rule that secrets stay in the keychain; and effective state reads the live value
//! whenever one has to be shown.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// FNV-1a, 64-bit. Not a cryptographic hash and not asked to be one: the question is whether a
/// value changed, the comparison is against a value we wrote ourselves, and there is no
/// adversary choosing inputs. It is deterministic across builds and platforms, which
/// `DefaultHasher` does not promise, and that matters for something written to disk.
pub fn hash(value: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        h ^= *byte as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{h:016x}")
}

/// What tapkey owns right now, and what it put there.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    #[serde(default)]
    pub profile: String,
    /// Tool, then slot, then the hash of the value written. The endpoint is filed under its
    /// own name here rather than as a slot, matching the response.
    #[serde(default)]
    pub owned: BTreeMap<String, BTreeMap<String, String>>,
}

const VERSION: u32 = 1;

impl State {
    pub fn read(path: &Path) -> State {
        // A state file that is missing or unreadable means tapkey owns nothing it can prove,
        // which is the safe reading: it reports no drift rather than inventing some.
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<State>(&b).ok())
            .filter(|s| s.version <= VERSION)
            .unwrap_or_default()
    }

    pub fn write(
        path: &Path,
        profile: &str,
        owned: BTreeMap<String, BTreeMap<String, String>>,
    ) -> io::Result<()> {
        let state = State {
            version: VERSION,
            profile: profile.to_string(),
            owned,
        };
        let bytes = serde_json::to_vec_pretty(&state)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        crate::atomic::write_atomically(path, &bytes, 0o600)
    }

    /// Whether `value` differs from what tapkey recorded writing. Unknown slots never drift:
    /// reporting drift against something we never owned would make the signal meaningless on
    /// a machine where nothing has been switched yet.
    pub fn drifted(&self, tool: &str, slot: &str, value: Option<&str>) -> bool {
        match self.owned.get(tool).and_then(|s| s.get(slot)) {
            Some(recorded) => value.map(hash).as_deref() != Some(recorded.as_str()),
            None => false,
        }
    }
}
