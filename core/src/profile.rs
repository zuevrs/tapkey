//! Profiles, read from a file the core owns.
//!
//! Managing them — rename, duplicate, delete, and what happens to the one currently switched
//! to — is its own surface with its own decisions, and belongs to a later stage.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Profiles {
    /// Providers live beside profiles rather than in a file of their own: their retention is the
    /// same, neither holds a secret, both are read on every switch, and a profile referring to a
    /// provider by id then closes in one atomic write instead of inventing a dangling reference
    /// as a new failure state. See `issues/14-a-provider-is-not-universal.md`.
    #[serde(default)]
    pub providers: Vec<Provider>,
    pub profiles: Vec<Profile>,
}

/// An endpoint that serves models, together with the credential used to reach it — an entity with
/// an identifier, not a URL a profile carries.
#[derive(Debug, Deserialize)]
pub struct Provider {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub base_url: String,
    /// The wire protocols this endpoint answers. `None` means **untested**, which is not the same
    /// as an empty set: an untested provider is permitted with an attention, a tested one that
    /// serves nothing we support is not. Only a Test fills this, and a Test establishes all of
    /// them at once, so partial knowledge is a state that cannot arise.
    #[serde(default)]
    pub formats: Option<Vec<String>>,
    /// ADR-0013 keeps every tool's registry filled with every enabled provider, which is why the
    /// core needs to see the ones no profile mentions.
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
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
    /// The **id** of a provider in the same file. Claude Code has one endpoint behind every slot
    /// and Codex one `model_provider` behind all five, so a provider is chosen once per tool.
    /// Reported per tool on the wire for the same reason; OpenCode's per-slot providers are not
    /// built, and shaping this around an unmeasured third case would be guessing.
    pub provider: Option<String>,
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

    pub fn provider(&self, id: &str) -> Option<&Provider> {
        self.providers.iter().find(|p| p.id == id)
    }
}
