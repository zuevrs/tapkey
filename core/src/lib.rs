//! The tapkey engine.
//!
//! Deliberately empty. This crate exists from the first commit so that CI compiles it —
//! including for Linux — rather than asserting portability it never checks. The engine
//! itself is built one decision at a time; nothing is added here ahead of the decision
//! that shapes it.
//!
//! The shape it grows into is fixed: one function, a JSON request in and a JSON response
//! out, behind a versioned schema, with three consumers sharing it.

pub mod adapters;
pub mod atomic;
pub mod env;
pub mod fingerprint;
pub mod fs;
pub mod helper;
pub mod instant;
pub mod json;
pub mod lock;
pub mod probe;
pub mod profile;
pub mod store;
pub mod toml;
pub mod transaction;
pub mod wire;

use env::{CredentialState, Env};
use wire::{Envelope, Failure, Request, Response};

/// The schema version carried by every request and response.
pub const SCHEMA_VERSION: u32 = 1;

/// The one entry point: a JSON request in, a JSON response out.
pub fn handle(request: &str) -> String {
    handle_with(&Env::real(), request)
}

/// The same call, told what world it is acting in rather than going and finding one.
///
/// This is the seam the tests and the fixture harness use. See ADR-0016: the public surface is
/// still one string in and one string out.
pub fn handle_with(env: &Env, request: &str) -> String {
    let response = dispatch(env, request);
    serde_json::to_string(&response).unwrap_or_else(|e| {
        // Serialising our own response cannot fail on well-formed data, but a panic here would
        // take the host application down, so it degrades to a refusal instead.
        format!(
            r#"{{"ok":false,"failure":{{"kind":"internal","detail":{:?}}}}}"#,
            e.to_string()
        )
    })
}

fn dispatch(env: &Env, request: &str) -> Response {
    let envelope: Envelope = match serde_json::from_str(request) {
        Ok(e) => e,
        Err(e) => return refuse("malformed_request", e.to_string()),
    };
    if envelope.version != SCHEMA_VERSION {
        return refuse(
            "unknown_version",
            format!("this core speaks version {SCHEMA_VERSION}"),
        );
    }
    match envelope.request {
        Request::EffectiveState {} => match adapters::effective_state(env) {
            Ok(tools) => Response::Ok {
                ok: true,
                outcome: None,
                tools,
            },
            Err(e) => refuse("unparsable", e),
        },
        Request::Switch { profile_id } => switch(env, &profile_id),
        Request::Test { provider_id } => test(env, &provider_id),
        Request::Restore { target } => restore(env, target),
    }
}

/// Apply a profile to every tool it covers, all or nothing.
fn switch(env: &Env, profile_id: &str) -> Response {
    let profiles = match profile::Profiles::read(&env.store().join("profiles.json")) {
        Ok(p) => p,
        Err(e) => return refuse("unknown_profile", e),
    };
    let Some(profile) = profiles.find(profile_id) else {
        return refuse(
            "unknown_profile",
            format!("no profile named {profile_id:?}"),
        );
    };
    if profile.tools.is_empty() {
        return refuse(
            "unknown_profile",
            "the profile covers no managed tool".into(),
        );
    }

    // A profile naming a provider the file does not hold is a broken instruction, not a condition
    // of the machine, and it is caught before anything is staged.
    let mut assignments = Vec::new();
    for (tool, assignment) in &profile.tools {
        let provider = match &assignment.provider {
            Some(id) => match profiles.provider(id) {
                Some(p) => Some(p),
                None => {
                    return refuse("unknown_provider", format!("no provider named {id:?}"));
                }
            },
            None => None,
        };
        // Probed for **presence** before anything is staged, and never for value. A config pointing
        // at a credential that is not there is a silent breakage — measured, the tool says nothing
        // and the endpoint answers 401, which the person reads as a fault of their provider — and
        // this is where refusing is cheaper than explaining. Absence and denial are distinguished,
        // because they lead to different sentences: *add a key* said to somebody whose key exists
        // and was withheld is the wrong sentence.
        if let Some(provider) = provider {
            match env.credentials().check(&provider.id) {
                CredentialState::Found => {}
                CredentialState::Absent => {
                    return refuse(
                        "credential_unavailable",
                        format!("no key is stored for {provider:?}"),
                    );
                }
                CredentialState::Denied => {
                    return refuse(
                        "keychain_denied",
                        "the keychain refused access — grant it and try again".into(),
                    );
                }
            }
        }
        assignments.push((tool.as_str(), assignment, provider));
    }

    // Planned before anything is staged, so a file we cannot parse is a refusal rather than a
    // rollback: nothing was touched. A tool that cannot participate contributes an attention and
    // no actions — it is left out of the transaction rather than allowed to cancel it, the same
    // reasoning that makes a gone tool a skip.
    let mut actions = Vec::new();
    let mut tool_of: std::collections::BTreeMap<std::path::PathBuf, &'static str> =
        std::collections::BTreeMap::new();
    let mut attentions: std::collections::BTreeMap<&str, Vec<wire::Attention>> =
        std::collections::BTreeMap::new();
    let adapters = adapters::all();
    for (tool, assignment, provider) in &assignments {
        // A profile naming a tool this core does not manage is not a reason to refuse the tools it
        // does; the row simply has nowhere to land.
        let Some(adapter) = adapters.iter().find(|a| a.name() == *tool) else {
            continue;
        };
        // A slot naming its own provider is an instruction only OpenCode can carry out. The check
        // lives here rather than in each adapter, because it is one rule and two implementations of
        // one rule eventually disagree; the *fact* it reads — can this tool do it — is the tool's.
        if !adapter.per_slot_providers() {
            for (slot, assigned) in &assignment.slots {
                if assigned.provider().is_some() {
                    attentions
                        .entry(adapter.name())
                        .or_default()
                        .push(wire::Attention {
                            kind: "slot_provider_ignored",
                            file: None,
                            key: Some(slot.clone()),
                        });
                }
            }
        }

        match adapter.plan_switch(env, assignment, *provider) {
            Ok((planned, attn)) => {
                for action in &planned {
                    tool_of.insert(action.path().clone(), adapter.name());
                }
                actions.extend(planned);
                if !attn.is_empty() {
                    attentions.insert(adapter.name(), attn);
                }
            }
            Err(e) => return refuse("unparsable", e),
        }
    }

    let store = match store::Store::open(env.store()) {
        Ok(s) => s,
        Err(e) => return refuse("permission_denied", e.to_string()),
    };
    // Held until this call returns. A read never takes it: blocking `effective_state` for the
    // duration of a write would blank the UI at the one moment it is most interesting.
    let _lock = match lock::Lock::acquire(env.store()) {
        Ok(l) => l,
        Err(lock::Busy(why)) => return refuse("busy", why),
    };
    let transaction = transaction::Transaction::new(actions);
    let mut disk = env.filesystem();
    let captured = match transaction.capture(&**disk, &tool_of) {
        Ok(c) => c,
        Err(e) => return refuse("permission_denied", e.to_string()),
    };

    // The floor under every restore goes down before the first change, over **every managed file**
    // rather than only the touched ones: full restoration is its job. With one tool the two sets
    // were the same and this read as an ordinary capture; with two they part, and a snapshot of
    // only the touched file would leave the other tool nothing to come back to.
    let snapshot_actions: Vec<transaction::Action> = adapters::managed_files(env)
        .keys()
        .map(|path| transaction::Action::Delete { path: path.clone() })
        .collect();
    let everything = transaction::Transaction::new(snapshot_actions);
    let all_managed = match everything.capture(&**disk, &adapters::managed_files(env)) {
        Ok(c) => c,
        Err(e) => return refuse("permission_denied", e.to_string()),
    };
    if !store.has_snapshot() && store.take_snapshot(&all_managed, env.now()).is_err() {
        return refuse(
            "permission_denied",
            "could not record the first-run snapshot".into(),
        );
    }
    if store
        .take_backup(&captured, &profile.name, env.now())
        .is_err()
    {
        return refuse("permission_denied", "could not record a backup".into());
    }

    if let Err(rolled_back) = transaction.apply(&mut **disk) {
        return Response::Failed {
            ok: false,
            outcome: "rolled back",
            failure: Failure {
                kind: "write_failed",
                detail: format!(
                    "{}: {} — {} file(s) restored",
                    rolled_back.failed_at.display(),
                    rolled_back.reason,
                    rolled_back.restored
                ),
            },
        };
    }

    // The read-back below goes to the real filesystem, so the borrow ends here.
    drop(disk);

    // Drift needs something to compare against, and it is written only after the change stuck.
    let mut owned = std::collections::BTreeMap::new();
    for (tool, assignment, _) in &assignments {
        let Some(adapter) = adapters.iter().find(|a| a.name() == *tool) else {
            continue;
        };
        owned.insert((*tool).to_string(), adapter.fingerprint(assignment));
    }
    let _ = fingerprint::State::write(&env.store().join("state.json"), &profile.name, owned);

    // Read back rather than reporting what was written: the invariant forbids reporting intent,
    // and reading back is the only way a project config or a shell export that beat us shows up.
    let mut tools = match adapters::effective_state(env) {
        Ok(tools) => tools,
        Err(e) => return refuse("unparsable", e),
    };
    for tool in tools.iter_mut() {
        if let Some(attn) = attentions.remove(tool.tool) {
            tool.attentions = attn;
        }
    }
    Response::Ok {
        ok: true,
        outcome: Some("applied"),
        tools,
    }
}

/// Go back to a stored state, transactionally and taking a backup of its own first.
fn restore(env: &Env, target: wire::RestoreTarget) -> Response {
    let store = match store::Store::open(env.store()) {
        Ok(s) => s,
        Err(e) => return refuse("permission_denied", e.to_string()),
    };
    let _lock = match lock::Lock::acquire(env.store()) {
        Ok(l) => l,
        Err(lock::Busy(why)) => return refuse("busy", why),
    };
    let which = match &target {
        wire::RestoreTarget::Snapshot => store::Target::Snapshot,
        wire::RestoreTarget::Backup { id } => store::Target::Backup(id.as_str().into()),
    };
    let plan = match store.restore_plan(which) {
        Ok(p) => p,
        // Kept and marked rather than deleted, so the refusal names it rather than hiding it.
        Err(e) => return refuse("backup_unreadable", e.to_string()),
    };

    let actions = plan
        .into_iter()
        .map(|action| match action {
            store::RestoreAction::Write { path, bytes, mode } => transaction::Action::Write {
                path,
                bytes,
                mode: mode.unwrap_or(0o600),
            },
            store::RestoreAction::Delete { path } => transaction::Action::Delete { path },
        })
        .collect();

    let transaction = transaction::Transaction::new(actions);
    let mut disk = env.filesystem();
    // A restore touches whatever the snapshot held, so which tool each file belongs to is read
    // back from the paths the adapters own rather than assumed.
    let tool_of = adapters::managed_files(env);
    let captured = match transaction.capture(&**disk, &tool_of) {
        Ok(c) => c,
        Err(e) => return refuse("permission_denied", e.to_string()),
    };
    // Restoring changes the user's files like any other change, so it is backed up like one —
    // otherwise the single action with no way back is the way back itself.
    if store.take_backup(&captured, "restore", env.now()).is_err() {
        return refuse("permission_denied", "could not record a backup".into());
    }
    if let Err(rolled_back) = transaction.apply(&mut **disk) {
        return Response::Failed {
            ok: false,
            outcome: "rolled back",
            failure: Failure {
                kind: "write_failed",
                detail: format!(
                    "{}: {}",
                    rolled_back.failed_at.display(),
                    rolled_back.reason
                ),
            },
        };
    }
    drop(disk);

    // Going back to the snapshot hands the tool to its own login, so tapkey owns nothing after
    // it — or drift would fire on values that are no longer ours.
    if matches!(target, wire::RestoreTarget::Snapshot) {
        let _ = fingerprint::State::write(
            &env.store().join("state.json"),
            "",
            std::collections::BTreeMap::new(),
        );
    }

    match adapters::effective_state(env) {
        Ok(tools) => Response::Ok {
            ok: true,
            outcome: Some("applied"),
            tools,
        },
        Err(e) => refuse("unparsable", e),
    }
}

fn refuse(kind: &'static str, detail: String) -> Response {
    Response::Refused {
        ok: false,
        failure: Failure { kind, detail },
    }
}

/// Establish which formats a provider answers, and record it.
///
/// A write to the store, so it takes the lock — ticket 29's rule is *any write to the store*, not
/// a list of operations, which is how the next writing operation would otherwise end up outside
/// it. The lock is taken **after** the probes, because a network round-trip while holding the
/// lock would block a switch behind somebody's slow endpoint for no reason: the probes change
/// nothing, and only the write needs mutual exclusion.
fn test(env: &Env, provider_id: &str) -> Response {
    let store_path = env.store().join("profiles.json");
    let mut profiles = match profile::Profiles::read(&store_path) {
        Ok(p) => p,
        Err(e) => return refuse("unknown_provider", e),
    };
    let Some(record) = profiles.providers.iter_mut().find(|p| p.id == provider_id) else {
        return refuse(
            "unknown_provider",
            format!("no provider named {provider_id:?}"),
        );
    };

    let verdict = probe::run(record, env);
    let tested_at = instant::format_utc(env.now());

    let (knowable, formats, because) = match &verdict {
        probe::Verdict::Served(served) => (
            true,
            Some(
                served
                    .iter()
                    .map(|(format, served)| wire::FormatProbe {
                        format,
                        served: *served,
                    })
                    .collect::<Vec<_>>(),
            ),
            None,
        ),
        probe::Verdict::CannotTell(why) => (false, None, Some(*why)),
    };

    // Held only across the write.
    let _lock = match lock::Lock::acquire(env.store()) {
        Ok(l) => l,
        Err(lock::Busy(why)) => return refuse("busy", why),
    };
    if let probe::Verdict::Served(served) = &verdict {
        let answered: Vec<String> = served
            .iter()
            .filter(|(_, served)| *served)
            .map(|(format, _)| (*format).to_string())
            .collect();
        record.formats = Some(answered);
    }
    record.tested_at = Some(tested_at.clone());
    let bytes = serde_json::to_vec_pretty(&profiles).unwrap_or_else(|_| Vec::new());
    if let Err(e) = atomic::write_atomically(&store_path, &bytes, 0o600) {
        return refuse("permission_denied", e.to_string());
    }

    Response::Tested {
        ok: true,
        knowable,
        formats,
        because,
        provider: provider_id.to_string(),
        tested_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The public wrapper is what three consumers actually call, so it needs a test of its
    /// own — and one that refuses before touching the filesystem, or it would read whatever
    /// this machine happens to have in it.
    #[test]
    fn the_public_entry_point_answers_with_json_even_when_the_request_is_nonsense() {
        let out = handle("not json at all");
        let parsed: serde_json::Value = serde_json::from_str(&out).expect("a JSON response");
        assert_eq!(parsed["ok"], serde_json::json!(false));
        assert_eq!(
            parsed["failure"]["kind"],
            serde_json::json!("malformed_request")
        );
    }

    #[test]
    fn an_unknown_schema_version_is_refused_rather_than_guessed_at() {
        let env = Env::for_test("/nonexistent".into(), "/nonexistent".into());
        let out = handle_with(
            &env,
            r#"{"version": 99, "op": "effective_state", "params": {}}"#,
        );
        assert!(out.contains("unknown_version"), "{out}");
    }
}
