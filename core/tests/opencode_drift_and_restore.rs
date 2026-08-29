//! Drift and restore for OpenCode.
//!
//! Measured: `/models` writes to the tool's own state rather than to config, so OpenCode is the
//! only one of the three that never contests tapkey's two headline slots — drift here should come
//! from a person editing the file, not from the tool doing its job.

use serde_json::{Value, json};

mod support;
use support::{Machine, call};

fn switch(profile: &str) -> Value {
    json!({"version": 1, "op": "switch", "params": {"profile_id": profile}})
}

fn effective_state() -> Value {
    json!({"version": 1, "op": "effective_state", "params": {}})
}

fn profiles() -> Value {
    json!({
        "providers": [{
            "id": "zai", "name": "Z.ai", "base_url": "https://api.z.ai/api/v1",
            "formats": ["openai_chat"], "enabled": true
        }],
        "profiles": [{
            "id": "glm", "name": "Z.ai GLM",
            "tools": {"opencode": {"provider": "zai", "slots": {"main": "glm-5.3"}}}
        }]
    })
}

fn config_path(machine: &Machine) -> std::path::PathBuf {
    machine
        .home()
        .join(".config")
        .join("opencode")
        .join("opencode.jsonc")
}

fn slot(response: &Value, name: &str) -> Value {
    response["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .find(|t| t["tool"] == json!("opencode"))
        .and_then(|t| t["slots"].as_array().cloned())
        .expect("slots")
        .into_iter()
        .find(|s| s["slot"] == json!(name))
        .unwrap_or_else(|| panic!("no {name} slot"))
}

#[test]
fn a_slot_edited_after_the_switch_reads_as_drifted() {
    let machine = Machine::new("oc-drift");
    machine.write_profiles(profiles());
    machine.write_opencode_config("opencode.jsonc", b"{}\n");
    call(&machine, switch("glm"));

    let after = std::fs::read_to_string(config_path(&machine)).expect("read");
    std::fs::write(
        config_path(&machine),
        after.replace("tapkey-zai/glm-5.3", "somebody/else"),
    )
    .expect("write");

    let main = slot(&call(&machine, effective_state()), "main");
    assert_eq!(main["effective"], json!("somebody/else"));
    assert_eq!(main["drifted"], json!(true), "{main}");
}

/// A slot moved to another provider is a change even when the model name is unchanged, because the
/// file holds the namespaced pair and that pair is what a comparison reads back.
#[test]
fn moving_a_slot_to_another_provider_is_drift_too() {
    let machine = Machine::new("oc-drift-provider");
    machine.write_profiles(profiles());
    machine.write_opencode_config("opencode.jsonc", b"{}\n");
    call(&machine, switch("glm"));

    let after = std::fs::read_to_string(config_path(&machine)).expect("read");
    std::fs::write(
        config_path(&machine),
        after.replace("tapkey-zai/glm-5.3", "someone-else/glm-5.3"),
    )
    .expect("write");

    let main = slot(&call(&machine, effective_state()), "main");
    assert_eq!(main["drifted"], json!(true), "{main}");
}

#[test]
fn a_value_tapkey_never_wrote_is_not_drift() {
    let machine = Machine::new("oc-firstsight");
    machine.write_profiles(profiles());
    machine.write_opencode_config("opencode.jsonc", b"{\n  \"model\": \"theirs/choice\"\n}\n");

    let main = slot(&call(&machine, effective_state()), "main");

    assert_eq!(main["effective"], json!("theirs/choice"));
    assert_eq!(main["drifted"], json!(false), "{main}");
}

#[test]
fn restoring_the_snapshot_returns_the_original_bytes() {
    let machine = Machine::new("oc-restore");
    machine.write_profiles(profiles());
    let original = b"{\n  // hand written\n  \"theme\": \"dark\",\n}\n";
    machine.write_opencode_config("opencode.jsonc", original);
    call(&machine, switch("glm"));
    assert_ne!(
        std::fs::read(config_path(&machine)).expect("read"),
        original,
        "the switch must have changed something, or this test proves nothing"
    );

    let response = call(
        &machine,
        json!({"version": 1, "op": "restore", "params": {"target": "snapshot"}}),
    );

    assert_eq!(response["outcome"], json!("applied"), "{response}");
    assert_eq!(
        std::fs::read(config_path(&machine)).expect("read"),
        original
    );
}

/// Nothing drifts the moment after a switch. Obvious, and it was the missing case: the tests above
/// all assert drift is *present*, so a fingerprint taken over the wrong value satisfied them just
/// as well — a defect injection dropping the provider from the hash failed none of them, while
/// leaving every switch immediately reporting drift against itself.
#[test]
fn a_switch_leaves_nothing_drifted_behind_it() {
    let machine = Machine::new("oc-settled");
    machine.write_profiles(profiles());
    machine.write_opencode_config("opencode.jsonc", b"{}\n");

    call(&machine, switch("glm"));

    let main = slot(&call(&machine, effective_state()), "main");
    assert_eq!(main["effective"], json!("tapkey-zai/glm-5.3"), "{main}");
    assert_eq!(
        main["drifted"],
        json!(false),
        "a switch that reports drift against its own write is telling on itself: {main}"
    );
}
