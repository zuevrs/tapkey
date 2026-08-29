//! Profiles, read from a file the core owns.
//!
//! Managing them — rename, duplicate, delete, and what happens to the one currently switched
//! to — is its own surface with its own decisions, and belongs to a later stage.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Deserialize, Serialize)]
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
#[derive(Debug, Deserialize, Serialize)]
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
    /// When Test last ran, as the store formats instants. Never interpreted: a result does not
    /// expire on its own, because expiry would move a provider from permitted to refused with
    /// nobody acting.
    #[serde(default)]
    pub tested_at: Option<String>,
}

fn yes() -> bool {
    true
}

/// What a profile says goes in one slot.
///
/// Measured on OpenCode 1.18.25: three slots named three different providers in one config, so a
/// slot may carry its own. Claude Code has one endpoint behind every slot and Codex one
/// `model_provider` behind all five, so for them a per-slot provider is an instruction the tool
/// cannot carry out — reported rather than obeyed, because effective state is about what the tool
/// will use.
#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SlotAssignment {
    /// `null`: no assignment, which is an instruction too.
    None_(Option<Nothing>),
    /// A bare model name, taking the tool's provider.
    Model(String),
    /// A model at a provider of this slot's own.
    AtProvider { provider: String, model: String },
}

/// Serde needs a type to fail on for the `null` arm to be distinguishable from the others.
#[derive(Debug, Deserialize, Serialize)]
pub enum Nothing {}

impl SlotAssignment {
    /// The model, or `None` for no assignment.
    pub fn model(&self) -> Option<&str> {
        match self {
            SlotAssignment::None_(_) => None,
            SlotAssignment::Model(m) => Some(m),
            SlotAssignment::AtProvider { model, .. } => Some(model),
        }
    }

    /// The provider this slot asked for, if it asked for one of its own.
    pub fn provider(&self) -> Option<&str> {
        match self {
            SlotAssignment::AtProvider { provider, .. } => Some(provider),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Profile {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tools: BTreeMap<String, ToolAssignment>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ToolAssignment {
    /// The **id** of a provider in the same file. Claude Code has one endpoint behind every slot
    /// and Codex one `model_provider` behind all five, so a provider is chosen once per tool.
    /// It is the default for any slot that did not name one of its own, which only OpenCode can.
    pub provider: Option<String>,
    /// A slot names a model, or `null` for *no assignment* — an instruction in its own right,
    /// and what it does to a file differs by tool. It may also name its own provider, which only
    /// OpenCode can honour; see `SlotAssignment`.
    #[serde(default)]
    pub slots: BTreeMap<String, SlotAssignment>,
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
