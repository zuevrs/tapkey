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
pub mod instant;
pub mod json;
pub mod lock;
pub mod profile;
pub mod store;
pub mod transaction;
pub mod wire;

use env::Env;
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
        Request::EffectiveState {} => match adapters::claude::effective_state(env) {
            Ok(tool) => Response::Ok {
                ok: true,
                outcome: None,
                tools: vec![tool],
            },
            Err(e) => refuse("unparsable", format!("{e:?}")),
        },
        Request::Switch { profile_id } => switch(env, &profile_id),
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
    let Some(assignment) = profile.tools.get("claude") else {
        return refuse(
            "unknown_profile",
            "the profile covers no managed tool".into(),
        );
    };

    // Planned before anything is staged, so a file we cannot parse is a refusal rather than a
    // rollback: nothing was touched.
    let actions = match adapters::claude::plan_switch(env, assignment) {
        Ok(a) => a,
        Err(e) => return refuse("unparsable", format!("{e:?}")),
    };

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
    let captured = match transaction.capture(&**disk, "claude") {
        Ok(c) => c,
        Err(e) => return refuse("permission_denied", e.to_string()),
    };

    // The floor under every restore goes down before the first change, over every managed
    // file rather than only the touched ones: full restoration is its job.
    if !store.has_snapshot() && store.take_snapshot(&captured, env.now()).is_err() {
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
    owned.insert(
        "claude".to_string(),
        adapters::claude::fingerprint(assignment),
    );
    let _ = fingerprint::State::write(&env.store().join("state.json"), &profile.name, owned);

    // Read back rather than reporting what was written: the invariant forbids reporting intent,
    // and reading back is the only way a project config or a shell export that beat us shows up.
    match adapters::claude::effective_state(env) {
        Ok(tool) => Response::Ok {
            ok: true,
            outcome: Some("applied"),
            tools: vec![tool],
        },
        Err(e) => refuse("unparsable", format!("{e:?}")),
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
    let captured = match transaction.capture(&**disk, "claude") {
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

    match adapters::claude::effective_state(env) {
        Ok(tool) => Response::Ok {
            ok: true,
            outcome: Some("applied"),
            tools: vec![tool],
        },
        Err(e) => refuse("unparsable", format!("{e:?}")),
    }
}

fn refuse(kind: &'static str, detail: String) -> Response {
    Response::Refused {
        ok: false,
        failure: Failure { kind, detail },
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
