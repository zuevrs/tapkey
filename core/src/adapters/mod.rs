//! The tools tapkey manages, and the one thing they have in common.
//!
//! The trait below was **extracted from two finished adapters, never designed up front** — the map
//! held that line from its first page, on the grounds that an abstraction guessed from one shape is
//! the shape of that one thing wearing a general name.
//!
//! The two shapes are far apart. One has no registry and one endpoint behind every slot; the other
//! has a provider map with reserved ids inside it. One is strict JSON edited by our own
//! span-recording reader; the other is TOML edited by `toml_edit`, preserving by restoration where
//! we preserve by construction. One takes a credential through an `env` block it also uses for
//! models; the other has no `env` block at all. One accepts a provider speaking any protocol behind
//! its endpoint; the other accepts exactly one. One re-serialises its whole file on every write; the
//! other keeps the bytes but can lose ours to a race.
//!
//! Almost none of that survives, and the interface is small because of it: **name yourself, say
//! which file you own, read your state, plan a switch, and hash what you wrote.** Everything else
//! stayed in the adapters, which is the honest outcome the ticket said to leave available.
//!
//! What made the seam real was not the similarity. It was the deletion test: without it the core
//! enumerated its tools in five separate places, and one of them — the response after a restore —
//! was written before Codex existed and never extended. A list somebody eventually forgets is
//! exactly the complexity a seam is supposed to absorb.

pub mod claude;
pub mod codex;
pub mod opencode;

use crate::env::Env;
use crate::profile::{Provider, ToolAssignment};
use crate::transaction::Action;
use crate::wire::{Attention, ToolState};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// What every managed tool owes the core.
///
/// Errors arrive as a rendered string rather than a shared error type: the two adapters read
/// different formats and their failures are genuinely different things, and a common enum would
/// have to be the union of both — a lowest common denominator that makes each side worse. The core
/// only ever turns them into one refusal, so a string is the whole of what it needs.
pub trait Adapter {
    /// The tool's canonical name, as it appears on the wire.
    fn name(&self) -> &'static str;

    /// The file this tool's configuration lives in. The snapshot goes over every one of these
    /// before the first change, rather than over the files a switch happened to touch — with one
    /// tool those were the same set, and with two they are not.
    fn config_path(&self, env: &Env) -> PathBuf;

    /// What the tool will actually use, resolved across the scopes it consults.
    fn effective_state(&self, env: &Env) -> Result<ToolState, String>;

    /// The writes that apply an assignment, and any observation that coexists with success.
    ///
    /// Nothing is written here: the transaction owns that, so the all-or-nothing guarantee lives in
    /// one place. A tool that cannot participate returns no actions and an attention, which is how
    /// it leaves the transaction without cancelling it.
    fn plan_switch(
        &self,
        env: &Env,
        assignment: &ToolAssignment,
        provider: Option<&Provider>,
    ) -> Result<(Vec<Action>, Vec<Attention>), String>;

    /// Whether a slot may name a provider of its own.
    ///
    /// Only OpenCode can: measured, three of its slots resolved to three different providers in one
    /// config. Claude Code has one endpoint behind every slot and Codex one `model_provider` behind
    /// all five, so for them such an assignment is an instruction the tool cannot carry out. The
    /// default is `false` because that is the shape two tools out of three have, and a fourth
    /// claiming otherwise should have to say so.
    fn per_slot_providers(&self) -> bool {
        false
    }

    /// A hash per owned slot of what was just written, so drift has something to disagree with.
    /// Values never leave this function — the store keeps hashes, never what they were made from.
    fn fingerprint(&self, assignment: &ToolAssignment) -> BTreeMap<String, String>;
}

/// Every adapter, in the order tools appear on the wire.
///
/// One list, so that adding a third tool is one edit rather than five. This is the function the
/// missing seam cost: `restore` reported Claude Code alone for as long as Codex existed.
pub fn all() -> Vec<Box<dyn Adapter>> {
    vec![
        Box::new(claude::Claude),
        Box::new(codex::Codex),
        Box::new(opencode::OpenCode),
    ]
}

/// Every file the managed tools own, and which tool owns it.
pub fn managed_files(env: &Env) -> BTreeMap<PathBuf, &'static str> {
    all()
        .iter()
        .map(|adapter| (adapter.config_path(env), adapter.name()))
        .collect()
}

/// What every tool will actually use. A read that fails anywhere fails the whole answer: a partial
/// picture presented as a complete one is the thing "effective state over intent" forbids.
pub fn effective_state(env: &Env) -> Result<Vec<ToolState>, String> {
    all().iter().map(|a| a.effective_state(env)).collect()
}
