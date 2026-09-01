//! OpenCode.
//!
//! The third shape, and the one that tests the trait. Where Claude Code has no registry and one
//! endpoint behind every slot, and Codex has a provider map with reserved ids, OpenCode has a
//! registry with **no** reserved ids, namespaced model ids, and slots that genuinely name different
//! providers — measured, three of them resolved to three different providers in one config.
//!
//! Three things about reading it were measured and two contradicted the note this was designed
//! from. All three global config files are **read and merged**, `.jsonc` highest, so an adapter
//! reading one of them reports the absence of keys that are plainly there. The project layer applies
//! with **no gate at all**, the opposite of Codex, so a repository somebody cloned can redirect
//! their requests from the moment the tool starts in it. And the model picker's choice lives in the
//! tool's own **SQLite database**, not the JSON file the research named — so it is reported as
//! unreadable rather than guessed at, because another program's schema is a coupling with no
//! contract, inside a read that is supposed to be honest.

use crate::env::Env;
use crate::json::{Document, Error};
use crate::profile::Provider;
use crate::transaction::Action;
use crate::wire::{Link, Resolved, SlotState, ToolState};
use std::path::PathBuf;

/// The three global files, **highest precedence first**. All are read; a write picks the first that
/// exists. That asymmetry is the tool's, not an awkwardness of ours.
pub const GLOBAL_FILES: [&str; 3] = ["opencode.jsonc", "opencode.json", "config.json"];

/// The one key whose absence makes the tool rewrite the file on a plain read — stripping a BOM and
/// inserting an LF into a CRLF file on the way. Measured: with it present, no write at all.
pub const SCHEMA_KEY: &str = "$schema";
pub const SCHEMA_URL: &str = "https://opencode.ai/config.json";

struct SlotSpec {
    name: &'static str,
    owned: bool,
    path: &'static [&'static str],
}

/// `model` and `small_model` are owned outright: measured, `/models` writes to the tool's own state
/// rather than to config, so OpenCode is the only one of the three that never contests them.
/// Per-agent and per-command slots are owned **only where no markdown file defines them** — a file
/// beats the config key and nothing beats the file — and that is decided per agent at read time.
const SLOTS: &[SlotSpec] = &[
    SlotSpec {
        name: "main",
        owned: true,
        path: &["model"],
    },
    SlotSpec {
        name: "utility",
        owned: true,
        path: &["small_model"],
    },
];

/// A scope tapkey cannot look into. Named rather than omitted: saying nothing would claim knowledge
/// we do not have, which is the failure "effective state over intent" exists to prevent.
struct Unseen {
    scope: &'static str,
    why: &'static str,
}

/// Highest first. The picker outranks every file because it is what the person last chose, and the
/// console tier sits above even the project layer — so it can overrule what somebody wrote in their
/// own repository, and they will not learn that from any file.
const UNSEEN: &[Unseen] = &[
    Unseen {
        scope: "picker",
        why: "the tool's own database, not a config file",
    },
    Unseen {
        scope: "console",
        why: "fetched over the network from your organisation",
    },
];

pub fn effective_state(env: &Env) -> Result<ToolState, Error> {
    let files = read_all(env)?;
    let state = crate::fingerprint::State::read(&env.store().join("state.json"));

    let slots = SLOTS
        .iter()
        .map(|spec| {
            let resolved = resolve(&files, spec.path);
            SlotState {
                slot: spec.name,
                owned: spec.owned,
                drifted: spec.owned
                    && state.drifted("opencode", spec.name, resolved.effective.as_deref()),
                resolved,
            }
        })
        .collect();

    Ok(ToolState {
        tool: "opencode",
        endpoint: endpoint(&files),
        slots,
        applies: crate::wire::ApplyMode::Next,
        attentions: Vec::new(),
    })
}

struct Layer {
    scope: &'static str,
    path: PathBuf,
    document: Document,
}

struct Files(Vec<Layer>);

/// Every config file that exists, highest precedence first.
///
/// The project layer comes first and needs nothing granted, which is the whole difference from
/// Codex. `~/.opencode` is a live configuration directory the tool creates beside its own binary,
/// and it outranks every project directory — an enumeration that omits it lies.
fn read_all(env: &Env) -> Result<Files, Error> {
    let mut out = Vec::new();
    let push = |scope: &'static str, path: PathBuf, out: &mut Vec<Layer>| -> Result<(), Error> {
        if let Ok(bytes) = std::fs::read(&path) {
            // JSONC for every one of them: measured, the tolerance belongs to the tool rather than
            // to the file's extension, and a comment in `opencode.json` resolves fine.
            out.push(Layer {
                scope,
                path,
                document: Document::parse_jsonc(&bytes)?,
            });
        }
        Ok(())
    };

    if let Some(project) = env.project() {
        for name in GLOBAL_FILES {
            push("project", project.join(name), &mut out)?;
        }
    }
    for name in GLOBAL_FILES {
        push("install", env.home().join(".opencode").join(name), &mut out)?;
    }
    for name in GLOBAL_FILES {
        push(
            "user",
            env.home().join(".config").join("opencode").join(name),
            &mut out,
        )?;
    }
    Ok(Files(out))
}

/// The file tapkey writes: the first of the three that exists, at user level, and `.jsonc` when
/// none does. Writing where the tool writes means the question of precedence between us never
/// arises; writing anywhere else would create a second file the person did not have.
pub fn config_path(env: &Env) -> PathBuf {
    let dir = env.home().join(".config").join("opencode");
    for name in GLOBAL_FILES {
        let candidate = dir.join(name);
        if candidate.exists() {
            return candidate;
        }
    }
    dir.join(GLOBAL_FILES[0])
}

fn endpoint(files: &Files) -> Resolved {
    // The endpoint belongs to a provider entry, and which provider a slot uses comes from the
    // namespaced model id — so the tool-level endpoint is the one the main model resolves through.
    let selected = files
        .0
        .iter()
        .find_map(|l| l.document.get_string(&["model"]))
        .and_then(|id| id.split('/').next().map(str::to_owned));

    let mut chain = Vec::new();
    for layer in &files.0 {
        let value = selected.as_ref().and_then(|id| {
            layer
                .document
                .get_string(&["provider", id, "options", "baseURL"])
                .map(str::to_owned)
        });
        chain.push(Link {
            source: super::wire_path(&layer.path),
            scope: layer.scope,
            key: "provider.<id>.options.baseURL".to_string(),
            value,
            observable: true,
            trusted: None,
            wins: false,
        });
    }
    settle(chain)
}

fn resolve(files: &Files, path: &[&str]) -> Resolved {
    let mut chain: Vec<Link> = UNSEEN
        .iter()
        .map(|u| Link {
            source: u.why.to_string(),
            scope: u.scope,
            key: String::new(),
            value: None,
            observable: false,
            trusted: None,
            wins: false,
        })
        .collect();

    for layer in &files.0 {
        chain.push(Link {
            source: super::wire_path(&layer.path),
            scope: layer.scope,
            key: path.join("."),
            value: layer.document.get_string(path).map(str::to_owned),
            observable: true,
            trusted: None,
            wins: false,
        });
    }
    settle(chain)
}

/// The first link with a value wins. An unobservable scope can never win: it has no value to offer,
/// only a statement that we could not look.
fn settle(mut chain: Vec<Link>) -> Resolved {
    let mut effective = None;
    for link in chain.iter_mut() {
        if effective.is_none()
            && let Some(value) = &link.value
        {
            link.wins = true;
            effective = Some(value.clone());
        }
    }
    Resolved { effective, chain }
}

/// Every id tapkey writes is prefixed. OpenCode has **no** reserved list — a config entry simply
/// deep-merges over a built-in of the same id — so the constraint is self-imposed, and worth
/// imposing: merging into somebody else's provider is harder to reason about and harder to undo
/// than adding beside it, and it leaves nobody able to see where their configuration ends and ours
/// begins. One rule across two tools beats two similar ones.
pub const ID_PREFIX: &str = "tapkey-";

pub fn table_id(provider_id: &str) -> String {
    format!("{ID_PREFIX}{provider_id}")
}

/// The npm package **is** the protocol here — there is no config key for it. `@ai-sdk/openai`
/// speaks Responses, `@ai-sdk/openai-compatible` speaks Chat Completions, which is how OpenCode
/// reaches the large population of gateways Codex cannot reach at all. An untested provider takes
/// the compatible one: erring toward the wider protocol is the cheaper mistake.
fn npm_package(provider: &crate::profile::Provider) -> &'static str {
    match &provider.formats {
        Some(formats) if formats.iter().any(|f| f == "openai_responses") => "@ai-sdk/openai",
        _ => "@ai-sdk/openai-compatible",
    }
}

/// Where tapkey keeps a credential it cannot put in a keychain. ADR-0007: OpenCode has neither a
/// keyring nor a command-backed credential, so the key is on disk in a file we own at `0600`.
pub fn credential_path(env: &Env, provider_id: &str) -> PathBuf {
    env.store().join("keys").join(provider_id)
}

pub fn plan_switch(
    env: &Env,
    assignment: &crate::profile::ToolAssignment,
    provider: Option<&crate::profile::Provider>,
) -> Result<(Vec<crate::transaction::Action>, Vec<crate::wire::Attention>), Error> {
    let path = config_path(env);
    let existing = std::fs::read(&path).unwrap_or_else(|_| b"{}".to_vec());
    let mut document = Document::parse_jsonc(&existing)?;

    // The one key whose absence makes the tool rewrite this file on every plain read, stripping a
    // BOM and inserting an LF into a CRLF file. It is the first top-level key tapkey writes that no
    // profile asked for, and it is written because it protects what we promised to preserve.
    document.set_string(&[SCHEMA_KEY], SCHEMA_URL)?;

    if let Some(provider) = provider {
        let id = table_id(&provider.id);
        document.set_string(&["provider", &id, "npm"], npm_package(provider))?;
        document.set_string(&["provider", &id, "name"], &provider.name)?;
        document.set_string(&["provider", &id, "options", "baseURL"], &provider.base_url)?;
        // A path, never a key. When the config breaks the tool prints the whole of it, so an inline
        // credential leaks and a reference discloses a path instead.
        document.set_string(
            &["provider", &id, "options", "apiKey"],
            &format!(
                "{{file:{}}}",
                super::wire_path(&credential_path(env, &provider.id))
            ),
        )?;
        // Appended only where the list already exists: creating one would turn "no restriction"
        // into "exactly one" and disable every provider not named in it.
        document.push_string(&["enabled_providers"], &id)?;

        for slot in SLOTS {
            if let Some(model) = assignment.slots.get(slot.name).and_then(|a| a.model()) {
                let slot_provider = assignment
                    .slots
                    .get(slot.name)
                    .and_then(|a| a.provider())
                    .unwrap_or(provider.id.as_str());
                let namespaced = format!("{}/{model}", table_id(slot_provider));
                document.set_string(slot.path, &namespaced)?;
                // Present, and nothing claimed about it: `limit` needs a number we do not have.
                document.ensure_object(&["provider", &table_id(slot_provider), "models", model])?;
            }
        }
    }

    Ok((
        vec![crate::transaction::Action::Write {
            path,
            bytes: document.to_bytes(),
            mode: 0o600,
        }],
        Vec::new(),
    ))
}

/// What tapkey has just written, so drift has something to disagree with.
///
/// Hashed on the **namespaced** value, because that is what the file holds and what a comparison
/// reads back: a slot moved to another provider is a change even when the model name is the same.
pub fn fingerprint(
    assignment: &crate::profile::ToolAssignment,
) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for slot in SLOTS.iter().filter(|s| s.owned) {
        let Some(model) = assignment.slots.get(slot.name).and_then(|a| a.model()) else {
            continue;
        };
        let provider = assignment
            .slots
            .get(slot.name)
            .and_then(|a| a.provider())
            .or(assignment.provider.as_deref());
        if let Some(provider) = provider {
            out.insert(
                slot.name.to_string(),
                crate::fingerprint::hash(&format!("{}/{model}", table_id(provider))),
            );
        }
    }
    out
}

/// The adapter, as the core sees it.
pub struct OpenCode;

impl super::Adapter for OpenCode {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn config_path(&self, env: &Env) -> PathBuf {
        config_path(env)
    }

    fn per_slot_providers(&self) -> bool {
        true
    }

    fn known_providers(&self, env: &Env) -> Vec<super::KnownProvider> {
        known_providers(env)
    }

    fn plan_removal(&self, env: &Env, provider: &Provider) -> Result<Vec<Action>, String> {
        plan_removal(env, provider)
    }

    fn install_paths(&self) -> Vec<std::path::PathBuf> {
        // Measured on this machine: the installer puts the binary here, not on the PATH.
        vec![dirs_home().join(".opencode").join("bin")]
    }

    fn effective_state(&self, env: &Env) -> Result<ToolState, String> {
        effective_state(env).map_err(|e| format!("{e:?}"))
    }

    fn plan_switch(
        &self,
        env: &Env,
        assignment: &crate::profile::ToolAssignment,
        provider: Option<&crate::profile::Provider>,
    ) -> Result<(Vec<crate::transaction::Action>, Vec<crate::wire::Attention>), String> {
        plan_switch(env, assignment, provider).map_err(|e| format!("{e:?}"))
    }

    fn fingerprint(
        &self,
        assignment: &crate::profile::ToolAssignment,
    ) -> std::collections::BTreeMap<String, String> {
        fingerprint(assignment)
    }
}

/// Every path tapkey may write, for the golden harness's statement of `merge-never-own`.
///
/// The provider entry is listed whole: an entry we created is ours entirely while every key in it
/// is ours, and the harness decides that per case rather than trusting an edit log. `$schema` is
/// listed too — tapkey writes it, so it is ours by the same rule that makes `wire_api` ours in a
/// Codex entry we created.
pub fn owned_paths(provider_id: &str) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = vec![
        vec![SCHEMA_KEY.to_string()],
        vec!["provider".to_string(), table_id(provider_id)],
        vec!["enabled_providers".to_string()],
    ];
    for slot in SLOTS {
        out.push(slot.path.iter().map(|s| (*s).to_string()).collect());
    }
    out
}

/// The merged user-level files, project directories excluded: harvesting a repository's providers
/// would adopt somebody else's configuration as the person's own.
fn known_providers(env: &Env) -> Vec<super::KnownProvider> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    // Highest precedence last, so a duplicate id from a lower file cannot overwrite a higher one.
    for scope in ["user", "install"] {
        for name in GLOBAL_FILES {
            let path = match scope {
                "install" => env.home().join(".opencode").join(name),
                _ => env.home().join(".config").join("opencode").join(name),
            };
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Ok(document) = Document::parse_jsonc(&bytes) else {
                continue;
            };
            for id in document.keys_at(&["provider"]) {
                if !seen.insert(id.clone()) {
                    continue;
                }
                let Some(base_url) = document.get_string(&["provider", &id, "options", "baseURL"])
                else {
                    continue;
                };
                let credential = match document.get_string(&["provider", &id, "options", "apiKey"])
                {
                    // A reference the person pointed their **tool** at: shown a path is not being
                    // given permission, and the value is never read.
                    Some(key) if key.starts_with("{env:") || key.starts_with("{file:") => {
                        super::CredentialSource::Referenced
                    }
                    Some(_) => super::CredentialSource::Inline,
                    None => super::CredentialSource::Absent,
                };
                out.push(super::KnownProvider {
                    id: id.clone(),
                    base_url: base_url.to_string(),
                    credential,
                });
            }
        }
    }
    out
}

/// Selected means the namespaced `model` names our provider. Removal takes the registry object out
/// and our id out of an existing `enabled_providers` list.
fn plan_removal(env: &Env, provider: &Provider) -> Result<Vec<Action>, String> {
    let path = config_path(env);
    let existing = std::fs::read(&path).unwrap_or_else(|_| b"{}".to_vec());
    let mut document = Document::parse_jsonc(&existing).map_err(|e| format!("{e:?}"))?;
    if let Some(model) = document.get_string(&["model"])
        && model.starts_with(&format!("{}/", table_id(&provider.id)))
    {
        return Err("OpenCode".to_string());
    }
    document
        .remove(&["provider", &table_id(&provider.id)])
        .map_err(|e| format!("{e:?}"))?;
    document
        .remove_from_array(&["enabled_providers"], &table_id(&provider.id))
        .map_err(|e| format!("{e:?}"))?;
    Ok(vec![Action::Write {
        path,
        bytes: document.to_bytes(),
        mode: 0o600,
    }])
}

/// `$HOME`, without a dependency for one lookup.
fn dirs_home() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
}
