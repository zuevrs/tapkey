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
}

#[derive(Debug, Serialize)]
pub struct SlotState {
    pub slot: &'static str,
    /// Owned means tapkey writes it; observed means tapkey reports it and never writes it.
    pub owned: bool,
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
    pub wins: bool,
}
