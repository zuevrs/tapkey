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
    Switch {
        profile_id: String,
    },
    /// Go back to a stored state. The target is tagged rather than inferred from the shape of
    /// an id: "snapshot cannot look like a timestamp" is a guard that lasts exactly until the
    /// format changes.
    Restore {
        #[serde(flatten)]
        target: RestoreTarget,
    },
    /// Establish which formats a provider's endpoint answers. Reads the network, writes the store.
    Test {
        provider_id: String,
    },
    /// List what the tools already know: the harvest offer. Reads other people's files and changes
    /// nothing, so it takes no lock.
    Harvest {},
    /// The store's profiles, for the panel's rows. A read, no lock.
    ListProfiles {},
    /// Take one candidate: the key is re-read from the tool's file at this moment and piped to the
    /// helper on stdin, so it lives in one buffer for one call and nowhere else.
    AcceptHarvest {
        tool: String,
        id: String,
    },
    /// Record that a candidate was declined, so an offer does not become a reminder. Reversible.
    DeclineHarvest {
        tool: String,
        id: String,
    },
    // -- Management. Operations on **profiles** write the store only; operations on **providers**
    // -- may touch the tools' files and are ADR-0005 verbatim.
    CreateProfile {
        profile: crate::profile::Profile,
    },
    RenameProfile {
        id: String,
        name: String,
    },
    DuplicateProfile {
        id: String,
        as_id: String,
    },
    DeleteProfile {
        id: String,
    },
    CreateProvider {
        id: String,
        name: String,
        base_url: String,
    },
    RenameProvider {
        id: String,
        name: String,
    },
    SetProviderEnabled {
        id: String,
        enabled: bool,
    },
    RemoveProvider {
        id: String,
    },
    /// Store a secret under a provider id, through the helper. The secret travels field →
    /// invoke → core → helper stdin: memory of one process, no disk, no log.
    SetCredential {
        provider_id: String,
        secret: String,
    },
    /// The store's providers, for the settings tab's cards. A read, no lock.
    ListProviders {},
    /// Which managed tools are on this machine, and which are configured. A read, no lock.
    ToolPresence {},
    /// The snapshot and every backup, for the History sheet. A read, no lock.
    ListHistory {},
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
        /// The id of the backup a successful switch took, so Undo can name it without browsing
        /// the store. Absent for a read, a refusal, and a rollback — the last changed nothing
        /// that survived to be undone.
        #[serde(skip_serializing_if = "Option::is_none")]
        backup: Option<String>,
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
    Tested {
        ok: bool,
        /// False when the method could not tell — and then `formats` is absent, because recording
        /// a verdict we cannot support is the failure the invariant names.
        knowable: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        formats: Option<Vec<FormatProbe>>,
        /// The reason nothing was recorded, when there is one.
        #[serde(skip_serializing_if = "Option::is_none")]
        because: Option<&'static str>,
        provider: String,
        tested_at: String,
    },
    Profiles {
        ok: bool,
        profiles: Vec<ProfileRow>,
    },
    Providers {
        ok: bool,
        providers: Vec<ProviderCard>,
    },
    Presence {
        ok: bool,
        tools: Vec<ToolPresence>,
    },
    History {
        ok: bool,
        entries: Vec<HistoryRow>,
    },
    Harvested {
        ok: bool,
        candidates: Vec<Candidate>,
        /// A profile describing what the tools hold now, so a first switch is reversible.
        #[serde(skip_serializing_if = "Option::is_none")]
        suggested_profile: Option<SuggestedProfile>,
    },
    Changed {
        ok: bool,
        /// What was managed, in the catalogue's vocabulary: a profile or a provider.
        what: &'static str,
        action: &'static str,
        id: String,
    },
    Accepted {
        ok: bool,
        provider: String,
        /// Raised at accept rather than at the first switch: measured, a token in Claude Code's
        /// `env` block outranks `apiKeyHelper`, so leaving the original in place is the condition
        /// of the transfer having happened.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        attentions: Vec<Attention>,
    },
}

/// One harvest candidate. `credential` names the kind of key seen, never its value.
#[derive(Debug, Serialize)]
pub struct Candidate {
    pub tool: &'static str,
    pub id: String,
    pub base_url: String,
    pub credential: &'static str,
    /// True when a provider with this id is already in the store. The person decides whether the
    /// two are one; nothing is merged and nothing is silently renamed.
    pub name_conflict: bool,
    pub declined: bool,
}

/// What the tools hold now, by slot — the profile harvest offers alongside its candidates.
#[derive(Debug, Serialize)]
pub struct SuggestedProfile {
    pub name: String,
    pub tools: Vec<SuggestedTool>,
}

#[derive(Debug, Serialize)]
pub struct SuggestedTool {
    pub tool: &'static str,
    pub provider: String,
    pub slots: Vec<(&'static str, Option<String>)>,
}

/// One format's answer. The names are the canonical three, and every one of them is reported —
/// an absent format is an answer too, and the useful one.
#[derive(Debug, Serialize)]
pub struct FormatProbe {
    pub format: &'static str,
    pub served: bool,
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

/// One panel row: what a profile is, not what a tool is using.
#[derive(Debug, Serialize)]
pub struct ProfileRow {
    pub id: String,
    pub name: String,
    /// How many tools this profile names. The row's scope line — `{count} of {total} tools`.
    pub tools: usize,
}

/// One settings card: what a provider is, never a secret's value.
#[derive(Debug, Serialize)]
pub struct ProviderCard {
    pub id: String,
    pub name: String,
    pub base_url: String,
    /// `None` is untested — the catalogue's `prov.apiFormat.unknown`, not an empty set.
    pub formats: Option<Vec<String>>,
    pub enabled: bool,
    pub tested_at: Option<String>,
}

/// One tool's presence: installed-ness is a fact about the machine, configured-ness about its
/// files, and onboarding shows them as separate chips.
#[derive(Debug, Serialize)]
pub struct ToolPresence {
    pub tool: &'static str,
    /// The binary is discoverable the way a shell would find it.
    pub installed: bool,
    /// tapkey found a config file of this tool's on the machine.
    pub configured: bool,
}

/// One restorable moment, as History lists it. The name is what was recorded, never a
/// dereference of a current profile — a history that follows renames lies retroactively.
#[derive(Debug, Serialize)]
pub struct HistoryRow {
    /// `snapshot` or `backup`; the sheet's own two sentences hang off this.
    pub kind: &'static str,
    /// The restore target's id, exactly as `restore` wants it back.
    pub id: String,
    /// The snapshot carries the catalogue's words; a backup, the profile name as it was.
    pub name: String,
    pub instant: String,
    pub restorable: bool,
    /// How many files the moment holds. The row's own count, from the manifest.
    pub files: u64,
}
