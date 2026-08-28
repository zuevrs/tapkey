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
pub mod instant;
pub mod json;
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
                tools: vec![tool],
            },
            Err(e) => refuse("unparsable", format!("{e:?}")),
        },
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
