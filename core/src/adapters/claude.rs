//! Claude Code.
//!
//! A profile reaches this tool through an `env` block in `~/.claude/settings.json` and nowhere
//! else: there is no settings key for the base URL, and the model slots tapkey owns exist only
//! as environment variables. Reading it back means resolving, per slot, the places the tool
//! consults — which are not the same places in the same order for every slot.

use crate::env::{Env, ShellVar};
use crate::json::Document;
use crate::wire::{Attention, Link, Resolved, SlotState, ToolState};
use std::path::{Path, PathBuf};

/// One place a value can come from, in the order a slot consults them.
#[derive(Clone, Copy)]
enum Source {
    /// An environment variable: every settings file's `env` block, then the login shell. A
    /// settings-file `env` block replaces what the shell exported, measured, so the shell sits
    /// below every file rather than beside them.
    Var(&'static str),
    /// A key in the settings files themselves.
    Key(&'static [&'static str]),
    /// Somewhere tapkey cannot look.
    Unseen {
        scope: &'static str,
        why: &'static str,
    },
}

/// The files, highest precedence first. Command line sits between managed and the project
/// files and cannot be observed, so it is a source of its own rather than a file.
const FILES: [&str; 4] = ["managed", "project local", "project", "user"];

const CLOUD: Source = Source::Unseen {
    scope: "cloud session",
    why: "reads neither user settings nor the shell",
};

/// A command line tapkey did not run can carry `--settings` or a per-key flag. It is declared
/// per slot rather than inserted afterwards, because inserting is how a scope lands in the
/// wrong place the moment the one above it is absent.
const COMMAND_LINE: Source = Source::Unseen {
    scope: "command line",
    why: "--settings or a per-key flag",
};

/// `/model` during a session outranks even the command line, and only for the main model.
const SESSION: Source = Source::Unseen {
    scope: "session",
    why: "/model during a session",
};

struct SlotSpec {
    name: &'static str,
    owned: bool,
    sources: &'static [Source],
}

/// Each slot's own precedence. `ANTHROPIC_MODEL` outranks the `model` key while
/// `ANTHROPIC_DEFAULT_MODEL` sits below it; the deprecated small/fast variable still wins the
/// background path though it lost its name. None of that generalises, which is the point.
const SLOTS: &[SlotSpec] = &[
    SlotSpec {
        name: "main",
        owned: true,
        sources: &[
            SESSION,
            COMMAND_LINE,
            Source::Var("ANTHROPIC_MODEL"),
            Source::Key(&["model"]),
            Source::Var("ANTHROPIC_DEFAULT_MODEL"),
        ],
    },
    SlotSpec {
        name: "utility",
        owned: true,
        sources: &[
            COMMAND_LINE,
            Source::Var("ANTHROPIC_SMALL_FAST_MODEL"),
            Source::Var("ANTHROPIC_DEFAULT_HAIKU_MODEL"),
        ],
    },
    SlotSpec {
        name: "subagent",
        owned: true,
        sources: &[COMMAND_LINE, Source::Var("CLAUDE_CODE_SUBAGENT_MODEL")],
    },
    SlotSpec {
        name: "opus",
        owned: true,
        sources: &[COMMAND_LINE, Source::Var("ANTHROPIC_DEFAULT_OPUS_MODEL")],
    },
    SlotSpec {
        name: "sonnet",
        owned: true,
        sources: &[COMMAND_LINE, Source::Var("ANTHROPIC_DEFAULT_SONNET_MODEL")],
    },
    SlotSpec {
        name: "fable",
        owned: true,
        sources: &[COMMAND_LINE, Source::Var("ANTHROPIC_DEFAULT_FABLE_MODEL")],
    },
    SlotSpec {
        name: "advisor",
        owned: false,
        sources: &[COMMAND_LINE, Source::Key(&["advisorModel"])],
    },
    SlotSpec {
        name: "fallback",
        owned: false,
        sources: &[COMMAND_LINE, Source::Key(&["fallbackModel"])],
    },
];

pub const ENDPOINT_VAR: &str = "ANTHROPIC_BASE_URL";

const ENDPOINT: &[Source] = &[COMMAND_LINE, Source::Var(ENDPOINT_VAR)];

/// Read what Claude Code will actually use, resolved across the scopes it consults.
pub fn effective_state(env: &Env) -> Result<ToolState, crate::json::Error> {
    let files = read_all(env)?;
    let state = crate::fingerprint::State::read(&env.store().join("state.json"));

    let endpoint = resolve(env, &files, ENDPOINT);
    let slots = SLOTS
        .iter()
        .map(|spec| {
            let resolved = resolve(env, &files, spec.sources);
            SlotState {
                slot: spec.name,
                owned: spec.owned,
                drifted: spec.owned
                    && state.drifted("claude", spec.name, resolved.effective.as_deref()),
                resolved,
            }
        })
        .collect();

    Ok(ToolState {
        tool: "claude",
        endpoint,
        slots,
        attentions: Vec::new(),
    })
}

/// Every settings file that exists, in precedence order, parsed once.
struct Files(Vec<(&'static str, PathBuf, Document)>);

fn read_all(env: &Env) -> Result<Files, crate::json::Error> {
    let mut out = Vec::new();
    for scope in FILES {
        let Some(path) = scope_path(env, scope) else {
            continue;
        };
        // A file that is not there is not an error: installing Claude Code creates none.
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        out.push((scope, path, Document::parse(&bytes)?));
    }
    Ok(Files(out))
}

fn resolve(env: &Env, files: &Files, sources: &[Source]) -> Resolved {
    let mut chain = Vec::new();
    for source in sources {
        match *source {
            Source::Unseen { scope, why } => chain.push(unseen(scope, why, "")),
            Source::Key(path) => {
                for (scope, file, doc) in &files.0 {
                    chain.push(Link {
                        source: display_path(file, env),
                        scope,
                        key: path.join("."),
                        value: doc.get_string(path).map(str::to_owned),
                        observable: true,
                        trusted: None,
                        wins: false,
                    });
                }
            }
            Source::Var(name) => {
                for (scope, file, doc) in &files.0 {
                    chain.push(Link {
                        source: display_path(file, env),
                        scope,
                        key: name.to_string(),
                        value: doc.get_string(&["env", name]).map(str::to_owned),
                        observable: true,
                        trusted: None,
                        wins: false,
                    });
                }
                chain.push(Link {
                    source: "login shell".to_string(),
                    scope: "shell",
                    key: name.to_string(),
                    // A withheld credential and an unset variable both read as no value. The
                    // difference is a credential concern, reported as an attention rather than
                    // by pretending to know a value we deliberately did not read.
                    value: match env.shell_var(name) {
                        Some(ShellVar::Value(v)) => Some(v.clone()),
                        Some(ShellVar::SetButWithheld) | None => None,
                    },
                    observable: true,
                    trusted: None,
                    wins: false,
                });
            }
        }
    }
    chain.push(unseen_from(CLOUD));

    settle(&mut chain);
    Resolved {
        effective: winning_value(&chain),
        chain,
    }
}

fn unseen(scope: &'static str, why: &str, key: &str) -> Link {
    Link {
        source: why.to_string(),
        scope,
        key: key.to_string(),
        value: None,
        observable: false,
        trusted: None,
        wins: false,
    }
}

fn unseen_from(source: Source) -> Link {
    match source {
        Source::Unseen { scope, why } => unseen(scope, why, ""),
        _ => unreachable!("only an Unseen source becomes an unseen link"),
    }
}

fn scope_path(env: &Env, scope: &str) -> Option<PathBuf> {
    match scope {
        "managed" => Some(env.managed().to_path_buf()),
        "project local" => Some(env.project()?.join(".claude").join("settings.local.json")),
        "project" => Some(env.project()?.join(".claude").join("settings.json")),
        "user" => Some(env.home().join(".claude").join("settings.json")),
        _ => None,
    }
}

/// Paths are reported as the person would type them, so a chain reads like their filesystem.
fn display_path(path: &Path, env: &Env) -> String {
    match path.strip_prefix(env.home()) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// The highest-precedence entry that actually carries a value wins. Being consulted is not the
/// same condition as having an opinion, and an unobservable scope can never be said to win: we
/// do not know what is there, and claiming otherwise would be intent dressed as observation.
fn settle(chain: &mut [Link]) {
    if let Some(link) = chain.iter_mut().find(|l| l.observable && l.value.is_some()) {
        link.wins = true;
    }
}

fn winning_value(chain: &[Link]) -> Option<String> {
    chain.iter().find(|l| l.wins).and_then(|l| l.value.clone())
}

// -------------------------------------------------------------------------------------------
// Writing
// -------------------------------------------------------------------------------------------

use crate::json::Error as JsonError;
use crate::profile::{Provider, ToolAssignment};
use crate::transaction::Action;

/// Which environment variable each owned slot writes to, and whether it carries a display
/// companion. The endpoint is not a slot: the glossary reserves that word for where a model
/// goes, and Claude Code chooses its provider once for the whole tool.
const OWNED: &[(&str, &str, bool)] = &[
    ("main", "ANTHROPIC_MODEL", false),
    ("utility", "ANTHROPIC_DEFAULT_HAIKU_MODEL", false),
    ("subagent", "CLAUDE_CODE_SUBAGENT_MODEL", false),
    ("opus", "ANTHROPIC_DEFAULT_OPUS_MODEL", true),
    ("sonnet", "ANTHROPIC_DEFAULT_SONNET_MODEL", true),
    ("fable", "ANTHROPIC_DEFAULT_FABLE_MODEL", true),
];

/// The variable that still wins the background path though it lost its name. Mirrored, never
/// created: an unconditional write would leave tapkey's fingerprints where nobody asked.
const DEPRECATED_UTILITY: &str = "ANTHROPIC_SMALL_FAST_MODEL";

/// Every place in a settings file tapkey may write, so a caller can cut them out and compare
/// everything else byte for byte. This is what makes merge-never-own a machine check rather
/// than a reviewer noticing a moved key in a diff.
pub fn owned_paths() -> Vec<Vec<&'static str>> {
    let mut out = vec![vec!["env", ENDPOINT_VAR], vec!["env", DEPRECATED_UTILITY]];
    for (_, var, has_companion) in OWNED {
        out.push(vec!["env", var]);
        if *has_companion {
            // Leaked deliberately and once per variable: the set is fixed at compile time and
            // a caller wants plain `&'static str` to compare against.
            out.push(vec![
                "env",
                Box::leak(format!("{var}_NAME").into_boxed_str()),
            ]);
        }
    }
    out
}

/// What tapkey has just written, ready to be recorded so drift has something to compare to.
pub fn fingerprint(assignment: &ToolAssignment) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for (slot, _, _) in OWNED {
        if let Some(model) = assignment.slots.get(*slot).and_then(|a| a.model()) {
            out.insert((*slot).to_string(), crate::fingerprint::hash(model));
        }
    }
    out
}

/// Turn one tool's assignment into the single write that applies it.
///
/// Nothing is written here; the transaction owns that, so the all-or-nothing guarantee lives
/// in one place.
pub fn plan_switch(
    env: &Env,
    assignment: &ToolAssignment,
    provider: Option<&Provider>,
) -> Result<Vec<Action>, JsonError> {
    let path = env.home().join(".claude").join("settings.json");
    let existing = std::fs::read(&path).unwrap_or_else(|_| b"{}".to_vec());
    let mut doc = Document::parse(&existing)?;

    // The endpoint comes from the provider record. An assignment naming no provider is the System
    // default shape: remove what we wrote, and neutralise a shell export if there is one.
    match provider {
        Some(p) => doc.set_string(&["env", ENDPOINT_VAR], &p.base_url)?,
        None => clear(&mut doc, env, ENDPOINT_VAR)?,
    }

    for (slot, var, has_companion) in OWNED {
        match assignment.slots.get(*slot).and_then(|a| a.model()) {
            Some(model) => {
                doc.set_string(&["env", var], model)?;
                if *has_companion {
                    // Without it the tool's own picker keeps announcing Opus 5 over somebody
                    // else's model, which is making another interface lie.
                    doc.set_string(&["env", &format!("{var}_NAME")], model)?;
                }
                if *slot == "utility" && doc.get_string(&["env", DEPRECATED_UTILITY]).is_some() {
                    doc.set_string(&["env", DEPRECATED_UTILITY], model)?;
                }
            }
            None => {
                clear(&mut doc, env, var)?;
                if *has_companion {
                    clear(&mut doc, env, &format!("{var}_NAME"))?;
                }
            }
        }
    }

    Ok(vec![Action::Write {
        path,
        bytes: doc.to_bytes(),
        // A file that already exists keeps its mode; this is only for one tapkey creates.
        mode: 0o600,
    }])
}

/// Remove what tapkey wrote, and neutralise an inherited export if there is one.
///
/// A settings file can set a variable and cannot unset one, so an export is overridden with an
/// empty value — measured to work, and only when an export exists: writing `""` unconditionally
/// would leave tapkey's fingerprints in a file it claims to have cleaned.
fn clear(doc: &mut Document, env: &Env, var: &str) -> Result<(), JsonError> {
    doc.remove(&["env", var])?;
    if env.shell_var(var).is_some() {
        doc.set_string(&["env", var], "")?;
    }
    Ok(())
}

/// The adapter, as the core sees it. The free functions above stay: the golden harness reaches for
/// `owned_paths` directly, and a trait method nobody but a test calls is interface nobody needs.
pub struct Claude;

impl super::Adapter for Claude {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn config_path(&self, env: &Env) -> PathBuf {
        env.home().join(".claude").join("settings.json")
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
        // Claude Code has nothing to report alongside a successful switch yet, so the attention
        // list is always empty here. It is in the signature because Codex has one and the core
        // must not care which is which.
        plan_switch(env, assignment, provider)
            .map(|actions| (actions, Vec::new()))
            .map_err(|e| format!("{e:?}"))
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
}

/// No registry, so the harvest is the one endpoint the tool is pointed at — and the **host** is the
/// only name the tool itself gives it. The user-level file only: a project's settings belong to a
/// repository and often to somebody else, and harvesting them would adopt a repo's configuration as
/// the person's own.
fn known_providers(env: &Env) -> Vec<super::KnownProvider> {
    let Some(bytes) = std::fs::read(env.home().join(".claude").join("settings.json")).ok() else {
        return Vec::new();
    };
    let Ok(document) = Document::parse(&bytes) else {
        return Vec::new();
    };
    let Some(url) = document.get_string(&["env", ENDPOINT_VAR]) else {
        return Vec::new();
    };
    // The id is the host, verbatim apart from the characters an id cannot carry. The token in the
    // same block is plaintext if present — it is what the file holds, and the value is not taken
    // here.
    let host = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("claude")
        .to_string();
    let credential = match (
        document.get_string(&["env", "ANTHROPIC_AUTH_TOKEN"]),
        document.get_string(&["env", "ANTHROPIC_API_KEY"]),
    ) {
        (Some(_), _) | (_, Some(_)) => super::CredentialSource::Inline,
        (None, None) => super::CredentialSource::Absent,
    };
    vec![super::KnownProvider {
        id: host,
        base_url: url.to_string(),
        credential,
    }]
}
