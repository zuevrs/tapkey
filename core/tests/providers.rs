//! A provider is an entity with an identifier, not a URL a profile carries.
//!
//! Three separate things needed it to become one, and only the third is about Claude Code: Codex
//! names a provider in `model_provider` and in a table name, so an identifier is unavoidable there;
//! ADR-0013 keeps every tool's registry filled with **every enabled provider**, which a profile
//! naming one endpoint cannot describe; and the set of API formats an endpoint answers has to hang
//! somewhere. See `issues/14-a-provider-is-not-universal.md`.

use serde_json::{Value, json};

mod support;
use support::{Machine, call};

fn switch(profile: &str) -> Value {
    json!({"version": 1, "op": "switch", "params": {"profile_id": profile}})
}

fn user_settings(machine: &Machine) -> String {
    std::fs::read_to_string(machine.home().join(".claude").join("settings.json")).expect("read")
}

#[test]
fn a_profile_names_a_provider_and_the_endpoint_comes_from_the_record() {
    let machine = Machine::new("prov-basic");
    machine.write_profiles(json!({
        "providers": [{
            "id": "zai",
            "name": "Z.ai",
            "base_url": "https://api.z.ai/api/anthropic",
            "formats": ["anthropic_messages"],
            "enabled": true
        }],
        "profiles": [{
            "id": "glm",
            "name": "Z.ai GLM",
            "tools": {"claude": {
                "provider": "zai",
                "slots": {"main": "glm-5.3"}
            }}
        }]
    }));
    machine.write_user_settings_raw(b"{\n  \"theme\": \"dark\"\n}\n");

    let response = call(&machine, switch("glm"));

    assert_eq!(response["outcome"], json!("applied"), "{response}");
    let after = user_settings(&machine);
    assert!(
        after.contains(r#""ANTHROPIC_BASE_URL": "https://api.z.ai/api/anthropic""#),
        "the endpoint did not come from the provider record: {after}"
    );
}

/// A profile pointing at a provider that is not in the file is a broken instruction, and nothing
/// is touched. It is a refusal rather than a rollback for the reason ticket 04 gives: the failure
/// is found before anything is staged.
#[test]
fn a_profile_naming_a_provider_that_is_not_there_is_refused() {
    let machine = Machine::new("prov-missing");
    machine.write_profiles(json!({
        "providers": [],
        "profiles": [{
            "id": "glm",
            "name": "Z.ai GLM",
            "tools": {"claude": {"provider": "zai", "slots": {"main": "glm-5.3"}}}
        }]
    }));
    machine.write_user_settings_raw(b"{\n  \"theme\": \"dark\"\n}\n");

    let response = call(&machine, switch("glm"));

    assert_eq!(response["ok"], json!(false), "{response}");
    assert_eq!(
        response["failure"]["kind"],
        json!("unknown_provider"),
        "{response}"
    );
    assert_eq!(
        user_settings(&machine),
        "{\n  \"theme\": \"dark\"\n}\n",
        "a refusal must leave the file exactly as it was"
    );
}
