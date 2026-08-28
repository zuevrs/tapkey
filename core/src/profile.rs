//! Profiles, read from a file the core owns.
//!
//! Managing them — rename, duplicate, delete, and what happens to the one currently switched
//! to — is its own surface with its own decisions, and belongs to a later stage.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Profiles {
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Deserialize)]
pub struct Profile {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tools: BTreeMap<String, ToolAssignment>,
}

#[derive(Debug, Deserialize)]
pub struct ToolAssignment {
    /// Claude Code has one endpoint behind every slot, so its provider is chosen once.
    pub endpoint: Option<String>,
    /// A slot names a model, or `null` for *no assignment* — an instruction in its own right,
    /// and what it does to a file differs by tool.
    #[serde(default)]
    pub slots: BTreeMap<String, Option<String>>,
}

impl Profiles {
    pub fn read(path: &Path) -> Result<Profiles, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn find(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }
}
