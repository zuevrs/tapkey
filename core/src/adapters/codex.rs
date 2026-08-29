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
use crate::toml::{Document, Error};
use crate::wire::{Link, Resolved, SlotState, ToolState};
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
    SlotSpec {
        name: "utility_consolidation",
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

    let endpoint = endpoint(&files);
    let slots = SLOTS
        .iter()
        .map(|spec| SlotState {
            slot: spec.name,
            owned: spec.owned,
            drifted: false,
            resolved: resolve(&files, spec.path),
        })
        .collect();

    Ok(ToolState {
        tool: "codex",
        endpoint,
        slots,
    })
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
            source: layer.path.display().to_string(),
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
            source: layer.path.display().to_string(),
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
