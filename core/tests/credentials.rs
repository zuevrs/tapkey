//! The credential seam.
//!
//! ADR-0016 records two rules that make this a seam rather than a direct call: a test must never
//! raise an access dialog, and the Linux runner has no Keychain at all. So the core asks an
//! interface, whose default implementation spawns the helper binary and whose test implementation
//! goes near neither a Keychain nor a process.

use serde_json::{Value, json};

mod support;
use support::{Machine, call};

fn switch(profile: &str) -> Value {
    json!({"version": 1, "op": "switch", "params": {"profile_id": profile}})
}

fn profiles() -> Value {
    json!({
        "providers": [{
            "id": "zai", "name": "Z.ai", "base_url": "https://api.z.ai/api/anthropic",
            "formats": ["anthropic_messages"], "enabled": true
        }],
        "profiles": [{
            "id": "glm", "name": "Z.ai GLM",
            "tools": {"claude": {"provider": "zai", "slots": {"main": "glm-5.3"}}}
        }]
    })
}

/// A configuration pointing at a credential that is not there is a silent breakage: measured on
/// Codex, the tool says nothing about credentials and the endpoint answers `401`, which the person
/// reads as a fault of their provider. Cheaper not to create that than to explain it afterwards —
/// so a switch probes for **presence** first, and refuses having touched nothing.
#[test]
fn a_switch_refuses_when_the_credential_is_absent() {
    let machine = Machine::new("cred-absent").with_credentials(&[]);
    machine.write_profiles(profiles());
    machine.write_user_settings_raw(b"{\n  \"theme\": \"dark\"\n}\n");

    let response = call(&machine, switch("glm"));

    assert_eq!(response["ok"], json!(false), "{response}");
    assert_eq!(
        response["failure"]["kind"],
        json!("credential_unavailable"),
        "{response}"
    );
    assert_eq!(
        std::fs::read_to_string(machine.home().join(".claude").join("settings.json"))
            .expect("read"),
        "{\n  \"theme\": \"dark\"\n}\n",
        "a refusal must leave the file exactly as it was"
    );
}

/// Presence, never value. The seam is asked whether a credential exists and is never asked what it
/// is: a credential tapkey does not need is a credential tapkey does not hold.
#[test]
fn a_switch_proceeds_when_the_credential_is_present() {
    let machine = Machine::new("cred-present").with_credentials(&["zai"]);
    machine.write_profiles(profiles());
    machine.write_user_settings_raw(b"{}");

    let response = call(&machine, switch("glm"));

    assert_eq!(response["outcome"], json!("applied"), "{response}");
}

/// Denial is not absence, and the core's outcomes differ: an absent item says *add a key*, a denied
/// one says *access was refused*. Saying the first to somebody whose key exists and was withheld is
/// the wrong sentence, which is why the helper answers with three exit codes rather than two.
#[test]
fn a_denied_keychain_is_reported_as_denial_rather_than_absence() {
    let machine = Machine::new("cred-denied").denying_credentials();
    machine.write_profiles(profiles());
    machine.write_user_settings_raw(b"{}");

    let response = call(&machine, switch("glm"));

    assert_eq!(response["ok"], json!(false), "{response}");
    assert_eq!(
        response["failure"]["kind"],
        json!("keychain_denied"),
        "{response}"
    );
}
