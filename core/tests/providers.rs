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

/// OpenCode names a provider per slot — measured, three slots resolved to three different
/// providers in one file — so a slot assignment may carry its own.
///
/// Claude Code and Codex have one endpoint behind every slot **physically**, so an assignment that
/// names a provider for one slot is an instruction they cannot carry out. Effective state reports
/// what the tool will use, not what was asked, so the slot shows the tool's endpoint and an
/// attention says the per-slot provider was not honoured. Reporting the asked-for endpoint would be
/// reporting intent, which is the one thing the invariant forbids.
#[test]
fn a_per_slot_provider_a_tool_cannot_honour_is_reported_not_obeyed() {
    let machine = Machine::new("prov-per-slot-claude");
    machine.write_profiles(json!({
        "providers": [
            {"id": "main", "name": "Main", "base_url": "https://main.test/v1",
             "formats": ["anthropic_messages"], "enabled": true},
            {"id": "cheap", "name": "Cheap", "base_url": "https://cheap.test/v1",
             "formats": ["anthropic_messages"], "enabled": true}
        ],
        "profiles": [{
            "id": "mixed",
            "name": "Mixed",
            "tools": {"claude": {
                "provider": "main",
                "slots": {
                    "main": "big-model",
                    "utility": {"provider": "cheap", "model": "small-model"}
                }
            }}
        }]
    }));
    machine.write_user_settings_raw(b"{}");

    let response = call(&machine, switch("mixed"));

    assert_eq!(response["outcome"], json!("applied"), "{response}");
    let claude = response["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .find(|t| t["tool"] == json!("claude"))
        .cloned()
        .expect("claude");

    // The model still lands: only the provider part was impossible.
    let utility = claude["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .find(|s| s["slot"] == json!("utility"))
        .cloned()
        .expect("a utility slot");
    assert_eq!(utility["effective"], json!("small-model"), "{utility}");
    assert_eq!(
        claude["endpoint"]["effective"],
        json!("https://main.test/v1"),
        "one endpoint serves every slot here, and it is the tool's: {claude}"
    );

    let attention = claude["attentions"]
        .as_array()
        .and_then(|a| a.first().cloned())
        .unwrap_or_else(|| panic!("the unhonoured provider must be said out loud: {claude}"));
    assert_eq!(attention["kind"], json!("slot_provider_ignored"));
    assert_eq!(attention["key"], json!("utility"));
}
