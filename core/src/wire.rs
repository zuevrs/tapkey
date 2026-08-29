//! The one envelope, in and out.

use serde::{Deserialize, Serialize};

/// A request. `version` is required and an unknown one is refused rather than best-effort
/// parsed: the app links the core statically, so a mismatch there is a broken build and should
/// be loud, and the real mismatches will come from an old CLI invocation in somebody's Makefile.
#[derive(Debug, Deserialize)]
pub struct Envelope {
    pub version: u32,
    #[serde(flatten)]
    pub request: Request,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", content = "params", rename_all = "snake_case")]
pub enum Request {
    /// What is every managed tool actually using right now.
    EffectiveState {},
    /// Apply a profile to every tool it covers, all or nothing.
    Switch { profile_id: String },
    /// Go back to a stored state. The target is tagged rather than inferred from the shape of
    /// an id: "snapshot cannot look like a timestamp" is a guard that lasts exactly until the
    /// format changes.
    Restore {
        #[serde(flatten)]
        target: RestoreTarget,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum RestoreTarget {
    Snapshot,
    Backup { id: String },
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Response {
    Ok {
        ok: bool,
        /// How a switch ended. Absent for a read, which does not end anything.
        #[serde(skip_serializing_if = "Option::is_none")]
        outcome: Option<&'static str>,
        tools: Vec<ToolState>,
    },
    /// A rollback undid work already done, which a refusal did not — the difference matters to
    /// the person reading it, so it is named rather than inferred from an error.
    Failed {
        ok: bool,
        outcome: &'static str,
        failure: Failure,
    },
    Refused {
        ok: bool,
        failure: Failure,
    },
}

#[derive(Debug, Serialize)]
pub struct Failure {
    pub kind: &'static str,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct ToolState {
    pub tool: &'static str,
    /// Claude Code has one endpoint behind every slot, so its provider is chosen once rather
    /// than per slot. It is not a Slot: the glossary reserves that word for where a model goes.
    pub endpoint: Resolved,
    pub slots: Vec<SlotState>,
    /// Observations about this tool that coexist with a successful outcome. A switch can apply
    /// while one tool shows drift or was skipped; merging these into the failure list would force
    /// every consumer to read "it worked, but note this" as an error.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attentions: Vec<Attention>,
}

/// A closed set, because the catalogue is the closed set of what a UI can render: an open code
/// would let the core return something no string exists for. Placeholders are typed fields per
/// variant rather than a map, so a consumer never discovers at runtime whether one is there.
#[derive(Debug, Serialize)]
pub struct Attention {
    pub kind: &'static str,
    /// The file the observation is about, where there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// The key that caused it, where there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SlotState {
    pub slot: &'static str,
    /// Owned means tapkey writes it; observed means tapkey reports it and never writes it.
    pub owned: bool,
    /// Changed by something other than tapkey since tapkey last wrote it. Defined on the slot
    /// and not on the file: the tool re-serialises its whole settings file constantly, so a
    /// file-level signal would fire after any action it took and stop being read.
    pub drifted: bool,
    #[serde(flatten)]
    pub resolved: Resolved,
}

/// One value, and every place that had an opinion about it, highest precedence first.
#[derive(Debug, Serialize)]
pub struct Resolved {
    pub effective: Option<String>,
    pub chain: Vec<Link>,
}

#[derive(Debug, Serialize)]
pub struct Link {
    /// Where this opinion came from, in terms a person can act on: a path, or `shell`.
    pub source: String,
    pub scope: &'static str,
    /// The variable or settings key this link read. Two links can name the same file and mean
    /// different things — the main model has both an ANTHROPIC_MODEL and a `model` opinion —
    /// so which file decided this is only half the question.
    pub key: String,
    pub value: Option<String>,
    /// False where tapkey cannot see what is there — a command line it did not run, a cloud
    /// session it has no access to. Such a scope is named rather than omitted, and can never
    /// be said to win.
    pub observable: bool,
    /// Present only where a gate decides whether the scope counts at all, and `false` when it is
    /// shut. Codex's project layer is read only if the *user's* config trusts the repository root,
    /// and when it does not, the tool ignores the file **and says nothing** — so this is the one
    /// place where a file that lost is worth showing, because the person can open the gate.
    /// A scope the tool simply never consults for a key is omitted instead; showing it as having
    /// lost would invite editing a file that was never involved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trusted: Option<bool>,
    pub wins: bool,
}
