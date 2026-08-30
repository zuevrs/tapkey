//! Codex.
//!
//! Where Claude Code hides its model slots in environment variables, Codex puts everything in one
//! TOML file and lets **no** environment variable choose a model or a provider. That makes its
//! effective state very nearly a function of files, which is what lets the adapter resolve the
//! stack itself instead of asking the tool — `codex doctor` costs real network and up to a minute,
//! and `codex exec` starts a session.
//!
//! Two things about that stack were measured against 0.150.1 and contradicted the research note
//! this was designed from. There is **no walk up the directory tree**: outside a git repository
//! only the current directory's `.codex/config.toml` is read, and inside one the repository root's
//! is read as well, with the current directory's winning. And the trust gate keys on the
//! **repository root**, not on the directory — trusting the root is enough to activate a
//! subdirectory's file. When it is shut, the project file is ignored in total silence.

use crate::env::Env;
use crate::profile::{Provider, ToolAssignment};
use crate::toml::{Document, Error};
use crate::transaction::Action;
use crate::wire::{Attention, Link, Resolved, SlotState, ToolState};
use std::path::PathBuf;

/// The layers this adapter can see, highest precedence first. The managed tiers below user config
/// and the enterprise cloud bundle are named in the chain rather than read; see `unseen`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// `.codex/config.toml` in the working directory, then the repository root's.
    Project,
    /// `$CODEX_HOME/config.toml`.
    User,
}

/// Keys a project-level file may not carry. The mechanism is **removal from the layer** rather
/// than losing a contest, so a denied key cannot even shadow a lower one — which is why a slot
/// whose key is on this list has no project entry in its chain at all. Twelve of them, measured;
/// the official documentation lists ten.
const PROJECT_DENYLIST: &[&str] = &[
    "model_provider",
    "model_providers",
    "profile",
    "profiles",
    "openai_base_url",
    "chatgpt_base_url",
    "apps_mcp_product_sku",
    "responses_api_metadata",
    "notify",
    "experimental_realtime_webrtc_call_base_url",
    "experimental_realtime_ws_base_url",
    "otel",
];

struct SlotSpec {
    name: &'static str,
    owned: bool,
    /// The key path in `config.toml`. Its first step is what the denylist is checked against.
    path: &'static [&'static str],
}

/// Five slots, not the one ADR-0013 recorded before they were measured. `review_model` and the
/// subagent slot inherit the session model when unset, so leaving them alone is safe; the two
/// `memories.*` slots fall back to hard-coded OpenAI slugs, which is why they are owned.
const SLOTS: &[SlotSpec] = &[
    SlotSpec {
        name: "main",
        owned: true,
        path: &["model"],
    },
    SlotSpec {
        name: "review",
        owned: true,
        path: &["review_model"],
    },
    SlotSpec {
        name: "subagent",
        owned: true,
        path: &["agents", "default_subagent_model"],
    },
    SlotSpec {
        name: "utility",
        owned: true,
        path: &["memories", "extract_model"],
    },
    // The utility slot is one slot reached through two keys. Both are written whenever it is
    // assigned, unconditionally on `features.memories`: left unset they fall back to hard-coded
    // OpenAI slugs, which after a switch are requested from the new provider with the new key and
    // fail in silence. Writing conditionally would make the feature flag an input to the switch and
    // leave a leak acquirable later with no file change tapkey would ever see.
    SlotSpec {
        name: "utility",
        owned: true,
        path: &["memories", "consolidation_model"],
    },
    SlotSpec {
        name: "effort",
        owned: true,
        path: &["model_reasoning_effort"],
    },
    SlotSpec {
        name: "verbosity",
        owned: false,
        path: &["model_verbosity"],
    },
];

/// Read what Codex will actually use.
pub fn effective_state(env: &Env) -> Result<ToolState, Error> {
    let files = read_all(env)?;

    let state = crate::fingerprint::State::read(&env.store().join("state.json"));

    let endpoint = endpoint(&files);
    let slots = SLOTS
        .iter()
        .map(|spec| {
            let resolved = resolve(&files, spec.path);
            SlotState {
                slot: spec.name,
                owned: spec.owned,
                // Per slot, not per file — for the opposite reason to Claude Code's. Codex keeps
                // the file's bytes across its own writes, but it writes the very keys tapkey owns,
                // so a file-level signal would fire on an unrelated `mcp add`. Its inode and mtime
                // change on every write including a repeat, so neither is a signal either.
                drifted: spec.owned
                    && state.drifted("codex", spec.name, resolved.effective.as_deref()),
                resolved,
            }
        })
        .collect();

    Ok(ToolState {
        tool: "codex",
        endpoint,
        slots,
        attentions: Vec::new(),
    })
}

/// Every id tapkey writes is prefixed, **always**, not on collision.
///
/// Codex rejects its built-in provider ids as reserved — `model_providers` holding `openai` is a
/// hard config-load error naming the collision. Prefixing only when the name clashes would tie our
/// table's name to a list that can grow: OpenAI reserves one more id and a table already written on
/// somebody's disk becomes illegal, a migration on live machines caused by another company's
/// release. It also makes ownership visible to whoever opens the file by hand.
pub const ID_PREFIX: &str = "tapkey-";

pub fn table_id(provider_id: &str) -> String {
    format!("{ID_PREFIX}{provider_id}")
}

/// The path to Codex's user config. tapkey creates it, and `~/.codex/` with it, when absent:
/// measured, Codex does not create a config file it did not find, so an installed-but-unconfigured
/// tool is an ordinary state — and it is exactly the case this app exists for. A missing *binary*
/// is `tool_gone` and a skip; the two absences are different.
pub fn config_path(env: &Env) -> PathBuf {
    env.home().join(".codex").join("config.toml")
}

/// `wire_api` has exactly one legal value at 0.150.1: `"chat"` is a hard config-load error and
/// there is no fallback, so Codex speaks the Responses API or nothing.
const WIRE_API: &str = "responses";

/// Turn one tool's assignment into the single write that applies it, or into a reason not to.
///
/// Nothing is written here; the transaction owns that, so the all-or-nothing guarantee lives in
/// one place.
pub fn plan_switch(
    env: &Env,
    assignment: &ToolAssignment,
    provider: Option<&Provider>,
) -> Result<(Vec<Action>, Vec<Attention>), Error> {
    let path = config_path(env);
    let existing = std::fs::read(&path).unwrap_or_default();
    let mut document = Document::parse(&existing)?;

    // A `profile` key anywhere in the merged config makes Codex refuse to start, before anything
    // else runs. The file parses; it is fatal rather than broken. So the tool is skipped and the
    // key is named — repairing it is forbidden by merge-never-own, and writing over it would make
    // tapkey the last hand on a file that does not work.
    if document.get_string(&["profile"]).is_some() {
        return Ok((
            Vec::new(),
            vec![Attention {
                kind: "tool_will_not_start",
                file: Some(super::wire_path(&path)),
                key: Some("profile".into()),
            }],
        ));
    }

    if let Some(provider) = provider {
        let id = table_id(&provider.id);
        document.set_string(&["model_provider"], &id)?;
        document.set_string(&["model_providers", &id, "name"], &provider.name)?;
        document.set_string(&["model_providers", &id, "base_url"], &provider.base_url)?;
        document.set_string(&["model_providers", &id, "wire_api"], WIRE_API)?;
    }

    for spec in SLOTS.iter().filter(|s| s.owned) {
        // Ownership is per assignment, not per key name: a slot the profile says nothing about is
        // left alone. Two of Codex's four non-main slots inherit the session model when unset, and
        // pinning them would freeze what moves along for free.
        let Some(assigned) = assignment.slots.get(spec.name) else {
            continue;
        };
        match assigned.model() {
            Some(model) => document.set_string(spec.path, model)?,
            // Nothing fires from the environment underneath in Codex, so deleting is enough —
            // unlike Claude Code, where ADR-0014 needs an empty value written over a shell export.
            None => document.remove(spec.path)?,
        }
    }

    Ok((
        vec![Action::Write {
            path,
            bytes: document.to_bytes(),
            // A file tapkey creates is created at 0600; an existing one keeps the mode it had.
            // Codex tightens it to 0600 on every write of its own, and that is Codex's business:
            // ADR-0018 preserves rather than predicts what another program will do.
            mode: 0o600,
        }],
        Vec::new(),
    ))
}

/// One layer that was found on disk, with whether it counts.
struct Layer {
    scope: Scope,
    path: PathBuf,
    document: Document,
    /// `None` where no gate applies. `Some(false)` is a project file the user's config does not
    /// trust: present, readable, and ignored by the tool without a word.
    trusted: Option<bool>,
}

struct Files(Vec<Layer>);

fn read_all(env: &Env) -> Result<Files, Error> {
    let mut out = Vec::new();

    // Project first: it outranks the user layer for every key the denylist lets through.
    if let Some(project) = env.project() {
        let path = project.join(".codex").join("config.toml");
        if let Ok(bytes) = std::fs::read(&path) {
            out.push(Layer {
                scope: Scope::Project,
                path,
                document: Document::parse(&bytes)?,
                trusted: Some(false),
            });
        }
    }

    let user = env.home().join(".codex").join("config.toml");
    let user_document = match std::fs::read(&user) {
        Ok(bytes) => Some(Document::parse(&bytes)?),
        Err(_) => None,
    };

    // The gate lives in the *user's* file and names the project root, so it can only be answered
    // once that file is parsed — which is why trust is filled in here rather than above.
    if let (Some(document), Some(project)) = (&user_document, env.project()) {
        let trusted =
            document.get_string(&["projects", &project.display().to_string(), "trust_level"])
                == Some("trusted");
        for layer in out.iter_mut().filter(|l| l.scope == Scope::Project) {
            layer.trusted = Some(trusted);
        }
    }

    if let Some(document) = user_document {
        out.push(Layer {
            scope: Scope::User,
            path: user,
            document,
            trusted: None,
        });
    }
    Ok(Files(out))
}

fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::Project => "project",
        Scope::User => "user",
    }
}

/// The endpoint is `model_providers.<model_provider>.base_url`, and both halves are on the
/// denylist, so only the user layer can decide it.
fn endpoint(files: &Files) -> Resolved {
    let mut chain = Vec::new();
    for layer in files.0.iter().filter(|l| consulted(l, "model_provider")) {
        let value = layer
            .document
            .get_string(&["model_provider"])
            .and_then(|id| {
                layer
                    .document
                    .get_string(&["model_providers", id, "base_url"])
            })
            .map(str::to_owned);
        chain.push(Link {
            source: super::wire_path(&layer.path),
            scope: scope_name(layer.scope),
            key: "model_providers.<id>.base_url".to_string(),
            value,
            observable: true,
            trusted: layer.trusted,
            wins: false,
        });
    }
    settle(chain)
}

fn resolve(files: &Files, path: &[&str]) -> Resolved {
    let mut chain = Vec::new();
    for layer in files.0.iter().filter(|l| consulted(l, path[0])) {
        chain.push(Link {
            source: super::wire_path(&layer.path),
            scope: scope_name(layer.scope),
            key: path.join("."),
            value: layer.document.get_string(path).map(str::to_owned),
            observable: true,
            trusted: layer.trusted,
            wins: false,
        });
    }
    settle(chain)
}

/// Whether this layer is asked about this key at all. A project layer is not consulted for a
/// denied key — the key is stripped from it — and a scope that is never consulted is omitted
/// rather than shown as having lost, which would invite editing a file that was never involved.
/// An untrusted project layer **is** still listed, because the person can open that gate.
fn consulted(layer: &Layer, first_step: &str) -> bool {
    !(layer.scope == Scope::Project && PROJECT_DENYLIST.contains(&first_step))
}

/// The first link with a value, and that is trusted where trust applies, wins.
fn settle(mut chain: Vec<Link>) -> Resolved {
    let mut effective = None;
    for link in chain.iter_mut() {
        if link.trusted == Some(false) {
            continue;
        }
        if effective.is_none()
            && let Some(value) = &link.value
        {
            link.wins = true;
            effective = Some(value.clone());
        }
    }
    Resolved { effective, chain }
}

/// What tapkey has just written, ready to be recorded so drift has something to compare to.
///
/// One slot reaches two keys, so the fingerprint is keyed on the **slot** and both writes settle to
/// the same hash — which is what makes "the utility model was changed" one fact rather than two.
pub fn fingerprint(assignment: &ToolAssignment) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for spec in SLOTS.iter().filter(|s| s.owned) {
        if let Some(model) = assignment.slots.get(spec.name).and_then(|a| a.model()) {
            out.insert(spec.name.to_string(), crate::fingerprint::hash(model));
        }
    }
    out
}

/// Every path tapkey may write, for the golden harness's mechanical statement of
/// `merge-never-own`: `before` minus these, cut out, must equal `after` minus the same.
///
/// The provider entry is included as a whole table. A table tapkey created is ours entirely while
/// every key in it is ours — the rule stage one settled for a created `env` block — and the harness
/// decides that per case rather than trusting an edit log.
pub fn owned_paths(provider_id: Option<&str>) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = vec![vec!["model_provider".to_string()]];
    for spec in SLOTS.iter().filter(|s| s.owned) {
        out.push(spec.path.iter().map(|s| (*s).to_string()).collect());
    }
    if let Some(id) = provider_id {
        out.push(vec!["model_providers".to_string(), table_id(id)]);
    }
    // The tables our slots live in are ours too when we created them, and the harness cuts a
    // created table whole. `[memories]` holds only keys tapkey writes; `[agents]` does not
    // necessarily, so it is left to be cut key by key.
    out.push(vec!["memories".to_string()]);
    out
}

/// The adapter, as the core sees it.
pub struct Codex;

impl super::Adapter for Codex {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn config_path(&self, env: &Env) -> PathBuf {
        config_path(env)
    }

    fn effective_state(&self, env: &Env) -> Result<ToolState, String> {
        effective_state(env).map_err(|e| format!("{e:?}"))
    }

    fn plan_switch(
        &self,
        env: &Env,
        assignment: &ToolAssignment,
        provider: Option<&Provider>,
    ) -> Result<(Vec<Action>, Vec<Attention>), String> {
        plan_switch(env, assignment, provider).map_err(|e| format!("{e:?}"))
    }

    fn fingerprint(
        &self,
        assignment: &ToolAssignment,
    ) -> std::collections::BTreeMap<String, String> {
        fingerprint(assignment)
    }

    fn known_providers(&self, env: &Env) -> Vec<super::KnownProvider> {
        known_providers(env)
    }

    fn install_paths(&self) -> Vec<std::path::PathBuf> {
        vec![
            std::path::PathBuf::from("/opt/homebrew/bin"),
            std::path::PathBuf::from("/usr/local/bin"),
        ]
    }

    fn plan_removal(&self, env: &Env, provider: &Provider) -> Result<Vec<Action>, String> {
        plan_removal(env, provider)
    }
}

/// The registry **is** the harvest: every `[model_providers.<id>]` the person wrote, at user level.
fn known_providers(env: &Env) -> Vec<super::KnownProvider> {
    let bytes = match std::fs::read(config_path(env)) {
        Ok(bytes) => bytes,
        Err(_) => return Vec::new(),
    };
    let Ok(document) = Document::parse(&bytes) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for id in document.keys_at(&["model_providers"]) {
        let Some(base_url) = document.get_string(&["model_providers", &id, "base_url"]) else {
            continue;
        };
        // A key never lives in this file unencrypted by design; the env_key names a variable the
        // person points their tool at, which is a reference and never read.
        let credential = match document.get_string(&["model_providers", &id, "env_key"]) {
            Some(_) => super::CredentialSource::Referenced,
            None => super::CredentialSource::Absent,
        };
        out.push(super::KnownProvider {
            id: id.clone(),
            base_url: base_url.to_string(),
            credential,
        });
    }
    out
}

/// Selected means `model_provider` names our table. Removal takes the table out; a person's own
/// entry is never touched, because removal only ever targets ids we namespaced.
fn plan_removal(env: &Env, provider: &Provider) -> Result<Vec<Action>, String> {
    let path = config_path(env);
    let existing = std::fs::read(&path).unwrap_or_default();
    let mut document = Document::parse(&existing).map_err(|e| format!("{e:?}"))?;
    if document.get_string(&["model_provider"]) == Some(table_id(&provider.id).as_str()) {
        return Err("Codex".to_string());
    }
    document
        .remove(&["model_providers", &table_id(&provider.id)])
        .map_err(|e| format!("{e:?}"))?;
    Ok(vec![Action::Write {
        path,
        bytes: document.to_bytes(),
        // An existing file keeps its mode; the transaction records what it captured.
        mode: 0o600,
    }])
}
