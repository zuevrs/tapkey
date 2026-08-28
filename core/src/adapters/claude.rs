//! Claude Code.
//!
//! A profile reaches this tool through an `env` block in `~/.claude/settings.json` and nowhere
//! else: there is no settings key for the base URL, and the model slots tapkey owns exist only
//! as environment variables. Reading it back means resolving the scopes it consults, in the
//! order it consults them.

use crate::env::{Env, ShellVar};
use crate::json::Document;
use crate::wire::{Link, Resolved, SlotState, ToolState};
use std::path::{Path, PathBuf};

pub const ENDPOINT_VAR: &str = "ANTHROPIC_BASE_URL";

/// Every place Claude Code takes a value from, highest precedence first.
///
/// Environment variables are not a level in this stack. A settings-file `env` block replaces
/// what the shell exported — measured, and true for credentials as well as models — so the
/// shell sits below every file rather than beside them. The order is a list rather than
/// control flow, because control flow is how a scope ends up in the wrong place when the one
/// above it happens to be absent.
const PRECEDENCE: [Scope; 6] = [
    Scope::File { name: "managed" },
    Scope::Unobservable {
        name: "command line",
        why: "--settings or a per-key flag",
    },
    Scope::File {
        name: "project local",
    },
    Scope::File { name: "project" },
    Scope::File { name: "user" },
    Scope::Shell,
    // Cloud sessions read only shared project settings and server-managed settings; a user
    // file never reaches them. Saying nothing here would read as "switched everywhere".
];

/// Named separately because it belongs below the shell and the array above is fixed-length.
const CLOUD: Scope = Scope::Unobservable {
    name: "cloud session",
    why: "reads neither user settings nor the shell",
};

#[derive(Clone, Copy)]
enum Scope {
    File {
        name: &'static str,
    },
    Unobservable {
        name: &'static str,
        why: &'static str,
    },
    Shell,
}

/// Read what Claude Code will actually use, resolved across the scopes it consults.
pub fn effective_state(env: &Env) -> Result<ToolState, crate::json::Error> {
    let chain = resolve_env_var(env, ENDPOINT_VAR)?;

    Ok(ToolState {
        tool: "claude",
        endpoint: Resolved {
            effective: winning_value(&chain),
            chain,
        },
        slots: Vec::<SlotState>::new(),
    })
}

/// Build the precedence chain for one environment variable.
///
/// Every scope consulted appears, including the ones that had no opinion: "this file was read
/// and said nothing" is part of the answer to *which file decided this*. A file that does not
/// exist is left out, because it was never consulted.
fn resolve_env_var(env: &Env, name: &str) -> Result<Vec<Link>, crate::json::Error> {
    let mut chain = Vec::new();
    for scope in PRECEDENCE.iter().chain(std::iter::once(&CLOUD)) {
        match *scope {
            Scope::File { name: scope_name } => {
                let Some(path) = scope_path(env, scope_name) else {
                    continue;
                };
                let Some(doc) = read_settings(&path)? else {
                    continue;
                };
                chain.push(Link {
                    source: display_path(&path, env),
                    scope: scope_name,
                    value: doc.get_string(&["env", name]).map(str::to_owned),
                    observable: true,
                    wins: false,
                });
            }
            Scope::Unobservable { name: n, why } => chain.push(unobservable(n, why)),
            Scope::Shell => chain.push(Link {
                source: "login shell".to_string(),
                scope: "shell",
                // A withheld credential and an unset variable both read as no value here. The
                // difference is a credential concern, reported as an attention rather than by
                // pretending to know a value we deliberately did not read.
                value: match env.shell_var(name) {
                    Some(ShellVar::Value(v)) => Some(v.clone()),
                    Some(ShellVar::SetButWithheld) | None => None,
                },
                observable: true,
                wins: false,
            }),
        }
    }

    settle(&mut chain);
    Ok(chain)
}

fn unobservable(scope: &'static str, source: &str) -> Link {
    Link {
        source: source.to_string(),
        scope,
        value: None,
        observable: false,
        wins: false,
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

fn read_settings(path: &Path) -> Result<Option<Document>, crate::json::Error> {
    match std::fs::read(path) {
        Ok(bytes) => Document::parse(&bytes).map(Some),
        // A file that is not there is not an error: installing Claude Code creates none.
        Err(_) => Ok(None),
    }
}

/// Paths are reported as the person would type them, so a chain reads like their filesystem.
fn display_path(path: &Path, env: &Env) -> String {
    match path.strip_prefix(env.home()) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// The highest-precedence entry that actually carries a value wins. An unobservable scope can
/// never be said to win: we do not know what is there, and claiming otherwise would be intent
/// dressed as observation.
fn settle(chain: &mut [Link]) {
    if let Some(link) = chain.iter_mut().find(|l| l.observable && l.value.is_some()) {
        link.wins = true;
    }
}

fn winning_value(chain: &[Link]) -> Option<String> {
    chain.iter().find(|l| l.wins).and_then(|l| l.value.clone())
}
